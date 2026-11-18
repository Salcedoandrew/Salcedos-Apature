use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::item::{kind_for_extension, Kind, PhotoItem, Variant};

#[derive(Clone, Debug, Default)]
pub struct ScanOptions {
    /// Descend into subdirectories. Off by default: a contact sheet is one folder.
    pub recursive: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),
    #[error("walk failed: {0}")]
    Walk(#[from] walkdir::Error),
}

/// Scan a folder and return photo items sorted by file name, with RAW+JPEG
/// pairs merged. Hidden files and files with unknown extensions are skipped.
pub fn scan(dir: &Path, opts: &ScanOptions) -> Result<Vec<PhotoItem>, ScanError> {
    if !dir.is_dir() {
        return Err(ScanError::NotADirectory(dir.to_path_buf()));
    }
    let depth = if opts.recursive { usize::MAX } else { 1 };

    // Group by (directory, lowercase stem) so that pairs in the same folder collapse.
    let mut groups: BTreeMap<(PathBuf, String), Vec<Variant>> = BTreeMap::new();
    // The sidecar names per directory, collected from this same walk: one
    // listing per directory, so `sidecar_path` never has to list one again.
    let mut sidecars: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();

    // Hidden directories (.Trashes, .Spotlight-V100, .fseventsd on a card) are
    // skipped before descent: macOS refuses to read some of them, and a
    // permission error there must not abort the whole scan. Any other
    // unreadable entry is skipped too, never fatal.
    let walker = WalkDir::new(dir)
        .max_depth(depth)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                log::warn!(
                    "scan: skipping {}: {err}",
                    err.path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let parent = path.parent().unwrap_or(dir).to_path_buf();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        // Every directory with files in it is recorded, even one with no
        // sidecars, so a sidecar removed since the last scan drops out below.
        let listing = sidecars.entry(parent.clone()).or_default();
        if ext.eq_ignore_ascii_case("xmp") {
            listing.push(name.into_owned());
        }
        let Some(kind) = kind_for_extension(ext) else {
            continue;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meta = entry.metadata().ok();
        let variant = Variant {
            path: path.to_path_buf(),
            kind,
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: meta.and_then(|m| m.modified().ok()),
        };
        groups
            .entry((parent, stem.to_lowercase()))
            .or_default()
            .push(variant);
    }

    // Publish the listings: this scan is what rebuilds an entry, and a
    // directory with no sidecars clears the one it had.
    for (dir, names) in &sidecars {
        crate::sidecars::note_dir(dir, names);
    }

    let mut items = Vec::with_capacity(groups.len());
    for ((_, _), mut variants) in groups {
        // RAW first, then JPEG, then HEIC; stable within a kind.
        variants.sort_by_key(|v| match v.kind {
            Kind::Raw => 0,
            Kind::Jpeg => 1,
            Kind::Tiff => 2,
            Kind::Png => 3,
            Kind::Heic => 4,
        });
        let primary = variants.remove(0);
        let stem = primary
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        items.push(PhotoItem {
            id: 0,
            stem,
            primary,
            secondary: variants,
        });
    }
    items.sort_by(|a, b| {
        a.primary
            .path
            .to_string_lossy()
            .to_lowercase()
            .cmp(&b.primary.path.to_string_lossy().to_lowercase())
    });
    for (i, item) in items.iter_mut().enumerate() {
        item.id = i;
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn pairs_raw_and_jpeg_and_skips_unknown() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "DSC_0001.NEF",
            "DSC_0001.JPG",
            "DSC_0002.JPG",
            "notes.txt",
            ".hidden.jpg",
            "dsc_0003.nef",
        ] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        let items = scan(dir.path(), &ScanOptions::default()).unwrap();
        let names: Vec<_> = items.iter().map(|i| i.display_name()).collect();
        assert_eq!(names, vec!["DSC_0001.NEF", "DSC_0002.JPG", "dsc_0003.nef"]);
        assert!(items[0].is_pair());
        assert_eq!(items[0].secondary[0].kind, Kind::Jpeg);
        assert!(!items[1].is_pair());
        assert_eq!(
            items[0].sidecar_path(),
            dir.path().join("DSC_0001.XMP"),
            "uppercase source extension → uppercase sidecar, like Photo Mechanic"
        );
        assert_eq!(items[2].sidecar_path(), dir.path().join("dsc_0003.xmp"));
        // An existing sidecar of the other case wins, with the spelling it
        // has on disk, on the next scan: nothing watches the folder, so a
        // scan is what rebuilds a directory's listing.
        fs::write(dir.path().join("DSC_0001.xmp"), b"x").unwrap();
        let items = scan(dir.path(), &ScanOptions::default()).unwrap();
        assert_eq!(items[0].sidecar_path(), dir.path().join("DSC_0001.xmp"));
        assert_eq!(
            items.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn tiff_and_png_are_photos_text_is_not() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.tif", "b.TIFF", "c.png", "d.xmp", "e.txt", "f.mp4"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        let items = scan(dir.path(), &ScanOptions::default()).unwrap();
        let names: Vec<_> = items.iter().map(|i| i.display_name()).collect();
        assert_eq!(names, vec!["a.tif", "b.TIFF", "c.png"]);
        assert!(items.iter().all(|i| i.primary.kind.is_displayable_image()));
    }

    #[test]
    fn non_recursive_by_default() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        fs::write(dir.path().join("sub/b.jpg"), b"x").unwrap();
        assert_eq!(scan(dir.path(), &ScanOptions::default()).unwrap().len(), 1);
        assert_eq!(
            scan(dir.path(), &ScanOptions { recursive: true })
                .unwrap()
                .len(),
            2
        );
    }

    /// The inspector asks for a sidecar path every frame, so after the scan it
    /// must never list the directory again. Removing the directory has to
    /// leave the resolved names untouched, on-disk case included.
    #[test]
    fn sidecar_paths_come_from_the_scan_listing_not_the_filesystem() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("card");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("DSC_0001.NEF"), b"x").unwrap();
        fs::write(dir.join("DSC_0001.xmp"), b"x").unwrap();
        fs::write(dir.join("DSC_0002.JPG"), b"x").unwrap();
        fs::write(dir.join("DSC_0002.XMP"), b"x").unwrap();
        fs::write(dir.join("DSC_0003.jpg"), b"x").unwrap();
        let items = scan(&dir, &ScanOptions::default()).unwrap();
        assert_eq!(items[0].sidecar_path(), dir.join("DSC_0001.xmp"));
        assert_eq!(items[1].sidecar_path(), dir.join("DSC_0002.XMP"));
        assert!(items[0].has_sidecar() && items[1].has_sidecar());
        assert!(!items[2].has_sidecar(), "DSC_0003 has no sidecar");

        // Nothing can be listed from here on: the directory is gone.
        fs::rename(&dir, root.path().join("gone")).unwrap();
        assert!(!dir.exists());
        assert_eq!(items[0].sidecar_path(), dir.join("DSC_0001.xmp"));
        assert_eq!(items[1].sidecar_path(), dir.join("DSC_0002.XMP"));
        assert_eq!(items[2].sidecar_path(), dir.join("DSC_0003.xmp"));
        assert!(items[0].has_sidecar() && items[1].has_sidecar());
        assert!(!items[2].has_sidecar());
    }

    /// Our own writes register the name they created (marks and edits write
    /// through `apature_metadata::xmp`), so a new sidecar shows without a
    /// rescan.
    #[test]
    fn a_sidecar_we_write_is_known_without_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("DSC_0007.NEF"), b"x").unwrap();
        let items = scan(dir.path(), &ScanOptions::default()).unwrap();
        assert!(!items[0].has_sidecar());
        let sc = items[0].sidecar_path();
        assert_eq!(sc, dir.path().join("DSC_0007.XMP"));
        fs::write(&sc, b"x").unwrap();
        crate::sidecars::note_written(&sc);
        assert!(items[0].has_sidecar());
        assert_eq!(items[0].sidecar_path(), sc);
    }
}
