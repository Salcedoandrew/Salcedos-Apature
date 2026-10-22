//! Read a JPEG's pixel dimensions from its headers without decoding.

/// Scan JPEG markers up to the first SOF and return `(width, height)`.
/// Returns `None` if `bytes` isn't a JPEG or no SOF is found before the
/// first scan. The walk skips whole segments, so even a camera JPEG with a
/// multi-megabyte APP1 (Nikon MakerNote plus embedded preview) costs a
/// handful of reads.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let limit = bytes.len();
    let mut i = 2;
    while i + 4 <= limit {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // Padding bytes.
        if marker == 0xFF {
            i += 1;
            continue;
        }
        // Standalone markers without a length.
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        // SOF0..SOF15, excluding DHT (C4), JPG (C8), DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if i + 9 > bytes.len() {
                return None;
            }
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        if marker == 0xDA {
            // Start of scan: no SOF seen, give up.
            return None;
        }
        i += 2 + len;
    }
    None
}

/// The first frame header (SOF) of a JPEG: its marker and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub marker: u8,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    /// Baseline (SOF0, SOF1) or progressive (SOF2): what an ordinary JPEG
    /// decoder opens. Lossless (SOF3, the raw data in CR2, DNG and many
    /// others), hierarchical and arithmetic-coded streams are not previews.
    pub fn decodable(&self) -> bool {
        matches!(self.marker, 0xC0..=0xC2)
    }
}

/// Like [`dimensions`], but also says which kind of frame it is.
pub fn frame(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        if matches!(marker, 0xDA | 0xD9) {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if i + 9 > bytes.len() {
                return None;
            }
            return Some(Frame {
                marker,
                height: u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32,
                width: u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32,
            });
        }
        i += 2 + len;
    }
    None
}

/// Length of the complete JPEG at the start of `bytes`, up to and including
/// its EOI, or `None` when there is no EOI. Segments are skipped by their
/// declared length, so an Exif thumbnail inside APP1 cannot end the outer
/// stream early.
pub fn byte_length(bytes: &[u8]) -> Option<usize> {
    if !is_jpeg(bytes) {
        return None;
    }
    let be16 = |i: usize| u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
    let mut i = 2;
    while i + 1 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        match bytes[i + 1] {
            0xFF => i += 1,
            0xD9 => return Some(i + 2),
            0xD8 | 0x01 | 0xD0..=0xD7 => i += 2,
            0xDA => {
                if i + 3 >= bytes.len() || be16(i + 2) < 2 {
                    return None;
                }
                i += 2 + be16(i + 2);
                // Entropy-coded data: stuffed 00 and restart markers continue it.
                while i + 1 < bytes.len() {
                    if bytes[i] != 0xFF {
                        i += 1;
                        continue;
                    }
                    match bytes[i + 1] {
                        0x00 | 0xD0..=0xD7 => i += 2,
                        0xFF => i += 1,
                        0xD9 => return Some(i + 2),
                        // Another scan (progressive) or table: back to segments.
                        _ => break,
                    }
                }
            }
            _ => {
                if i + 3 >= bytes.len() || be16(i + 2) < 2 {
                    return None;
                }
                i += 2 + be16(i + 2);
            }
        }
    }
    None
}

/// Where the TIFF inside the first `Exif\0\0` APP1 segment sits: `(start, len)`
/// relative to `bytes`.
pub fn exif_tiff(bytes: &[u8]) -> Option<(usize, usize)> {
    if !is_jpeg(bytes) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0xDA | 0xD9) {
            return None;
        }
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let start = i + 4;
        if marker == 0xE1 && len >= 16 && bytes.get(start..start + 6) == Some(&b"Exif\0\0"[..]) {
            let end = (i + 2 + len).min(bytes.len());
            return Some((start + 6, end.saturating_sub(start + 6)));
        }
        i += 2 + len;
    }
    None
}

/// True if `bytes` starts with a JPEG SOI marker.
pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sof0_dimensions() {
        // SOI, APP0 (len 16), SOF0 (len 17: precision, h=480, w=640, 3 comps...), then junk.
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(&[0u8; 14]);
        v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 8, 0x01, 0xE0, 0x02, 0x80, 3]);
        v.extend_from_slice(&[0u8; 9]);
        assert_eq!(dimensions(&v), Some((640, 480)));
        assert!(is_jpeg(&v));
        assert_eq!(dimensions(b"II*\0"), None);
    }

    #[test]
    fn frame_kind_length_and_exif() {
        // SOI, APP1 Exif (TIFF header only), SOF3 lossless 2x1, SOS, data, EOI.
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x10];
        v.extend_from_slice(b"Exif\0\0II*\0\x08\0\0\0");
        v.extend_from_slice(&[0xFF, 0xC3, 0x00, 0x0B, 8, 0, 1, 0, 2, 1, 1, 0x11, 0]);
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0]);
        v.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56, 0xFF, 0xD9]);
        let total = v.len();
        v.extend_from_slice(b"trailing raw data");
        let f = frame(&v).unwrap();
        assert_eq!((f.marker, f.width, f.height), (0xC3, 2, 1));
        assert!(!f.decodable());
        assert_eq!(byte_length(&v), Some(total));
        assert_eq!(exif_tiff(&v), Some((12, 8)));
        assert_eq!(byte_length(&v[..total - 1]), None);
    }
}
