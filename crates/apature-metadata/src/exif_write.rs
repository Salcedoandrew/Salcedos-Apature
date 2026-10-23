//! Writing EXIF for exported JPEGs. Two cases:
//!
//! * the source JPEG had an Exif APP1: it is copied and only the fields the
//!   export changed are patched in place (Orientation → 1 because the pixels
//!   are now upright, PixelX/YDimension → the new size);
//! * the source was a RAW whose embedded preview has no EXIF: a minimal
//!   block is built from the [`ExifSummary`] so the desk still sees the
//!   camera, lens, capture time and exposure.

use apature_core::tiff::{tag, Tiff};

use crate::ExifSummary;

const T_ASCII: u16 = 2;
const T_SHORT: u16 = 3;
const T_LONG: u16 = 4;
const T_RATIONAL: u16 = 5;
const T_UNDEFINED: u16 = 7;

const EXIF_IFD: u16 = 0x8769;
const PIXEL_X: u16 = 0xA002;
const PIXEL_Y: u16 = 0xA003;

struct E {
    tag: u16,
    typ: u16,
    count: u32,
    data: Vec<u8>,
}

fn ascii(tag: u16, s: &str) -> E {
    let mut data = s.as_bytes().to_vec();
    data.push(0);
    E {
        tag,
        typ: T_ASCII,
        count: data.len() as u32,
        data,
    }
}
fn short(tag: u16, v: u16) -> E {
    E {
        tag,
        typ: T_SHORT,
        count: 1,
        data: v.to_le_bytes().to_vec(),
    }
}
fn long(tag: u16, v: u32) -> E {
    E {
        tag,
        typ: T_LONG,
        count: 1,
        data: v.to_le_bytes().to_vec(),
    }
}
fn rational(tag: u16, n: u32, d: u32) -> E {
    let mut data = n.to_le_bytes().to_vec();
    data.extend_from_slice(&d.to_le_bytes());
    E {
        tag,
        typ: T_RATIONAL,
        count: 1,
        data,
    }
}
fn undefined(tag: u16, bytes: &[u8]) -> E {
    E {
        tag,
        typ: T_UNDEFINED,
        count: bytes.len() as u32,
        data: bytes.to_vec(),
    }
}

/// Size of an IFD with its out-of-line values.
fn ifd_size(entries: &[E]) -> usize {
    2 + entries.len() * 12
        + 4
        + entries
            .iter()
            .filter(|e| e.data.len() > 4)
            .map(|e| (e.data.len() + 1) & !1)
            .sum::<usize>()
}

/// Append an IFD at the current end of `buf` (little-endian).
fn write_ifd(buf: &mut Vec<u8>, mut entries: Vec<E>, next: u32) {
    entries.sort_by_key(|e| e.tag);
    let base = buf.len();
    let mut data_at = base + 2 + entries.len() * 12 + 4;
    buf.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut tail: Vec<u8> = Vec::new();
    for e in &entries {
        buf.extend_from_slice(&e.tag.to_le_bytes());
        buf.extend_from_slice(&e.typ.to_le_bytes());
        buf.extend_from_slice(&e.count.to_le_bytes());
        if e.data.len() <= 4 {
            let mut v = [0u8; 4];
            v[..e.data.len()].copy_from_slice(&e.data);
            buf.extend_from_slice(&v);
        } else {
            buf.extend_from_slice(&(data_at as u32).to_le_bytes());
            tail.extend_from_slice(&e.data);
            if e.data.len() % 2 == 1 {
                tail.push(0);
            }
            data_at += (e.data.len() + 1) & !1;
        }
    }
    buf.extend_from_slice(&next.to_le_bytes());
    buf.extend_from_slice(&tail);
}

fn to_rational(v: f64) -> (u32, u32) {
    if v <= 0.0 {
        return (0, 1);
    }
    let d = if v < 10.0 { 1000 } else { 10 };
    ((v * d as f64).round() as u32, d)
}

/// A TIFF block (no "Exif\0\0" prefix) with IFD0 and an Exif IFD built from
/// `s`, sized `width`×`height`, upright.
pub fn minimal(
    s: &ExifSummary,
    width: u32,
    height: u32,
    artist: Option<&str>,
    copyright: Option<&str>,
) -> Vec<u8> {
    minimal_oriented(s, width, height, artist, copyright, 1)
}

/// [`minimal`] for pixels that are not upright (a RAW's embedded preview):
/// Orientation says how to turn them.
pub fn minimal_oriented(
    s: &ExifSummary,
    width: u32,
    height: u32,
    artist: Option<&str>,
    copyright: Option<&str>,
    orientation: u16,
) -> Vec<u8> {
    let mut ifd0 = vec![
        short(tag::ORIENTATION, orientation.clamp(1, 8)),
        ascii(0x0131, "Apature"),
        short(0x0213, 1), // YCbCrPositioning centered
        rational(0x011A, 72, 1),
        rational(0x011B, 72, 1),
        short(0x0128, 2),
    ];
    if let Some(v) = &s.make {
        ifd0.push(ascii(tag::MAKE, v));
    }
    if let Some(v) = &s.model {
        ifd0.push(ascii(tag::MODEL, v));
    }
    if let Some(v) = &s.date_time_original {
        ifd0.push(ascii(0x0132, v));
    }
    if let Some(a) = artist.filter(|a| !a.is_empty()) {
        ifd0.push(ascii(0x013B, a));
    }
    if let Some(c) = copyright.filter(|c| !c.is_empty()) {
        ifd0.push(ascii(0x8298, c));
    }
    let mut exif = vec![
        undefined(0x9000, b"0232"),
        undefined(0x9101, &[1, 2, 3, 0]), // ComponentsConfiguration YCbCr
        short(0xA001, 1),                 // sRGB
        long(PIXEL_X, width),
        long(PIXEL_Y, height),
        undefined(0xA000, b"0100"),
    ];
    if let Some((n, d)) = s.exposure_time {
        exif.push(rational(0x829A, n, d));
    }
    if let Some(f) = s.f_number {
        let (n, d) = to_rational(f);
        exif.push(rational(0x829D, n, d));
    }
    if let Some(iso) = s.iso {
        exif.push(short(0x8827, iso.min(65535) as u16));
    }
    if let Some(v) = &s.date_time_original {
        exif.push(ascii(0x9003, v));
        exif.push(ascii(0x9004, v));
    }
    if let Some(mm) = s.focal_length_mm {
        let (n, d) = to_rational(mm);
        exif.push(rational(0x920A, n, d));
    }
    if let Some(l) = &s.lens {
        exif.push(ascii(0xA434, l));
    }
    let exif_off = 8 + ifd_size(&ifd0) + 12; // + the pointer entry itself
    ifd0.push(long(EXIF_IFD, exif_off as u32));
    let mut buf = Vec::with_capacity(512);
    buf.extend_from_slice(b"II\x2A\x00\x08\x00\x00\x00");
    write_ifd(&mut buf, ifd0, 0);
    debug_assert_eq!(buf.len(), exif_off);
    write_ifd(&mut buf, exif, 0);
    buf
}

/// Patch a copied EXIF block so it describes the exported pixels:
/// Orientation becomes 1 and the Exif IFD's PixelX/YDimension the new size
/// (only when those tags exist; nothing is added). Returns false when the
/// block could not be parsed, in which case it is left untouched.
/// A TIFF block holding only IFD0's Orientation.
pub fn orientation_only(orientation: u16) -> Vec<u8> {
    let mut buf = b"II\x2A\x00\x08\x00\x00\x00".to_vec();
    write_ifd(
        &mut buf,
        vec![short(tag::ORIENTATION, orientation.clamp(1, 8))],
        0,
    );
    buf
}

/// Set IFD0's Orientation in a copied block in place. False when the block
/// can't be parsed or has no one-value SHORT Orientation to set.
pub fn set_orientation(block: &mut [u8], orientation: u16) -> bool {
    let little = block.starts_with(b"II");
    let pos = {
        let Ok((t, first)) = Tiff::parse(block) else {
            return false;
        };
        let Ok(ifd0) = t.read_ifd(first) else {
            return false;
        };
        match ifd0.get(tag::ORIENTATION) {
            Some(e) if e.typ == T_SHORT && e.count == 1 => e.pos,
            _ => return false,
        }
    };
    let o = orientation.clamp(1, 8);
    let b = if little {
        o.to_le_bytes()
    } else {
        o.to_be_bytes()
    };
    match block.get_mut(pos..pos + 2) {
        Some(s) => {
            s.copy_from_slice(&b);
            true
        }
        None => false,
    }
}

pub fn patch_for_export(block: &mut [u8], width: u32, height: u32) -> bool {
    let little = block.starts_with(b"II");
    let mut writes: Vec<(usize, u16, u32)> = Vec::new();
    {
        let Ok((t, first)) = Tiff::parse(block) else {
            return false;
        };
        let Ok(ifd0) = t.read_ifd(first) else {
            return false;
        };
        if let Some(e) = ifd0.get(tag::ORIENTATION) {
            if e.typ == T_SHORT && e.count == 1 {
                writes.push((e.pos, T_SHORT, 1));
            }
        }
        if let Some(p) = ifd0.get(EXIF_IFD).and_then(|e| t.uint(e, 0)) {
            if let Ok(exif) = t.read_ifd(p as u32) {
                for (tg, v) in [(PIXEL_X, width), (PIXEL_Y, height)] {
                    if let Some(e) = exif.get(tg) {
                        if (e.typ == T_SHORT || e.typ == T_LONG) && e.count == 1 {
                            writes.push((e.pos, e.typ, v));
                        }
                    }
                }
            }
        }
    }
    for (pos, typ, v) in writes {
        match typ {
            T_SHORT => {
                let b = if little {
                    (v as u16).to_le_bytes()
                } else {
                    (v as u16).to_be_bytes()
                };
                if let Some(s) = block.get_mut(pos..pos + 2) {
                    s.copy_from_slice(&b);
                }
            }
            _ => {
                let b = if little {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                };
                if let Some(s) = block.get_mut(pos..pos + 4) {
                    s.copy_from_slice(&b);
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif;

    fn summary() -> ExifSummary {
        ExifSummary {
            make: Some("NIKON CORPORATION".into()),
            model: Some("NIKON Z 6_2".into()),
            lens: Some("NIKKOR Z 24-70mm f/2.8 S".into()),
            date_time_original: Some("2026:10:06 14:03:11".into()),
            orientation: 1,
            exposure_time: Some((1, 640)),
            f_number: Some(2.8),
            iso: Some(400),
            focal_length_mm: Some(70.0),
        }
    }

    #[test]
    fn minimal_block_reads_back_with_our_own_reader() {
        let block = minimal(&summary(), 2048, 1365, Some("Caleb"), Some("© 2026"));
        let back = exif::from_bytes(&block).expect("parses");
        assert_eq!(back.model.as_deref(), Some("NIKON Z 6_2"));
        assert_eq!(
            back.date_time_original.as_deref(),
            Some("2026:10:06 14:03:11")
        );
        assert_eq!(back.exposure_time, Some((1, 640)));
        assert_eq!(back.f_number, Some(2.8));
        assert_eq!(back.iso, Some(400));
        assert_eq!(back.focal_length_mm, Some(70.0));
        assert_eq!(back.lens.as_deref(), Some("NIKKOR Z 24-70mm f/2.8 S"));
        assert_eq!(back.orientation, 1);
        // Pixel dimensions live in the Exif IFD and are readable by the walker.
        let (t, first) = Tiff::parse(&block).unwrap();
        let ifd0 = t.read_ifd(first).unwrap();
        let p = t.uint(ifd0.get(EXIF_IFD).unwrap(), 0).unwrap() as u32;
        let ex = t.read_ifd(p).unwrap();
        assert_eq!(t.uint(ex.get(PIXEL_X).unwrap(), 0), Some(2048));
        assert_eq!(t.uint(ex.get(PIXEL_Y).unwrap(), 0), Some(1365));
    }

    #[test]
    fn patch_resets_orientation_and_dimensions() {
        let mut block = minimal(&summary(), 6000, 4000, None, None);
        // Pretend the camera said "rotate 90°".
        {
            let (t, first) = Tiff::parse(&block).unwrap();
            let pos = t
                .read_ifd(first)
                .unwrap()
                .get(tag::ORIENTATION)
                .unwrap()
                .pos;
            block[pos..pos + 2].copy_from_slice(&6u16.to_le_bytes());
        }
        assert_eq!(exif::from_bytes(&block).unwrap().orientation, 6);
        assert!(patch_for_export(&mut block, 1365, 2048));
        assert_eq!(exif::from_bytes(&block).unwrap().orientation, 1);
        let (t, first) = Tiff::parse(&block).unwrap();
        let ifd0 = t.read_ifd(first).unwrap();
        let p = t.uint(ifd0.get(EXIF_IFD).unwrap(), 0).unwrap() as u32;
        let ex = t.read_ifd(p).unwrap();
        assert_eq!(t.uint(ex.get(PIXEL_X).unwrap(), 0), Some(1365));
        assert_eq!(t.uint(ex.get(PIXEL_Y).unwrap(), 0), Some(2048));
        assert!(!patch_for_export(&mut [0u8; 4], 1, 1), "garbage is refused");
    }
}
