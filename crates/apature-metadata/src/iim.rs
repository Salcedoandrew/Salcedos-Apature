//! IPTC-IIM ("legacy IPTC", records 1 and 2) inside a Photoshop APP13
//! segment. Newspaper systems and wire desks still read this block first,
//! so every delivery JPEG carries it next to the XMP packet with the same
//! values. Field lengths follow IIM 4.1; longer values are kept (readers
//! tolerate them, as Photo Mechanic's output shows) rather than truncated.

use crate::Iptc;

/// One dataset: 0x1C, record, dataset, big-endian length, bytes.
fn ds(out: &mut Vec<u8>, rec: u8, set: u8, data: &[u8]) {
    let n = data.len().min(32767);
    out.push(0x1C);
    out.push(rec);
    out.push(set);
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&data[..n]);
}

fn text(out: &mut Vec<u8>, set: u8, v: &Option<String>) {
    if let Some(v) = v {
        if !v.trim().is_empty() {
            ds(out, 2, set, v.as_bytes());
        }
    }
}

/// "2026-10-06T19:30:00-04:00" → ("20261006", Some("193000-0400")).
/// IIM TimeCreated must carry a zone, so a time without one is dropped
/// rather than invented.
fn date_parts(s: &str) -> Option<(String, Option<String>)> {
    let digits: Vec<char> = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return None;
    }
    let d: String = digits[..8].iter().collect();
    let t = if digits.len() >= 14 {
        let hms: String = digits[8..14].iter().collect();
        // Zone: the sign after the time, then four digits; "Z" means +0000.
        let after_time = s
            .find(|c: char| c.is_ascii_digit())
            .map(|_| s)
            .unwrap_or("");
        let time_end = after_time
            .char_indices()
            .filter(|(_, c)| c.is_ascii_digit())
            .nth(13)
            .map(|(i, _)| i + 1)
            .unwrap_or(after_time.len());
        let rest = after_time[time_end..].trim();
        if rest.starts_with('Z') {
            Some(format!("{hms}+0000"))
        } else if (rest.starts_with('+') || rest.starts_with('-')) && rest.len() >= 5 {
            let zd: String = rest[1..]
                .chars()
                .filter(|c| c.is_ascii_digit())
                .take(4)
                .collect();
            if zd.len() == 4 {
                Some(format!("{hms}{}{zd}", &rest[..1]))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    Some((d, t))
}

/// The IIM datasets for `iptc`, UTF-8 declared with 1:90.
pub fn datasets(iptc: &Iptc) -> Vec<u8> {
    let mut o = Vec::new();
    ds(&mut o, 1, 90, b"\x1b%G"); // coded character set: UTF-8
    ds(&mut o, 2, 0, &4u16.to_be_bytes()); // record version
    text(&mut o, 5, &iptc.title); // ObjectName
    if let Some(ks) = &iptc.keywords {
        for k in ks.iter().filter(|k| !k.trim().is_empty()) {
            ds(&mut o, 2, 25, k.as_bytes());
        }
    }
    text(&mut o, 40, &iptc.instructions);
    if let Some((d, t)) = iptc.date_created.as_deref().and_then(date_parts) {
        ds(&mut o, 2, 55, d.as_bytes());
        if let Some(t) = t {
            ds(&mut o, 2, 60, t.as_bytes());
        }
    }
    if let Some(cs) = &iptc.creator {
        for c in cs.iter().filter(|c| !c.trim().is_empty()) {
            ds(&mut o, 2, 80, c.as_bytes());
        }
    }
    text(&mut o, 85, &iptc.creator_title);
    text(&mut o, 90, &iptc.city);
    text(&mut o, 92, &iptc.sublocation);
    text(&mut o, 95, &iptc.state);
    text(&mut o, 100, &iptc.country_code);
    text(&mut o, 101, &iptc.country);
    text(&mut o, 103, &iptc.transmission_reference);
    text(&mut o, 105, &iptc.headline);
    text(&mut o, 110, &iptc.credit);
    text(&mut o, 115, &iptc.source);
    text(&mut o, 116, &iptc.copyright);
    text(&mut o, 120, &iptc.description);
    text(&mut o, 122, &iptc.caption_writer);
    o
}

/// A complete APP13 segment (marker included) holding the IIM block as
/// Photoshop image resource 0x0404.
/// Errors when the fields are too long for one segment.
pub fn app13_segment(iptc: &Iptc) -> Result<Vec<u8>, crate::jpeg_xmp::JpegXmpError> {
    let payload = datasets(iptc);
    let mut irb = Vec::with_capacity(payload.len() + 16);
    irb.extend_from_slice(b"8BIM");
    irb.extend_from_slice(&0x0404u16.to_be_bytes());
    irb.extend_from_slice(&[0, 0]); // empty Pascal name, padded to even
    irb.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    irb.extend_from_slice(&payload);
    if payload.len() % 2 == 1 {
        irb.push(0);
    }
    crate::jpeg_xmp::segment(0xED, "IPTC-IIM", b"Photoshop 3.0\0", &irb)
}

/// Parse datasets back into (record, dataset, bytes); for tests and readers.
pub fn parse(mut d: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
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

/// The IIM bytes inside an APP13 payload (after "Photoshop 3.0\0").
pub fn from_app13(payload: &[u8]) -> Option<&[u8]> {
    let mut p = payload.strip_prefix(b"Photoshop 3.0\0")?;
    while p.len() >= 12 && &p[..4] == b"8BIM" {
        let id = u16::from_be_bytes([p[4], p[5]]);
        let name_len = p[6] as usize;
        let name_total = (1 + name_len + 1) & !1; // Pascal string padded to even
        let len_at = 6 + name_total;
        let len = u32::from_be_bytes(p.get(len_at..len_at + 4)?.try_into().ok()?) as usize;
        let data_at = len_at + 4;
        let data = p.get(data_at..data_at + len)?;
        if id == 0x0404 {
            return Some(data);
        }
        p = &p[data_at + ((len + 1) & !1)..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Iptc {
        Iptc {
            title: Some("100726-SPO-BGLF-001".into()),
            headline: Some("100726 SPO BGLF WPIAL Championships".into()),
            description: Some("Caption with ü. Photog/Example Press".into()),
            keywords: Some(vec!["golf".into(), "Oakmont".into()]),
            creator: Some(vec!["Caleb McCartney".into()]),
            creator_title: Some("Photographer".into()),
            credit: Some("Example Press".into()),
            source: Some("Example Press".into()),
            copyright: Some("© 2026 Example Press".into()),
            city: Some("Oakmont".into()),
            state: Some("Pa.".into()),
            country: Some("USA".into()),
            date_created: Some("2026-10-06T00:00:00".into()),
            ..Default::default()
        }
    }

    #[test]
    fn fields_too_long_for_one_segment_are_an_error() {
        let long = "x".repeat(30_000);
        let iptc = Iptc {
            description: Some(long.clone()),
            headline: Some(long.clone()),
            title: Some(long),
            ..Default::default()
        };
        assert!(app13_segment(&iptc).is_err());
    }

    #[test]
    fn round_trips_through_the_segment() {
        let seg = app13_segment(&sample()).unwrap();
        assert_eq!(&seg[..2], &[0xFF, 0xED]);
        let len = u16::from_be_bytes([seg[2], seg[3]]) as usize;
        assert_eq!(
            len + 2,
            seg.len(),
            "segment length counts everything after the marker"
        );
        let payload = &seg[4..];
        let iim = from_app13(payload).expect("8BIM 0x0404 found");
        let sets = parse(iim);
        let get = |r: u8, s: u8| -> Vec<String> {
            sets.iter()
                .filter(|(a, b, _)| *a == r && *b == s)
                .map(|(_, _, v)| String::from_utf8(v.clone()).unwrap())
                .collect()
        };
        assert_eq!(get(1, 90), vec!["\x1b%G"]);
        assert_eq!(get(2, 120), vec!["Caption with ü. Photog/Example Press"]);
        assert_eq!(get(2, 25), vec!["golf", "Oakmont"]);
        assert_eq!(get(2, 55), vec!["20261006"]);
        assert!(get(2, 60).is_empty(), "no zone → no TimeCreated");
        assert_eq!(
            date_parts("2026-10-06T19:30:00-04:00"),
            Some(("20261006".into(), Some("193000-0400".into())))
        );
        assert_eq!(
            date_parts("2026-10-06T19:30:00Z"),
            Some(("20261006".into(), Some("193000+0000".into())))
        );
        assert_eq!(
            date_parts("2026:10:06 19:30:00"),
            Some(("20261006".into(), None))
        );
        assert_eq!(get(2, 116), vec!["© 2026 Example Press"]);
        assert_eq!(get(2, 80), vec!["Caleb McCartney"]);
        assert_eq!(get(2, 105), vec!["100726 SPO BGLF WPIAL Championships"]);
        assert!(get(2, 92).is_empty(), "no sublocation → no dataset");
    }

    #[test]
    fn empty_fields_are_skipped_and_order_is_record_sorted() {
        let sets = parse(&datasets(&Iptc::default()));
        assert_eq!(sets.len(), 2, "charset and version only");
        let d = datasets(&sample());
        let sets = parse(&d);
        let keys: Vec<(u8, u8)> = sets.iter().map(|(r, s, _)| (*r, *s)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}
