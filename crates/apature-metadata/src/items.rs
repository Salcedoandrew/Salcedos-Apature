//! One policy for where an item's marks and captions live:
//!
//! * a RAW's marks live in its `.xmp` sidecar;
//! * by default ([`JpegMarks::Sidecar`]) a JPEG's marks live in a sidecar
//!   too (`name.xmp`, or the RAW's sidecar for a RAW+JPEG pair), so marking
//!   never rewrites an image file;
//! * with [`JpegMarks::Embed`] (opt-in) they also go inside each JPEG, the
//!   way Photo Mechanic embeds them, keeping the file's dates.
//!
//! Reading merges field by field and the sidecar wins: marks embedded by an
//! earlier version (or another app) still show until the sidecar says
//! otherwise.

use std::path::PathBuf;

use apature_core::{ColorClassScheme, Kind, Marks, PhotoItem};

use crate::iptc::{self, Iptc};
use crate::jpeg_xmp;
use crate::xmp::{self, Conflict, XmpMarks};

#[derive(Debug, thiserror::Error)]
pub enum ItemError {
    #[error("sidecar {path}: {source}")]
    Sidecar {
        path: PathBuf,
        source: xmp::XmpError,
    },
    #[error("jpeg {path}: {source}")]
    Jpeg {
        path: PathBuf,
        source: jpeg_xmp::JpegXmpError,
    },
}

/// Where marks and captions for JPEG files are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JpegMarks {
    /// In the XMP sidecar only; JPEG files are never rewritten.
    #[default]
    Sidecar,
    /// Also inside each JPEG (dates kept); an existing sidecar is updated
    /// too so it never goes stale.
    Embed,
}

/// Everything readable about an item's marks, before any precedence is applied.
#[derive(Debug, Default, Clone)]
pub struct MarkSources {
    pub sidecar: Option<XmpMarks>,
    /// One entry per JPEG variant that carries a packet.
    pub embedded: Vec<(PathBuf, XmpMarks)>,
}

impl MarkSources {
    /// Precedence, field by field: the sidecar, then the first JPEG packet,
    /// then unmarked. Colour class and label travel together. Unreadable
    /// sources count as absent.
    pub fn effective(&self, _item: &PhotoItem, scheme: &ColorClassScheme) -> Marks {
        let emb = self.embedded.first().map(|(_, m)| m);
        let merged = match (&self.sidecar, emb) {
            (None, None) => return Marks::default(),
            (Some(s), None) => s.clone(),
            (None, Some(e)) => e.clone(),
            (Some(s), Some(e)) => {
                let side_class = s.color_class.is_some() || s.label.is_some();
                XmpMarks {
                    rating: s.rating.or(e.rating),
                    label: if side_class {
                        s.label.clone()
                    } else {
                        e.label.clone()
                    },
                    color_class: if side_class {
                        s.color_class
                    } else {
                        e.color_class
                    },
                    tagged: s.tagged.or(e.tagged),
                }
            }
        };
        merged.to_marks(scheme)
    }

    /// Disagreements between the sidecar and any embedded packet. Only in
    /// [`JpegMarks::Embed`] mode, where both are meant to agree; by default
    /// the sidecar is the record and older embedded marks are just history.
    pub fn conflicts(&self, policy: JpegMarks) -> Vec<(PathBuf, Conflict)> {
        let Some(side) = &self.sidecar else {
            return Vec::new();
        };
        if policy == JpegMarks::Sidecar {
            return Vec::new();
        }
        self.embedded
            .iter()
            .flat_map(|(p, emb)| {
                xmp::conflicts(side, emb)
                    .into_iter()
                    .map(move |c| (p.clone(), c))
            })
            .collect()
    }
}

/// Read every source for an item. I/O errors on individual files are
/// treated as "no marks there"; a damaged JPEG must not hide the sidecar.
pub fn read(item: &PhotoItem) -> MarkSources {
    let sidecar = xmp::read_sidecar(&item.sidecar_path()).ok().flatten();
    let embedded = item
        .variants()
        .filter(|v| v.kind == Kind::Jpeg)
        .filter_map(|v| {
            xmp::read_embedded(&v.path)
                .ok()
                .flatten()
                .map(|m| (v.path.clone(), m))
        })
        .collect();
    MarkSources { sidecar, embedded }
}

/// Convenience: the marks the app should show.
pub fn read_marks(item: &PhotoItem, scheme: &ColorClassScheme) -> Marks {
    read(item).effective(item, scheme)
}

/// True when `policy` sends this item's writes to its sidecar: always, except
/// a JPEG-only item in [`JpegMarks::Embed`] mode with no sidecar yet.
fn uses_sidecar(item: &PhotoItem, policy: JpegMarks) -> bool {
    policy == JpegMarks::Sidecar
        || item.variants().any(|v| v.kind == Kind::Raw)
        || item.sidecar_path().is_file()
}

/// Write marks everywhere `policy` says they belong. Returns the paths
/// written. Stops at the first failure; paths written before it stay written
/// (each write is itself atomic), and the error names the file.
pub fn write(
    item: &PhotoItem,
    marks: &Marks,
    scheme: &ColorClassScheme,
    policy: JpegMarks,
) -> Result<Vec<PathBuf>, ItemError> {
    let xm = XmpMarks::from_marks(marks, scheme);
    let mut written = Vec::new();
    if uses_sidecar(item, policy) {
        let p = item.sidecar_path();
        xmp::write_marks(&p, &xm).map_err(|source| ItemError::Sidecar {
            path: p.clone(),
            source,
        })?;
        written.push(p);
    }
    if policy == JpegMarks::Sidecar {
        return Ok(written);
    }
    for v in item.variants().filter(|v| v.kind == Kind::Jpeg) {
        jpeg_xmp::write_marks(&v.path, &xm).map_err(|source| ItemError::Jpeg {
            path: v.path.clone(),
            source,
        })?;
        written.push(v.path.clone());
    }
    Ok(written)
}

/// Read IPTC for an item: the first JPEG packet with the sidecar's fields
/// laid over it (the sidecar wins field by field). Empty when nothing is
/// there.
pub fn read_iptc(item: &PhotoItem) -> Iptc {
    let side_text = std::fs::read_to_string(item.sidecar_path()).ok();
    let from_jpeg = || {
        item.variants()
            .filter(|v| v.kind == Kind::Jpeg)
            .find_map(|v| {
                let data = std::fs::read(&v.path).ok()?;
                jpeg_xmp::packet(&data).ok().flatten().map(iptc::parse)
            })
    };
    match (from_jpeg(), side_text) {
        (Some(j), Some(t)) => {
            // Sidecar fields win, including ones cleared on purpose.
            let side = iptc::parse(&t);
            let mut m = j.overlay(&side);
            for f in iptc::cleared_with(&t, &side) {
                m.clear(f);
            }
            m
        }
        (_, Some(t)) => iptc::parse(&t),
        (j, None) => j.unwrap_or_default(),
    }
}

/// Write IPTC everywhere marks go under `policy`.
pub fn write_iptc(
    item: &PhotoItem,
    fields: &Iptc,
    policy: JpegMarks,
) -> Result<Vec<PathBuf>, ItemError> {
    let mut written = Vec::new();
    if uses_sidecar(item, policy) {
        let p = item.sidecar_path();
        xmp::write_sidecar_with(
            &p,
            |t| iptc::apply(t, fields),
            |t| iptc::satisfies(fields, &iptc::parse(t)),
        )
        .map_err(|source| ItemError::Sidecar {
            path: p.clone(),
            source,
        })?;
        written.push(p);
    }
    if policy == JpegMarks::Sidecar {
        return Ok(written);
    }
    for v in item.variants().filter(|v| v.kind == Kind::Jpeg) {
        jpeg_xmp::write_with(
            &v.path,
            |t| iptc::apply(t, fields),
            |t| iptc::satisfies(fields, &iptc::parse(t)),
        )
        .map_err(|source| ItemError::Jpeg {
            path: v.path.clone(),
            source,
        })?;
        written.push(v.path.clone());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::{ColorClass, Rating, ScanOptions};
    use std::fs;

    fn jpeg_skeleton() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(b"JFIF\0\x01\x01\x01\0\x48\0\x48\0\0");
        v.extend_from_slice(&[
            0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0, 0x12, 0xFF, 0xD9,
        ]);
        v
    }

    #[test]
    fn pair_gets_sidecar_and_embedded_and_reads_back_consistently() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("DSC_0001.NEF"), b"not a real nef").unwrap();
        fs::write(dir.path().join("DSC_0001.JPG"), jpeg_skeleton()).unwrap();
        fs::write(dir.path().join("DSC_0002.JPG"), jpeg_skeleton()).unwrap();
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        let scheme = ColorClassScheme::default();
        let marks = Marks {
            rating: Rating(3),
            color_class: ColorClass(2),
            tagged: true,
        };

        let written = write(&items[0], &marks, &scheme, JpegMarks::Embed).unwrap();
        assert_eq!(written.len(), 2);
        assert!(written[0].ends_with("DSC_0001.XMP"));
        assert!(written[1].ends_with("DSC_0001.JPG"));
        let src = read(&items[0]);
        assert_eq!(src.sidecar.as_ref().unwrap().rating, Some(3));
        assert_eq!(src.embedded.len(), 1);
        assert!(src.conflicts(JpegMarks::Embed).is_empty());
        assert_eq!(src.effective(&items[0], &scheme), marks);

        // JPEG-only item: embedded only, no sidecar created.
        let written = write(&items[1], &marks, &scheme, JpegMarks::Embed).unwrap();
        assert_eq!(written, vec![dir.path().join("DSC_0002.JPG")]);
        assert!(!dir.path().join("DSC_0002.xmp").exists());
        assert_eq!(read_marks(&items[1], &scheme), marks);

        // IPTC goes to both files of the pair and reads back from the sidecar.
        let fields = Iptc {
            description: Some("A caption".into()),
            keywords: Some(vec!["k1".into(), "k2".into()]),
            ..Default::default()
        };
        let w = write_iptc(&items[0], &fields, JpegMarks::Embed).unwrap();
        assert_eq!(w.len(), 2);
        let back = read_iptc(&items[0]);
        assert_eq!(back.description.as_deref(), Some("A caption"));
        assert_eq!(back.keywords, Some(vec!["k1".into(), "k2".into()]));
        assert_eq!(
            read_marks(&items[0], &scheme),
            marks,
            "marks survive an IPTC write"
        );
        let jpg_only = read_iptc(&items[1]);
        assert!(jpg_only.is_empty());

        // A later sidecar edit by another app wins for the pair, and is reported.
        xmp::write_marks(
            &items[0].sidecar_path(),
            &XmpMarks {
                rating: Some(5),
                ..Default::default()
            },
        )
        .unwrap();
        let src = read(&items[0]);
        assert_eq!(src.effective(&items[0], &scheme).rating, Rating(5));
        assert_eq!(src.conflicts(JpegMarks::Embed).len(), 1);
        assert_eq!(src.conflicts(JpegMarks::Embed)[0].1.field, "rating");
    }

    /// Set a fixed, old modified date so "unchanged" can't pass by luck.
    fn age(path: &std::path::Path) -> std::time::SystemTime {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
        t
    }

    fn snapshot(path: &std::path::Path) -> (Vec<u8>, std::time::SystemTime) {
        (
            fs::read(path).unwrap(),
            fs::metadata(path).unwrap().modified().unwrap(),
        )
    }

    #[test]
    fn marking_a_jpeg_by_default_leaves_it_untouched_and_uses_a_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("DSC_0002.JPG");
        let nef = dir.path().join("DSC_0001.NEF");
        let pair_jpg = dir.path().join("DSC_0001.JPG");
        fs::write(&nef, b"not a real nef").unwrap();
        fs::write(&pair_jpg, jpeg_skeleton()).unwrap();
        fs::write(&jpg, jpeg_skeleton()).unwrap();
        age(&jpg);
        age(&pair_jpg);
        let before = (snapshot(&jpg), snapshot(&pair_jpg));
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        let scheme = ColorClassScheme::default();
        let marks = Marks {
            rating: Rating(4),
            color_class: ColorClass(3),
            tagged: true,
        };
        let pad = Iptc {
            headline: Some("Opening day".into()),
            ..Default::default()
        };
        for it in &items {
            write(it, &marks, &scheme, JpegMarks::default()).unwrap();
            write_iptc(it, &pad, JpegMarks::default()).unwrap();
        }
        assert_eq!(
            (snapshot(&jpg), snapshot(&pair_jpg)),
            before,
            "JPEG bytes and dates unchanged"
        );
        // The standalone JPEG got its own sidecar; the pair used the RAW's.
        assert!(dir.path().join("DSC_0002.XMP").is_file());
        assert!(dir.path().join("DSC_0001.XMP").is_file());
        for it in &items {
            assert_eq!(read_marks(it, &scheme), marks, "round trip");
            assert_eq!(read_iptc(it).headline.as_deref(), Some("Opening day"));
            assert!(read(it).embedded.is_empty());
        }
    }

    #[test]
    fn sidecar_wins_over_embedded_field_by_field() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("IMG_1.jpg");
        fs::write(&jpg, jpeg_skeleton()).unwrap();
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        let scheme = ColorClassScheme::default();
        // Marks an earlier version embedded: they still show.
        jpeg_xmp::write_marks(
            &jpg,
            &XmpMarks {
                rating: Some(2),
                tagged: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read_marks(&items[0], &scheme).rating, Rating(2));
        // A sidecar with only a rating (say from another app) wins for the
        // rating; the embedded tag still shows.
        xmp::write_marks(
            &dir.path().join("IMG_1.xmp"),
            &XmpMarks {
                rating: Some(5),
                ..Default::default()
            },
        )
        .unwrap();
        let m = read_marks(&items[0], &scheme);
        assert_eq!((m.rating, m.tagged), (Rating(5), true));
    }

    #[test]
    fn embedding_is_opt_in_and_keeps_the_dates() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("IMG_2.JPG");
        fs::write(&jpg, jpeg_skeleton()).unwrap();
        let old = age(&jpg);
        // Birth-time preservation is macOS-only by design (see
        // `jpeg_xmp::keep_dates`), so sample it only where it can be checked.
        #[cfg(target_os = "macos")]
        let created = fs::metadata(&jpg).unwrap().created().ok();
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        let scheme = ColorClassScheme::default();
        let marks = Marks {
            rating: Rating(3),
            ..Default::default()
        };
        let w = write(&items[0], &marks, &scheme, JpegMarks::Embed).unwrap();
        assert_eq!(w, vec![jpg.clone()], "no sidecar is invented in embed mode");
        assert_ne!(fs::read(&jpg).unwrap(), jpeg_skeleton(), "marks went in");
        let m = fs::metadata(&jpg).unwrap();
        assert_eq!(m.modified().unwrap(), old);
        // `keep_dates` sets creation time on macOS only; elsewhere the
        // atomic temp-file-and-rename yields a new inode with a fresh birth
        // time, so assert it only where the behavior is implemented.
        #[cfg(target_os = "macos")]
        if let (Some(c), Ok(now)) = (created, m.created()) {
            assert_eq!(now, c);
        }
        assert_eq!(read_marks(&items[0], &scheme), marks);
    }

    #[test]
    fn clearing_embedded_caption_stays_cleared_and_jpeg_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("IMG_3.JPG");
        let nef = dir.path().join("DSC_7.NEF");
        let pair = dir.path().join("DSC_7.JPG");
        fs::write(&nef, b"not a real nef").unwrap();
        // Captions another app (or an older Apature) embedded in the JPEGs.
        let old = Iptc {
            headline: Some("Old headline".into()),
            description: Some("Old caption".into()),
            keywords: Some(vec!["a".into(), "b".into()]),
            copyright: Some("© Kept".into()),
            ..Default::default()
        };
        for p in [&jpg, &pair] {
            fs::write(p, jpeg_skeleton()).unwrap();
            jpeg_xmp::write_with(p, |t| iptc::apply(t, &old), |_| true).unwrap();
        }
        let before = (fs::read(&jpg).unwrap(), fs::read(&pair).unwrap());
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        assert_eq!(items.len(), 2);
        for it in &items {
            assert_eq!(read_iptc(it).description.as_deref(), Some("Old caption"));
        }
        // The user clears caption, headline and keywords (what the Caption
        // workspace sends: empty values).
        let clear = Iptc {
            headline: Some(String::new()),
            description: Some(String::new()),
            keywords: Some(Vec::new()),
            ..Default::default()
        };
        for it in &items {
            write_iptc(it, &clear, JpegMarks::Sidecar).unwrap();
        }
        // Reopen the folder: still cleared, untouched fields still embedded.
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        for it in &items {
            let back = read_iptc(it);
            assert_eq!(back.description, None, "{}", it.display_name());
            assert_eq!(back.headline, None);
            assert!(back.keywords.unwrap_or_default().is_empty());
            assert_eq!(back.copyright.as_deref(), Some("© Kept"));
        }
        assert_eq!((fs::read(&jpg).unwrap(), fs::read(&pair).unwrap()), before);
        // Typing a new caption later replaces the explicit clear.
        let again = Iptc {
            description: Some("New caption".into()),
            ..Default::default()
        };
        write_iptc(&items[0], &again, JpegMarks::Sidecar).unwrap();
        assert_eq!(
            read_iptc(&items[0]).description.as_deref(),
            Some("New caption")
        );
    }

    #[test]
    fn no_conflict_marker_by_default_when_the_sidecar_has_moved_on() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("DSC_1.NEF"), b"nef").unwrap();
        let jpg = dir.path().join("DSC_1.JPG");
        fs::write(&jpg, jpeg_skeleton()).unwrap();
        jpeg_xmp::write_marks(
            &jpg,
            &XmpMarks {
                rating: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        let items = apature_core::scan(dir.path(), &ScanOptions::default()).unwrap();
        let scheme = ColorClassScheme::default();
        let marks = Marks {
            rating: Rating(5),
            ..Default::default()
        };
        write(&items[0], &marks, &scheme, JpegMarks::Sidecar).unwrap();
        let src = read(&items[0]);
        assert_eq!(src.effective(&items[0], &scheme).rating, Rating(5));
        assert!(src.conflicts(JpegMarks::Sidecar).is_empty());
        assert_eq!(src.conflicts(JpegMarks::Embed).len(), 1);
    }
}
