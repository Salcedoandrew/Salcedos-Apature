//! Copy Photos…: copy chosen photos from the contact sheet to a folder.
//!
//! The same rules as ingest: plan first, copy bytes as they are (never
//! re-encoded) through the verified copy, never touch the originals. On top
//! of that every copy gets its original's modified, accessed and (on macOS)
//! created dates, so the copy sorts and reads exactly like the source.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use apature_core::PhotoItem;
use apature_metadata::{iptc, xmp, Iptc};
use rayon::prelude::*;

use crate::copy::{copy_verified, IngestError};
use crate::plan::{civil_from_days, facts_for, PlanError, Skip};
use crate::template::{expand, sanitize_component, Facts};

/// What to do when a file of that name is already at the destination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OnCollision {
    /// Leave the existing file and do not copy that photo.
    Skip,
    /// Add `-1`, `-2`… to the name until it is free (the default).
    #[default]
    AddNumber,
    /// Replace the existing file (never one of the photos being copied).
    Replace,
}

/// Subfolders by capture date under the destination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DateFolders {
    #[default]
    None,
    /// `2026-10-22`
    Day,
    /// `2026/10/22`
    YearMonthDay,
}

/// Where the copies go (Photo Mechanic's Destination group).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DestMode {
    /// Next to each original, in its own folder.
    OriginalFolder,
    /// In a subfolder (`subfolder`, e.g. SELECTS) of each original's folder.
    Subfolder,
    /// `dest`, plus the optional sub-path `dest_extra`.
    #[default]
    ThisFolder,
    /// Ask for a folder each time Copy is pressed; it becomes `dest`.
    AlwaysPick,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CopyOptions {
    pub dest_mode: DestMode,
    /// The chosen folder for This folder (and the last pick for Always pick).
    pub dest: PathBuf,
    /// Appended to `dest` in This folder mode: `2nd` or `Smith/2nd`.
    pub dest_extra: String,
    /// Folder name for Create subfolder.
    pub subfolder: String,
    /// Open the destination in a new tab when the copy finishes.
    pub open_after: bool,
    /// File name template without extension (the Rename Photos tokens);
    /// empty keeps the original names.
    pub rename_template: String,
    pub seq_start: u32,
    /// Copy the other half of a RAW+JPEG pair too.
    pub partners: bool,
    /// Copy the XMP sidecar too (it takes the photo's new name).
    pub sidecars: bool,
    pub on_collision: OnCollision,
    pub date_folders: DateFolders,
    /// `{headline}` for the rename template.
    pub headline: Option<String>,
}

impl Default for CopyOptions {
    fn default() -> Self {
        CopyOptions {
            dest_mode: DestMode::default(),
            dest: PathBuf::new(),
            dest_extra: String::new(),
            subfolder: "SELECTS".into(),
            open_after: false,
            rename_template: String::new(),
            seq_start: 1,
            partners: false,
            sidecars: false,
            on_collision: OnCollision::default(),
            date_folders: DateFolders::default(),
            headline: None,
        }
    }
}

/// Safe folder names from a typed sub-path: `/`-separated, each part
/// sanitised; empty, `.` and `..` parts are dropped so it stays inside.
fn sub_path(text: &str) -> PathBuf {
    text.split(['/', '\\'])
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .filter_map(sanitize_component)
        .collect()
}

/// The folder a photo's copies go to before any dated subfolders, or `None`
/// when the options don't name one yet (no folder chosen, no subfolder name).
pub fn dest_base(opts: &CopyOptions, item: &PhotoItem) -> Option<PathBuf> {
    let own = item.path().parent().map(Path::to_path_buf);
    match opts.dest_mode {
        DestMode::OriginalFolder => own,
        DestMode::Subfolder => {
            let sub = sub_path(&opts.subfolder);
            (!sub.as_os_str().is_empty())
                .then(|| own.map(|o| o.join(sub)))
                .flatten()
        }
        DestMode::ThisFolder | DestMode::AlwaysPick => {
            if opts.dest.as_os_str().is_empty() {
                return None;
            }
            Some(match opts.dest_mode {
                DestMode::ThisFolder => opts.dest.join(sub_path(&opts.dest_extra)),
                _ => opts.dest.clone(),
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyEntry {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub size: u64,
    /// Index into the items handed to [`plan_copy`].
    pub item: usize,
    /// An XMP sidecar, not a photo file.
    pub sidecar: bool,
    /// The destination exists and is to be replaced.
    pub replace: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyPlan {
    pub entries: Vec<CopyEntry>,
    pub skipped: Vec<Skip>,
    pub total_bytes: u64,
    /// Photos planned (a pair counts once).
    pub photos: usize,
    /// Photos that got a `-N` because their name was taken.
    pub numbered: usize,
    /// Files that will replace one already at the destination.
    pub replacing: usize,
}

/// Civil date (UTC) of a file time, for photos without an EXIF date.
/// The days-to-civil conversion itself is `plan`'s, shared with `today`.
fn civil(t: std::time::SystemTime) -> (i32, u32, u32) {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs.div_euclid(86_400))
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn with_ext(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

/// Plan copying `items` (in the order given; `{seqn}` follows it).
pub fn plan_copy(items: &[&PhotoItem], opts: &CopyOptions) -> Result<CopyPlan, PlanError> {
    let mut plan = CopyPlan::default();
    // Every original, so a copy can never replace (or be) one of them.
    let originals: BTreeSet<PathBuf> = items
        .iter()
        .flat_map(|it| {
            it.variants()
                .map(|v| v.path.clone())
                .chain(std::iter::once(it.sidecar_path()))
        })
        .map(|p| fs::canonicalize(&p).unwrap_or(p))
        .collect();
    let is_original = |p: &Path| {
        let c = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        originals.contains(&c)
    };
    let mut planned: BTreeSet<PathBuf> = BTreeSet::new();
    let mut seq = opts.seq_start.max(1);

    // The EXIF read behind `facts_for` is independent per photo, so it
    // runs in parallel; the planning itself stays sequential, in the
    // order the photos were handed over (`{seqn}` follows it, and the
    // collision state builds on it).
    let facts_all: Vec<Facts> = items
        .par_iter()
        .map(|item| facts_for(item, item.path(), opts.headline.as_deref()))
        .collect();

    for (idx, item) in items.iter().enumerate() {
        let facts = &facts_all[idx];
        let folder = match opts.date_folders {
            DateFolders::None => PathBuf::new(),
            mode => {
                let (y, m, d) = facts
                    .date
                    .or_else(|| item.primary.modified.map(civil))
                    .unwrap_or((1970, 1, 1));
                match mode {
                    DateFolders::Day => PathBuf::from(format!("{y:04}-{m:02}-{d:02}")),
                    _ => PathBuf::from(format!("{y:04}/{m:02}/{d:02}")),
                }
            }
        };
        let Some(base) = dest_base(opts, item) else {
            plan.skipped.push(Skip {
                source: item.path().to_path_buf(),
                reason: "no destination folder".into(),
            });
            continue;
        };
        let dir = base.join(&folder);

        // The files of this photo, each with its extension.
        let mut files: Vec<(PathBuf, u64, bool)> =
            vec![(item.primary.path.clone(), item.primary.size, false)];
        if opts.partners {
            files.extend(
                item.secondary
                    .iter()
                    .map(|v| (v.path.clone(), v.size, false)),
            );
        }
        if opts.sidecars {
            let sc = item.sidecar_path();
            if sc.is_file() {
                let size = fs::metadata(&sc).map(|m| m.len()).unwrap_or(0);
                files.push((sc, size, true));
            }
        }

        // One stem for the whole photo so a pair and its sidecar stay together.
        let stem = if opts.rename_template.trim().is_empty() {
            item.stem.clone()
        } else {
            let mut s = expand(&opts.rename_template, facts, seq);
            for (p, _, _) in &files {
                let suffix = format!(".{}", ext_of(p)).to_lowercase();
                if suffix.len() > 1 && s.to_lowercase().ends_with(&suffix) {
                    s.truncate(s.len() - suffix.len());
                }
            }
            sanitize_component(&s).ok_or_else(|| PlanError::BadName(item.path().to_path_buf()))?
        };
        let names = |stem: &str| -> Vec<PathBuf> {
            files
                .iter()
                .map(|(p, _, _)| dir.join(with_ext(stem, &ext_of(p))))
                .collect()
        };

        // Pick the stem: always free within this plan; on disk per the policy.
        let mut k = 0u32;
        let (dests, skip_reason) = loop {
            let cand = if k == 0 {
                stem.clone()
            } else {
                format!("{stem}-{k}")
            };
            let dests = names(&cand);
            let in_plan = dests.iter().any(|d| planned.contains(d));
            let on_disk = dests.iter().any(|d| d.exists());
            let hits_original = dests.iter().any(|d| d.exists() && is_original(d));
            if !in_plan && !on_disk {
                break (dests, None);
            }
            if !in_plan {
                match opts.on_collision {
                    OnCollision::Skip => break (dests, Some("already at the destination")),
                    // Beside the originals nothing is ever replaced: an
                    // existing name there may be another original.
                    OnCollision::Replace if opts.dest_mode == DestMode::OriginalFolder => {}
                    OnCollision::Replace if hits_original => {
                        break (dests, Some("would replace the original"))
                    }
                    OnCollision::Replace => break (dests, None),
                    OnCollision::AddNumber => {}
                }
            }
            k += 1;
            if k > 9999 {
                break (dests, Some("no free name"));
            }
        };
        if let Some(reason) = skip_reason {
            plan.skipped.push(Skip {
                source: item.path().to_path_buf(),
                reason: reason.into(),
            });
            continue;
        }
        if k > 0 {
            plan.numbered += 1;
        }
        for ((src, size, sidecar), dst) in files.into_iter().zip(dests) {
            let replace = dst.exists();
            plan.replacing += replace as usize;
            planned.insert(dst.clone());
            plan.total_bytes += size;
            plan.entries.push(CopyEntry {
                source: src,
                destination: dst,
                size,
                item: idx,
                sidecar,
                replace,
            });
        }
        plan.photos += 1;
        seq += 1;
    }
    Ok(plan)
}

/// Give `dst` the modified, accessed and (macOS) created dates of `src`.
pub fn copy_dates(src: &Path, dst: &Path) -> std::io::Result<()> {
    let m = fs::metadata(src)?;
    let mut times = FileTimes::new();
    if let Ok(t) = m.modified() {
        times = times.set_modified(t);
    }
    if let Ok(t) = m.accessed() {
        times = times.set_accessed(t);
    }
    #[cfg(target_os = "macos")]
    if let Ok(t) = m.created() {
        use std::os::macos::fs::FileTimesExt;
        times = times.set_created(t);
    }
    File::options().write(true).open(dst)?.set_times(times)
}

/// Verified byte-for-byte copy that keeps the dates. With `replace`, an
/// existing destination is swapped out only after the new copy verified.
pub fn copy_preserving(src: &Path, dst: &Path, replace: bool) -> Result<u64, IngestError> {
    if replace && dst.exists() {
        let name = dst
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = dst.with_file_name(format!(".{name}.replace{}", std::process::id()));
        let r = copy_verified(src, &tmp)?;
        let done = copy_dates(src, &tmp).and_then(|_| fs::rename(&tmp, dst));
        if let Err(e) = done {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        return Ok(r.bytes);
    }
    let r = copy_verified(src, dst)?;
    copy_dates(src, dst)?;
    Ok(r.bytes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyProgress {
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyOutcome {
    /// (source, destination) of every verified copy.
    pub copied: Vec<(PathBuf, PathBuf)>,
    pub failed: Vec<(PathBuf, String)>,
    pub bytes: u64,
    pub cancelled: bool,
    /// Sidecars the caption was written to, and failures doing so.
    pub pad_written: usize,
    pub pad_failed: usize,
}

/// The sidecar beside a copied photo, from that photo's entries (in plan
/// order): the copied one if there is one, otherwise `stem.xmp` (`.XMP`
/// when the photo's extension is upper case).
fn dest_sidecar(entries: &[&CopyEntry]) -> Option<PathBuf> {
    let first = entries.first()?;
    if let Some(e) = entries.iter().find(|e| e.sidecar) {
        return Some(e.destination.clone());
    }
    let ext = ext_of(&first.destination);
    let upper = !ext.is_empty() && ext.chars().all(|c| !c.is_ascii_lowercase());
    Some(
        first
            .destination
            .with_extension(if upper { "XMP" } else { "xmp" }),
    )
}

/// Copy every entry in order. Originals are only read. With `pad`, each
/// fully copied photo's destination sidecar gets those IPTC fields (the
/// copied photo files themselves are never modified).
pub fn run_copy(
    plan: &CopyPlan,
    cancel: &AtomicBool,
    pad: Option<&Iptc>,
    mut on_progress: impl FnMut(&CopyProgress),
) -> CopyOutcome {
    let mut out = CopyOutcome::default();
    for (i, e) in plan.entries.iter().enumerate() {
        on_progress(&CopyProgress {
            files_done: i,
            files_total: plan.entries.len(),
            bytes_done: out.bytes,
            bytes_total: plan.total_bytes,
            current: e.source.clone(),
        });
        if cancel.load(Ordering::Relaxed) {
            out.cancelled = true;
            break;
        }
        match copy_preserving(&e.source, &e.destination, e.replace) {
            Ok(n) => {
                out.bytes += n;
                out.copied.push((e.source.clone(), e.destination.clone()));
            }
            Err(err) => out.failed.push((e.source.clone(), err.to_string())),
        }
    }
    if let Some(fields) = pad {
        // Group the entries per photo and index the copied destinations
        // once: testing every photo against the whole plan and the whole
        // copied list made this pass quadratic in the number of photos.
        let copied: HashSet<&Path> =
            out.copied.iter().map(|(_, d)| d.as_path()).collect();
        let mut by_item: BTreeMap<usize, Vec<&CopyEntry>> = BTreeMap::new();
        for e in &plan.entries {
            by_item.entry(e.item).or_default().push(e);
        }
        for entries in by_item.values() {
            // Only photos whose every file arrived.
            if !entries
                .iter()
                .all(|e| copied.contains(e.destination.as_path()))
            {
                continue;
            }
            let Some(p) = dest_sidecar(entries) else {
                continue;
            };
            let r = xmp::write_sidecar_with(
                &p,
                |t| iptc::apply(t, fields),
                |t| iptc::satisfies(fields, &iptc::parse(t)),
            );
            match r {
                Ok(()) => out.pad_written += 1,
                Err(_) => out.pad_failed += 1,
            }
        }
    }
    on_progress(&CopyProgress {
        files_done: out.copied.len() + out.failed.len(),
        files_total: plan.entries.len(),
        bytes_done: out.bytes,
        bytes_total: plan.total_bytes,
        current: PathBuf::new(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn shoot(dir: &Path, files: &[&str]) -> Vec<PhotoItem> {
        fs::create_dir_all(dir).unwrap();
        for f in files {
            fs::write(dir.join(f), format!("bytes of {f}")).unwrap();
        }
        apature_core::scan(dir, &apature_core::ScanOptions::default()).unwrap()
    }

    fn opts(dest: &Path) -> CopyOptions {
        CopyOptions {
            dest: dest.to_path_buf(),
            seq_start: 1,
            partners: true,
            sidecars: true,
            ..Default::default()
        }
    }

    fn names(plan: &CopyPlan) -> Vec<String> {
        plan.entries
            .iter()
            .map(|e| {
                e.destination
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn pairs_and_sidecars_travel_together_under_the_new_name() {
        let d = tempfile::tempdir().unwrap();
        let items = shoot(
            &d.path().join("src"),
            &[
                "DSC_0001.NEF",
                "DSC_0001.JPG",
                "DSC_0001.XMP",
                "DSC_0002.NEF",
            ],
        );
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let mut o = opts(&d.path().join("out"));
        o.rename_template = "game-{seqn:3}".into();
        let plan = plan_copy(&refs, &o).unwrap();
        assert_eq!(
            names(&plan),
            [
                "game-001.NEF",
                "game-001.JPG",
                "game-001.XMP",
                "game-002.NEF"
            ]
        );
        assert_eq!(plan.photos, 2);
        assert!(plan.entries[2].sidecar);
        // Without partners or sidecars only the primary goes.
        o.partners = false;
        o.sidecars = false;
        let plan = plan_copy(&refs, &o).unwrap();
        assert_eq!(names(&plan), ["game-001.NEF", "game-002.NEF"]);
    }

    #[test]
    fn collisions_skip_number_or_replace() {
        let d = tempfile::tempdir().unwrap();
        let items = shoot(&d.path().join("src"), &["A.NEF", "A.JPG", "B.NEF"]);
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let out = d.path().join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("A.JPG"), b"older copy").unwrap();
        let mut o = opts(&out);

        o.on_collision = OnCollision::Skip;
        let p = plan_copy(&refs, &o).unwrap();
        assert_eq!(names(&p), ["B.NEF"]);
        assert_eq!(p.skipped.len(), 1);

        o.on_collision = OnCollision::AddNumber;
        let p = plan_copy(&refs, &o).unwrap();
        // The whole pair moves to -1 so it stays a pair.
        assert_eq!(names(&p), ["A-1.NEF", "A-1.JPG", "B.NEF"]);
        assert_eq!(p.numbered, 1);

        o.on_collision = OnCollision::Replace;
        let p = plan_copy(&refs, &o).unwrap();
        assert_eq!(names(&p), ["A.NEF", "A.JPG", "B.NEF"]);
        assert_eq!(p.replacing, 1);
        let r = run_copy(&p, &AtomicBool::new(false), None, |_| {});
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(fs::read(out.join("A.JPG")).unwrap(), b"bytes of A.JPG");

        // A template without a counter repeats names within the batch; those
        // are numbered whatever the policy, so no photo is lost.
        o.rename_template = "same".into();
        o.dest = d.path().join("out2");
        o.on_collision = OnCollision::Skip;
        let p = plan_copy(&refs, &o).unwrap();
        assert_eq!(names(&p), ["same.NEF", "same.JPG", "same-1.NEF"]);
    }

    #[test]
    fn never_replaces_an_original() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        let items = shoot(&src, &["A.NEF"]);
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let mut o = opts(&src);
        o.on_collision = OnCollision::Replace;
        let p = plan_copy(&refs, &o).unwrap();
        assert!(p.entries.is_empty());
        assert_eq!(p.skipped[0].reason, "would replace the original");
        o.on_collision = OnCollision::AddNumber;
        assert_eq!(names(&plan_copy(&refs, &o).unwrap()), ["A-1.NEF"]);
    }

    #[test]
    fn copies_keep_bytes_and_dates_and_originals() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for f in ["A.NEF", "A.XMP"] {
            fs::write(src.join(f), format!("bytes of {f}")).unwrap();
            File::options()
                .write(true)
                .open(src.join(f))
                .unwrap()
                .set_times(FileTimes::new().set_modified(old).set_accessed(old))
                .unwrap();
        }
        let items = apature_core::scan(&src, &apature_core::ScanOptions::default()).unwrap();
        let before = fs::metadata(src.join("A.NEF")).unwrap();
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let mut o = opts(&d.path().join("out"));
        o.date_folders = DateFolders::Day;
        let p = plan_copy(&refs, &o).unwrap();
        // No EXIF date in these files: the folder comes from the file date.
        assert_eq!(
            p.entries[0].destination,
            d.path().join("out/2020-09-13/A.NEF")
        );
        let r = run_copy(&p, &AtomicBool::new(false), None, |_| {});
        assert_eq!(r.copied.len(), 2);
        for (s, t) in &r.copied {
            assert_eq!(fs::read(s).unwrap(), fs::read(t).unwrap());
            let (ms, mt) = (fs::metadata(s).unwrap(), fs::metadata(t).unwrap());
            assert_eq!(ms.modified().unwrap(), mt.modified().unwrap());
            #[cfg(target_os = "macos")]
            assert_eq!(ms.created().unwrap(), mt.created().unwrap());
        }
        let after = fs::metadata(src.join("A.NEF")).unwrap();
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(fs::read(src.join("A.NEF")).unwrap(), b"bytes of A.NEF");
    }

    #[test]
    fn caption_on_copy_goes_to_the_copy_sidecar_only() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        let items = shoot(&src, &["A.JPG"]);
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let p = plan_copy(&refs, &opts(&d.path().join("out"))).unwrap();
        let pad = Iptc {
            headline: Some("Opening day".into()),
            ..Default::default()
        };
        let r = run_copy(&p, &AtomicBool::new(false), Some(&pad), |_| {});
        assert_eq!((r.pad_written, r.pad_failed), (1, 0));
        let sc = fs::read_to_string(d.path().join("out/A.XMP")).unwrap();
        assert!(sc.contains("Opening day"));
        assert_eq!(
            fs::read(d.path().join("out/A.JPG")).unwrap(),
            b"bytes of A.JPG"
        );
        assert!(!src.join("A.xmp").exists());
    }

    #[test]
    fn cancel_stops_before_the_next_file() {
        let d = tempfile::tempdir().unwrap();
        let items = shoot(&d.path().join("src"), &["A.NEF", "B.NEF"]);
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let p = plan_copy(&refs, &opts(&d.path().join("out"))).unwrap();
        let cancel = AtomicBool::new(false);
        let r = run_copy(&p, &cancel, None, |pr| {
            if pr.files_done == 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(r.cancelled);
        assert_eq!(r.copied.len(), 1);
    }

    #[test]
    fn destination_modes_resolve_per_photo() {
        let d = tempfile::tempdir().unwrap();
        let a = shoot(&d.path().join("day1"), &["A.NEF"]);
        let b = shoot(&d.path().join("day2"), &["B.NEF"]);
        let items = [&a[0], &b[0]];
        let out = d.path().join("out");
        let mut o = opts(&out);
        let dirs = |o: &CopyOptions| -> Vec<PathBuf> {
            plan_copy(&items, o)
                .unwrap()
                .entries
                .iter()
                .map(|e| {
                    e.destination
                        .parent()
                        .unwrap()
                        .strip_prefix(d.path())
                        .unwrap()
                        .to_path_buf()
                })
                .collect()
        };
        // This folder, then with a + sub-path (unsafe parts dropped).
        assert_eq!(dirs(&o), [PathBuf::from("out"), PathBuf::from("out")]);
        o.dest_extra = " Smith / ../2nd/ ".into();
        assert_eq!(
            dirs(&o),
            [
                PathBuf::from("out/Smith/2nd"),
                PathBuf::from("out/Smith/2nd")
            ]
        );
        // Always pick ignores the sub-path: the picked folder is the folder.
        o.dest_mode = DestMode::AlwaysPick;
        assert_eq!(dirs(&o), [PathBuf::from("out"), PathBuf::from("out")]);
        // Create subfolder: inside each original's own folder.
        o.dest_mode = DestMode::Subfolder;
        assert_eq!(o.subfolder, "SELECTS");
        assert_eq!(
            dirs(&o),
            [PathBuf::from("day1/SELECTS"), PathBuf::from("day2/SELECTS")]
        );
        // Original folder: beside each original, with dated folders under it.
        o.dest_mode = DestMode::OriginalFolder;
        o.date_folders = DateFolders::Day;
        let got = dirs(&o);
        assert!(
            got[0].starts_with("day1") && got[0] != PathBuf::from("day1"),
            "{got:?}"
        );
        assert!(got[1].starts_with("day2"));
        // Nothing to resolve yet: no folder chosen, or an empty subfolder name.
        let mut none = opts(Path::new(""));
        assert_eq!(plan_copy(&items, &none).unwrap().skipped.len(), 2);
        none.dest_mode = DestMode::Subfolder;
        none.subfolder = " / ".into();
        assert!(plan_copy(&items, &none).unwrap().entries.is_empty());
    }

    #[test]
    fn original_folder_never_overwrites_or_copies_onto_itself() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("shoot");
        let items = shoot(&src, &["C.NEF", "C.JPG", "D.NEF"]);
        let refs: Vec<&PhotoItem> = items.iter().collect();
        let before: Vec<(PathBuf, Vec<u8>)> = fs::read_dir(&src)
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| (p.clone(), fs::read(&p).unwrap()))
            .collect();
        for policy in [
            OnCollision::AddNumber,
            OnCollision::Replace,
            OnCollision::Skip,
        ] {
            let mut o = opts(Path::new(""));
            o.dest_mode = DestMode::OriginalFolder;
            o.on_collision = policy;
            // Rename D onto C's name too: still must not touch C.
            o.rename_template = if policy == OnCollision::Replace {
                "C".into()
            } else {
                String::new()
            };
            let plan = plan_copy(&refs, &o).unwrap();
            for e in &plan.entries {
                assert_ne!(e.source, e.destination);
                assert!(!e.replace, "{policy:?} would replace {:?}", e.destination);
            }
            let out = run_copy(&plan, &AtomicBool::new(false), None, |_| {});
            assert!(out.failed.is_empty());
        }
        for (p, bytes) in before {
            assert_eq!(fs::read(&p).unwrap(), bytes, "{p:?} changed");
        }
        // Add a number put numbered copies beside them.
        assert!(src.join("C-1.NEF").is_file() && src.join("C-1.JPG").is_file());
    }

    #[test]
    fn older_prefs_keep_their_folder_and_get_the_new_defaults() {
        let old = r#"{"dest":"/x/out","rename_template":"","seq_start":1,"partners":false,"sidecars":true,"on_collision":"AddNumber","date_folders":"None","headline":null}"#;
        let o: CopyOptions = serde_json::from_str(old).unwrap();
        assert_eq!(o.dest_mode, DestMode::ThisFolder);
        assert_eq!(o.dest, PathBuf::from("/x/out"));
        assert_eq!(o.subfolder, "SELECTS");
        assert!(o.dest_extra.is_empty() && !o.open_after);
    }
}
