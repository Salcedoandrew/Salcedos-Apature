//! Sidecar names as spelled on disk, remembered per directory.
//!
//! [`crate::PhotoItem::sidecar_path`] has to return the sidecar's real
//! spelling: Photo Mechanic writes `IMG.XMP` next to `IMG.NEF`, and a guessed
//! `IMG.xmp` is a different file name on a case-sensitive volume. Finding out
//! used to mean a full `read_dir` of the photo's folder on every call, and the
//! inspector asks once per frame — a folder of thousands of files listed on the
//! UI thread every frame.
//!
//! So the listing happens once, inside [`crate::scan`], and lands here: per
//! directory, the `.xmp` file names it contains. Lookups are memory only, and
//! every successful sidecar write records the name it created, so marks
//! written by the app show up without a rescan. Nothing watches the
//! filesystem: a sidecar that appears or disappears behind our back is picked
//! up by the next scan of that directory, which replaces the entry wholesale.
//!
//! The index is process-wide because `sidecar_path` is also called by the
//! library crates (export, ingest, CLI) on items that came from `scan`. Only
//! sidecar names are kept, so a folder of thousands of photos costs a few
//! strings per directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The sidecar names one directory listing had.
#[derive(Debug, Default)]
struct Listing {
    /// Lowercased file name → the spellings actually present. Two only when a
    /// case-sensitive directory holds both `IMG.XMP` and `IMG.xmp`.
    names: HashMap<String, Vec<String>>,
}

impl Listing {
    fn add(&mut self, name: &str) {
        let spellings = self.names.entry(name.to_ascii_lowercase()).or_default();
        if !spellings.iter().any(|s| s == name) {
            spellings.push(name.to_string());
        }
    }
}

fn listings() -> &'static RwLock<HashMap<PathBuf, Listing>> {
    static INDEX: OnceLock<RwLock<HashMap<PathBuf, Listing>>> = OnceLock::new();
    INDEX.get_or_init(|| RwLock::new(HashMap::new()))
}

fn read() -> RwLockReadGuard<'static, HashMap<PathBuf, Listing>> {
    listings().read().unwrap_or_else(|e| e.into_inner())
}

fn write() -> RwLockWriteGuard<'static, HashMap<PathBuf, Listing>> {
    listings().write().unwrap_or_else(|e| e.into_inner())
}

/// Replace what is known about `dir` with `names`, the sidecar file names its
/// listing had (`.xmp`, any case). `scan` calls this once per directory it
/// visits, including directories with no sidecars: that is what drops a
/// sidecar another app removed since the last scan.
pub fn note_dir(dir: &Path, names: &[String]) {
    let mut listing = Listing::default();
    for name in names {
        listing.add(name);
    }
    write().insert(dir.to_path_buf(), listing);
}

/// Record the sidecar just written at `path`, so lookups return that spelling
/// and the item counts as having a sidecar without a rescan.
pub fn note_written(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    write().entry(dir.to_path_buf()).or_default().add(name);
}

/// True when a scan has recorded a listing for `dir`, i.e. the index is the
/// whole truth about its sidecars and need not be checked against the disk.
pub fn listed(dir: &Path) -> bool {
    read().contains_key(dir)
}

/// The sidecar name from `p`'s directory listing, spelled as it is on disk:
/// the exact spelling first (a case-sensitive directory can hold both
/// `IMG.XMP` and `IMG.xmp` at once), then any spelling that differs only by
/// case. None when the listing has no such sidecar, or the directory was never
/// listed.
pub fn resolve(p: &Path) -> Option<PathBuf> {
    let parent = p.parent()?;
    let want = p.file_name()?.to_str()?;
    let dirs = read();
    let spellings = dirs.get(parent)?.names.get(&want.to_ascii_lowercase())?;
    let name = spellings
        .iter()
        .find(|s| s.as_str() == want)
        .or_else(|| spellings.first())?;
    Some(parent.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_decides_the_spelling_exact_first() {
        let dir = std::env::temp_dir().join(format!("apature-sidecars-{}", std::process::id()));
        note_dir(&dir, &["IMG.XMP".into()]);
        assert!(listed(&dir));
        assert_eq!(resolve(&dir.join("IMG.xmp")), Some(dir.join("IMG.XMP")));
        assert_eq!(resolve(&dir.join("IMG.XMP")), Some(dir.join("IMG.XMP")));
        assert_eq!(resolve(&dir.join("other.xmp")), None);
        assert!(!listed(&dir.join("sub")));

        // A case-sensitive directory can hold both; the exact spelling wins.
        note_dir(&dir, &["IMG.XMP".into(), "IMG.xmp".into()]);
        assert_eq!(resolve(&dir.join("IMG.xmp")), Some(dir.join("IMG.xmp")));
        assert_eq!(resolve(&dir.join("IMG.XMP")), Some(dir.join("IMG.XMP")));

        // A write records its own name; a rescan without it clears it.
        note_dir(&dir, &[]);
        assert_eq!(resolve(&dir.join("IMG.xmp")), None);
        note_written(&dir.join("IMG.xmp"));
        assert_eq!(resolve(&dir.join("IMG.XMP")), Some(dir.join("IMG.xmp")));
    }
}
