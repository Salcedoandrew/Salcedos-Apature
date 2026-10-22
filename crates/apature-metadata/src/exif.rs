use std::fs::File;
use std::path::Path;

use apature_core::tiff::{tag, Tiff};

/// The EXIF fields worth showing on a contact sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExifSummary {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// "YYYY:MM:DD HH:MM:SS" as written by the camera.
    pub date_time_original: Option<String>,
    /// EXIF orientation 1..=8; 1 when absent.
    pub orientation: u32,
    pub exposure_time: Option<(u32, u32)>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f64>,
}

impl ExifSummary {
    /// e.g. "1/640 · f/2.8 · ISO 400 · 70mm"
    pub fn exposure_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some((n, d)) = self.exposure_time {
            if n != 0 && d != 0 {
                if n >= d {
                    parts.push(format!("{}s", trim_float(n as f64 / d as f64)));
                } else {
                    parts.push(format!("1/{}", trim_float(d as f64 / n as f64)));
                }
            }
        }
        if let Some(f) = self.f_number {
            parts.push(format!("f/{}", trim_float(f)));
        }
        if let Some(iso) = self.iso {
            parts.push(format!("ISO {iso}"));
        }
        if let Some(fl) = self.focal_length_mm {
            parts.push(format!("{}mm", trim_float(fl)));
        }
        parts.join(" · ")
    }

    pub fn camera_line(&self) -> String {
        match (&self.make, &self.model) {
            (Some(mk), Some(md)) if !md.to_lowercase().starts_with(&mk.to_lowercase()) => {
                format!("{mk} {md}")
            }
            (_, Some(md)) => md.clone(),
            (Some(mk), None) => mk.clone(),
            _ => String::new(),
        }
    }
}

fn trim_float(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExifError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("no EXIF/TIFF structure found")]
    NotFound,
}

/// Read the summary from a file (RAW/TIFF or JPEG).
pub fn read(path: &Path) -> Result<ExifSummary, ExifError> {
    let file = File::open(path)?;
    // SAFETY: read-only mapping, see apature_preview::Source for the rationale.
    let map = unsafe { memmap2::Mmap::map(&file)? };
    from_bytes(&map).ok_or(ExifError::NotFound)
}

/// Read the summary from in-memory bytes. Returns `None` if no TIFF header
/// can be found (neither a TIFF container nor a JPEG with an Exif APP1).
pub fn from_bytes(data: &[u8]) -> Option<ExifSummary> {
    // Fujifilm RAF keeps its Exif (orientation included) in the embedded
    // JPEG named by the big-endian offset and length at bytes 84 and 88.
    if let Some(jpeg) = raf_jpeg(data) {
        return from_bytes(jpeg);
    }
    if let Some(tiff_bytes) = exif_segment_in_jpeg(data) {
        return from_tiff(tiff_bytes);
    }
    if let Some(s) = from_cr3(data) {
        return Some(s);
    }
    from_tiff(data)
}

/// Canon CR3 is an ISO-BMFF file: IFD0 (make, model, orientation) is a
/// whole TIFF stream in the `CMT1` box and the Exif IFD is one in `CMT2`,
/// both inside the Canon `uuid` box near the start of `moov`.
fn from_cr3(data: &[u8]) -> Option<ExifSummary> {
    if data.get(4..8)? != b"ftyp" || data.get(8..12)? != b"crx " {
        return None;
    }
    let boxed = |name: &[u8; 4]| -> Option<&[u8]> {
        let head = &data[..data.len().min(1 << 20)];
        let at = head.windows(4).position(|w| w == name)?;
        let size = u32::from_be_bytes(data.get(at.checked_sub(4)?..at)?.try_into().ok()?) as usize;
        data.get(at + 4..(at - 4).checked_add(size)?)
    };
    let mut s = boxed(b"CMT1").and_then(from_tiff).unwrap_or(ExifSummary {
        orientation: 1,
        ..Default::default()
    });
    let exif = boxed(b"CMT2")?;
    let (t, first) = Tiff::parse(exif).ok()?;
    read_exif_ifd(&t, first, &mut s);
    Some(s)
}

fn raf_jpeg(data: &[u8]) -> Option<&[u8]> {
    if !data.starts_with(b"FUJIFILMCCD-RAW ") {
        return None;
    }
    let be = |at: usize| -> Option<usize> {
        Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize)
    };
    let (off, len) = (be(84)?, be(88)?);
    let jpeg = data.get(off..off.checked_add(len)?)?;
    jpeg.starts_with(&[0xFF, 0xD8]).then_some(jpeg)
}

fn from_tiff(data: &[u8]) -> Option<ExifSummary> {
    let (t, first) = Tiff::parse(data).ok()?;
    let ifd0 = t.read_ifd(first).ok()?;
    let mut s = ExifSummary {
        orientation: 1,
        ..Default::default()
    };
    s.make = ifd0
        .get(tag::MAKE)
        .and_then(|e| t.ascii(e))
        .filter(|v| !v.is_empty());
    s.model = ifd0
        .get(tag::MODEL)
        .and_then(|e| t.ascii(e))
        .filter(|v| !v.is_empty());
    if let Some(o) = ifd0.get(tag::ORIENTATION).and_then(|e| t.uint(e, 0)) {
        if (1..=8).contains(&o) {
            s.orientation = o as u32;
        }
    }
    if let Some(exif_off) = ifd0.get(tag::EXIF_IFD).and_then(|e| t.uint(e, 0)) {
        read_exif_ifd(&t, exif_off as u32, &mut s);
    }
    Some(s)
}

/// Date, exposure, ISO, focal length and lens from the Exif IFD at `at`.
fn read_exif_ifd(t: &Tiff, at: u32, s: &mut ExifSummary) {
    if let Ok(exif) = t.read_ifd(at) {
        s.date_time_original = exif
            .get(tag::DATE_TIME_ORIGINAL)
            .and_then(|e| t.ascii(e))
            .filter(|v| !v.is_empty());
        s.exposure_time = exif.get(tag::EXPOSURE_TIME).and_then(|e| t.rational(e, 0));
        s.f_number = exif
            .get(tag::F_NUMBER)
            .and_then(|e| t.rational(e, 0))
            .filter(|(_, d)| *d != 0)
            .map(|(n, d)| n as f64 / d as f64);
        s.iso = exif
            .get(tag::ISO)
            .and_then(|e| t.uint(e, 0))
            .map(|v| v as u32);
        s.focal_length_mm = exif
            .get(tag::FOCAL_LENGTH)
            .and_then(|e| t.rational(e, 0))
            .filter(|(_, d)| *d != 0)
            .map(|(n, d)| n as f64 / d as f64);
        s.lens = exif
            .get(tag::LENS_MODEL)
            .and_then(|e| t.ascii(e))
            .filter(|v| !v.is_empty());
    }
}

/// If `data` is a JPEG, return the TIFF bytes inside its Exif APP1 segment.
fn exif_segment_in_jpeg(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    let limit = data.len().min(1 << 20);
    while i + 4 <= limit {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let seg = data.get(i + 4..i + 2 + len)?;
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return Some(&seg[6..]);
        }
        if marker == 0xDA {
            return None;
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::tiff::testutil::build_le;

    /// A little-endian TIFF with one ASCII entry in its first IFD.
    fn tiff_ascii(tag: u16, text: &str) -> Vec<u8> {
        let mut v = b"II*\0".to_vec();
        v.extend(8u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(tag.to_le_bytes());
        v.extend(2u16.to_le_bytes());
        v.extend((text.len() as u32 + 1).to_le_bytes());
        v.extend(26u32.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend(text.as_bytes());
        v.push(0);
        v
    }

    fn bmff(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(name);
        v.extend(payload);
        v
    }

    #[test]
    fn cr3_reads_camera_and_date_from_cmt_boxes() {
        let mut f = bmff(b"ftyp", b"crx \0\0\0\x01crx isom");
        let mut uuid = vec![0u8; 16];
        uuid.extend(bmff(b"CMT1", &tiff_ascii(0x010F, "Canon")));
        uuid.extend(bmff(b"CMT2", &tiff_ascii(0x9003, "2021:05:12 12:21:21")));
        f.extend(bmff(b"moov", &bmff(b"uuid", &uuid)));
        let s = from_bytes(&f).expect("summary");
        assert_eq!(s.make.as_deref(), Some("Canon"));
        assert_eq!(s.date_time_original.as_deref(), Some("2021:05:12 12:21:21"));
        assert_eq!(s.orientation, 1);
    }

    #[test]
    fn reads_fields_from_tiff_and_jpeg_wrapper() {
        // IFD0: Model at 100, Orientation 6, ExifIFD at 150.
        // Exif IFD (at 150): DateTimeOriginal (20 bytes at 200), ExposureTime rational at 220, ISO 800.
        let mut payload = vec![0u8; 0];
        let mut data = build_le(
            &[
                (tag::MODEL, 2, 6, 100u32.to_le_bytes()),
                (tag::ORIENTATION, 3, 1, [6, 0, 0, 0]),
                (tag::EXIF_IFD, 4, 1, 150u32.to_le_bytes()),
            ],
            &payload,
            100,
        );
        data.extend_from_slice(b"Z 6_2\0");
        data.resize(150, 0);
        // Exif IFD with 3 entries.
        data.extend_from_slice(&3u16.to_le_bytes());
        for (tg, ty, ct, val) in [
            (tag::DATE_TIME_ORIGINAL, 2u16, 20u32, 200u32.to_le_bytes()),
            (tag::EXPOSURE_TIME, 5, 1, 220u32.to_le_bytes()),
            (tag::ISO, 3, 1, [0x20, 0x03, 0, 0]),
        ] {
            data.extend_from_slice(&tg.to_le_bytes());
            data.extend_from_slice(&ty.to_le_bytes());
            data.extend_from_slice(&ct.to_le_bytes());
            data.extend_from_slice(&val);
        }
        data.extend_from_slice(&0u32.to_le_bytes());
        data.resize(200, 0);
        data.extend_from_slice(b"2026:10:06 14:03:11\0");
        data.resize(220, 0);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&640u32.to_le_bytes());
        payload.clear();

        let s = from_bytes(&data).unwrap();
        assert_eq!(s.model.as_deref(), Some("Z 6_2"));
        assert_eq!(s.orientation, 6);
        assert_eq!(s.date_time_original.as_deref(), Some("2026:10:06 14:03:11"));
        assert_eq!(s.iso, Some(800));
        assert_eq!(s.exposure_line(), "1/640 · ISO 800");

        // Wrap the same TIFF in a JPEG APP1 segment.
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        let seg_len = 2 + 6 + data.len();
        jpg.extend_from_slice(&(seg_len as u16).to_be_bytes());
        jpg.extend_from_slice(b"Exif\0\0");
        jpg.extend_from_slice(&data);
        jpg.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        let j = from_bytes(&jpg).unwrap();
        assert_eq!(j.model.as_deref(), Some("Z 6_2"));
        assert_eq!(from_bytes(b"nothing here"), None);

        // The same JPEG inside a Fujifilm RAF header: orientation comes through.
        let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
        raf.resize(100, 0);
        raf[84..88].copy_from_slice(&100u32.to_be_bytes());
        raf[88..92].copy_from_slice(&(jpg.len() as u32).to_be_bytes());
        raf.extend_from_slice(&jpg);
        let r = from_bytes(&raf).unwrap();
        assert_eq!(r.orientation, 6);
        assert_eq!(r.model.as_deref(), Some("Z 6_2"));
    }
}
