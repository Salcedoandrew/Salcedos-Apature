//! Fast previews without decoding RAW data.
//!
//! The architectural rule of the whole project lives here: **never demosaic a
//! RAW to show a thumbnail when the camera already embedded a JPEG.** Every
//! RAW format we care about embeds one or more full-size or reduced JPEG
//! previews; we locate them by walking the container (TIFF IFDs and maker
//! notes, the RAF header, CR3 boxes, CRW heaps, X3F sections), decode the smallest one that satisfies the requested size
//! with DCT-domain downscaling, and cache the result on disk.

pub mod cache;
pub mod decode;
pub mod embedded;
pub mod jpeg_info;
pub mod source;

pub use cache::ThumbCache;
pub use decode::{decode_jpeg, rotate, thumbnail, DecodeError, Rgb};
pub use embedded::{
    container, find_embedded_jpegs, rgb_thumbnail, Container, EmbeddedJpeg, Prefer, RgbThumbnail,
    DEFAULT_MAX_BYTES,
};
pub use source::{extract, list_all, Source, SourceError};
