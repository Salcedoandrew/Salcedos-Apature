//! A deliberately small TIFF/IFD reader over an in-memory byte slice.
//!
//! RAW files from Nikon (NEF), Canon (CR2), Sony (ARW), Adobe (DNG) and most
//! others are TIFF containers, and the Exif block inside a JPEG is one too.
//! All we ever need is to walk IFDs and read a handful of tags, so this stays
//! tiny and panic-free on hostile input: every read is bounds-checked and
//! IFD entry counts are capped.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    Little,
    Big,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TiffError {
    TooShort,
    BadMagic,
    OutOfBounds(usize),
}

impl fmt::Display for TiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TiffError::TooShort => write!(f, "data too short for a TIFF header"),
            TiffError::BadMagic => write!(f, "not a TIFF header"),
            TiffError::OutOfBounds(o) => write!(f, "offset {o} is outside the data"),
        }
    }
}
impl std::error::Error for TiffError {}

/// One directory entry. `pos` is the byte offset of the 4-byte value/offset
/// field, so values can be resolved lazily and without copying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub tag: u16,
    pub typ: u16,
    pub count: u32,
    pub pos: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Ifd {
    pub entries: Vec<Entry>,
    pub next: u32,
}

impl Ifd {
    pub fn get(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag)
    }
}

pub mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 0x00FE;
    pub const IMAGE_WIDTH: u16 = 0x0100;
    pub const IMAGE_LENGTH: u16 = 0x0101;
    pub const COMPRESSION: u16 = 0x0103;
    pub const MAKE: u16 = 0x010F;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const SUB_IFDS: u16 = 0x014A;
    pub const JPEG_INTERCHANGE_FORMAT: u16 = 0x0201;
    pub const JPEG_INTERCHANGE_FORMAT_LENGTH: u16 = 0x0202;
    pub const EXPOSURE_TIME: u16 = 0x829A;
    pub const F_NUMBER: u16 = 0x829D;
    pub const EXIF_IFD: u16 = 0x8769;
    pub const ISO: u16 = 0x8827;
    pub const DATE_TIME_ORIGINAL: u16 = 0x9003;
    pub const FOCAL_LENGTH: u16 = 0x920A;
    pub const MAKER_NOTE: u16 = 0x927C;
    pub const LENS_MODEL: u16 = 0xA434;
}

const MAX_ENTRIES: usize = 4096;

#[derive(Clone, Copy)]
pub struct Tiff<'a> {
    data: &'a [u8],
    order: ByteOrder,
}

impl fmt::Debug for Tiff<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tiff")
            .field("len", &self.data.len())
            .field("order", &self.order)
            .finish()
    }
}

impl<'a> Tiff<'a> {
    /// Parse a TIFF header at the start of `data`. Returns the reader and the
    /// offset of the first IFD.
    pub fn parse(data: &'a [u8]) -> Result<(Self, u32), TiffError> {
        if data.len() < 8 {
            return Err(TiffError::TooShort);
        }
        let order = match &data[0..2] {
            b"II" => ByteOrder::Little,
            b"MM" => ByteOrder::Big,
            _ => return Err(TiffError::BadMagic),
        };
        let t = Tiff { data, order };
        // 42 = TIFF, 0x4F52 = Olympus ORF, 0x5352 = Olympus ORF (IIRS), 0x55 = Panasonic RW2.
        match t.u16(2)? {
            42 | 0x4F52 | 0x5352 | 0x0055 => {}
            _ => return Err(TiffError::BadMagic),
        }
        let first = t.u32(4)?;
        Ok((t, first))
    }

    pub fn from_parts(data: &'a [u8], order: ByteOrder) -> Self {
        Tiff { data, order }
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn order(&self) -> ByteOrder {
        self.order
    }

    pub fn u16(&self, off: usize) -> Result<u16, TiffError> {
        let b = self
            .data
            .get(off..off + 2)
            .ok_or(TiffError::OutOfBounds(off))?;
        Ok(match self.order {
            ByteOrder::Little => u16::from_le_bytes([b[0], b[1]]),
            ByteOrder::Big => u16::from_be_bytes([b[0], b[1]]),
        })
    }

    pub fn u32(&self, off: usize) -> Result<u32, TiffError> {
        let b = self
            .data
            .get(off..off + 4)
            .ok_or(TiffError::OutOfBounds(off))?;
        Ok(match self.order {
            ByteOrder::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            ByteOrder::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        })
    }

    pub fn read_ifd(&self, offset: u32) -> Result<Ifd, TiffError> {
        let off = offset as usize;
        let count = self.u16(off)? as usize;
        let count = count.min(MAX_ENTRIES);
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = off + 2 + i * 12;
            if e + 12 > self.data.len() {
                break;
            }
            entries.push(Entry {
                tag: self.u16(e)?,
                typ: self.u16(e + 2)?,
                count: self.u32(e + 4)?,
                pos: e + 8,
            });
        }
        let next_pos = off + 2 + entries.len() * 12;
        let next = self.u32(next_pos).unwrap_or(0);
        Ok(Ifd { entries, next })
    }

    pub fn type_size(typ: u16) -> usize {
        match typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 => 8,
            _ => 1,
        }
    }

    /// The raw bytes of an entry's value, whether inline or at an offset.
    pub fn value_bytes(&self, e: &Entry) -> Option<&'a [u8]> {
        let total = Self::type_size(e.typ).checked_mul(e.count as usize)?;
        if total <= 4 {
            self.data.get(e.pos..e.pos + total)
        } else {
            let off = self.u32(e.pos).ok()? as usize;
            self.data.get(off..off.checked_add(total)?)
        }
    }

    /// Read the `i`th integer of an entry (BYTE, SHORT, LONG, IFD, or signed).
    pub fn uint(&self, e: &Entry, i: usize) -> Option<u64> {
        if i >= e.count as usize {
            return None;
        }
        let bytes = self.value_bytes(e)?;
        let sz = Self::type_size(e.typ);
        let b = bytes.get(i * sz..(i + 1) * sz)?;
        Some(match sz {
            1 => b[0] as u64,
            2 => match self.order {
                ByteOrder::Little => u16::from_le_bytes([b[0], b[1]]) as u64,
                ByteOrder::Big => u16::from_be_bytes([b[0], b[1]]) as u64,
            },
            4 => match self.order {
                ByteOrder::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64,
                ByteOrder::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64,
            },
            _ => return None,
        })
    }

    /// Every integer of an entry. The list is built from the bytes actually
    /// in the file, so a damaged count (up to ~4 billion) can't loop.
    pub fn uints(&self, e: &Entry) -> Vec<u64> {
        // The declared count comes straight from the file and can claim
        // billions of values; iterating to it would spin for minutes on a
        // corrupt entry. Derive the count from the bytes actually present
        // instead: when `value_bytes` has the value, it holds exactly
        // `count` values, and when it does not, every `uint` read fails
        // anyway — so the result is the same, capped by the data itself.
        let Some(bytes) = self.value_bytes(e) else {
            return Vec::new();
        };
        let n = bytes.len() / Self::type_size(e.typ);
        (0..n).filter_map(|i| self.uint(e, i)).collect()
    }

    /// Read a RATIONAL/SRATIONAL as (numerator, denominator).
    pub fn rational(&self, e: &Entry, i: usize) -> Option<(u32, u32)> {
        if e.typ != 5 && e.typ != 10 {
            return None;
        }
        let bytes = self.value_bytes(e)?;
        let b = bytes.get(i * 8..(i + 1) * 8)?;
        let rd = |x: &[u8]| match self.order {
            ByteOrder::Little => u32::from_le_bytes([x[0], x[1], x[2], x[3]]),
            ByteOrder::Big => u32::from_be_bytes([x[0], x[1], x[2], x[3]]),
        };
        Some((rd(&b[0..4]), rd(&b[4..8])))
    }

    /// Read an ASCII entry, trimming trailing NULs and whitespace.
    pub fn ascii(&self, e: &Entry) -> Option<String> {
        let bytes = self.value_bytes(e)?;
        let s = String::from_utf8_lossy(bytes);
        Some(s.trim_end_matches('\0').trim().to_string())
    }
}

/// Build a tiny little-endian TIFF in memory; used by tests across crates.
#[doc(hidden)]
pub mod testutil {
    /// `entries`: (tag, type, count, inline-4-bytes-or-offset).
    /// Writes IFD0 at offset 8 followed by `payload` at `payload_offset`.
    pub fn build_le(
        entries: &[(u16, u16, u32, [u8; 4])],
        payload: &[u8],
        payload_offset: usize,
    ) -> Vec<u8> {
        let mut v = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        v.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, typ, count, val) in entries {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&typ.to_le_bytes());
            v.extend_from_slice(&count.to_le_bytes());
            v.extend_from_slice(val);
        }
        v.extend_from_slice(&0u32.to_le_bytes());
        if v.len() < payload_offset {
            v.resize(payload_offset, 0);
        }
        v.extend_from_slice(payload);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_count_does_not_loop() {
        // StripOffsets claiming u32::MAX LONGs in a file of a few bytes.
        let data = testutil::build_le(
            &[(273, 4, u32::MAX, 64u32.to_le_bytes())],
            &[1, 0, 0, 0],
            64,
        );
        let (t, first) = Tiff::parse(&data).unwrap();
        let ifd = t.read_ifd(first).unwrap();
        let t0 = std::time::Instant::now();
        assert!(t.uints(ifd.get(273).unwrap()).is_empty());
        assert!(t0.elapsed() < std::time::Duration::from_millis(100));
        // An honest count still reads every value.
        let data = testutil::build_le(&[(273, 3, 3, 64u32.to_le_bytes())], &[1, 0, 2, 0, 3, 0], 64);
        let (t, first) = Tiff::parse(&data).unwrap();
        let ifd = t.read_ifd(first).unwrap();
        assert_eq!(t.uints(ifd.get(273).unwrap()), [1, 2, 3]);
    }

    #[test]
    fn parses_header_and_ifd() {
        let data = testutil::build_le(
            &[
                (tag::IMAGE_WIDTH, 3, 1, 640u32.to_le_bytes()),
                (tag::ORIENTATION, 3, 1, [8, 0, 0, 0]),
                (tag::MODEL, 2, 6, 64u32.to_le_bytes()),
            ],
            b"Z 6_2\0",
            64,
        );
        let (t, first) = Tiff::parse(&data).unwrap();
        assert_eq!(first, 8);
        let ifd = t.read_ifd(first).unwrap();
        assert_eq!(ifd.entries.len(), 3);
        assert_eq!(t.uint(ifd.get(tag::IMAGE_WIDTH).unwrap(), 0), Some(640));
        assert_eq!(t.uint(ifd.get(tag::ORIENTATION).unwrap(), 0), Some(8));
        assert_eq!(
            t.ascii(ifd.get(tag::MODEL).unwrap()).as_deref(),
            Some("Z 6_2")
        );
        assert_eq!(ifd.next, 0);
    }

    #[test]
    fn uints_caps_work_at_the_bytes_present() {
        // A corrupt entry declaring u32::MAX LONGs: the old loop iterated
        // the declared count and stalled for minutes even though no value
        // can be read. Now it returns at once with what the bytes hold.
        let data = testutil::build_le(
            &[
                (tag::SUB_IFDS, 4, u32::MAX, 9999u32.to_le_bytes()),
                (tag::STRIP_OFFSETS, 4, 2, 64u32.to_le_bytes()),
            ],
            &[100u8, 0, 0, 0, 200, 0, 0, 0],
            64,
        );
        let (t, first) = Tiff::parse(&data).unwrap();
        let ifd = t.read_ifd(first).unwrap();
        assert_eq!(t.uints(ifd.get(tag::SUB_IFDS).unwrap()), Vec::<u64>::new());
        // A sane entry still yields all of its values.
        assert_eq!(
            t.uints(ifd.get(tag::STRIP_OFFSETS).unwrap()),
            vec![100, 200]
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Tiff::parse(b"abc").unwrap_err(), TiffError::TooShort);
        assert_eq!(Tiff::parse(b"XXXXXXXXXX").unwrap_err(), TiffError::BadMagic);
        let data = testutil::build_le(&[(tag::MODEL, 2, 100, 9999u32.to_le_bytes())], b"", 0);
        let (t, first) = Tiff::parse(&data).unwrap();
        let ifd = t.read_ifd(first).unwrap();
        assert!(t.ascii(ifd.get(tag::MODEL).unwrap()).is_none());
        assert!(t.read_ifd(100_000).is_err());
    }
}
