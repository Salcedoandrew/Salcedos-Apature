use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What kind of file a variant is. Only the kinds we can show previews for
/// are scanned; everything else is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Raw,
    Jpeg,
    /// A single-image TIFF or PNG. Neither carries an embedded JPEG preview,
    /// so the preview crate decodes the file itself.
    Tiff,
    Png,
    Heic,
}

impl Kind {
    /// True for the kinds the contact sheet lists. HEIC is recognised (so
    /// ingest still copies camera HEICs) but not listed: the preview decoder
    /// has no HEIC path, so its cells would be blank.
    pub fn is_displayable_image(self) -> bool {
        matches!(self, Kind::Raw | Kind::Jpeg | Kind::Tiff | Kind::Png)
    }
}

/// Map a file extension (any case) to a [`Kind`]. Unknown extensions return `None`.
pub fn kind_for_extension(ext: &str) -> Option<Kind> {
    match ext.to_ascii_lowercase().as_str() {
        "nef" | "nrw" | "cr2" | "cr3" | "crw" | "arw" | "srf" | "sr2" | "dng" | "raf" | "orf"
        | "rw2" | "rwl" | "pef" | "srw" | "3fr" | "fff" | "iiq" | "x3f" => Some(Kind::Raw),
        "jpg" | "jpeg" | "jpe" => Some(Kind::Jpeg),
        "tif" | "tiff" => Some(Kind::Tiff),
        "png" => Some(Kind::Png),
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
    /// case of the source extension), so with no sidecar yet the spelling
    /// follows the primary's extension. The spelling of one that exists comes
    /// from the directory listing [`crate::scan`] took, never from a guess: on
    /// a case-insensitive volume `IMG.XMP` and `IMG.xmp` are one file, and the
    /// wrong spelling is a different name on a case-sensitive destination.
    ///
    /// That lookup is memory only. The inspector asks for this once per frame,
    /// and a directory listing here read every file in the folder on the UI
    /// thread each time; only an item from a directory no scan has listed (one
    /// built by hand, say) still gets the one-off listing.
    pub fn sidecar_path(&self) -> PathBuf {
        let preferred = self.preferred_sidecar_path();
        if let Some(found) = crate::sidecars::resolve(&preferred) {
            return found;
        }
        match preferred.parent() {
            Some(parent) if crate::sidecars::listed(parent) => preferred,
            _ => existing_with_case(&preferred).unwrap_or(preferred),
        }
    }

    /// True when this item has a sidecar. The scan's listing answers this, so
    /// callers on the draw path can show the state without a `stat`; only an
    /// item from a directory no scan has listed falls back to the disk.
    pub fn has_sidecar(&self) -> bool {
        let p = self.primary.path.with_extension("xmp");
        match p.parent() {
            Some(parent) if crate::sidecars::listed(parent) => {
                crate::sidecars::resolve(&p).is_some()
            }
            _ => p.is_file(),
        }
    }

    /// The sidecar name with no listing taken into account: the spelling that
    /// follows the primary's extension case.
    fn preferred_sidecar_path(&self) -> PathBuf {
        let ext_is_upper = self
            .primary
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.chars().any(|c| c.is_ascii_uppercase()));
        self.primary
            .path
            .with_extension(if ext_is_upper { "XMP" } else { "xmp" })
    }

    pub fn variants(&self) -> impl Iterator<Item = &Variant> {
        std::iter::once(&self.primary).chain(self.secondary.iter())
    }
}

/// The directory entry whose name equals `p`'s file name ignoring ASCII case,
/// with its real spelling, preferring the exact spelling when both exist (a
/// case-sensitive directory can hold `IMG.XMP` and `IMG.xmp` at once); None
/// when there is none. Only for a directory no scan has listed: a scan puts
/// the same information in [`crate::sidecars`] without touching the disk.
fn existing_with_case(p: &Path) -> Option<PathBuf> {
    let parent = p.parent()?;
    let want = p.file_name()?.to_str()?;
    let mut folded = None;
    for e in std::fs::read_dir(parent).ok()?.flatten() {
        let name = e.file_name();
        let Some(s) = name.to_str() else { continue };
        if s == want {
            return Some(parent.join(name));
        }
        if folded.is_none() && s.eq_ignore_ascii_case(want) {
            folded = Some(parent.join(name));
        }
    }
    folded
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
        assert_eq!(kind_for_extension("TIF"), Some(Kind::Tiff));
        assert_eq!(kind_for_extension("tiff"), Some(Kind::Tiff));
        assert_eq!(kind_for_extension("png"), Some(Kind::Png));
        assert_eq!(kind_for_extension("heic"), Some(Kind::Heic));
        assert!(Kind::Raw.is_displayable_image());
        assert!(Kind::Png.is_displayable_image());
        assert!(
            !Kind::Heic.is_displayable_image(),
            "the decoder has no HEIC path"
        );
        assert_eq!(kind_for_extension("xmp"), None);
        assert_eq!(kind_for_extension("txt"), None);
    }
}
