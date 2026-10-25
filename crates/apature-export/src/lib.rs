//! Delivery export: one photo in, one JPEG out, sized for the desk, with
//! EXIF, XMP, IPTC-IIM and the ICC profile carried across.
//!
//! Pixels come from the JPEG of a pair or a plain JPEG; a RAW alone uses its
//! largest embedded preview (never a RAW decode: there is no copyleft-free
//! demosaic in this workspace and the camera's own rendering is what the
//! photographer saw). The image is cropped and rotated as the sidecar says
//! (`crs:Crop*`, `tiff:Orientation`; see [`apature_metadata::edits`]), else
//! rotated upright by its EXIF, resized so the long side is
//! `Settings::long_side`, and re-encoded at `Settings::quality`.
//!
//! Colour: the source's ICC profile is copied (an Adobe RGB JPEG stays
//! Adobe RGB and tagged); no conversion to sRGB happens for JPEG sources.
//!
//! RAW decode: with [`RawPixels::SystemDecode`] a RAW alone is rendered by
//! the operating system's RAW engine (macOS ImageIO through `sips`,
//! matched to sRGB) instead of its embedded preview. Slower, full quality,
//! macOS only; other platforms fall back to the preview and say so.

pub mod extract;

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};

use apature_core::{Kind, PhotoItem};
use apature_metadata::jpeg_xmp::{header_segments, is_exif_segment, segment as jpeg_segment};
use apature_metadata::{edits, exif, exif_write, iim, iptc, Iptc};
use apature_preview::{decode_jpeg, find_embedded_jpegs, rotate, Rgb};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RawPixels {
    /// The camera's embedded full-size JPEG (fast, the camera's rendering).
    #[default]
    EmbeddedPreview,
    /// Demosaic with the system RAW engine (macOS), matched to sRGB.
    SystemDecode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Long side in pixels; 0 keeps the source size.
    pub long_side: u32,
    /// JPEG quality 1..=100.
    pub quality: u8,
    pub keep_exif: bool,
    pub write_xmp: bool,
    pub write_iim: bool,
    pub keep_icc: bool,
    pub overwrite: bool,
    pub raw: RawPixels,
    /// Apply the rotation and crop stored in the sidecar.
    pub apply_edits: bool,
    /// Container: JPEG, or JPEG XL (the finished JPEG is transcoded losslessly
    /// by `cjxl`, metadata included; needs libjxl's `cjxl` on the Mac).
    pub format: Format,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Jpeg,
    JpegXl,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::JpegXl => "jxl",
        }
    }
}

/// Where `cjxl` lives, if anywhere: `APATURE_CJXL`, Homebrew, /usr/local, PATH.
pub fn cjxl_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("APATURE_CJXL") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    for p in [
        "/opt/homebrew/bin/cjxl",
        "/usr/local/bin/cjxl",
        "/usr/bin/cjxl",
    ] {
        if Path::new(p).is_file() {
            return Some(PathBuf::from(p));
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("cjxl"))
            .find(|p| p.is_file())
    })
}

/// Transcode a finished JPEG to JPEG XL losslessly (`cjxl --lossless_jpeg=1`),
/// which keeps the JPEG's EXIF, XMP and ICC and makes the file about a fifth
/// smaller; the JPEG can be reconstructed bit for bit with `djxl`.
fn transcode_jxl(jpeg: &Path, jxl: &Path) -> Result<(), String> {
    let cjxl = cjxl_path().ok_or_else(|| {
        "JPEG XL needs cjxl (brew install jpeg-xl) or APATURE_CJXL set".to_string()
    })?;
    let out = std::process::Command::new(cjxl)
        .arg(jpeg)
        .arg(jxl)
        .arg("--lossless_jpeg=1")
        .arg("--quiet")
        .output()
        .map_err(|e| format!("cjxl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cjxl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            long_side: 2048,
            quality: 90,
            keep_exif: true,
            write_xmp: true,
            write_iim: true,
            keep_icc: true,
            overwrite: false,
            raw: RawPixels::EmbeddedPreview,
            apply_edits: true,
            format: Format::Jpeg,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExifSource {
    /// Copied from the source JPEG, orientation and size patched.
    Copied,
    /// Built from the EXIF summary (RAW-only source).
    Minimal,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelSource {
    Jpeg,
    EmbeddedPreview,
    SystemRawDecode,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub source: PathBuf,
    pub pixels: PixelSource,
    pub out: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub exif: ExifSource,
    /// The sidecar's rotation and/or crop were applied.
    pub edited: bool,
    pub millis: u128,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}: no JPEG and no embedded preview to export")]
    NoPixels(PathBuf),
    #[error("{0}: HEIC export is not supported yet")]
    Heic(PathBuf),
    #[error("{0} exists; export does not overwrite")]
    Exists(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("decode {0}: {1}")]
    Decode(PathBuf, apature_preview::DecodeError),
    #[error("encode: {0}")]
    Encode(image::ImageError),
    #[error("metadata: {0}")]
    Xmp(#[from] apature_metadata::xmp::XmpError),
    #[error("metadata: {0}")]
    Iim(#[from] apature_metadata::iim::IimError),
    #[error("JPEG segment of {0} bytes exceeds the 65,535-byte segment limit")]
    SegmentTooLarge(usize),
    #[error("{0}")]
    Jpeg(#[from] apature_metadata::jpeg_xmp::JpegXmpError),
    #[error("system RAW decode of {0} failed: {1}")]
    RawDecode(PathBuf, String),
}

/// Sequence for `sips` temp names: exports run in parallel (the desktop's
/// export pool), and two RAWs sharing a stem — the same shot name in
/// different folders — must never render into the same temp file.
#[cfg(target_os = "macos")]
static RAW_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Render a RAW with the macOS RAW engine (ImageIO via `sips`) to a JPEG at
/// quality 100, matched to sRGB, optionally resampled so the long side is
/// `long_side` (0 = full size). Returns the JPEG bytes.
#[cfg(target_os = "macos")]
pub fn system_decode_raw(path: &Path, long_side: u32) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!(
        "apature-raw-{}-{}-{}.jpg",
        std::process::id(),
        RAW_TMP_SEQ.fetch_add(1, Ordering::Relaxed),
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let mut cmd = std::process::Command::new("/usr/bin/sips");
    cmd.args(["-s", "format", "jpeg", "-s", "formatOptions", "100"]);
    cmd.args([
        "--matchTo",
        "/System/Library/ColorSync/Profiles/sRGB Profile.icc",
    ]);
    if long_side > 0 {
        cmd.args(["--resampleHeightWidthMax", &long_side.to_string()]);
    }
    cmd.arg(path).arg("--out").arg(&tmp);
    let out = cmd.output().map_err(|e| e.to_string())?;
    let result = if out.status.success() && tmp.is_file() {
        std::fs::read(&tmp).map_err(|e| e.to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

#[cfg(not(target_os = "macos"))]
pub fn system_decode_raw(_path: &Path, _long_side: u32) -> Result<Vec<u8>, String> {
    Err("system RAW decode is only available on macOS".into())
}

/// Which file supplies the pixels, and whether it is a real JPEG (with its
/// own EXIF/ICC) or a preview inside a RAW.
fn pixel_source(item: &PhotoItem) -> Result<(PathBuf, bool), ExportError> {
    if let Some(v) = item.variants().find(|v| v.kind == Kind::Jpeg) {
        return Ok((v.path.clone(), true));
    }
    match item.primary.kind {
        Kind::Raw => Ok((item.primary.path.clone(), false)),
        Kind::Heic => Err(ExportError::Heic(item.primary.path.clone())),
        Kind::Jpeg => Ok((item.primary.path.clone(), true)),
    }
}

fn resize_to(img: Rgb, long_side: u32) -> Rgb {
    if long_side == 0 || img.long_side() <= long_side {
        return img;
    }
    let s = long_side as f64 / img.long_side() as f64;
    let w = ((img.width as f64) * s).round().max(1.0) as u32;
    let h = ((img.height as f64) * s).round().max(1.0) as u32;
    let Some(src) = img.into_image() else {
        return Rgb {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
    };
    let out = image::imageops::resize(&src, w, h, image::imageops::FilterType::Lanczos3);
    Rgb::from_image(out)
}

fn crop_to(img: Rgb, c: edits::Crop) -> Rgb {
    let (x, y, w, h) = c.pixels(img.width, img.height);
    if (x, y, w, h) == (0, 0, img.width, img.height) {
        return img;
    }
    let Some(src) = img.into_image() else {
        return Rgb {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
    };
    Rgb::from_image(image::imageops::crop_imm(&src, x, y, w, h).to_image())
}

/// One APP1 segment (marker included), built by the shared guard in
/// `apature_metadata::jpeg_xmp`: a body too big for the 16-bit length is an
/// error, never a wrapped length that would silently corrupt the JPEG.
pub(crate) fn app1(sig: &[u8], body: &[u8]) -> Result<Vec<u8>, ExportError> {
    jpeg_segment(0xE1, "APP1", sig, body).map_err(|e| match e {
        // The shared guard counts sig + body; our error counts the whole
        // length field, which is two bytes more.
        apature_metadata::jpeg_xmp::JpegXmpError::SegmentTooLarge(_, n) => {
            ExportError::SegmentTooLarge(n + 2)
        }
        other => ExportError::Jpeg(other),
    })
}

/// `encoded` with `segs` spliced in right after SOI and the encoder's
/// APP0 (JFIF header) when it comes first.
pub(crate) fn splice_after_app0(encoded: &[u8], segs: &[Vec<u8>]) -> Vec<u8> {
    let mut ins = 2;
    if encoded.len() > 4 && encoded[2] == 0xFF && encoded[3] == 0xE0 {
        let len = u16::from_be_bytes([encoded[4], encoded[5]]) as usize;
        ins = 2 + 2 + len;
    }
    let mut file = Vec::with_capacity(encoded.len() + segs.iter().map(Vec::len).sum::<usize>());
    file.extend_from_slice(&encoded[..ins]);
    for s in segs {
        file.extend_from_slice(s);
    }
    file.extend_from_slice(&encoded[ins..]);
    file
}

/// Export `item` to `out` with `settings`, embedding `iptc` as XMP and IIM.
pub fn export(
    item: &PhotoItem,
    iptc: &Iptc,
    settings: &Settings,
    out: &Path,
) -> Result<Outcome, ExportError> {
    let t0 = std::time::Instant::now();
    if out.exists() && !settings.overwrite {
        return Err(ExportError::Exists(out.to_path_buf()));
    }
    let (src_path, mut is_jpeg) = pixel_source(item)?;
    let mut pixels = if is_jpeg {
        PixelSource::Jpeg
    } else {
        PixelSource::EmbeddedPreview
    };
    let mut data = std::fs::read(&src_path).map_err(|e| ExportError::Io(src_path.clone(), e))?;
    let edits = if settings.apply_edits {
        edits::read(&item.sidecar_path()).unwrap_or_else(|e| {
            log::warn!(
                "{}: sidecar edits unreadable ({e}); exporting unedited",
                src_path.display()
            );
            edits::Edits::default()
        })
    } else {
        edits::Edits::default()
    };
    if edits.straightened_crop {
        log::warn!(
            "{}: the sidecar's crop has a straighten angle Apature cannot apply; exporting uncropped",
            src_path.display()
        );
    }
    // A crop keeps only part of the frame, so decode (or render) enough
    // pixels for the cropped part to still reach the long side.
    let target = match (settings.long_side, edits.crop) {
        (0, _) => None,
        (n, None) => Some(n),
        (n, Some(c)) => {
            let keep = (c.right - c.left).min(c.bottom - c.top).max(0.01);
            Some(((n as f32) / keep).ceil() as u32)
        }
    };
    // The RAW's own orientation, read from the bytes already in memory;
    // a system decode replaces `data` with the render, whose summary
    // alone can't always supply it (see `file_orientation` below).
    let mut raw_orientation = 1;
    if !is_jpeg && settings.raw == RawPixels::SystemDecode {
        raw_orientation = exif::from_bytes(&data).map(|s| s.orientation).unwrap_or(1);
        match system_decode_raw(&src_path, target.unwrap_or(0)) {
            Ok(rendered) => {
                data = rendered;
                is_jpeg = true;
                pixels = PixelSource::SystemRawDecode;
            }
            Err(e) => {
                log::warn!("{}: {e}; using the embedded preview", src_path.display());
            }
        }
    }
    let jpeg_bytes: &[u8] = if is_jpeg {
        &data
    } else {
        let best = find_embedded_jpegs(&data)
            .into_iter()
            .max_by_key(|e| e.long_side())
            .ok_or_else(|| ExportError::NoPixels(src_path.clone()))?;
        best.bytes(&data)
    };
    // Parse the summary from the bytes already loaded: `exif::read` would
    // map the file and run this same parser over it, a second full read
    // of a file `data` already holds (the source, or the system render).
    let summary = exif::from_bytes(&data).unwrap_or_default();
    let mut img =
        decode_jpeg(jpeg_bytes, target).map_err(|e| ExportError::Decode(src_path.clone(), e))?;
    // The crop is in the stored frame, so it comes before the rotation. A
    // system RAW render may already be turned (its own tag says how); bring
    // it back to the RAW's stored frame first so crop and turn mean the same
    // thing for every pixel source.
    let file_orientation = if pixels == PixelSource::SystemRawDecode {
        if edits.is_edited() {
            img = rotate(
                rotate(img, summary.orientation),
                edits::inverse(raw_orientation),
            );
            raw_orientation
        } else {
            summary.orientation
        }
    } else {
        summary.orientation
    };
    let img = match edits.crop {
        Some(c) => crop_to(img, c),
        None => img,
    };
    let img = rotate(img, edits.orientation_or(file_orientation));
    let img = resize_to(img, settings.long_side);
    let (width, height) = (img.width, img.height);
    let encoded = apature_preview::decode::encode_jpeg(&img, settings.quality.clamp(1, 100))
        .map_err(ExportError::Encode)?;

    // ---- metadata segments ----
    let mut segs: Vec<Vec<u8>> = Vec::new();
    let mut exif_source = ExifSource::None;
    let src_segs = if is_jpeg {
        header_segments(&data)?
    } else {
        Vec::new()
    };
    if settings.keep_exif {
        // A system RAW render carries the RAW's TIFF-flavoured IFD0, which
        // validators reject in a JPEG; build a clean block from the summary.
        let copied = if pixels == PixelSource::SystemRawDecode {
            None
        } else {
            src_segs
                .iter()
                .find(|s| is_exif_segment(&data, s))
                .map(|s| s.payload(&data)[6..].to_vec())
        };
        match copied {
            Some(mut block) => {
                exif_write::patch_for_export(&mut block, width, height);
                segs.push(app1(b"Exif\0\0", &block)?);
                exif_source = ExifSource::Copied;
            }
            None => {
                let block = exif_write::minimal(
                    &summary,
                    width,
                    height,
                    iptc.creator
                        .as_ref()
                        .and_then(|c| c.first())
                        .map(String::as_str),
                    iptc.copyright.as_deref(),
                );
                segs.push(app1(b"Exif\0\0", &block)?);
                exif_source = ExifSource::Minimal;
            }
        }
    }
    if settings.write_xmp {
        let packet = iptc::apply("", iptc)?;
        segs.push(app1(b"http://ns.adobe.com/xap/1.0/\0", packet.as_bytes())?);
    }
    if settings.keep_icc {
        for s in src_segs
            .iter()
            .filter(|s| s.marker == 0xE2 && s.payload(&data).starts_with(b"ICC_PROFILE\0"))
        {
            segs.push(data[s.start..s.end].to_vec());
        }
    }
    if settings.write_iim && !iptc.is_empty() {
        segs.push(iim::app13_segment(iptc)?);
    }

    // ---- splice after SOI (and the encoder's APP0 when present) ----
    let file = splice_after_app0(&encoded, &segs);

    // ---- atomic write ----
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ExportError::Io(parent.to_path_buf(), e))?;
    }
    // JPEG XL: the finished JPEG goes to a temp file and cjxl writes the
    // final .jxl next to it; the JPEG temp is removed either way.
    let out_path: PathBuf = if settings.format == Format::JpegXl {
        out.with_extension("jxl")
    } else {
        out.to_path_buf()
    };
    let out = out_path.as_path();
    let tmp = out.with_extension("apature-export.tmp.jpg");
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&file)?;
        f.sync_all()?;
        if out.exists() && !settings.overwrite {
            return Err(std::io::Error::other("destination appeared during export"));
        }
        if settings.format == Format::JpegXl {
            let part = out.with_extension("jxl.part");
            transcode_jxl(&tmp, &part).map_err(std::io::Error::other)?;
            let _ = std::fs::remove_file(&tmp);
            std::fs::rename(&part, out)
        } else {
            std::fs::rename(&tmp, out)
        }
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(ExportError::Io(out.to_path_buf(), e));
    }
    let bytes_written = std::fs::metadata(out)
        .map(|m| m.len())
        .unwrap_or(file.len() as u64);
    Ok(Outcome {
        source: src_path,
        pixels,
        out: out.to_path_buf(),
        width,
        height,
        bytes: bytes_written,
        exif: exif_source,
        edited: edits.is_edited(),
        millis: t0.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::item::Variant;

    fn item_for(path: &Path) -> PhotoItem {
        PhotoItem {
            id: 0,
            stem: path.file_stem().unwrap().to_string_lossy().into_owned(),
            primary: Variant {
                path: path.to_path_buf(),
                kind: Kind::Jpeg,
                size: 0,
                modified: None,
            },
            secondary: Vec::new(),
        }
    }

    /// Test-local IIM dataset walker: unpack the Photoshop resource in
    /// an APP13 payload and walk its datasets (the production read-back
    /// pair in apature-metadata was deleted as dead code).
    fn iim_sets(payload: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
        let p = payload.strip_prefix(b"Photoshop 3.0\0").unwrap();
        assert_eq!(&p[..4], b"8BIM");
        assert_eq!(u16::from_be_bytes([p[4], p[5]]), 0x0404);
        let name_len = p[6] as usize;
        let name_total = (1 + name_len + 1) & !1; // Pascal string padded to even
        let len_at = 6 + name_total;
        let len = u32::from_be_bytes(p[len_at..len_at + 4].try_into().unwrap()) as usize;
        let mut d = &p[len_at + 4..len_at + 4 + len];
        let mut out = Vec::new();
        while d.len() >= 5 && d[0] == 0x1C {
            let n = u16::from_be_bytes([d[3], d[4]]) as usize;
            if d.len() < 5 + n {
                break;
            }
            out.push((d[1], d[2], d[5..5 + n].to_vec()));
            d = &d[5 + n..];
        }
        out
    }

    fn sample_iptc() -> Iptc {
        Iptc {
            description: Some("A caption. Photog/Example Press".into()),
            credit: Some("Example Press".into()),
            creator: Some(vec!["Caleb".into()]),
            copyright: Some("© 2026 Example Press".into()),
            headline: Some("100726 SPO BGLF".into()),
            ..Default::default()
        }
    }

    /// 400×200, left half black, right half white.
    fn half_and_half(path: &Path) {
        let mut data = Vec::with_capacity(400 * 200 * 3);
        for _y in 0..200 {
            for x in 0..400 {
                let v = if x < 200 { 0 } else { 255 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let img = Rgb {
            width: 400,
            height: 200,
            data,
        };
        std::fs::write(
            path,
            apature_preview::decode::encode_jpeg(&img, 95).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn sidecar_rotation_and_crop_shape_the_export() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0001.jpg");
        half_and_half(&src);
        let item = item_for(&src);
        // Keep the white right half, then a quarter turn clockwise.
        edits::write(
            &item.sidecar_path(),
            &edits::Change {
                orientation: Some(Some(6)),
                crop: Some(Some(edits::Crop {
                    top: 0.0,
                    left: 0.5,
                    bottom: 1.0,
                    right: 1.0,
                })),
            },
        )
        .unwrap();
        let out = dir.path().join("edited.jpg");
        let o = export(
            &item,
            &Iptc::default(),
            &Settings {
                long_side: 0,
                ..Default::default()
            },
            &out,
        )
        .unwrap();
        assert!(o.edited);
        assert_eq!((o.width, o.height), (200, 200), "half of 400×200, turned");
        let back = decode_jpeg(&std::fs::read(&out).unwrap(), None).unwrap();
        let mean = back.data.iter().map(|&v| v as u64).sum::<u64>() / back.data.len() as u64;
        assert!(mean > 240, "only the white half is left, mean {mean}");

        // Rotation alone swaps the sides; the long side still lands on 100.
        edits::write(
            &item.sidecar_path(),
            &edits::Change {
                crop: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        let out2 = dir.path().join("turned.jpg");
        let o2 = export(
            &item,
            &Iptc::default(),
            &Settings {
                long_side: 100,
                ..Default::default()
            },
            &out2,
        )
        .unwrap();
        assert_eq!((o2.width, o2.height), (50, 100));
        // Switched off, the sidecar is ignored.
        let out3 = dir.path().join("plain.jpg");
        let o3 = export(
            &item,
            &Iptc::default(),
            &Settings {
                long_side: 100,
                apply_edits: false,
                ..Default::default()
            },
            &out3,
        )
        .unwrap();
        assert!(!o3.edited);
        assert_eq!((o3.width, o3.height), (100, 50));
    }

    #[test]
    fn oversized_xmp_packet_errors_instead_of_wrapping() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("big.jpg");
        half_and_half(&src);
        // A 70 KB caption makes the XMP APP1 pass the 65,535-byte segment
        // limit; the export must fail, not write a wrapped length.
        let big = Iptc {
            description: Some("x".repeat(70_000)),
            ..Default::default()
        };
        let out = dir.path().join("big-out.jpg");
        assert!(matches!(
            export(&item_for(&src), &big, &Settings::default(), &out),
            Err(ExportError::SegmentTooLarge(_))
        ));
        assert!(!out.exists(), "no wrapped file was written");
    }

    #[test]
    fn a_tight_crop_still_reaches_the_long_side() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0002.jpg");
        half_and_half(&src);
        let item = item_for(&src);
        edits::write(
            &item.sidecar_path(),
            &edits::Change {
                crop: Some(Some(edits::Crop {
                    top: 0.25,
                    left: 0.25,
                    bottom: 0.75,
                    right: 0.75,
                })),
                ..Default::default()
            },
        )
        .unwrap();
        let out = dir.path().join("tight.jpg");
        let o = export(
            &item,
            &Iptc::default(),
            &Settings {
                long_side: 180,
                ..Default::default()
            },
            &out,
        )
        .unwrap();
        assert_eq!((o.width, o.height), (180, 90));
    }

    #[test]
    fn synthetic_jpeg_resizes_and_carries_every_block() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.jpg");
        let img = Rgb {
            width: 400,
            height: 200,
            data: (0..400 * 200 * 3).map(|i| (i % 251) as u8).collect(),
        };
        std::fs::write(
            &src,
            apature_preview::decode::encode_jpeg(&img, 85).unwrap(),
        )
        .unwrap();
        let out = dir.path().join("out/wide-export.jpg");
        let o = export(
            &item_for(&src),
            &sample_iptc(),
            &Settings {
                long_side: 100,
                ..Default::default()
            },
            &out,
        )
        .unwrap();
        assert_eq!((o.width, o.height), (100, 50));
        assert_eq!(
            o.exif,
            ExifSource::Minimal,
            "a plain encoder JPEG has no EXIF"
        );
        let bytes = std::fs::read(&out).unwrap();
        let segs = header_segments(&bytes).unwrap();
        let markers: Vec<u8> = segs.iter().map(|s| s.marker).collect();
        assert!(
            markers.contains(&0xE1) && markers.contains(&0xED),
            "{markers:?}"
        );
        assert!(segs.iter().any(|s| is_exif_segment(&bytes, s)));
        let packet = apature_metadata::xmp::embedded_packet_in_jpeg(&bytes).expect("xmp");
        let back = iptc::parse(packet);
        assert_eq!(
            back.description.as_deref(),
            Some("A caption. Photog/Example Press")
        );
        let app13 = segs.iter().find(|s| s.marker == 0xED).unwrap();
        let sets = iim_sets(app13.payload(&bytes));
        assert!(sets
            .iter()
            .any(|(r, s, v)| *r == 2 && *s == 110 && v == b"Example Press"));
        // Decodes again at the new size.
        let back = decode_jpeg(&bytes, None).unwrap();
        assert_eq!((back.width, back.height), (100, 50));
        // Refuses to overwrite.
        assert!(matches!(
            export(&item_for(&src), &sample_iptc(), &Settings::default(), &out),
            Err(ExportError::Exists(_))
        ));
        assert!(!dir
            .path()
            .join("out/wide-export.apature-export.tmp")
            .exists());
    }
}
