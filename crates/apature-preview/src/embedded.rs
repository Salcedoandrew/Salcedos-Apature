//! Locate the JPEG previews cameras embed in RAW files.
//!
//! The container walkers for RAF, CR3, CRW and X3F, the RW2, Olympus and
//! Pentax preview tags, and the rule that only baseline or progressive JPEGs
//! count (lossless SOF3 streams are raw data, not previews) follow
//! radenkovic/extract-raw-preview (MIT); see THIRD_PARTY_NOTICES.md.

use apature_core::tiff::{tag, ByteOrder, Entry, Ifd, Tiff};

use crate::jpeg_info;

/// One JPEG blob found inside a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedJpeg {
    pub offset: usize,
    pub length: usize,
    pub width: u32,
    pub height: u32,
    /// Where it came from, for diagnostics ("IFD0", "SubIFD1", "MakerNote", "file").
    pub source: String,
}

impl EmbeddedJpeg {
    pub fn long_side(&self) -> u32 {
        self.width.max(self.height)
    }
    pub fn bytes<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[self.offset..self.offset + self.length]
    }
}

/// Which preview [`extract`] picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Prefer {
    /// The biggest picture (the default).
    #[default]
    Largest,
    /// The smallest picture, usually the Exif thumbnail.
    Smallest,
    /// The smallest picture at least this many pixels on its long side,
    /// or the largest when none is that big.
    AtLeast(u32),
}

/// Default ceiling for [`extract`]: previews bigger than this are skipped.
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

/// The container a file uses, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Jpeg,
    /// TIFF and its relatives: CR2, NEF, NRW, ARW, SRF, SR2, PEF, DNG, 3FR,
    /// FFF, IIQ, SRW, ORF (`IIRO`/`MMOR`/`IIRS`), RW2 and RWL (`IIU`).
    Tiff,
    /// Fujifilm RAF.
    Raf,
    /// Canon CR3 (ISO base media file).
    Cr3,
    /// Canon CRW (CIFF).
    Crw,
    /// Sigma X3F.
    X3f,
    Unknown,
}

pub fn container(data: &[u8]) -> Container {
    if jpeg_info::is_jpeg(data) {
        Container::Jpeg
    } else if data.starts_with(b"FUJIFILMCCD-RAW ") {
        Container::Raf
    } else if data.len() >= 14 && data[0..2] == *b"II" && data[6..14] == *b"HEAPCCDR" {
        Container::Crw
    } else if data.starts_with(b"FOVb") {
        Container::X3f
    } else if data.get(4..8) == Some(b"ftyp") && data.get(8..12) == Some(b"crx ") {
        Container::Cr3
    } else if Tiff::parse(data).is_ok() {
        Container::Tiff
    } else {
        Container::Unknown
    }
}

/// Every embedded baseline or progressive JPEG in `data`, largest first.
///
/// A plain JPEG file is returned as one candidate covering the whole file.
/// TIFF-based RAWs are walked through IFD0 and its chain, SubIFDs, the Exif
/// IFD and the Nikon, Olympus and Pentax maker notes (plus the RW2
/// `JpgFromRaw` tag); RAF, CR3, CRW and X3F have walkers of their own.
pub fn find_embedded_jpegs(data: &[u8]) -> Vec<EmbeddedJpeg> {
    let mut out = Vec::new();
    match container(data) {
        Container::Jpeg => {
            out.push(EmbeddedJpeg {
                offset: 0,
                length: data.len(),
                width: 0,
                height: 0,
                source: "file".into(),
            });
            return finish(data, out);
        }
        Container::Raf => raf(data, &mut out),
        Container::Cr3 => bmff(data, 0, data.len(), 0, &mut out),
        Container::Crw => {
            let hlen = le32(data, 2).unwrap_or(0) as usize;
            ciff(data, hlen, data.len(), 0, &mut out);
        }
        Container::X3f => x3f(data, &mut out),
        Container::Tiff => tiff_previews(data, 0, &mut out),
        Container::Unknown => {}
    }
    // A preview JPEG often carries its own Exif thumbnail.
    let outer: Vec<(usize, usize)> = out.iter().map(|c| (c.offset, c.length)).collect();
    for (off, len) in outer {
        if let Some(jpeg) = data.get(off..off.saturating_add(len)) {
            if let Some((t, _)) = jpeg_info::exif_tiff(jpeg) {
                tiff_previews(data, off + t, &mut out);
            }
        }
    }
    finish(data, out)
}

/// Every usable preview, largest first (the same list as [`find_embedded_jpegs`]).
pub fn list_all(data: &[u8]) -> Vec<EmbeddedJpeg> {
    find_embedded_jpegs(data)
}

/// The preview `prefer` asks for among those no bigger than `max_bytes`.
pub fn extract(data: &[u8], prefer: Prefer, max_bytes: usize) -> Option<EmbeddedJpeg> {
    pick(find_embedded_jpegs(data), prefer, max_bytes)
}

/// Choose from a largest-first candidate list.
pub fn pick(all: Vec<EmbeddedJpeg>, prefer: Prefer, max_bytes: usize) -> Option<EmbeddedJpeg> {
    let fit: Vec<EmbeddedJpeg> = all.into_iter().filter(|c| c.length <= max_bytes).collect();
    match prefer {
        Prefer::Largest => fit.into_iter().next(),
        Prefer::Smallest => fit.into_iter().last(),
        Prefer::AtLeast(n) => {
            let largest = fit.first().cloned();
            fit.into_iter()
                .filter(|c| c.long_side() >= n)
                .min_by_key(|c| c.long_side())
                .or(largest)
        }
    }
}

fn finish(data: &[u8], mut out: Vec<EmbeddedJpeg>) -> Vec<EmbeddedJpeg> {
    // Keep only in-range, decodable JPEGs; take the size from the frame
    // header (what the decoder will produce); dedupe; largest first.
    out.retain_mut(|c| {
        let Some(end) = c.offset.checked_add(c.length) else {
            return false;
        };
        if c.length <= 4 || end > data.len() || !jpeg_info::is_jpeg(&data[c.offset..]) {
            return false;
        }
        match jpeg_info::frame(&data[c.offset..end]) {
            Some(f) if f.decodable() && f.width > 0 && f.height > 0 => {
                c.width = f.width;
                c.height = f.height;
                true
            }
            _ => false,
        }
    });
    out.sort_by_key(|c| c.offset);
    out.dedup_by_key(|c| c.offset);
    out.sort_by(|a, b| {
        b.long_side()
            .cmp(&a.long_side())
            .then(b.length.cmp(&a.length))
    });
    out
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn le32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn le16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

/// A JPEG starting at `offset`, its length found by walking to its EOI
/// (never past `end`).
fn jpeg_at(data: &[u8], offset: usize, end: usize, source: &str) -> Option<EmbeddedJpeg> {
    let bytes = data.get(offset..end.min(data.len()))?;
    let length = jpeg_info::byte_length(bytes)?;
    Some(EmbeddedJpeg {
        offset,
        length,
        width: 0,
        height: 0,
        source: source.into(),
    })
}

/// Fujifilm RAF: a big-endian JPEG offset and length at bytes 84 and 88.
pub fn raf_jpeg(data: &[u8]) -> Option<(usize, usize)> {
    if !data.starts_with(b"FUJIFILMCCD-RAW ") {
        return None;
    }
    let (off, len) = (be32(data, 84)? as usize, be32(data, 88)? as usize);
    data.get(off..off.checked_add(len)?)?;
    Some((off, len))
}

fn raf(data: &[u8], out: &mut Vec<EmbeddedJpeg>) {
    if let Some((offset, length)) = raf_jpeg(data) {
        out.push(EmbeddedJpeg {
            offset,
            length,
            width: 0,
            height: 0,
            source: "RAF header".into(),
        });
    } else if let Some(off) = be32(data, 84) {
        if let Some(c) = jpeg_at(data, off as usize, data.len(), "RAF header") {
            out.push(c);
        }
    }
}

/// Canon CR3 is ISO base media: previews sit at the start of the `THMB`,
/// `PRVW` and `mdat` payloads. Container boxes are walked; `mdat` is not.
fn bmff(data: &[u8], start: usize, end: usize, depth: u8, out: &mut Vec<EmbeddedJpeg>) {
    const CONTAINERS: [&[u8; 4]; 12] = [
        b"moov", b"trak", b"mdia", b"minf", b"stbl", b"dinf", b"udta", b"meta", b"iprp", b"ipco",
        b"uuid", b"CRAW",
    ];
    if depth > 8 {
        return;
    }
    let mut off = start;
    let mut n = 0;
    while off + 8 <= end && n < 256 {
        let Some(size32) = be32(data, off) else {
            return;
        };
        let typ = &data[off + 4..off + 8];
        let (mut size, mut header) = (size32 as usize, 8usize);
        if size32 == 1 {
            let Some(big) = data.get(off + 8..off + 16) else {
                return;
            };
            size = u64::from_be_bytes(big.try_into().unwrap_or_default()) as usize;
            header = 16;
        } else if size32 == 0 {
            size = end - off;
        }
        if size < header || off.saturating_add(size) > end {
            return;
        }
        let (payload, box_end) = (off + header, off + size);
        // A JPEG within the first 64 bytes of the payload.
        let scan_end = (payload + 64).min(box_end.saturating_sub(2));
        if let Some(i) = (payload..scan_end)
            .find(|&i| data[i] == 0xFF && data[i + 1] == 0xD8 && data[i + 2] == 0xFF)
        {
            let name = format!("CR3 {}", String::from_utf8_lossy(typ));
            if let Some(c) = jpeg_at(data, i, box_end, &name) {
                out.push(c);
            }
        }
        if CONTAINERS.iter().any(|c| c[..] == *typ) && box_end > payload {
            let inner = match typ {
                b"meta" => payload + 4,
                b"uuid" => payload + 16,
                _ => payload,
            };
            if inner < box_end {
                bmff(data, inner, box_end, depth + 1, out);
            }
        }
        off = box_end;
        n += 1;
    }
}

/// Canon CRW (CIFF): heaps of 10-byte records; 0x2007 is the JPEG, records
/// of type 0x28xx and 0x30xx are sub-heaps.
fn ciff(data: &[u8], start: usize, end: usize, depth: u8, out: &mut Vec<EmbeddedJpeg>) {
    if depth > 4 || end <= start + 4 || end > data.len() {
        return;
    }
    let Some(table) = le32(data, end - 4).map(|t| start + t as usize) else {
        return;
    };
    let Some(count) = le16(data, table) else {
        return;
    };
    for i in 0..count.min(512) as usize {
        let rec = table + 2 + i * 10;
        let (Some(typ), Some(len), Some(roff)) =
            (le16(data, rec), le32(data, rec + 2), le32(data, rec + 6))
        else {
            return;
        };
        let at = start + roff as usize;
        let len = len as usize;
        if at.saturating_add(len) > end {
            continue;
        }
        if matches!(typ >> 8, 0x28 | 0x30) {
            ciff(data, at, at + len, depth + 1, out);
        } else if typ == 0x2007 {
            out.push(EmbeddedJpeg {
                offset: at,
                length: len,
                width: 0,
                height: 0,
                source: "CRW JpgFromRaw".into(),
            });
        }
    }
}

/// Sigma X3F: a `SECd` directory (its offset in the file's last 4 bytes)
/// lists `IMA2`/`IMAG` sections; a JPEG preview follows a 28-byte `SECi` header.
fn x3f(data: &[u8], out: &mut Vec<EmbeddedJpeg>) {
    let Some(dir) = data.len().checked_sub(4).and_then(|e| le32(data, e)) else {
        return;
    };
    let dir = dir as usize;
    if data.get(dir..dir + 4) != Some(b"SECd") {
        return;
    }
    let Some(n) = le32(data, dir + 8) else {
        return;
    };
    for i in 0..n.min(256) as usize {
        let e = dir + 12 + i * 12;
        let (Some(off), Some(len)) = (le32(data, e), le32(data, e + 4)) else {
            return;
        };
        let kind = data.get(e + 8..e + 12);
        if kind != Some(b"IMA2") && kind != Some(b"IMAG") {
            continue;
        }
        let (off, len) = (off as usize, len as usize);
        if len > 28 && data.get(off..off + 4) == Some(b"SECi") {
            out.push(EmbeddedJpeg {
                offset: off + 28,
                length: len - 28,
                width: 0,
                height: 0,
                source: "X3F IMA2".into(),
            });
        }
    }
}

/// Walk the TIFF at `data[base..]`. Its offsets are relative to `base`;
/// candidates are reported file-relative.
fn tiff_previews(data: &[u8], base: usize, out: &mut Vec<EmbeddedJpeg>) {
    let Some(sub) = data.get(base..) else {
        return;
    };
    let Ok((tiff, first)) = Tiff::parse(sub) else {
        return;
    };
    let mut found = Vec::new();
    let mut visited = Vec::new();
    let mut ifd_offset = first;
    let mut n = 0;
    while ifd_offset != 0 && n < 16 {
        walk_ifd(
            &tiff,
            ifd_offset,
            &format!("IFD{n}"),
            0,
            &mut found,
            &mut visited,
        );
        let next = tiff.read_ifd(ifd_offset).map(|i| i.next).unwrap_or(0);
        if next == ifd_offset {
            break;
        }
        ifd_offset = next;
        n += 1;
    }
    for mut c in found {
        c.offset += base;
        out.push(c);
    }
}

fn walk_ifd(
    tiff: &Tiff,
    offset: u32,
    name: &str,
    depth: u8,
    out: &mut Vec<EmbeddedJpeg>,
    visited: &mut Vec<u32>,
) {
    if depth > 4 || visited.contains(&offset) || visited.len() > 64 {
        return;
    }
    visited.push(offset);
    let Ok(ifd) = tiff.read_ifd(offset) else {
        return;
    };

    candidates_in_ifd(tiff, &ifd, name, out);

    // Panasonic RW2 / Leica RWL: JpgFromRaw, stored as the tag's own value.
    if let Some(e) = ifd.get(0x002E) {
        if let Some(b) = tiff.value_bytes(e) {
            out.push(EmbeddedJpeg {
                offset: pos_in(tiff, b),
                length: b.len(),
                width: 0,
                height: 0,
                source: format!("{name}:JpgFromRaw"),
            });
        }
    }

    if let Some(e) = ifd.get(tag::SUB_IFDS) {
        for (i, sub) in tiff.uints(e).into_iter().enumerate().take(8) {
            walk_ifd(
                tiff,
                sub as u32,
                &format!("{name}/SubIFD{i}"),
                depth + 1,
                out,
                visited,
            );
        }
    }
    if let Some(e) = ifd.get(tag::EXIF_IFD) {
        if let Some(exif_off) = tiff.uint(e, 0) {
            walk_ifd(
                tiff,
                exif_off as u32,
                &format!("{name}/Exif"),
                depth + 1,
                out,
                visited,
            );
        }
    }
    if let Some(e) = ifd.get(tag::MAKER_NOTE) {
        nikon_makernote(tiff, e, out);
        olympus_makernote(tiff, e, out);
        pentax_makernote(tiff, e, out);
    }
}

/// Offset of a slice of `tiff`'s data within that data.
fn pos_in(tiff: &Tiff, b: &[u8]) -> usize {
    b.as_ptr() as usize - tiff.data().as_ptr() as usize
}

fn candidates_in_ifd(tiff: &Tiff, ifd: &Ifd, name: &str, out: &mut Vec<EmbeddedJpeg>) {
    // Old-style JPEG pointer pair (Exif thumbnails, Nikon and Sony previews).
    if let (Some(o), Some(l)) = (
        ifd.get(tag::JPEG_INTERCHANGE_FORMAT),
        ifd.get(tag::JPEG_INTERCHANGE_FORMAT_LENGTH),
    ) {
        if let (Some(off), Some(len)) = (tiff.uint(o, 0), tiff.uint(l, 0)) {
            out.push(EmbeddedJpeg {
                offset: off as usize,
                length: len as usize,
                width: 0,
                height: 0,
                source: format!("{name}:JPEGInterchangeFormat"),
            });
        }
    }

    // Strip-based JPEG (compression 6 = old JPEG, 7 = new JPEG). Nikon SubIFD0,
    // Canon CR2 IFD0 and DNG previews look like this: a single strip holding
    // one whole JPEG stream. Lossless raw data looks the same and is dropped
    // later by its SOF3 frame.
    let compression = ifd.get(tag::COMPRESSION).and_then(|e| tiff.uint(e, 0));
    if matches!(compression, Some(6) | Some(7)) {
        if let (Some(o), Some(l)) = (ifd.get(tag::STRIP_OFFSETS), ifd.get(tag::STRIP_BYTE_COUNTS)) {
            if o.count == 1 && l.count == 1 {
                if let (Some(off), Some(len)) = (tiff.uint(o, 0), tiff.uint(l, 0)) {
                    out.push(EmbeddedJpeg {
                        offset: off as usize,
                        length: len as usize,
                        width: 0,
                        height: 0,
                        source: format!("{name}:strip"),
                    });
                }
            }
        }
    }
}

/// Nikon MakerNote: "Nikon\0" + 4 bytes version, then a complete TIFF header
/// at +10 whose offsets are relative to that header. Tag 0x0011 points at a
/// PreviewIFD holding a JPEGInterchangeFormat pair, also relative to +10.
fn nikon_makernote(tiff: &Tiff, e: &Entry, out: &mut Vec<EmbeddedJpeg>) {
    let Some(mn) = tiff.value_bytes(e) else {
        return;
    };
    if mn.len() < 18 || !mn.starts_with(b"Nikon\0") {
        return;
    }
    let base_rel = 10usize;
    let Some(inner) = mn.get(base_rel..) else {
        return;
    };
    let Ok((inner_tiff, first)) = Tiff::parse(inner) else {
        return;
    };
    let Ok(ifd) = inner_tiff.read_ifd(first) else {
        return;
    };
    let Some(pv) = ifd.get(0x0011) else { return };
    let Some(pv_off) = inner_tiff.uint(pv, 0) else {
        return;
    };
    let Ok(pifd) = inner_tiff.read_ifd(pv_off as u32) else {
        return;
    };
    let (Some(o), Some(l)) = (
        pifd.get(tag::JPEG_INTERCHANGE_FORMAT),
        pifd.get(tag::JPEG_INTERCHANGE_FORMAT_LENGTH),
    ) else {
        return;
    };
    let (Some(off), Some(len)) = (inner_tiff.uint(o, 0), inner_tiff.uint(l, 0)) else {
        return;
    };
    out.push(EmbeddedJpeg {
        offset: pos_in(tiff, mn) + base_rel + off as usize,
        length: len as usize,
        width: 0,
        height: 0,
        source: "MakerNote:PreviewIFD".into(),
    });
}

fn order_at(b: &[u8]) -> Option<ByteOrder> {
    match b.get(0..2)? {
        b"II" => Some(ByteOrder::Little),
        b"MM" => Some(ByteOrder::Big),
        _ => None,
    }
}

/// Push the JPEG a start/length tag pair names, offsets counted from `rel_pos`.
fn push_pair(
    t: &Tiff,
    ifd: &Ifd,
    (start, len): (u16, u16),
    rel_pos: usize,
    source: &str,
    out: &mut Vec<EmbeddedJpeg>,
) {
    let (Some(o), Some(l)) = (ifd.get(start), ifd.get(len)) else {
        return;
    };
    if let (Some(off), Some(len)) = (t.uint(o, 0), t.uint(l, 0)) {
        if len > 0 {
            out.push(EmbeddedJpeg {
                offset: rel_pos + off as usize,
                length: len as usize,
                width: 0,
                height: 0,
                source: source.into(),
            });
        }
    }
}

/// Olympus MakerNote. "OLYMPUS\0II" and "OM SYSTEM\0..." notes count offsets
/// from the note's start; the old "OLYMP\0" note counts them from the TIFF
/// header. The preview is in the CameraSettings sub-IFD (0x2020) as
/// PreviewImageStart/Length (0x0101/0x0102); old cameras keep it in the main
/// note (0x0088/0x0089).
fn olympus_makernote(tiff: &Tiff, e: &Entry, out: &mut Vec<EmbeddedJpeg>) {
    let Some(mn) = tiff.value_bytes(e) else {
        return;
    };
    let mn_pos = pos_in(tiff, mn);
    let (rel, rel_pos, ifd_at, order) = if mn.starts_with(b"OLYMPUS\0") {
        (mn, mn_pos, 12u32, order_at(&mn[8..]))
    } else if mn.starts_with(b"OM SYSTEM\0") {
        (mn, mn_pos, 16u32, mn.get(12..).and_then(order_at))
    } else if mn.starts_with(b"OLYMP\0") {
        (tiff.data(), 0, mn_pos as u32 + 8, Some(tiff.order()))
    } else {
        return;
    };
    let Some(order) = order else { return };
    let t = Tiff::from_parts(rel, order);
    let Ok(ifd) = t.read_ifd(ifd_at) else { return };
    push_pair(
        &t,
        &ifd,
        (0x0088, 0x0089),
        rel_pos,
        "MakerNote:Olympus Preview",
        out,
    );
    let cs_off = ifd.get(0x2020).and_then(|cs| t.uint(cs, 0));
    if let Some(Ok(cs)) = cs_off.map(|o| t.read_ifd(o as u32)) {
        push_pair(
            &t,
            &cs,
            (0x0101, 0x0102),
            rel_pos,
            "MakerNote:Olympus CameraSettings Preview",
            out,
        );
    }
}

/// Pentax MakerNote: "AOC\0" + byte order (offsets from the TIFF header) or
/// "PENTAX \0" + byte order (offsets from the note's start). The preview is
/// PreviewImageStart/Length (0x0004/0x0003).
fn pentax_makernote(tiff: &Tiff, e: &Entry, out: &mut Vec<EmbeddedJpeg>) {
    let Some(mn) = tiff.value_bytes(e) else {
        return;
    };
    let mn_pos = pos_in(tiff, mn);
    let (rel, rel_pos, ifd_at, order) = if mn.starts_with(b"AOC\0") {
        let order = mn.get(4..).and_then(order_at).unwrap_or(tiff.order());
        (tiff.data(), 0, mn_pos as u32 + 6, order)
    } else if mn.starts_with(b"PENTAX \0") {
        let Some(order) = mn.get(8..).and_then(order_at) else {
            return;
        };
        (mn, mn_pos, 10u32, order)
    } else {
        return;
    };
    let t = Tiff::from_parts(rel, order);
    let Ok(ifd) = t.read_ifd(ifd_at) else { return };
    push_pair(
        &t,
        &ifd,
        (0x0004, 0x0003),
        rel_pos,
        "MakerNote:Pentax Preview",
        out,
    );
}

/// An uncompressed 8-bit RGB picture stored in a TIFF IFD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbThumbnail {
    pub width: u32,
    pub height: u32,
    /// Where it came from ("IFD0", "IFD0/SubIFD1", ...).
    pub source: String,
    pub data: Vec<u8>,
}

/// The largest uncompressed 8-bit RGB picture in a TIFF-based RAW.
///
/// Hasselblad 3FR/FFF and Phase One IIQ files carry no JPEG at all; their
/// only preview is a plain RGB strip (1440x1080 on an X1D, a few hundred
/// pixels on older backs). [`crate::Source`] re-encodes it as a JPEG when a
/// file has no JPEG preview.
pub fn rgb_thumbnail(data: &[u8]) -> Option<RgbThumbnail> {
    if container(data) != Container::Tiff {
        return None;
    }
    let (tiff, first) = Tiff::parse(data).ok()?;
    let mut best: Option<RgbThumbnail> = None;
    let mut offset = first;
    let mut n = 0;
    let mut seen = Vec::new();
    while offset != 0 && n < 8 && !seen.contains(&offset) {
        seen.push(offset);
        let Ok(ifd) = tiff.read_ifd(offset) else {
            break;
        };
        let name = format!("IFD{n}");
        let mut ifds = vec![(name.clone(), ifd.clone())];
        if let Some(e) = ifd.get(tag::SUB_IFDS) {
            for (i, o) in tiff.uints(e).into_iter().enumerate().take(8) {
                if let Ok(sub) = tiff.read_ifd(o as u32) {
                    ifds.push((format!("{name}/SubIFD{i}"), sub));
                }
            }
        }
        for (name, ifd) in ifds {
            if let Some(t) = rgb_in_ifd(&tiff, &ifd, &name) {
                if best
                    .as_ref()
                    .is_none_or(|b| t.width * t.height > b.width * b.height)
                {
                    best = Some(t);
                }
            }
        }
        offset = ifd.next;
        n += 1;
    }
    best
}

fn rgb_in_ifd(tiff: &Tiff, ifd: &Ifd, name: &str) -> Option<RgbThumbnail> {
    let get = |t: u16, default: u64| ifd.get(t).and_then(|e| tiff.uint(e, 0)).unwrap_or(default);
    // Uncompressed, RGB, 3 samples, chunky.
    if get(tag::COMPRESSION, 1) != 1
        || get(0x0106, 0) != 2
        || get(0x0115, 1) != 3
        || get(0x011C, 1) != 1
    {
        return None;
    }
    if !tiff.uints(ifd.get(0x0102)?).iter().all(|&b| b == 8) {
        return None;
    }
    let (w, h) = (get(tag::IMAGE_WIDTH, 0), get(tag::IMAGE_LENGTH, 0));
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return None;
    }
    let need = (w * h * 3) as usize;
    let offs = tiff.uints(ifd.get(tag::STRIP_OFFSETS)?);
    let lens = tiff.uints(ifd.get(tag::STRIP_BYTE_COUNTS)?);
    if offs.len() != lens.len() || offs.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(need);
    for (o, l) in offs.into_iter().zip(lens) {
        let (o, l) = (o as usize, l as usize);
        out.extend_from_slice(tiff.data().get(o..o.checked_add(l)?)?);
    }
    if out.len() < need {
        return None;
    }
    out.truncate(need);
    Some(RgbThumbnail {
        width: w as u32,
        height: h as u32,
        source: name.into(),
        data: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::tiff::testutil::build_le;

    fn fake_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 8];
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        // Three 3-byte component specs complete the 17-byte SOF segment.
        v.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn plain_jpeg_is_its_own_preview() {
        let j = fake_jpeg(300, 200);
        let found = find_embedded_jpegs(&j);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].width, found[0].height), (300, 200));
        assert_eq!(found[0].source, "file");
    }

    #[test]
    fn finds_interchange_and_strip_jpegs_in_tiff() {
        let small = fake_jpeg(160, 120);
        let big = fake_jpeg(6000, 4000);
        let payload_off = 200usize;
        let big_off = payload_off + small.len();
        let mut payload = small.clone();
        payload.extend_from_slice(&big);
        let data = build_le(
            &[
                (tag::COMPRESSION, 3, 1, [6, 0, 0, 0]),
                (tag::STRIP_OFFSETS, 4, 1, (big_off as u32).to_le_bytes()),
                (
                    tag::STRIP_BYTE_COUNTS,
                    4,
                    1,
                    (big.len() as u32).to_le_bytes(),
                ),
                (
                    tag::JPEG_INTERCHANGE_FORMAT,
                    4,
                    1,
                    (payload_off as u32).to_le_bytes(),
                ),
                (
                    tag::JPEG_INTERCHANGE_FORMAT_LENGTH,
                    4,
                    1,
                    (small.len() as u32).to_le_bytes(),
                ),
            ],
            &payload,
            payload_off,
        );
        let found = find_embedded_jpegs(&data);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].long_side(), 6000);
        assert_eq!(found[1].long_side(), 160);
        assert_eq!(found[0].bytes(&data), &big[..]);
    }

    #[test]
    fn ignores_out_of_range_pointers() {
        let data = build_le(
            &[
                (tag::JPEG_INTERCHANGE_FORMAT, 4, 1, 100_000u32.to_le_bytes()),
                (
                    tag::JPEG_INTERCHANGE_FORMAT_LENGTH,
                    4,
                    1,
                    10u32.to_le_bytes(),
                ),
            ],
            b"",
            0,
        );
        assert!(find_embedded_jpegs(&data).is_empty());
        assert!(find_embedded_jpegs(b"").is_empty());
    }

    fn cand(w: u32, len: usize) -> EmbeddedJpeg {
        EmbeddedJpeg {
            offset: 0,
            length: len,
            width: w,
            height: w / 2,
            source: String::new(),
        }
    }

    #[test]
    fn rejects_lossless_jpeg_streams() {
        let mut lossless = fake_jpeg(6000, 4000);
        lossless[3] = 0xC3;
        assert!(find_embedded_jpegs(&lossless).is_empty());
        let mut progressive = fake_jpeg(600, 400);
        progressive[3] = 0xC2;
        assert_eq!(find_embedded_jpegs(&progressive).len(), 1);
    }

    #[test]
    fn raf_header_points_at_the_jpeg() {
        let j = fake_jpeg(6240, 4160);
        let mut raf = b"FUJIFILMCCD-RAW 0201FF129502".to_vec();
        raf.resize(148, 0);
        raf[84..88].copy_from_slice(&148u32.to_be_bytes());
        raf[88..92].copy_from_slice(&(j.len() as u32).to_be_bytes());
        raf.extend_from_slice(&j);
        assert_eq!(container(&raf), Container::Raf);
        let found = find_embedded_jpegs(&raf);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].offset, found[0].long_side()), (148, 6240));
    }

    #[test]
    fn cr3_preview_in_a_uuid_box() {
        fn bx(typ: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut v = ((8 + body.len()) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(typ);
            v.extend_from_slice(body);
            v
        }
        let mut prvw = vec![0u8; 24];
        prvw.extend_from_slice(&fake_jpeg(1620, 1080));
        let mut uuid = vec![0u8; 16];
        uuid.extend_from_slice(&bx(b"PRVW", &prvw));
        let mut file = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
        file.extend_from_slice(&bx(b"moov", &bx(b"uuid", &uuid)));
        assert_eq!(container(&file), Container::Cr3);
        let found = find_embedded_jpegs(&file);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].long_side(), 1620);
    }

    #[test]
    fn crw_heap_record_2007_is_the_jpeg() {
        let j = fake_jpeg(2048, 1360);
        let mut d = b"II".to_vec();
        d.extend_from_slice(&26u32.to_le_bytes());
        d.extend_from_slice(b"HEAPCCDRV1\0\0\0\0\0\0\0\0\0\0");
        let heap = d.len();
        d.extend_from_slice(&j);
        let table = d.len() - heap;
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&0x2007u16.to_le_bytes());
        d.extend_from_slice(&(j.len() as u32).to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&(table as u32).to_le_bytes());
        assert_eq!(container(&d), Container::Crw);
        let found = find_embedded_jpegs(&d);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].offset, found[0].long_side()), (heap, 2048));
    }

    #[test]
    fn x3f_ima2_section_holds_the_jpeg() {
        let j = fake_jpeg(1792, 1184);
        let mut d = b"FOVb".to_vec();
        d.resize(64, 0);
        let sec = d.len();
        d.extend_from_slice(b"SECi");
        d.resize(sec + 28, 0);
        d.extend_from_slice(&j);
        let dir = d.len();
        d.extend_from_slice(b"SECd");
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&(sec as u32).to_le_bytes());
        d.extend_from_slice(&((28 + j.len()) as u32).to_le_bytes());
        d.extend_from_slice(b"IMA2");
        d.extend_from_slice(&(dir as u32).to_le_bytes());
        let found = find_embedded_jpegs(&d);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].offset, found[0].long_side()), (sec + 28, 1792));
    }

    #[test]
    fn pick_honours_preference_and_size_cap() {
        let all = vec![cand(6000, 9_000_000), cand(1620, 300_000), cand(160, 8_000)];
        let p = |prefer, max| pick(all.clone(), prefer, max).map(|c| c.width);
        assert_eq!(p(Prefer::Largest, usize::MAX), Some(6000));
        assert_eq!(p(Prefer::Largest, DEFAULT_MAX_BYTES), Some(1620));
        assert_eq!(p(Prefer::Smallest, DEFAULT_MAX_BYTES), Some(160));
        assert_eq!(p(Prefer::AtLeast(1000), usize::MAX), Some(1620));
        assert_eq!(p(Prefer::AtLeast(9000), usize::MAX), Some(6000));
        assert_eq!(p(Prefer::Largest, 100), None);
    }

    #[test]
    fn finds_uncompressed_rgb_thumbnail() {
        let (w, h) = (4u32, 3u32);
        let pixels: Vec<u8> = (0..w * h * 3).map(|i| i as u8).collect();
        let data = build_le(
            &[
                (tag::IMAGE_WIDTH, 3, 1, [4, 0, 0, 0]),
                (tag::IMAGE_LENGTH, 3, 1, [3, 0, 0, 0]),
                (0x0102, 3, 1, [8, 0, 0, 0]),
                (tag::COMPRESSION, 3, 1, [1, 0, 0, 0]),
                (0x0106, 3, 1, [2, 0, 0, 0]),
                (tag::STRIP_OFFSETS, 4, 1, 200u32.to_le_bytes()),
                (0x0115, 3, 1, [3, 0, 0, 0]),
                (tag::STRIP_BYTE_COUNTS, 4, 1, 36u32.to_le_bytes()),
            ],
            &pixels,
            200,
        );
        assert!(find_embedded_jpegs(&data).is_empty());
        let t = rgb_thumbnail(&data).unwrap();
        assert_eq!((t.width, t.height, t.data), (4, 3, pixels));
    }
}
