//! XMP packets embedded in JPEG files (APP1 segment with the
//! `http://ns.adobe.com/xap/1.0/` signature).
//!
//! Photo Mechanic and Lightroom keep a JPEG's metadata *inside* the JPEG and
//! ignore sidecars for it, so for RAW+JPEG pairs and JPEG-only folders our
//! marks must live there too. The edit is a splice: every byte of the file
//! outside the XMP segment is copied unchanged, the packet itself is edited
//! with the same in-place rules as sidecars, and the file is replaced
//! atomically after a read-back check. Image data is never re-encoded.

use std::fs;
use std::io;
use std::path::Path;

use crate::xmp::{self, XmpError, XmpMarks};

pub const XMP_SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// Largest standard packet that fits one APP1 segment (65535 - 2 length
/// bytes - 29 signature bytes). Bigger packets need Extended XMP, which we
/// do not write.
pub const MAX_PACKET: usize = 65_504;
/// Most bytes after the marker and length that one JPEG segment can hold
/// (the 2-byte length counts itself, so 65535 - 2).
pub const MAX_SEGMENT_BODY: usize = 65_533;
/// Whitespace padding appended to a freshly created packet so later edits
/// by other tools can grow it in place (the XMP spec's recommendation).
const PADDING: usize = 2048;

#[derive(Debug, thiserror::Error)]
pub enum JpegXmpError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("not a JPEG file")]
    NotJpeg,
    #[error("malformed JPEG segment at byte {0}")]
    Malformed(usize),
    #[error("XMP packet is {0} bytes; more than {MAX_PACKET} needs Extended XMP, which is not supported")]
    PacketTooLarge(usize),
    #[error("the {0} block is {1} bytes, more than one JPEG segment holds ({MAX_SEGMENT_BODY}); shorten the caption, keywords or other long fields")]
    SegmentTooLarge(&'static str, usize),
    #[error("{0}")]
    Xmp(#[from] XmpError),
    #[error("embedded packet in {0} did not read back with the requested marks; original left untouched")]
    Verify(std::path::PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub marker: u8,
    /// Offset of the 0xFF byte.
    pub start: usize,
    /// Offset one past the segment's last byte.
    pub end: usize,
}

impl Segment {
    /// Bytes after the 2-byte length field.
    pub fn payload<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        data.get(self.start + 4..self.end).unwrap_or(&[])
    }
}

/// Walk the marker segments that precede the first scan.
pub fn header_segments(data: &[u8]) -> Result<Vec<Segment>, JpegXmpError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(JpegXmpError::NotJpeg);
    }
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return Err(JpegXmpError::Malformed(i));
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
            out.push(Segment {
                marker,
                start: i,
                end: i + 2,
            });
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 || i + 2 + len > data.len() {
            return Err(JpegXmpError::Malformed(i));
        }
        out.push(Segment {
            marker,
            start: i,
            end: i + 2 + len,
        });
        if marker == 0xDA {
            break;
        }
        i += 2 + len;
    }
    Ok(out)
}

fn is_xmp_segment(data: &[u8], s: &Segment) -> bool {
    s.marker == 0xE1
        && data
            .get(s.start + 4..s.end)
            .is_some_and(|p| p.starts_with(XMP_SIG))
}

pub fn is_exif_segment(data: &[u8], s: &Segment) -> bool {
    s.marker == 0xE1
        && data
            .get(s.start + 4..s.end)
            .is_some_and(|p| p.starts_with(b"Exif\0\0"))
}

/// The existing packet text, if any.
pub fn packet(data: &[u8]) -> Result<Option<&str>, JpegXmpError> {
    for s in header_segments(data)? {
        if is_xmp_segment(data, &s) {
            let body = &data[s.start + 4 + XMP_SIG.len()..s.end];
            return Ok(std::str::from_utf8(body).ok());
        }
    }
    Ok(None)
}

/// One marker segment: FF `marker`, length, `sig`, `body`. A body too big
/// for the 16-bit length is an error, never a wrapped length (which would
/// leave a corrupt file). `what` names the block in the error.
pub fn segment(
    marker: u8,
    what: &'static str,
    sig: &[u8],
    body: &[u8],
) -> Result<Vec<u8>, JpegXmpError> {
    let n = sig.len() + body.len();
    if n > MAX_SEGMENT_BODY {
        return Err(JpegXmpError::SegmentTooLarge(what, n));
    }
    let mut v = Vec::with_capacity(n + 4);
    v.extend_from_slice(&[0xFF, marker]);
    v.extend_from_slice(&((n + 2) as u16).to_be_bytes());
    v.extend_from_slice(sig);
    v.extend_from_slice(body);
    Ok(v)
}

fn build_segment(packet: &str) -> Result<Vec<u8>, JpegXmpError> {
    let body_len = XMP_SIG.len() + packet.len();
    if packet.len() > MAX_PACKET {
        return Err(JpegXmpError::PacketTooLarge(packet.len()));
    }
    let mut seg = Vec::with_capacity(body_len + 4);
    seg.extend_from_slice(&[0xFF, 0xE1]);
    seg.extend_from_slice(&((body_len + 2) as u16).to_be_bytes());
    seg.extend_from_slice(XMP_SIG);
    seg.extend_from_slice(packet.as_bytes());
    Ok(seg)
}

/// Return a copy of `data` with `packet` as its XMP segment: replacing the
/// existing one in place, or inserting after the Exif APP1 (else after the
/// JFIF APP0, else after SOI). All other bytes are copied unchanged.
pub fn with_packet(data: &[u8], packet: &str) -> Result<Vec<u8>, JpegXmpError> {
    let segs = header_segments(data)?;
    let seg = build_segment(packet)?;
    if let Some(existing) = segs.iter().find(|s| is_xmp_segment(data, s)) {
        let mut out = Vec::with_capacity(data.len() - (existing.end - existing.start) + seg.len());
        out.extend_from_slice(&data[..existing.start]);
        out.extend_from_slice(&seg);
        out.extend_from_slice(&data[existing.end..]);
        return Ok(out);
    }
    let after = segs
        .iter()
        .find(|s| is_exif_segment(data, s))
        .or_else(|| segs.iter().find(|s| s.marker == 0xE0))
        .map(|s| s.end)
        .unwrap_or(2);
    let mut out = Vec::with_capacity(data.len() + seg.len());
    out.extend_from_slice(&data[..after]);
    out.extend_from_slice(&seg);
    out.extend_from_slice(&data[after..]);
    Ok(out)
}

fn fresh_packet(marks: &XmpMarks) -> Result<String, XmpError> {
    let mut p = xmp::apply("", marks)?;
    // Pad before the trailer so other writers can edit in place.
    if let Some(idx) = p.rfind("<?xpacket end=") {
        p.insert_str(idx, &" ".repeat(PADDING));
    }
    Ok(p)
}

/// Give the rewritten file at `tmp` the modified, accessed and (macOS)
/// created dates of the JPEG it replaces, so embedding marks does not make a
/// photo look new to Finder, backups or date sorting.
fn keep_dates(original: &Path, tmp: &Path) -> std::io::Result<()> {
    let m = fs::metadata(original)?;
    #[allow(unused_mut)]
    let mut times = fs::FileTimes::new()
        .set_modified(m.modified()?)
        .set_accessed(m.accessed()?);
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::FileTimesExt;
        if let Ok(c) = m.created() {
            times = times.set_created(c);
        }
    }
    fs::OpenOptions::new()
        .write(true)
        .open(tmp)?
        .set_times(times)
}

/// Generic packet edit on the JPEG at `path`: `edit` receives the existing
/// packet (or an empty padded one), `verify` checks the packet read back from
/// the bytes about to be written. Atomic temp → fsync → rename.
pub fn write_with(
    path: &Path,
    edit: impl FnOnce(&str) -> Result<String, XmpError>,
    verify: impl FnOnce(&str) -> bool,
) -> Result<(), JpegXmpError> {
    let data = fs::read(path)?;
    let base = match packet(&data)? {
        Some(existing) => existing.to_string(),
        None => {
            let mut p = xmp::apply("", &XmpMarks::default())?;
            if let Some(idx) = p.rfind("<?xpacket end=") {
                p.insert_str(idx, &" ".repeat(PADDING));
            }
            p
        }
    };
    let new_packet = edit(&base)?;
    let out = with_packet(&data, &new_packet)?;
    if out == data {
        return Ok(());
    }
    let back = packet(&out)?.unwrap_or("");
    if !verify(back) {
        return Err(JpegXmpError::Verify(path.to_path_buf()));
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.xmp-tmp{}", std::process::id()));
    let result = (|| {
        fs::write(&tmp, &out)?;
        fs::File::open(&tmp)?.sync_all()?;
        keep_dates(path, &tmp)?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

/// Write `marks` into the JPEG at `path`, editing an existing packet in place
/// or adding one. Atomic: temp file in the same directory, fsync, rename; the
/// original is untouched unless the new file reads back correctly.
pub fn write_marks(path: &Path, marks: &XmpMarks) -> Result<(), JpegXmpError> {
    let data = fs::read(path)?;
    let new_packet = match packet(&data)? {
        Some(existing) => xmp::apply(existing, marks)?,
        None => fresh_packet(marks)?,
    };
    let out = with_packet(&data, &new_packet)?;
    if out == data {
        return Ok(());
    }
    // Read-back check on the bytes we are about to write.
    let back = packet(&out)?.map(xmp::parse).unwrap_or_default();
    if !xmp::satisfies(marks, &back) {
        return Err(JpegXmpError::Verify(path.to_path_buf()));
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.xmp-tmp{}", std::process::id()));
    let result = (|| {
        fs::write(&tmp, &out)?;
        fs::File::open(&tmp)?.sync_all()?;
        keep_dates(path, &tmp)?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_length_never_wraps() {
        let body = vec![b'x'; MAX_SEGMENT_BODY - 4];
        let seg = segment(0xE1, "XMP", b"sig\0", &body).unwrap();
        assert_eq!(u16::from_be_bytes([seg[2], seg[3]]), 0xFFFF);
        assert_eq!(seg.len(), 2 + 0xFFFF);
        assert!(matches!(
            segment(0xE1, "XMP", b"sig\0", &[body.as_slice(), b"y"].concat()),
            Err(JpegXmpError::SegmentTooLarge("XMP", 65_534))
        ));
    }

    /// Structurally valid JPEG skeleton: SOI, APP0, DQT, SOF0, SOS, scan bytes, EOI.
    fn skeleton(with_exif: bool) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        v.extend_from_slice(b"JFIF\0\x01\x01\x01\0\x48\0\x48\0\0");
        if with_exif {
            let tiff = b"II*\0\x08\0\0\0\0\0";
            let len = 2 + 6 + tiff.len();
            v.extend_from_slice(&[0xFF, 0xE1]);
            v.extend_from_slice(&(len as u16).to_be_bytes());
            v.extend_from_slice(b"Exif\0\0");
            v.extend_from_slice(tiff);
        }
        v.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0x00, 0x01]);
        v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0]);
        v.extend_from_slice(&[0x12, 0x34, 0x56, 0xFF, 0x00, 0x78]);
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    fn marks() -> XmpMarks {
        XmpMarks {
            rating: Some(4),
            label: Some("Red".into()),
            color_class: Some(1),
            tagged: Some(true),
        }
    }

    #[test]
    fn inserts_after_exif_and_preserves_every_other_byte() {
        let src = skeleton(true);
        let out = with_packet(&src, &fresh_packet(&marks()).unwrap()).unwrap();
        let segs = header_segments(&out).unwrap();
        let idx = segs.iter().position(|s| is_xmp_segment(&out, s)).unwrap();
        assert!(
            is_exif_segment(&out, &segs[idx - 1]),
            "XMP goes right after Exif"
        );
        let xmp_seg = segs[idx];
        let mut stripped = out[..xmp_seg.start].to_vec();
        stripped.extend_from_slice(&out[xmp_seg.end..]);
        assert_eq!(
            stripped, src,
            "everything outside the new segment is unchanged"
        );
        assert_eq!(xmp::parse(packet(&out).unwrap().unwrap()), marks());
    }

    #[test]
    fn inserts_after_app0_without_exif_and_replaces_in_place() {
        let src = skeleton(false);
        let out = with_packet(&src, &fresh_packet(&marks()).unwrap()).unwrap();
        let segs = header_segments(&out).unwrap();
        assert_eq!(segs[0].marker, 0xE0, "SOI is not listed; APP0 comes first");
        assert!(is_xmp_segment(&out, &segs[1]));
        // Second write replaces rather than adds.
        let m2 = XmpMarks {
            rating: Some(2),
            ..marks()
        };
        let out2 = with_packet(
            &out,
            &xmp::apply(packet(&out).unwrap().unwrap(), &m2).unwrap(),
        )
        .unwrap();
        let n = header_segments(&out2)
            .unwrap()
            .iter()
            .filter(|s| is_xmp_segment(&out2, s))
            .count();
        assert_eq!(n, 1);
        assert_eq!(xmp::parse(packet(&out2).unwrap().unwrap()).rating, Some(2));
        assert_eq!(
            &out2[out2.len() - 8..],
            &src[src.len() - 8..],
            "scan data and EOI intact"
        );
    }

    #[test]
    fn rejects_non_jpeg_and_oversize_packets() {
        assert!(matches!(
            with_packet(b"II*\0", "x"),
            Err(JpegXmpError::NotJpeg)
        ));
        let big = "x".repeat(MAX_PACKET + 1);
        assert!(matches!(
            with_packet(&skeleton(false), &big),
            Err(JpegXmpError::PacketTooLarge(_))
        ));
        let truncated = &skeleton(true)[..15];
        assert!(matches!(
            header_segments(truncated),
            Err(JpegXmpError::Malformed(_))
        ));
    }

    #[test]
    fn write_is_atomic_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("IMG_0001.JPG");
        fs::write(&p, skeleton(true)).unwrap();
        write_marks(&p, &marks()).unwrap();
        let first = fs::read(&p).unwrap();
        assert_eq!(xmp::parse(packet(&first).unwrap().unwrap()), marks());
        write_marks(&p, &marks()).unwrap();
        assert_eq!(fs::read(&p).unwrap(), first, "same marks → no rewrite");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file left"
        );
    }

    /// Real camera JPEG + exiftool, when both are available.
    #[test]
    fn exiftool_reads_marks_from_a_real_jpeg() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/850_7783.JPG");
        if !fixture.is_file()
            || std::process::Command::new("exiftool")
                .arg("-ver")
                .output()
                .is_err()
        {
            eprintln!("SKIPPED: fixture or exiftool missing");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("850_7783.JPG");
        fs::copy(&fixture, &p).unwrap();
        let before = fs::metadata(&p).unwrap().len();
        write_marks(&p, &marks()).unwrap();
        let o = std::process::Command::new("exiftool")
            .args([
                "-validate",
                "-warning",
                "-error",
                "-s",
                "-Rating",
                "-Label",
                "-ColorClass",
                "-Tagged",
                "-ImageSize",
                "-Model",
            ])
            .arg(&p)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&o.stdout).to_string();
        assert!(!text.contains("Error"), "{text}");
        for want in [
            "Rating                          : 4",
            "Label                           : Red",
            "ColorClass                      : 1",
            "Tagged                          : Yes",
            "ImageSize                       : 8256x5504",
            "Model                           : NIKON D850",
        ] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        let after = fs::metadata(&p).unwrap().len();
        assert!(
            after > before && after - before < 4096,
            "only a small packet was added: {before} → {after}"
        );
    }
}
