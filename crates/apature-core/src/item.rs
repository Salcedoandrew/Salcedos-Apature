use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What kind of file a variant is. Only the kinds we can show previews for
/// are scanned; everything else is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Raw,
    Jpeg,
    Heic,
}

/// Map a file extension (any case) to a [`Kind`]. Unknown extensions return `None`.
pub fn kind_for_extension(ext: &str) -> Option<Kind> {
    match ext.to_ascii_lowercase().as_str() {
        "nef" | "nrw" | "cr2" | "cr3" | "crw" | "arw" | "srf" | "sr2" | "dng" | "raf" | "orf"
        | "rw2" | "rwl" | "pef" | "srw" | "3fr" | "fff" | "iiq" | "x3f" => Some(Kind::Raw),
        "jpg" | "jpeg" | "jpe" => Some(Kind::Jpeg),
        "heic" | "heif" | "hif" => Some(Kind::Heic),
        _ => None,
    }
}

/// One physical file on disk.
#[derive(Clone, Debug)]
pub struct Variant {
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Variant {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// A logical photo: a RAW+JPEG pair collapses into one item, the way Photo
/// Mechanic shows a single contact-sheet cell for `DSC_0001.NEF` + `DSC_0001.JPG`.
///
/// `primary` is the RAW when one exists, otherwise the JPEG/HEIC. `secondary`
/// holds the other variants with the same stem in the same directory.
#[derive(Clone, Debug)]
pub struct PhotoItem {
    pub id: usize,
    pub stem: String,
    pub primary: Variant,
    pub secondary: Vec<Variant>,
}

impl PhotoItem {
    pub fn path(&self) -> &Path {
        &self.primary.path
    }

    pub fn display_name(&self) -> String {
        self.primary.file_name()
    }

    pub fn is_pair(&self) -> bool {
        !self.secondary.is_empty()
    }

    /// Where the XMP sidecar for this item lives: same directory, same stem,
    /// `.xmp` extension. This matches Photo Mechanic and Lightroom.
    ///
    /// Case: Photo Mechanic writes `IMG.XMP` next to `IMG.NEF` (it copies the
    /// case of the source extension). An existing sidecar of either case is
    /// used as-is so we never create a second file on a case-sensitive
    /// filesystem; when none exists, the case follows the primary's extension.
    pub fn sidecar_path(&self) -> PathBuf {
        let lower = self.primary.path.with_extension("xmp");
        let upper = self.primary.path.with_extension("XMP");
        let ext_is_upper = self
            .primary
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.chars().any(|c| c.is_ascii_uppercase()))
            .unwrap_or(false);
        let (preferred, other) = if ext_is_upper {
            (upper, lower)
        } else {
            (lower, upper)
        };
        // Exact probes first: on a case-sensitive filesystem a hit *is* the
        // real spelling, and on a case-insensitive one any spelling opens
        // the same file, so either hit can be answered directly.
        if preferred.is_file() {
            return preferred;
        }
        if other.is_file() {
            return other;
        }
        // Neither spelling is there. A sidecar can still exist under a
        // third spelling (IMG.Xmp) that only a directory scan finds; the
        // scan used to run for every item that *has* a sidecar, and now
        // runs only on this miss path.
        if let Some(real) = existing_with_case(&preferred) {
            return real;
        }
        preferred
    }

    pub fn variants(&self) -> impl Iterator<Item = &Variant> {
        std::iter::once(&self.primary).chain(self.secondary.iter())
    }
}

/// The directory entry whose name equals `p`'s file name ignoring ASCII case,
/// with its real spelling; None when there is none.
fn existing_with_case(p: &Path) -> Option<PathBuf> {
    let parent = p.parent()?;
    let want = p.file_name()?.to_str()?;
    for e in std::fs::read_dir(parent).ok()?.flatten() {
        let name = e.file_name();
        if name.to_str().is_some_and(|s| s.eq_ignore_ascii_case(want)) {
            return Some(parent.join(name));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_kinds() {
        assert_eq!(kind_for_extension("NEF"), Some(Kind::Raw));
        for ext in ["CRW", "srf", "SR2", "rwl", "FFF", "x3f"] {
            assert_eq!(kind_for_extension(ext), Some(Kind::Raw), "{ext}");
        }
        assert_eq!(kind_for_extension("jpg"), Some(Kind::Jpeg));
        assert_eq!(kind_for_extension("Jpeg"), Some(Kind::Jpeg));
        assert_eq!(kind_for_extension("heic"), Some(Kind::Heic));
        assert_eq!(kind_for_extension("xmp"), None);
        assert_eq!(kind_for_extension("txt"), None);
    }
}
