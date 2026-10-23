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
    /// Firmware, exposure program, white balance and the like, already
    /// human-readable; only the ones the file has.
    pub details: CameraDetails,
}

/// Photo Mechanic-style camera fields beyond the exposure line. ISO,
/// aperture, shutter and focal length are on `exposure_line` already.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraDetails {
    pub firmware: Option<String>,
    pub exposure_comp: Option<String>,
    pub flash_comp: Option<String>,
    pub program: Option<String>,
    pub focus_mode: Option<String>,
    pub white_balance: Option<String>,
    pub icc_profile: Option<String>,
    pub contrast: Option<String>,
    pub sharpening: Option<String>,
    pub quality: Option<String>,
}

impl CameraDetails {
    /// (label, value) for each field present, in Photo Mechanic's order.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        [
            ("Firmware", &self.firmware),
            ("Exp. Comp.", &self.exposure_comp),
            ("Flash Comp.", &self.flash_comp),
            ("Program", &self.program),
            ("Focus Mode", &self.focus_mode),
            ("White Bal.", &self.white_balance),
            ("ICC Profile", &self.icc_profile),
            ("Contrast", &self.contrast),
            ("Sharpening", &self.sharpening),
            ("Quality", &self.quality),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.clone().map(|v| (k, v)))
        .collect()
    }
}

// Standard tags read for the details (not needed elsewhere, so kept here).
const SOFTWARE: u16 = 0x0131;
const EXPOSURE_PROGRAM: u16 = 0x8822;
const EXPOSURE_BIAS: u16 = 0x9204;
const COLOR_SPACE: u16 = 0xA001;
const INTEROP_IFD: u16 = 0xA005;
const WHITE_BALANCE: u16 = 0xA403;
const CONTRAST: u16 = 0xA408;
const SHARPNESS: u16 = 0xA40A;

/// "-0.3 EV", "+1 EV", "0 EV".
pub fn format_ev(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r == 0.0 {
        "0 EV".into()
    } else if r.fract() == 0.0 {
        format!("{r:+.0} EV")
    } else {
        format!("{r:+.1} EV")
    }
}

/// Canon's EV encoding (1/32 steps, with 0x0c and 0x14 meaning 1/3 and 2/3).
pub fn canon_ev(raw: i16) -> f64 {
    let sign = if raw < 0 { -1.0 } else { 1.0 };
    let v = (raw as i32).abs();
    let frac = v & 0x1f;
    let whole = (v - frac) as f64;
    let frac = match frac {
        0x0c => 32.0 / 3.0,
        0x14 => 64.0 / 3.0,
        f => f as f64,
    };
    sign * (whole + frac) / 32.0
}

/// EXIF ExposureProgram.
pub fn exposure_program(v: u64) -> Option<&'static str> {
    Some(match v {
        1 => "Manual",
        2 => "Program AE",
        3 => "Aperture-priority AE",
        4 => "Shutter speed priority AE",
        5 => "Creative (Slow speed)",
        6 => "Action (High speed)",
        7 => "Portrait",
        8 => "Landscape",
        9 => "Bulb",
        _ => return None,
    })
}

/// EXIF WhiteBalance (Auto or Manual only).
pub fn exif_white_balance(v: u64) -> Option<&'static str> {
    match v {
        0 => Some("Auto"),
        1 => Some("Manual"),
        _ => None,
    }
}

/// EXIF Contrast (`sharpness` false) or Sharpness (true): 0 Normal, 1 Low/Soft, 2 High/Hard.
pub fn exif_level(v: u64, sharpness: bool) -> Option<&'static str> {
    match (v, sharpness) {
        (0, _) => Some("Normal"),
        (1, false) => Some("Low"),
        (2, false) => Some("High"),
        (1, true) => Some("Soft"),
        (2, true) => Some("Hard"),
        _ => None,
    }
}

/// EXIF ColorSpace / Canon ColorSpace: 1 sRGB, 2 Adobe RGB.
pub fn color_space(v: u64) -> Option<&'static str> {
    match v {
        1 => Some("sRGB"),
        2 => Some("Adobe RGB"),
        0xFFFF => Some("Uncalibrated"),
        _ => None,
    }
}

/// Canon CameraSettings FocusMode.
pub fn canon_focus_mode(v: i16) -> Option<&'static str> {
    Some(match v {
        0 => "One-shot AF",
        1 => "AI Servo AF",
        2 => "AI Focus AF",
        3 | 6 => "Manual Focus",
        4 => "Single",
        5 => "Continuous",
        16 => "Pan Focus",
        256 => "One-shot AF (Live View)",
        257 => "AI Servo AF (Live View)",
        258 => "AI Focus AF (Live View)",
        512 => "Movie Snap Focus",
        519 => "Movie Servo AF",
        _ => return None,
    })
}

/// Canon CameraSettings Quality.
pub fn canon_quality(v: i16) -> Option<&'static str> {
    Some(match v {
        1 => "Economy",
        2 => "Normal",
        3 => "Fine",
        4 => "RAW",
        5 => "Superfine",
        7 => "CRAW",
        130 => "Light (RAW)",
        131 => "Standard (RAW)",
        _ => return None,
    })
}

/// Canon ShotInfo WhiteBalance.
pub fn canon_white_balance(v: i16) -> Option<&'static str> {
    Some(match v {
        0 => "Auto",
        1 => "Daylight",
        2 => "Cloudy",
        3 => "Tungsten",
        4 => "Fluorescent",
        5 => "Flash",
        6 => "Custom",
        7 => "Black & White",
        8 => "Shade",
        9 => "Manual Temperature (Kelvin)",
        14 => "Daylight Fluorescent",
        15 => "Custom 1",
        16 => "Custom 2",
        17 => "Underwater",
        _ => return None,
    })
}

/// Canon's signed picture-style levels: 0 Normal, else "+1" / "-1"; 0x7fff is n/a.
pub fn canon_level(v: i16) -> Option<String> {
    match v {
        i16::MIN..=-100 | 100..=i16::MAX => None,
        0 => Some("Normal".into()),
        v if v > 0 => Some(format!("+{v}")),
        v => Some(v.to_string()),
    }
}

/// Canon picture-style strength (ProcessingInfo), "0".."10"; out of range is n/a.
pub fn canon_strength(v: i16) -> Option<String> {
    (0..=10).contains(&v).then(|| v.to_string())
}

/// "Firmware Version 1.0.2" → "1.0.2"; other text as written.
pub fn firmware_text(s: &str) -> String {
    let s = s.trim();
    for p in ["Firmware Version ", "Firmware Ver.", "Firmware "] {
        if let Some(rest) = s.strip_prefix(p) {
            return rest.trim().to_string();
        }
    }
    s.to_string()
}

/// IFD0 Software is the camera firmware only when no editor wrote it.
pub fn software_is_firmware(s: &str) -> bool {
    let l = s.to_lowercase();
    let editors = [
        "adobe",
        "lightroom",
        "photoshop",
        "capture one",
        "gimp",
        "darktable",
        "rawtherapee",
        "photo mechanic",
        "apature",
        "windows",
        "mac os",
        "macos",
        "picasa",
        "affinity",
        "luminar",
        "dxo",
        "snapseed",
        "digital photo professional",
        "aftershoot",
        "imagemagick",
    ];
    !l.is_empty() && l.chars().any(|c| c.is_ascii_digit()) && !editors.iter().any(|e| l.contains(e))
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
    // CMT3 is the Canon maker notes as their own TIFF stream.
    if let Some((t, first)) = boxed(b"CMT3").and_then(|b| Tiff::parse(b).ok()) {
        read_canon_makernote(&t, first, &mut s);
    }
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
    s.details.firmware = ifd0
        .get(SOFTWARE)
        .and_then(|e| t.ascii(e))
        .filter(|v| software_is_firmware(v))
        .map(|v| firmware_text(&v));
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
        read_exif_details(t, &exif, s);
        // Canon maker notes are a plain IFD with offsets from the TIFF start.
        if is_canon(s) {
            if let Some(e) = exif.get(tag::MAKER_NOTE) {
                if let Ok(off) = t.u32(e.pos) {
                    read_canon_makernote(t, off, s);
                }
            }
        }
    }
}

fn is_canon(s: &ExifSummary) -> bool {
    s.make
        .as_deref()
        .is_some_and(|m| m.to_lowercase().starts_with("canon"))
}

/// Exposure comp, program, white balance, colour space, contrast,
/// and sharpness from the Exif IFD.
fn read_exif_details(t: &Tiff, exif: &apature_core::tiff::Ifd, s: &mut ExifSummary) {
    let d = &mut s.details;
    let u = |tg: u16| exif.get(tg).and_then(|e| t.uint(e, 0));
    if let Some((n, den)) = exif
        .get(EXPOSURE_BIAS)
        .and_then(|e| t.rational(e, 0))
        .filter(|(_, den)| *den != 0)
    {
        d.exposure_comp = Some(format_ev(n as i32 as f64 / den as i32 as f64));
    }
    d.program = u(EXPOSURE_PROGRAM)
        .and_then(exposure_program)
        .map(Into::into);
    d.white_balance = u(WHITE_BALANCE)
        .and_then(exif_white_balance)
        .map(Into::into);
    d.contrast = u(CONTRAST)
        .and_then(|v| exif_level(v, false))
        .map(Into::into);
    d.sharpening = u(SHARPNESS)
        .and_then(|v| exif_level(v, true))
        .map(Into::into);
    d.icc_profile = u(COLOR_SPACE).and_then(color_space).map(Into::into);
    // Uncalibrated with interop index R03 is how cameras mark Adobe RGB.
    if d.icc_profile.as_deref() == Some("Uncalibrated") {
        let r03 = u(INTEROP_IFD)
            .and_then(|off| t.read_ifd(off as u32).ok())
            .and_then(|i| i.get(0x0001).and_then(|e| t.ascii(e)))
            .is_some_and(|v| v == "R03");
        if r03 {
            d.icc_profile = Some("Adobe RGB".into());
        }
    }
}

/// Firmware, focus mode, quality, white balance, flash comp (and
/// what the Exif IFD lacked) from a Canon maker-note IFD at `at`.
fn read_canon_makernote(t: &Tiff, at: u32, s: &mut ExifSummary) {
    let Ok(mn) = t.read_ifd(at) else { return };
    let d = &mut s.details;
    let short = |tg: u16, i: usize| -> Option<i16> {
        mn.get(tg)
            .and_then(|e| t.uint(e, i))
            .map(|v| v as u16 as i16)
    };
    if let Some(fw) = mn
        .get(0x0007)
        .and_then(|e| t.ascii(e))
        .filter(|v| !v.is_empty())
    {
        d.firmware = Some(firmware_text(&fw));
    }
    // CameraSettings (0x0001): 3 quality, 7 focus mode, 13 contrast, 15 sharpness.
    if let Some(q) = short(0x0001, 3).and_then(canon_quality) {
        d.quality = Some(q.into());
    }
    if let Some(f) = short(0x0001, 7).and_then(canon_focus_mode) {
        d.focus_mode = Some(f.into());
    }
    if d.contrast.is_none() {
        d.contrast = short(0x0001, 13).and_then(canon_level);
    }
    if d.sharpening.is_none() {
        // ProcessingInfo (0x00a0) 2 is the picture style's sharpness
        // strength on newer bodies (0 = none), shown as the number.
        d.sharpening = short(0x00a0, 2)
            .and_then(canon_strength)
            .or_else(|| short(0x0001, 15).and_then(canon_level));
    }
    // ShotInfo (0x0004): 6 exposure comp, 7 white balance, 15 flash comp.
    if d.exposure_comp.is_none() {
        d.exposure_comp = short(0x0004, 6).map(|v| format_ev(canon_ev(v)));
    }
    if let Some(wb) = short(0x0004, 7).and_then(canon_white_balance) {
        d.white_balance = Some(wb.into());
    }
    d.flash_comp = short(0x0004, 15).map(|v| format_ev(canon_ev(v)));
    if d.icc_profile.is_none() || d.icc_profile.as_deref() == Some("Uncalibrated") {
        if let Some(c) = mn
            .get(0x00b4)
            .and_then(|e| t.uint(e, 0))
            .and_then(color_space)
        {
            d.icc_profile = Some(c.into());
        }
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

    #[test]
    fn detail_value_helpers() {
        assert_eq!(format_ev(-1.0 / 3.0), "-0.3 EV");
        assert_eq!(format_ev(2.0 / 3.0), "+0.7 EV");
        assert_eq!(format_ev(1.0), "+1 EV");
        assert_eq!(format_ev(0.0), "0 EV");
        assert_eq!(format_ev(-0.01), "0 EV");
        assert!((canon_ev(-12) + 1.0 / 3.0).abs() < 1e-9);
        assert!((canon_ev(0x14) - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(canon_ev(32), 1.0);
        assert_eq!(canon_ev(0), 0.0);
        assert_eq!(exposure_program(2), Some("Program AE"));
        assert_eq!(exposure_program(1), Some("Manual"));
        assert_eq!(exposure_program(0), None);
        assert_eq!(exif_white_balance(0), Some("Auto"));
        assert_eq!(exif_level(0, false), Some("Normal"));
        assert_eq!(exif_level(2, true), Some("Hard"));
        assert_eq!(exif_level(7, true), None);
        assert_eq!(color_space(1), Some("sRGB"));
        assert_eq!(color_space(2), Some("Adobe RGB"));
        assert_eq!(canon_focus_mode(1), Some("AI Servo AF"));
        assert_eq!(canon_focus_mode(6), Some("Manual Focus"));
        assert_eq!(canon_quality(4), Some("RAW"));
        assert_eq!(canon_quality(-1), None);
        assert_eq!(canon_white_balance(1), Some("Daylight"));
        assert_eq!(canon_level(0).as_deref(), Some("Normal"));
        assert_eq!(canon_level(2).as_deref(), Some("+2"));
        assert_eq!(canon_level(-1).as_deref(), Some("-1"));
        assert_eq!(canon_level(0x7fff), None);
        assert_eq!(canon_strength(0).as_deref(), Some("0"));
        assert_eq!(canon_strength(0x7fff), None);
        assert_eq!(firmware_text("Firmware Version 1.0.2"), "1.0.2");
        assert_eq!(firmware_text("Ver.1.10"), "Ver.1.10");
        assert!(software_is_firmware("Ver.1.10"));
        assert!(!software_is_firmware(
            "Adobe Lightroom Classic 13.2 (Macintosh)"
        ));
        assert!(!software_is_firmware("Picture Style Editor"));
    }

    #[test]
    fn detail_rows_skip_missing_and_keep_order() {
        let d = CameraDetails {
            quality: Some("RAW".into()),
            firmware: Some("1.0.2".into()),
            program: Some("Manual".into()),
            ..Default::default()
        };
        assert_eq!(
            d.rows(),
            vec![
                ("Firmware", "1.0.2".to_string()),
                ("Program", "Manual".to_string()),
                ("Quality", "RAW".to_string()),
            ]
        );
        assert!(CameraDetails::default().rows().is_empty());
    }
}
