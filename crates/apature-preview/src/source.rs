//! A memory-mapped photo file plus its embedded-JPEG index.

use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::decode::{decode_jpeg, thumbnail, DecodeError, Rgb};
use crate::embedded::{find_embedded_jpegs, pick, rgb_thumbnail, EmbeddedJpeg, Prefer};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("{path}: {err}")]
    Io { path: PathBuf, err: std::io::Error },
    #[error("{0}: no embedded JPEG preview found")]
    NoPreview(PathBuf),
    #[error("{0}: every embedded preview is larger than {1} bytes")]
    TooLarge(PathBuf, usize),
    #[error("{0}: {1}")]
    Decode(PathBuf, DecodeError),
}

pub struct Source {
    path: PathBuf,
    map: Mmap,
    candidates: Vec<EmbeddedJpeg>,
    /// A JPEG made from an uncompressed RGB thumbnail, for files with no
    /// JPEG preview (Hasselblad, Phase One). Its candidate is the only one.
    encoded: Option<Vec<u8>>,
}

impl Source {
    /// Memory-map the file and index its embedded JPEGs. Only the pages that
    /// hold IFDs are actually read; a 50 MB NEF costs a few page faults.
    pub fn open(path: &Path) -> Result<Self, SourceError> {
        let file = File::open(path).map_err(|err| SourceError::Io {
            path: path.into(),
            err,
        })?;
        // SAFETY: the file is opened read-only and we never write through the map.
        // If the file is truncated underneath us a SIGBUS is possible, as with any
        // mmap; the contact sheet never modifies originals, so this is accepted.
        let map = unsafe { Mmap::map(&file) }.map_err(|err| SourceError::Io {
            path: path.into(),
            err,
        })?;
        let mut candidates = find_embedded_jpegs(&map);
        let mut encoded = None;
        if candidates.is_empty() {
            if let Some((c, jpeg)) =
                encode_rgb_thumbnail(&map).or_else(|| decode_flat_image(path, &map))
            {
                candidates.push(c);
                encoded = Some(jpeg);
            }
        }
        Ok(Source {
            path: path.to_path_buf(),
            map,
            candidates,
            encoded,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data(&self) -> &[u8] {
        &self.map
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// All embedded JPEGs, largest first.
    pub fn candidates(&self) -> &[EmbeddedJpeg] {
        &self.candidates
    }

    pub fn largest(&self) -> Option<&EmbeddedJpeg> {
        self.candidates.first()
    }

    /// The smallest candidate whose long side is at least `long_side`, or
    /// the largest available if none is big enough.
    pub fn best_for(&self, long_side: u32) -> Option<&EmbeddedJpeg> {
        self.candidates
            .iter()
            .filter(|c| c.long_side() >= long_side)
            .min_by_key(|c| c.long_side())
            .or_else(|| self.largest())
    }

    pub fn bytes(&self, c: &EmbeddedJpeg) -> &[u8] {
        match &self.encoded {
            Some(jpeg) => jpeg,
            None => c.bytes(&self.map),
        }
    }

    /// Decode the best candidate for `long_side`, downscaled in the DCT domain.
    pub fn decode_for(&self, long_side: u32) -> Result<Rgb, SourceError> {
        let c = self
            .best_for(long_side)
            .ok_or_else(|| SourceError::NoPreview(self.path.clone()))?;
        decode_jpeg(self.bytes(c), Some(long_side))
            .map_err(|e| SourceError::Decode(self.path.clone(), e))
    }
}

/// The preview `prefer` asks for, no bigger than `max_bytes`
/// ([`crate::DEFAULT_MAX_BYTES`] is 8 MiB), with its bytes.
pub fn extract(
    path: &Path,
    prefer: Prefer,
    max_bytes: usize,
) -> Result<(EmbeddedJpeg, Vec<u8>), SourceError> {
    let src = Source::open(path)?;
    if src.candidates.is_empty() {
        return Err(SourceError::NoPreview(path.into()));
    }
    let c = pick(src.candidates.clone(), prefer, max_bytes)
        .ok_or_else(|| SourceError::TooLarge(path.into(), max_bytes))?;
    let bytes = src.bytes(&c).to_vec();
    Ok((c, bytes))
}

/// Every embedded preview in the file, largest first.
pub fn list_all(path: &Path) -> Result<Vec<EmbeddedJpeg>, SourceError> {
    Ok(Source::open(path)?.candidates)
}

/// The largest preview built from a PNG or TIFF file. Its full decode is
/// bounded so a 100-megapixel scan is not held in memory just to fill a cell.
const FLAT_MAX_SIDE: u32 = 2048;

/// PNG and single-image TIFF files have no embedded JPEG preview. Decode one
/// directly and re-encode it as a JPEG so thumbnails and previews use the
/// same path as every other file.
fn decode_flat_image(path: &Path, data: &[u8]) -> Option<(EmbeddedJpeg, Vec<u8>)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "tif" | "tiff") {
        return None;
    }
    let img = image::load_from_memory(data).ok()?.to_rgb8();
    let rgb = thumbnail(&Rgb::from_image(img), FLAT_MAX_SIDE);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode(
            &rgb.data,
            rgb.width,
            rgb.height,
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    let c = EmbeddedJpeg {
        offset: 0,
        length: jpeg.len(),
        width: rgb.width,
        height: rgb.height,
        source: format!("{ext} decoded"),
    };
    Some((c, jpeg))
}

/// Re-encode a RAW's uncompressed RGB thumbnail as a JPEG (quality 90).
fn encode_rgb_thumbnail(data: &[u8]) -> Option<(EmbeddedJpeg, Vec<u8>)> {
    let t = rgb_thumbnail(data)?;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode(&t.data, t.width, t.height, image::ExtendedColorType::Rgb8)
        .ok()?;
    let c = EmbeddedJpeg {
        offset: 0,
        length: jpeg.len(),
        width: t.width,
        height: t.height,
        source: format!("{}:RGB thumbnail (re-encoded)", t.source),
    };
    Some((c, jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_without_embedded_jpeg_decodes_and_previews() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flat.png");
        let img = image::RgbImage::from_pixel(40, 30, image::Rgb([10, 20, 30]));
        img.save(&path).unwrap();
        let src = Source::open(&path).unwrap();
        let rgb = src.decode_for(320).unwrap();
        assert_eq!((rgb.width, rgb.height), (40, 30));
        assert!(rgb.data.chunks(3).all(|p| p == [10, 20, 30]));
    }
}
