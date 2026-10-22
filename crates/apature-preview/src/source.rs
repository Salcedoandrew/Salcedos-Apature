//! A memory-mapped photo file plus its embedded-JPEG index.

use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::decode::{decode_jpeg, DecodeError, Rgb};
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
            if let Some((c, jpeg)) = encode_rgb_thumbnail(&map) {
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
