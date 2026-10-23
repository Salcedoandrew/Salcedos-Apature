//! Renaming photos in place and moving them to the Trash. Both act on a
//! whole [`PhotoItem`]: every variant (RAW, JPEG) and the XMP sidecar move
//! together, so a pair never splits. Rules, like ingest: plan first, refuse
//! any collision, never overwrite, never delete (the Trash is the system's
//! and Finder › Put Back restores).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::PhotoItem;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenamePlan {
    /// (from, to) for every file that changes name.
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// Targets that cannot be used, with the reason.
    pub collisions: Vec<(PathBuf, String)>,
    /// Items whose name does not change (template gave the same stem).
    pub unchanged: usize,
}

impl RenamePlan {
    pub fn can_run(&self) -> bool {
        self.collisions.is_empty() && !self.moves.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FileOpError {
    #[error("the plan has collisions; nothing was renamed")]
    Collisions,
    #[error("new name {0:?} is not a plain file name")]
    BadName(String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("could not move to the Trash: {0}")]
    Trash(String),
}

/// The files of `item` that move with it: every variant plus the sidecar
/// when one exists.
pub fn item_files(item: &PhotoItem) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = item.variants().map(|x| x.path.clone()).collect();
    let sc = item.sidecar_path();
    if sc.is_file() {
        v.push(sc);
    }
    v
}

fn with_stem(path: &Path, stem: &str) -> PathBuf {
    match path.extension() {
        Some(ext) => path.with_file_name(format!("{stem}.{}", ext.to_string_lossy())),
        None => path.with_file_name(stem),
    }
}

/// Plan renaming each item to its new stem (no extension, no separators).
pub fn plan_rename(items: &[(&PhotoItem, String)]) -> Result<RenamePlan, FileOpError> {
    let mut plan = RenamePlan::default();
    let mut sources: HashSet<PathBuf> = HashSet::new();
    for (it, _) in items {
        for f in item_files(it) {
            sources.insert(f);
        }
    }
    let mut targets: HashSet<PathBuf> = HashSet::new();
    for (it, stem) in items {
        let stem = stem.trim();
        if stem.is_empty()
            || stem.contains('/')
            || stem.contains('\\')
            || stem.contains(':')
            || stem == "."
            || stem == ".."
        {
            return Err(FileOpError::BadName(stem.to_string()));
        }
        if stem == it.stem {
            plan.unchanged += 1;
            continue;
        }
        for from in item_files(it) {
            let to = with_stem(&from, stem);
            if to == from {
                continue;
            }
            if !targets.insert(to.clone()) {
                plan.collisions
                    .push((to.clone(), "two photos would get this name".into()));
                continue;
            }
            // Case-only renames on a case-insensitive disk report the target
            // as existing; the two-phase rename below handles those, so only
            // a target that is not also a source (by path, or the same file
            // under another case) counts as a collision.
            if to.exists() && !sources.contains(&to) && !is_one_of(&to, &sources) {
                plan.collisions
                    .push((to.clone(), "a file with this name already exists".into()));
                continue;
            }
            plan.moves.push((from, to));
        }
    }
    Ok(plan)
}

/// Is `p` the same file on disk as one of `sources` (e.g. `SHOT.jpg` and
/// `shot.jpg` on a case-insensitive disk)?
fn is_one_of(p: &Path, sources: &HashSet<PathBuf>) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let id = |q: &Path| std::fs::metadata(q).ok().map(|m| (m.dev(), m.ino()));
        id(p).is_some_and(|t| sources.iter().any(|s| id(s) == Some(t)))
    }
    #[cfg(not(unix))]
    {
        let low = p.to_string_lossy().to_lowercase();
        sources.iter().any(|s| s.to_string_lossy().to_lowercase() == low)
    }
}

/// Execute a plan. Two phases (every source to a temporary name, then every
/// temporary to its target) so swaps and case changes work; any failure rolls
/// back what was done. Returns the completed moves.
pub fn rename(plan: &RenamePlan) -> Result<Vec<(PathBuf, PathBuf)>, FileOpError> {
    if !plan.collisions.is_empty() {
        return Err(FileOpError::Collisions);
    }
    let tmp_of = |p: &Path, i: usize| -> PathBuf {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        p.with_file_name(format!(".{name}.apature-rename-{i}"))
    };
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new(); // (from, tmp)
    for (i, (from, _)) in plan.moves.iter().enumerate() {
        let tmp = tmp_of(from, i);
        if let Err(e) = std::fs::rename(from, &tmp) {
            for (f, t) in staged.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            return Err(FileOpError::Io(from.clone(), e));
        }
        staged.push((from.clone(), tmp));
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new(); // (tmp, to)
    for ((_, tmp), (_, to)) in staged.iter().zip(plan.moves.iter()) {
        if to.exists() {
            // Appeared since planning: roll everything back.
            for (t, d) in done.iter().rev() {
                let _ = std::fs::rename(d, t);
            }
            for (f, t) in staged.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            return Err(FileOpError::Io(
                to.clone(),
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "appeared during rename"),
            ));
        }
        if let Err(e) = std::fs::rename(tmp, to) {
            for (t, d) in done.iter().rev() {
                let _ = std::fs::rename(d, t);
            }
            for (f, t) in staged.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            return Err(FileOpError::Io(to.clone(), e));
        }
        done.push((tmp.clone(), to.clone()));
    }
    Ok(plan.moves.clone())
}

/// Move files to the system Trash. On macOS this goes through
/// NSFileManager rather than Finder scripting: no automation prompt, no
/// Finder time-outs; the files sit in the Trash until it is emptied.
pub fn trash(paths: &[PathBuf]) -> Result<(), FileOpError> {
    if paths.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete_all(paths)
            .map_err(|e| FileOpError::Trash(e.to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete_all(paths).map_err(|e| FileOpError::Trash(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Kind, Variant};

    fn item(dir: &Path, stem: &str, exts: &[&str]) -> PhotoItem {
        let mut vs: Vec<Variant> = exts
            .iter()
            .map(|e| {
                let p = dir.join(format!("{stem}.{e}"));
                std::fs::write(&p, e).unwrap();
                Variant {
                    path: p,
                    kind: if *e == "NEF" { Kind::Raw } else { Kind::Jpeg },
                    size: 1,
                    modified: None,
                }
            })
            .collect();
        let primary = vs.remove(0);
        PhotoItem {
            id: 0,
            stem: stem.into(),
            primary,
            secondary: vs,
        }
    }

    #[test]
    fn trash_refuses_a_missing_path_and_accepts_nothing() {
        let d = tempfile::tempdir().unwrap();
        assert!(trash(&[]).is_ok());
        assert!(matches!(
            trash(&[d.path().join("not-there.JPG")]),
            Err(FileOpError::Trash(_))
        ));
    }

    /// Moves real files into your Trash, so it only runs when asked:
    /// `cargo test -p apature-core -- --ignored trash_moves`.
    #[test]
    #[ignore]
    fn trash_moves_every_file_of_an_item() {
        let d = tempfile::tempdir().unwrap();
        let a = item(d.path(), "apature-trash-test", &["NEF", "JPG"]);
        let files = item_files(&a);
        assert_eq!(files.len(), 2);
        trash(&files).unwrap();
        assert!(files.iter().all(|f| !f.exists()));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn case_only_rename_is_not_a_collision() {
        let d = tempfile::tempdir().unwrap();
        let a = item(d.path(), "shot", &["JPG"]);
        let plan = plan_rename(&[(&a, "SHOT".into())]).unwrap();
        assert!(plan.collisions.is_empty(), "{:?}", plan.collisions);
        rename(&plan).unwrap();
        let names: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["SHOT.JPG"]);
        // A different file under the new name is still a collision.
        let b = item(d.path(), "other", &["JPG"]);
        let plan = plan_rename(&[(&b, "SHOT".into())]).unwrap();
        assert_eq!(plan.collisions.len(), 1);
    }

    #[test]
    fn pair_and_sidecar_move_together_and_collisions_refuse() {
        let d = tempfile::tempdir().unwrap();
        let a = item(d.path(), "DSC_0001", &["NEF", "JPG"]);
        std::fs::write(d.path().join("DSC_0001.xmp"), "x").unwrap();
        let b = item(d.path(), "DSC_0002", &["NEF"]);
        let plan = plan_rename(&[(&a, "BUTLR-001".into()), (&b, "BUTLR-002".into())]).unwrap();
        assert_eq!(plan.moves.len(), 4, "NEF+JPG+xmp, NEF");
        assert!(plan.can_run());
        let done = rename(&plan).unwrap();
        assert_eq!(done.len(), 4);
        for n in [
            "BUTLR-001.NEF",
            "BUTLR-001.JPG",
            "BUTLR-001.xmp",
            "BUTLR-002.NEF",
        ] {
            assert!(d.path().join(n).is_file(), "{n}");
        }
        assert!(!d.path().join("DSC_0001.NEF").exists());
        assert!(std::fs::read_dir(d.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("apature-rename")));

        // Collision with an existing file, and two items to one name.
        let c = item(d.path(), "DSC_0003", &["NEF"]);
        let e = item(d.path(), "DSC_0004", &["NEF"]);
        let plan = plan_rename(&[(&c, "BUTLR-001".into()), (&e, "BUTLR-001".into())]).unwrap();
        assert_eq!(plan.collisions.len(), 2);
        assert!(!plan.can_run());
        assert!(matches!(rename(&plan), Err(FileOpError::Collisions)));
        assert!(d.path().join("DSC_0003.NEF").is_file(), "nothing moved");
    }

    #[test]
    fn swap_and_unchanged_and_bad_names() {
        let d = tempfile::tempdir().unwrap();
        let a = item(d.path(), "A", &["JPG"]);
        let b = item(d.path(), "B", &["JPG"]);
        let plan = plan_rename(&[(&a, "B".into()), (&b, "A".into())]).unwrap();
        assert!(
            plan.can_run(),
            "swap is not a collision: {:?}",
            plan.collisions
        );
        rename(&plan).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.path().join("A.JPG")).unwrap(),
            "JPG"
        );
        let plan = plan_rename(&[(&a, "A".into())]).unwrap();
        assert_eq!(plan.unchanged, 1);
        assert!(!plan.can_run());
        assert!(matches!(
            plan_rename(&[(&a, "x/y".into())]),
            Err(FileOpError::BadName(_))
        ));
    }
}
