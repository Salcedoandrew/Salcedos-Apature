//! IPTC Core fields in XMP: the editorial set a newsroom needs.
//!
//! Same discipline as marks: each property is located in the existing
//! document and replaced in place; missing ones are inserted; nothing else
//! is touched. Language alternatives (`rdf:Alt`), ordered lists (`rdf:Seq`)
//! and bags (`rdf:Bag`) are handled as whole elements.

use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::xmp::{self, XmpError};

pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub const NS_PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
pub const NS_IPTC_CORE: &str = "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/";
pub const NS_XMP_RIGHTS: &str = "http://ns.adobe.com/xap/1.0/rights/";
pub const NS_EXIF: &str = "http://ns.adobe.com/exif/1.0/";

/// Editorial metadata. `None` = leave untouched when writing / absent when
/// reading; `Some("")` or `Some(vec![])` = remove the property.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Iptc {
    pub title: Option<String>,
    pub headline: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub creator: Option<Vec<String>>,
    pub creator_title: Option<String>,
    pub credit: Option<String>,
    pub source: Option<String>,
    pub copyright: Option<String>,
    pub caption_writer: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub sublocation: Option<String>,
    pub date_created: Option<String>,
    pub instructions: Option<String>,
    pub transmission_reference: Option<String>,
    pub usage_terms: Option<String>,
    /// XMP exif:GPSLatitude / GPSLongitude in their own "DD,MM.mmmmN" form;
    /// use `set_gps` and `gps_decimal` for decimal degrees.
    pub gps_latitude: Option<String>,
    pub gps_longitude: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Simple,
    Alt,
    Seq,
    Bag,
}

#[derive(Clone, Copy, Debug)]
pub enum Field {
    Title,
    Headline,
    Description,
    Keywords,
    Creator,
    CreatorTitle,
    Credit,
    Source,
    Copyright,
    CaptionWriter,
    City,
    State,
    Country,
    CountryCode,
    Sublocation,
    DateCreated,
    Instructions,
    TransmissionReference,
    UsageTerms,
    GpsLatitude,
    GpsLongitude,
}

/// (field, prefix, namespace, local name, kind). Prefix is what we write and
/// the conventional one every reader expects; `Iptc4xmpCore` keeps its
/// capital I (Photo Mechanic, Lightroom and exiftool all do).
const TABLE: &[(Field, &str, &str, &str, Kind)] = &[
    (Field::Title, "dc", NS_DC, "title", Kind::Alt),
    (
        Field::Headline,
        "photoshop",
        NS_PHOTOSHOP,
        "Headline",
        Kind::Simple,
    ),
    (Field::Description, "dc", NS_DC, "description", Kind::Alt),
    (Field::Keywords, "dc", NS_DC, "subject", Kind::Bag),
    (Field::Creator, "dc", NS_DC, "creator", Kind::Seq),
    (
        Field::CreatorTitle,
        "photoshop",
        NS_PHOTOSHOP,
        "AuthorsPosition",
        Kind::Simple,
    ),
    (
        Field::Credit,
        "photoshop",
        NS_PHOTOSHOP,
        "Credit",
        Kind::Simple,
    ),
    (
        Field::Source,
        "photoshop",
        NS_PHOTOSHOP,
        "Source",
        Kind::Simple,
    ),
    (Field::Copyright, "dc", NS_DC, "rights", Kind::Alt),
    (
        Field::CaptionWriter,
        "photoshop",
        NS_PHOTOSHOP,
        "CaptionWriter",
        Kind::Simple,
    ),
    (Field::City, "photoshop", NS_PHOTOSHOP, "City", Kind::Simple),
    (
        Field::State,
        "photoshop",
        NS_PHOTOSHOP,
        "State",
        Kind::Simple,
    ),
    (
        Field::Country,
        "photoshop",
        NS_PHOTOSHOP,
        "Country",
        Kind::Simple,
    ),
    (
        Field::GpsLatitude,
        "exif",
        NS_EXIF,
        "GPSLatitude",
        Kind::Simple,
    ),
    (
        Field::GpsLongitude,
        "exif",
        NS_EXIF,
        "GPSLongitude",
        Kind::Simple,
    ),
    (
        Field::CountryCode,
        "Iptc4xmpCore",
        NS_IPTC_CORE,
        "CountryCode",
        Kind::Simple,
    ),
    (
        Field::Sublocation,
        "Iptc4xmpCore",
        NS_IPTC_CORE,
        "Location",
        Kind::Simple,
    ),
    (
        Field::DateCreated,
        "photoshop",
        NS_PHOTOSHOP,
        "DateCreated",
        Kind::Simple,
    ),
    (
        Field::Instructions,
        "photoshop",
        NS_PHOTOSHOP,
        "Instructions",
        Kind::Simple,
    ),
    (
        Field::TransmissionReference,
        "photoshop",
        NS_PHOTOSHOP,
        "TransmissionReference",
        Kind::Simple,
    ),
    (
        Field::UsageTerms,
        "xmpRights",
        NS_XMP_RIGHTS,
        "UsageTerms",
        Kind::Alt,
    ),
];

impl Iptc {
    fn get(&self, f: Field) -> Option<Vec<String>> {
        let one = |o: &Option<String>| o.as_ref().map(|s| vec![s.clone()]);
        match f {
            Field::Title => one(&self.title),
            Field::Headline => one(&self.headline),
            Field::Description => one(&self.description),
            Field::Keywords => self.keywords.clone(),
            Field::Creator => self.creator.clone(),
            Field::CreatorTitle => one(&self.creator_title),
            Field::Credit => one(&self.credit),
            Field::Source => one(&self.source),
            Field::Copyright => one(&self.copyright),
            Field::CaptionWriter => one(&self.caption_writer),
            Field::City => one(&self.city),
            Field::State => one(&self.state),
            Field::Country => one(&self.country),
            Field::CountryCode => one(&self.country_code),
            Field::Sublocation => one(&self.sublocation),
            Field::DateCreated => one(&self.date_created),
            Field::Instructions => one(&self.instructions),
            Field::TransmissionReference => one(&self.transmission_reference),
            Field::UsageTerms => one(&self.usage_terms),
            Field::GpsLatitude => one(&self.gps_latitude),
            Field::GpsLongitude => one(&self.gps_longitude),
        }
    }

    fn set(&mut self, f: Field, v: Vec<String>) {
        let one = |v: Vec<String>| v.into_iter().next().filter(|s| !s.is_empty());
        match f {
            Field::Title => self.title = one(v),
            Field::Headline => self.headline = one(v),
            Field::Description => self.description = one(v),
            Field::Keywords => self.keywords = Some(v),
            Field::Creator => self.creator = Some(v),
            Field::CreatorTitle => self.creator_title = one(v),
            Field::Credit => self.credit = one(v),
            Field::Source => self.source = one(v),
            Field::Copyright => self.copyright = one(v),
            Field::CaptionWriter => self.caption_writer = one(v),
            Field::City => self.city = one(v),
            Field::State => self.state = one(v),
            Field::Country => self.country = one(v),
            Field::CountryCode => self.country_code = one(v),
            Field::Sublocation => self.sublocation = one(v),
            Field::DateCreated => self.date_created = one(v),
            Field::Instructions => self.instructions = one(v),
            Field::TransmissionReference => self.transmission_reference = one(v),
            Field::UsageTerms => self.usage_terms = one(v),
            Field::GpsLatitude => self.gps_latitude = one(v),
            Field::GpsLongitude => self.gps_longitude = one(v),
        }
    }

    /// Fields that are `Some` in `other` override this one (a stationery
    /// pad applied over existing metadata).
    pub fn overlay(&self, other: &Iptc) -> Iptc {
        let mut out = self.clone();
        for (f, ..) in TABLE {
            if let Some(v) = other.get(*f) {
                out.set(*f, v);
            }
        }
        out
    }

    /// Empty `field` (a cleared value: `None`, or no keywords/creators).
    pub fn clear(&mut self, field: Field) {
        self.set(field, Vec::new());
    }

    pub fn is_empty(&self) -> bool {
        TABLE.iter().all(|(f, ..)| self.get(*f).is_none())
    }

    /// Store a location in XMP's own GPS notation ("40,26.7676N" "79,53.7512W").
    pub fn set_gps(&mut self, lat: f64, lon: f64) {
        self.gps_latitude = Some(gps_to_xmp(lat, if lat < 0.0 { 'S' } else { 'N' }));
        self.gps_longitude = Some(gps_to_xmp(lon, if lon < 0.0 { 'W' } else { 'E' }));
    }

    pub fn gps_decimal(&self) -> Option<(f64, f64)> {
        Some((
            gps_from_xmp(self.gps_latitude.as_deref()?)?,
            gps_from_xmp(self.gps_longitude.as_deref()?)?,
        ))
    }
}

fn gps_to_xmp(v: f64, hemi: char) -> String {
    let a = v.abs();
    let deg = a.floor();
    let min = (a - deg) * 60.0;
    format!("{},{:.4}{}", deg as u32, min, hemi)
}

/// Accepts "40,26.7676N", "40,26,45.6N" and plain decimal "40.4461".
pub fn gps_from_xmp(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (body, hemi) = match s.chars().last() {
        Some(c) if "NSEW".contains(c) => (&s[..s.len() - 1], Some(c)),
        _ => (s, None),
    };
    let parts: Vec<&str> = body.split(',').collect();
    let mut v = match parts.as_slice() {
        [d] => d.trim().parse::<f64>().ok()?,
        [d, m] => d.trim().parse::<f64>().ok()? + m.trim().parse::<f64>().ok()? / 60.0,
        [d, m, sec] => {
            d.trim().parse::<f64>().ok()?
                + m.trim().parse::<f64>().ok()? / 60.0
                + sec.trim().parse::<f64>().ok()? / 3600.0
        }
        _ => return None,
    };
    if matches!(hemi, Some('S') | Some('W')) {
        v = -v;
    }
    Some(v)
}

static RE_LI: OnceLock<Regex> = OnceLock::new();
static RE_XDEFAULT: OnceLock<Regex> = OnceLock::new();

fn li_re() -> &'static Regex {
    RE_LI.get_or_init(|| Regex::new(r"(?s)<rdf:li(?:\s[^>]*)?>(.*?)</rdf:li>").unwrap())
}

fn xdefault_re() -> &'static Regex {
    RE_XDEFAULT.get_or_init(|| {
        Regex::new(r#"(?s)<rdf:li[^>]*xml:lang="x-default"[^>]*>(.*?)</rdf:li>"#).unwrap()
    })
}

fn element_re(prefix: &str, name: &str) -> Regex {
    Regex::new(&format!(
        r"(?s)<{p}:{n}(?:\s[^>]*)?>.*?</{p}:{n}\s*>|<{p}:{n}\s*/>",
        p = regex::escape(prefix),
        n = regex::escape(name)
    ))
    .unwrap()
}

/// The compiled pattern for one TABLE row: the property pattern for
/// `Kind::Simple`, the element pattern for the structured kinds. A row's
/// pattern depends only on the row, so all of them are built once.
struct FieldRes {
    re: Regex,
}

static FIELD_RES: LazyLock<Vec<FieldRes>> = LazyLock::new(|| {
    TABLE
        .iter()
        .map(|(_, prefix, _, name, kind)| FieldRes {
            re: match kind {
                Kind::Simple => Regex::new(&xmp::prop_pattern(prefix, name)).unwrap(),
                _ => element_re(prefix, name),
            },
        })
        .collect()
});

/// Read IPTC fields from sidecar or packet text.
/// Fields present in `text` but empty: cleared on purpose (see [`apply`]).
pub fn cleared(text: &str) -> Vec<Field> {
    cleared_with(text, &parse(text))
}

/// [`cleared`] with the parse of `text` already in hand, so a caller that
/// parsed the text itself does not pay for a second parse.
pub fn cleared_with(text: &str, parsed: &Iptc) -> Vec<Field> {
    TABLE
        .iter()
        .zip(FIELD_RES.iter())
        .filter(|((f, ..), res)| {
            res.re.is_match(text)
                && parsed
                    .get(*f)
                    .is_none_or(|v| v.iter().all(|s| s.is_empty()))
        })
        .map(|((f, ..), _)| *f)
        .collect()
}

pub fn parse(text: &str) -> Iptc {
    let mut out = Iptc::default();
    for ((f, _, _, _, kind), res) in TABLE.iter().zip(FIELD_RES.iter()) {
        match kind {
            Kind::Simple => {
                if let Some(v) = xmp::value_of(&res.re, text) {
                    out.set(*f, vec![v]);
                }
            }
            Kind::Alt | Kind::Seq | Kind::Bag => {
                if let Some(m) = res.re.find(text) {
                    let inner = m.as_str();
                    let items: Vec<String> = li_re()
                        .captures_iter(inner)
                        .map(|c| xmp::unescape(c[1].trim()))
                        .collect();
                    if *kind == Kind::Alt {
                        // Prefer the x-default alternative, else the first.
                        let xdef = xdefault_re();
                        let v = xdef
                            .captures(inner)
                            .map(|c| xmp::unescape(c[1].trim()))
                            .or_else(|| items.first().cloned());
                        if let Some(v) = v {
                            out.set(*f, vec![v]);
                        }
                    } else if !items.is_empty() {
                        out.set(*f, items);
                    }
                }
            }
        }
    }
    out
}

fn structured(prefix: &str, name: &str, kind: Kind, items: &[String]) -> String {
    let (open, close) = match kind {
        Kind::Alt => ("rdf:Alt", "rdf:Alt"),
        Kind::Seq => ("rdf:Seq", "rdf:Seq"),
        Kind::Bag => ("rdf:Bag", "rdf:Bag"),
        Kind::Simple => unreachable!(),
    };
    let mut s = format!("<{prefix}:{name}>\n    <{open}>\n");
    for it in items {
        if kind == Kind::Alt {
            s.push_str(&format!(
                "     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n",
                xmp::escape(it)
            ));
        } else {
            s.push_str(&format!("     <rdf:li>{}</rdf:li>\n", xmp::escape(it)));
        }
    }
    s.push_str(&format!("    </{close}>\n   </{prefix}:{name}>"));
    s
}

/// Find the first `<rdf:Description ...>` start tag: (start, end_of_tag, self_closing).
fn description_tag(text: &str) -> Option<(usize, usize, bool)> {
    let start = text.find("<rdf:Description")?;
    let bytes = text.as_bytes();
    let mut i = start;
    let mut in_quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match in_quote {
            Some(q) => {
                if b == q {
                    in_quote = None;
                }
            }
            None => {
                if b == b'"' || b == b'\'' {
                    in_quote = Some(b);
                } else if b == b'>' {
                    let self_closing = i > 0 && bytes[i - 1] == b'/';
                    return Some((start, i + 1, self_closing));
                }
            }
        }
        i += 1;
    }
    None
}

fn ensure_ns(text: &mut String, prefix: &str, ns: &str) -> Result<(), XmpError> {
    if xmp::declared_prefix(text, ns).is_some() || text.contains(&format!("xmlns:{prefix}=")) {
        return Ok(());
    }
    let (start, _, _) = description_tag(text).ok_or(XmpError::NoDescription)?;
    let insert_at = start + "<rdf:Description".len();
    text.insert_str(insert_at, &format!("\n    xmlns:{prefix}=\"{ns}\""));
    Ok(())
}

/// Replace or insert a whole structured element inside the first Description.
fn set_structured(
    text: &mut String,
    prefix: &str,
    ns: &str,
    name: &str,
    kind: Kind,
    items: Option<&[String]>,
) -> Result<(), XmpError> {
    let prefix_used = xmp::declared_prefix(text, ns).unwrap_or_else(|| prefix.to_string());
    let re = element_re(&prefix_used, name);
    if let Some(m) = re.find(text) {
        let range = m.range();
        // Also eat the whitespace before the element so removal leaves no blank line.
        let ws_start = text[..range.start]
            .rfind(|c: char| !c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(range.start);
        let replacement = match items {
            Some(it) => format!("\n   {}", structured(&prefix_used, name, kind, it)),
            None => String::new(),
        };
        text.replace_range(ws_start..range.end, &replacement);
        return Ok(());
    }
    let Some(it) = items else {
        return Ok(());
    };
    ensure_ns(text, prefix, ns)?;
    let (_, tag_end, self_closing) = description_tag(text).ok_or(XmpError::NoDescription)?;
    let element = format!("\n   {}", structured(&prefix_used, name, kind, it));
    if self_closing {
        // `<rdf:Description .../>` → `<rdf:Description ...>` + children + `</rdf:Description>`
        let slash = text[..tag_end].rfind('/').unwrap();
        text.replace_range(slash..tag_end, ">");
        let open_end = slash + 1;
        text.insert_str(open_end, &format!("{element}\n  </rdf:Description>"));
    } else {
        let close = text[tag_end..]
            .find("</rdf:Description>")
            .map(|i| i + tag_end)
            .ok_or(XmpError::NoDescription)?;
        let ws_start = text[..close]
            .rfind(|c: char| !c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(close);
        text.replace_range(ws_start..close, &format!("{element}\n  "));
    }
    Ok(())
}

/// Apply `iptc` to sidecar/packet text; `Some` fields are written, `None`
/// left alone. An empty `Some` (a cleared field) is written as present but
/// empty (`name=""`, or an empty `rdf:Alt`/`Bag`/`Seq`), so a cleared
/// sidecar field still overrides text embedded in the JPEG; see [`cleared`].
pub fn apply(text: &str, iptc: &Iptc) -> Result<String, XmpError> {
    let mut out = if text.trim().is_empty() {
        xmp::empty_sidecar()
    } else {
        text.to_string()
    };
    for (f, prefix, ns, name, kind) in TABLE {
        let Some(values) = iptc.get(*f) else { continue };
        match kind {
            Kind::Simple => {
                let v = values.first().map(String::as_str).unwrap_or("");
                xmp::set_prop(&mut out, &[prefix], ns, name, Some(v))?;
            }
            _ => set_structured(&mut out, prefix, ns, name, *kind, Some(&values))?,
        }
    }
    Ok(out)
}

/// True when every `Some` field of `want` reads back equal from `got`.
pub fn satisfies(want: &Iptc, got: &Iptc) -> bool {
    TABLE.iter().all(|(f, ..)| match want.get(*f) {
        None => true,
        Some(w) => {
            let w: Vec<String> = w.into_iter().filter(|s| !s.is_empty()).collect();
            let g = got.get(*f).unwrap_or_default();
            w == g
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gps_round_trips_through_xmp_notation() {
        let mut i = Iptc::default();
        i.set_gps(40.4461, -79.8955);
        assert_eq!(i.gps_latitude.as_deref(), Some("40,26.7660N"));
        assert_eq!(i.gps_longitude.as_deref(), Some("79,53.7300W"));
        let (la, lo) = i.gps_decimal().unwrap();
        assert!((la - 40.4461).abs() < 1e-4 && (lo + 79.8955).abs() < 1e-4);
        assert_eq!(
            gps_from_xmp("40,26,45.6N"),
            Some(40.0 + 26.0 / 60.0 + 45.6 / 3600.0)
        );
        assert_eq!(gps_from_xmp("-79.8955"), Some(-79.8955));
        assert_eq!(gps_from_xmp("nope"), None);
    }

    const BUTLER: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:Iptc4xmpCore="http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:photomechanic="http://ns.camerabits.com/photomechanic/1.0/"
    photoshop:Credit="Example Press"
    photoshop:Headline="100726 SPO BGLF WPIAL Championships"
    photoshop:City="Oakmont"
    photoshop:State="Pa."
    Iptc4xmpCore:Location="Oakmont Country Club"
    xmp:Rating="4"
    photomechanic:ColorClass="1">
   <dc:creator><rdf:Seq><rdf:li>Caleb McCartney</rdf:li></rdf:Seq></dc:creator>
   <dc:rights><rdf:Alt><rdf:li xml:lang="x-default">© 2026 Example Press</rdf:li></rdf:Alt></dc:rights>
   <dc:description><rdf:Alt><rdf:li xml:lang="x-default">Old caption &amp; text</rdf:li></rdf:Alt></dc:description>
   <dc:subject><rdf:Bag><rdf:li>WPIAL</rdf:li><rdf:li>golf</rdf:li></rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn parses_simple_and_structured() {
        let i = parse(BUTLER);
        assert_eq!(
            i.headline.as_deref(),
            Some("100726 SPO BGLF WPIAL Championships")
        );
        assert_eq!(i.credit.as_deref(), Some("Example Press"));
        assert_eq!(i.sublocation.as_deref(), Some("Oakmont Country Club"));
        assert_eq!(i.description.as_deref(), Some("Old caption & text"));
        assert_eq!(i.copyright.as_deref(), Some("© 2026 Example Press"));
        assert_eq!(i.creator, Some(vec!["Caleb McCartney".into()]));
        assert_eq!(i.keywords, Some(vec!["WPIAL".into(), "golf".into()]));
        assert_eq!(i.country, None);
    }

    #[test]
    fn edits_in_place_and_leaves_marks_alone() {
        let edit = Iptc {
            description: Some("North Catholic's Christopher Faber tees off on No. 1. Caleb McCartney/Special to the Eagle".into()),
            keywords: Some(vec!["WPIAL".into(), "golf".into(), "North Catholic".into()]),
            city: Some("Oakmont".into()),
            country: Some("USA".into()),
            credit: Some(String::new()),
            ..Default::default()
        };
        let out = apply(BUTLER, &edit).unwrap();
        let back = parse(&out);
        assert!(satisfies(&edit, &back), "{out}");
        assert_eq!(back.keywords.unwrap().len(), 3);
        assert_eq!(back.credit, None, "empty string removes");
        assert!(out.contains(r#"photoshop:Country="USA""#));
        assert_eq!(out.matches("<dc:description>").count(), 1);
        assert_eq!(out.matches("<dc:subject>").count(), 1);
        // Untouched properties survive verbatim.
        assert!(out.contains(r#"xmp:Rating="4""#));
        assert!(out.contains(r#"photomechanic:ColorClass="1""#));
        assert!(out.contains("<dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">© 2026 Example Press</rdf:li></rdf:Alt></dc:rights>"));
        assert_eq!(crate::xmp::parse(&out).rating, Some(4));
        // Idempotent.
        assert_eq!(apply(&out, &edit).unwrap(), out);
    }

    #[test]
    fn inserts_into_self_closing_description_and_declares_namespaces() {
        let pm = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
   xmp:Rating="3"/>
 </rdf:RDF>
</x:xmpmeta>
"#;
        let edit = Iptc {
            description: Some("A caption".into()),
            keywords: Some(vec!["a".into(), "b".into()]),
            headline: Some("H".into()),
            ..Default::default()
        };
        let out = apply(pm, &edit).unwrap();
        assert!(
            out.contains("xmlns:dc=\"http://purl.org/dc/elements/1.1/\""),
            "{out}"
        );
        assert!(out.contains("xmlns:photoshop="), "{out}");
        assert!(out.contains("</rdf:Description>"), "{out}");
        assert!(
            !out.contains("/>\n </rdf:RDF>"),
            "self-closing tag must be opened: {out}"
        );
        let back = parse(&out);
        assert!(satisfies(&edit, &back), "{out}");
        assert_eq!(crate::xmp::parse(&out).rating, Some(3));
    }

    #[test]
    fn fresh_sidecar_and_overlay() {
        let base = Iptc {
            credit: Some("Example Press".into()),
            city: Some("Oakmont".into()),
            ..Default::default()
        };
        let pad = Iptc {
            city: Some("Springfield".into()),
            headline: Some("Slug".into()),
            ..Default::default()
        };
        let merged = base.overlay(&pad);
        assert_eq!(merged.city.as_deref(), Some("Springfield"));
        assert_eq!(merged.credit.as_deref(), Some("Example Press"));
        let out = apply("", &merged).unwrap();
        assert!(satisfies(&merged, &parse(&out)));
        assert!(Iptc::default().is_empty());
    }

    #[test]
    fn cleared_fields_are_written_present_and_empty() {
        let start = apply(
            "",
            &Iptc {
                headline: Some("H".into()),
                description: Some("D".into()),
                keywords: Some(vec!["k".into()]),
                city: Some("C".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let out = apply(
            &start,
            &Iptc {
                headline: Some(String::new()),
                description: Some(String::new()),
                keywords: Some(Vec::new()),
                ..Default::default()
            },
        )
        .unwrap();
        let back = parse(&out);
        assert_eq!((back.headline, back.description), (None, None));
        assert_eq!(back.city.as_deref(), Some("C"));
        let gone: Vec<String> = cleared(&out).iter().map(|f| format!("{f:?}")).collect();
        assert_eq!(gone, ["Headline", "Description", "Keywords"]);
        assert!(cleared(&start).is_empty());
        // Clearing a field that was never there still records the clear.
        let fresh = apply(
            "",
            &Iptc {
                description: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cleared(&fresh).len(), 1);
    }
}
