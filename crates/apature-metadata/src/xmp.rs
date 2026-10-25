//! XMP sidecar read/write for culling marks.
//!
//! Design: a sidecar may already contain IPTC captions, Lightroom develop
//! settings, Photo Mechanic preferences and more. We never re-serialise the
//! document. Instead each property is located as an attribute or element on
//! the first `rdf:Description` and edited in place, and missing ones are
//! inserted as attributes. The file is written atomically.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use regex::Regex;

use apature_core::{ColorClass, ColorClassScheme, Marks, Rating};

pub const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
pub const NS_PM: &str = "http://ns.camerabits.com/photomechanic/1.0/";

/// Marks as they appear in XMP. `None` means the property is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XmpMarks {
    pub rating: Option<i8>,
    pub label: Option<String>,
    pub color_class: Option<u8>,
    pub tagged: Option<bool>,
}

impl XmpMarks {
    /// Convert to in-app marks.
    ///
    /// Colour class: Photo Mechanic writes `photomechanic:ColorClass` and
    /// `xmp:Label` together and keeps them consistent. Capture One and
    /// Lightroom only know `xmp:Label`, so after one of them edits a sidecar
    /// the number is stale (verified with Capture One 16.8: Label "Yellow",
    /// ColorClass still 4). Therefore a label that names a known class wins
    /// over a disagreeing number; the number is used when there is no label
    /// or the label is not a class name we know.
    pub fn to_marks(&self, scheme: &ColorClassScheme) -> Marks {
        let from_label = self
            .label
            .as_deref()
            .and_then(|l| scheme.class_for_label(l));
        let color_class = match (self.color_class, from_label) {
            (Some(c), Some(l)) if c != l.0 => l,
            (Some(c), _) => ColorClass::clamp(c),
            (None, Some(l)) => l,
            (None, None) => ColorClass::NONE,
        };
        Marks {
            rating: Rating::clamp(self.rating.unwrap_or(0)),
            color_class,
            tagged: self.tagged.unwrap_or(false),
        }
    }

    pub fn from_marks(m: &Marks, scheme: &ColorClassScheme) -> Self {
        XmpMarks {
            rating: Some(m.rating.0),
            label: Some(scheme.name(m.color_class).unwrap_or("").to_string()),
            color_class: Some(m.color_class.0),
            tagged: Some(m.tagged),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum XmpError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("sidecar has no rdf:Description element to edit")]
    NoDescription,
    #[error(
        "edited sidecar {0} did not read back with the requested marks; original left untouched"
    )]
    Verify(std::path::PathBuf),
}

fn re(cache: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cache.get_or_init(|| Regex::new(pat).expect("valid regex"))
}

/// Patterns built from property names and document prefixes repeat across
/// parses and edits, so compiled regexes are kept by pattern text. The set
/// of prefixes a document can bind is small in practice; the cap keeps a
/// pathological run from growing the cache without bound.
static PATTERN_CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();

pub(crate) fn cached_re(pat: &str) -> Result<Regex, regex::Error> {
    let cache = PATTERN_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(re) = map.get(pat) {
        return Ok(re.clone());
    }
    let compiled = Regex::new(pat)?;
    if map.len() < 256 {
        map.insert(pat.to_string(), compiled.clone());
    }
    Ok(compiled)
}

static RE_RATING: OnceLock<Regex> = OnceLock::new();
static RE_LABEL: OnceLock<Regex> = OnceLock::new();
static RE_CLASS: OnceLock<Regex> = OnceLock::new();
static RE_TAGGED: OnceLock<Regex> = OnceLock::new();
static RE_DESC_OPEN: OnceLock<Regex> = OnceLock::new();

/// Matches `prefix:Name="v"` or `<prefix:Name>v</prefix:Name>`; group 2 or 4 is the value.
pub(crate) fn prop_pattern(prefixes: &str, name: &str) -> String {
    format!(
        r#"(?s)(?:\b(?:{p}):{n}\s*=\s*"([^"]*)")|(?:<(?:{p}):{n}(?:\s[^>]*)?>\s*(.*?)\s*</(?:{p}):{n}\s*>)"#,
        p = prefixes,
        n = name
    )
}

pub(crate) fn value_of(re: &Regex, text: &str) -> Option<String> {
    let c = re.captures(text)?;
    c.get(1).or_else(|| c.get(2)).map(|m| unescape(m.as_str()))
}

pub(crate) fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

pub(crate) fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Parse marks out of sidecar text.
pub fn parse(text: &str) -> XmpMarks {
    let rating = value_of(re(&RE_RATING, &prop_pattern("xmp|xap", "Rating")), text)
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|v| v.round() as i8);
    let label = value_of(re(&RE_LABEL, &prop_pattern("xmp|xap", "Label")), text)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let color_class = value_of(
        re(&RE_CLASS, &prop_pattern("photomechanic", "ColorClass")),
        text,
    )
    .and_then(|v| v.trim().parse::<u8>().ok());
    let tagged = value_of(
        re(&RE_TAGGED, &prop_pattern("photomechanic", "Tagged")),
        text,
    )
    .map(|v| matches!(v.trim().to_lowercase().as_str(), "true" | "1"));
    XmpMarks {
        rating,
        label,
        color_class,
        tagged,
    }
}

/// Read a sidecar. `Ok(None)` when the file does not exist.
pub fn read_sidecar(path: &Path) -> Result<Option<XmpMarks>, XmpError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(parse(&text))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn empty_sidecar() -> String {
    "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Apature 0.1\">\n\
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\">\n\
  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>\n"
        .to_string()
}

/// Find the prefix a document binds to a namespace URI, if any.
pub(crate) fn declared_prefix(text: &str, ns: &str) -> Option<String> {
    let pat = cached_re(&format!(
        r#"xmlns:([A-Za-z_][\w.-]*)\s*=\s*"{}""#,
        regex::escape(ns)
    ))
    .expect("valid regex");
    pat.captures(text).map(|c| c[1].to_string())
}

/// Set or replace one property.
///
/// The property is searched under every prefix in `prefixes` (so a legacy
/// `xap:Rating` is updated rather than joined by a second `xmp:Rating`).
/// Attribute form is replaced in place, element form is replaced in place,
/// otherwise an attribute is inserted on the first `rdf:Description`, using
/// whatever prefix the document already binds to `ns` (declaring
/// `prefixes[0]` if it binds none). `value = None` removes the property.
pub(crate) fn set_prop(
    text: &mut String,
    prefixes: &[&str],
    ns: &str,
    name: &str,
    value: Option<&str>,
) -> Result<(), XmpError> {
    let pat = cached_re(&prop_pattern(&prefixes.join("|"), name)).expect("valid regex");
    if let Some(c) = pat.captures(text) {
        let whole = c.get(0).expect("group 0");
        let replacement = match value {
            Some(v) => {
                // Keep the prefix the document already used for this property.
                let matched = whole.as_str();
                let used = prefixes
                    .iter()
                    .find(|p| {
                        matched
                            .trim_start_matches('<')
                            .starts_with(&format!("{p}:"))
                    })
                    .copied()
                    .unwrap_or(prefixes[0]);
                if c.get(1).is_some() {
                    format!("{used}:{name}=\"{}\"", escape(v))
                } else {
                    format!("<{used}:{name}>{}</{used}:{name}>", escape(v))
                }
            }
            None => String::new(),
        };
        text.replace_range(whole.range(), &replacement);
        return Ok(());
    }
    let Some(v) = value else { return Ok(()) };
    let open = re(&RE_DESC_OPEN, r"<rdf:Description\b");
    let m = open.find(text).ok_or(XmpError::NoDescription)?;
    let mut insert = String::new();
    let prefix = match declared_prefix(text, ns) {
        Some(p) => p,
        None => {
            insert.push_str(&format!("\n    xmlns:{}=\"{ns}\"", prefixes[0]));
            prefixes[0].to_string()
        }
    };
    insert.push_str(&format!("\n    {prefix}:{name}=\"{}\"", escape(v)));
    text.insert_str(m.end(), &insert);
    Ok(())
}

/// Apply `marks` to the sidecar text, creating a minimal document if `text` is empty.
pub fn apply(text: &str, marks: &XmpMarks) -> Result<String, XmpError> {
    let mut out = if text.trim().is_empty() {
        empty_sidecar()
    } else {
        text.to_string()
    };
    if let Some(r) = marks.rating {
        set_prop(
            &mut out,
            &["xmp", "xap"],
            NS_XMP,
            "Rating",
            Some(&r.to_string()),
        )?;
    }
    if let Some(l) = &marks.label {
        set_prop(
            &mut out,
            &["xmp", "xap"],
            NS_XMP,
            "Label",
            if l.is_empty() { None } else { Some(l) },
        )?;
    }
    if let Some(c) = marks.color_class {
        set_prop(
            &mut out,
            &["photomechanic"],
            NS_PM,
            "ColorClass",
            Some(&c.to_string()),
        )?;
    }
    if let Some(t) = marks.tagged {
        set_prop(
            &mut out,
            &["photomechanic"],
            NS_PM,
            "Tagged",
            Some(if t { "True" } else { "False" }),
        )?;
    }
    Ok(out)
}

/// True when `got` carries every mark that `want` asked for.
pub fn satisfies(want: &XmpMarks, got: &XmpMarks) -> bool {
    let check = |w: Option<i64>, g: Option<i64>| w.is_none() || w == g;
    check(want.rating.map(i64::from), got.rating.map(i64::from))
        && check(
            want.color_class.map(i64::from),
            got.color_class.map(i64::from),
        )
        && check(want.tagged.map(i64::from), got.tagged.map(i64::from))
        && !want.label.as_ref().is_some_and(|l| {
            let w = if l.is_empty() { None } else { Some(l.clone()) };
            w != got.label
        })
}

/// Generic atomic sidecar edit: read (or start empty), `edit` the text,
/// `verify` the result, then temp → fsync → rename. The original is never
/// touched unless verification passes.
pub fn write_sidecar_with(
    path: &Path,
    edit: impl FnOnce(&str) -> Result<String, XmpError>,
    verify: impl FnOnce(&str) -> bool,
) -> Result<(), XmpError> {
    let existing = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let updated = edit(&existing)?;
    if updated == existing {
        return Ok(());
    }
    if !verify(&updated) {
        return Err(XmpError::Verify(path.to_path_buf()));
    }
    let tmp = path.with_extension(format!("xmp.tmp{}", std::process::id()));
    let result = (|| {
        fs::write(&tmp, updated.as_bytes())?;
        fs::File::open(&tmp)?.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

/// Write marks into the sidecar at `path`, creating it if needed. Atomic:
/// writes a temp file beside it and renames.
pub fn write_marks(path: &Path, marks: &XmpMarks) -> Result<(), XmpError> {
    let existing = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let updated = apply(&existing, marks)?;
    if updated == existing {
        return Ok(());
    }
    // Read-back check before the file is replaced: if the edited text does
    // not yield the marks we meant to write, leave the original untouched.
    if !satisfies(marks, &parse(&updated)) {
        return Err(XmpError::Verify(path.to_path_buf()));
    }
    let tmp = path.with_extension(format!("xmp.tmp{}", std::process::id()));
    let result = (|| {
        fs::write(&tmp, updated.as_bytes())?;
        fs::File::open(&tmp)?.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

/// Extract the XMP packet embedded in a JPEG (APP1 segment with the
/// `http://ns.adobe.com/xap/1.0/` signature), if any. Extended XMP (APP1
/// chunks with the `/xmp/extension/` signature) is not reassembled; the
/// standard packet carries ratings and labels.
pub fn embedded_packet_in_jpeg(data: &[u8]) -> Option<&str> {
    const SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let seg = data.get(i + 4..i + 2 + len)?;
        if marker == 0xE1 && seg.starts_with(SIG) {
            return std::str::from_utf8(&seg[SIG.len()..]).ok();
        }
        if marker == 0xDA {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Read marks embedded in a JPEG file. `Ok(None)` when it carries no XMP.
pub fn read_embedded(path: &Path) -> Result<Option<XmpMarks>, XmpError> {
    let data = fs::read(path)?;
    Ok(embedded_packet_in_jpeg(&data).map(parse))
}

/// Where two sources disagree about a mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub field: &'static str,
    pub sidecar: String,
    pub embedded: String,
}

/// Compare sidecar marks with marks embedded in a sibling JPEG. Only fields
/// present on *both* sides can conflict; an absent value is not a conflict.
pub fn conflicts(sidecar: &XmpMarks, embedded: &XmpMarks) -> Vec<Conflict> {
    let mut out = Vec::new();
    if let (Some(a), Some(b)) = (sidecar.rating, embedded.rating) {
        if a != b {
            out.push(Conflict {
                field: "rating",
                sidecar: a.to_string(),
                embedded: b.to_string(),
            });
        }
    }
    if let (Some(a), Some(b)) = (&sidecar.label, &embedded.label) {
        if a != b {
            out.push(Conflict {
                field: "label",
                sidecar: a.clone(),
                embedded: b.clone(),
            });
        }
    }
    if let (Some(a), Some(b)) = (sidecar.color_class, embedded.color_class) {
        if a != b {
            out.push(Conflict {
                field: "color_class",
                sidecar: a.to_string(),
                embedded: b.to_string(),
            });
        }
    }
    if let (Some(a), Some(b)) = (sidecar.tagged, embedded.tagged) {
        if a != b {
            out.push(Conflict {
                field: "tagged",
                sidecar: a.to_string(),
                embedded: b.to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PM_STYLE: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Photo Mechanic">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:photomechanic="http://ns.camerabits.com/photomechanic/1.0/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
   photomechanic:ColorClass="1"
   photomechanic:Tagged="False"
   xmp:Rating="4"
   xmp:Label="Winner">
   <dc:description><rdf:Alt><rdf:li xml:lang="x-default">A caption &amp; more.</rdf:li></rdf:Alt></dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn parses_photo_mechanic_attributes() {
        let m = parse(PM_STYLE);
        assert_eq!(m.rating, Some(4));
        assert_eq!(m.label.as_deref(), Some("Winner"));
        assert_eq!(m.color_class, Some(1));
        assert_eq!(m.tagged, Some(false));
    }

    /// Shape of a sidecar Photo Mechanic 2026.6 wrote on 2026-10-13 (no xpacket
    /// wrapper, crd/plus/aux namespaces, Prefs and PMVersion).
    const PM_2026: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core 6.0.0">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:crd="http://ns.adobe.com/camera-raw-defaults/1.0/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:plus="http://ns.useplus.org/ldf/xmp/1.0/"
    xmlns:photomechanic="http://ns.camerabits.com/photomechanic/1.0/"
    xmlns:aux="http://ns.adobe.com/exif/1.0/aux/"
   xmp:CreatorTool="NIKON D850 Ver.1.30     "
   xmp:CreateDate="2025-04-10T19:10:41.62-04:00"
   xmp:Label="Yellow"
   xmp:Rating="3"
   crd:HueAdjustmentRed="0"
   photoshop:DateCreated="2025-04-10T19:10:41.62-04:00"
   plus:DataMining="http://ns.useplus.org/ldf/vocab/DMI-UNSPECIFIED"
   photomechanic:ColorClass="2"
   photomechanic:Tagged="True"
   photomechanic:Prefs="1:2:3:007802"
   photomechanic:PMVersion="2026.6.9175"
   aux:ImageNumber="7802"/>
 </rdf:RDF>
</x:xmpmeta>
"#;

    #[test]
    fn photo_mechanic_2026_sidecar_round_trips() {
        let m = parse(PM_2026);
        assert_eq!(
            m,
            XmpMarks {
                rating: Some(3),
                label: Some("Yellow".into()),
                color_class: Some(2),
                tagged: Some(true)
            }
        );
        let scheme = ColorClassScheme::default();
        assert_eq!(
            m.to_marks(&scheme),
            Marks {
                rating: Rating(3),
                color_class: ColorClass(2),
                tagged: true
            }
        );
        // Edit it the way the app would and check PM's own fields survive.
        let out = apply(
            PM_2026,
            &XmpMarks::from_marks(
                &Marks {
                    rating: Rating(5),
                    color_class: ColorClass(1),
                    tagged: false,
                },
                &scheme,
            ),
        )
        .unwrap();
        assert!(out.contains(r#"photomechanic:Prefs="1:2:3:007802""#));
        assert!(out.contains(r#"photomechanic:PMVersion="2026.6.9175""#));
        assert!(out.contains(r#"xmp:Label="Red""#));
        assert!(out.contains(r#"photomechanic:ColorClass="1""#));
        assert!(out.contains(r#"photomechanic:Tagged="False""#));
        assert_eq!(without_marks(&out), without_marks(PM_2026));
    }

    /// Sidecar as rewritten by Capture One 16.8 `sync metadata` on 2026-10-13
    /// over a Photo Mechanic one: element form, Label updated, ColorClass stale.
    const CAPTURE_ONE: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core 5.5.0">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:photomechanic="http://ns.camerabits.com/photomechanic/1.0/"
    xmlns:exif="http://ns.adobe.com/exif/1.0/">
   <xmp:CreateDate>2025-04-10T19:10:41.62-04:00</xmp:CreateDate>
   <xmp:Rating>5</xmp:Rating>
   <xmp:Label>Yellow</xmp:Label>
   <photoshop:Urgency>7</photoshop:Urgency>
   <photomechanic:ColorClass>4</photomechanic:ColorClass>
   <photomechanic:Tagged>True</photomechanic:Tagged>
   <photomechanic:Prefs>1:7:5:007802</photomechanic:Prefs>
   <exif:DateTimeOriginal>2025-04-10T19:10:41.62Z</exif:DateTimeOriginal>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

    #[test]
    fn capture_one_label_wins_over_stale_color_class() {
        let m = parse(CAPTURE_ONE);
        assert_eq!(m.rating, Some(5));
        assert_eq!(m.label.as_deref(), Some("Yellow"));
        assert_eq!(m.color_class, Some(4), "the stale number is still parsed");
        let scheme = ColorClassScheme::default();
        assert_eq!(
            m.to_marks(&scheme).color_class,
            ColorClass(2),
            "label wins when it names a known class"
        );
        // A label we cannot map leaves the number in charge.
        let odd = XmpMarks {
            label: Some("Second edit".into()),
            color_class: Some(4),
            ..Default::default()
        };
        assert_eq!(odd.to_marks(&scheme).color_class, ColorClass(4));
        // Writing back repairs the number and keeps element form.
        let out = apply(
            CAPTURE_ONE,
            &XmpMarks::from_marks(&m.to_marks(&scheme), &scheme),
        )
        .unwrap();
        assert!(
            out.contains("<photomechanic:ColorClass>2</photomechanic:ColorClass>"),
            "{out}"
        );
        assert!(out.contains("<xmp:Label>Yellow</xmp:Label>"));
        assert!(out.contains("<photoshop:Urgency>7</photoshop:Urgency>"));
    }

    #[test]
    fn parses_element_form_and_xap_prefix() {
        let t = r#"<rdf:Description xmlns:xap="http://ns.adobe.com/xap/1.0/"><xap:Rating>3</xap:Rating><xap:Label>Red</xap:Label></rdf:Description>"#;
        let m = parse(t);
        assert_eq!(m.rating, Some(3));
        assert_eq!(m.label.as_deref(), Some("Red"));
        assert_eq!(m.color_class, None);
        assert_eq!(
            m.to_marks(&ColorClassScheme::default()).color_class,
            ColorClass(1)
        );
    }

    #[test]
    fn edits_in_place_and_preserves_everything_else() {
        let new = XmpMarks {
            rating: Some(2),
            label: Some("Superior".into()),
            color_class: Some(3),
            tagged: Some(true),
        };
        let out = apply(PM_STYLE, &new).unwrap();
        assert!(out.contains(r#"xmp:Rating="2""#));
        assert!(out.contains(r#"xmp:Label="Superior""#));
        assert!(out.contains(r#"photomechanic:ColorClass="3""#));
        assert!(out.contains(r#"photomechanic:Tagged="True""#));
        assert!(
            out.contains("A caption &amp; more."),
            "caption must survive"
        );
        assert_eq!(out.matches("xmp:Rating").count(), 1);
        assert_eq!(parse(&out), new);
    }

    #[test]
    fn inserts_into_foreign_sidecar_and_declares_namespace() {
        let lr = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+0.5"></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let m = XmpMarks {
            rating: Some(5),
            label: None,
            color_class: Some(2),
            tagged: None,
        };
        let out = apply(lr, &m).unwrap();
        assert!(out.contains(r#"xmlns:xmp="http://ns.adobe.com/xap/1.0/""#));
        assert!(
            out.contains(r#"xmlns:photomechanic="http://ns.camerabits.com/photomechanic/1.0/""#)
        );
        assert!(out.contains(r#"crs:Exposure2012="+0.5""#));
        assert_eq!(parse(&out).rating, Some(5));
        assert_eq!(parse(&out).color_class, Some(2));
    }

    #[test]
    fn clearing_label_removes_it() {
        let m = XmpMarks {
            rating: None,
            label: Some(String::new()),
            color_class: Some(0),
            tagged: None,
        };
        let out = apply(PM_STYLE, &m).unwrap();
        assert!(!out.contains("xmp:Label"));
        assert_eq!(parse(&out).label, None);
        assert_eq!(parse(&out).rating, Some(4), "untouched props stay");
    }

    #[test]
    fn write_creates_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("DSC_0001.xmp");
        let scheme = ColorClassScheme::default();
        let marks = Marks {
            rating: Rating(3),
            color_class: ColorClass(5),
            tagged: true,
        };
        write_marks(&p, &XmpMarks::from_marks(&marks, &scheme)).unwrap();
        let back = read_sidecar(&p).unwrap().unwrap().to_marks(&scheme);
        assert_eq!(back, marks);
        assert!(read_sidecar(&dir.path().join("missing.xmp"))
            .unwrap()
            .is_none());
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("<?xpacket begin="));
        assert!(text.contains(r#"xmp:Label="Purple""#));
    }

    /// A Lightroom Classic style sidecar: multi-line Description with crs
    /// develop settings, dc bags, xmpMM history, and no photomechanic namespace.
    const LIGHTROOM: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:aux="http://ns.adobe.com/exif/1.0/aux/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:xmpMM="http://ns.adobe.com/xap/1.0/mm/"
    xmlns:stEvt="http://ns.adobe.com/xap/1.0/sType/ResourceEvent#"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmlns:myvendor="http://example.com/custom/1.0/"
   xmp:ModifyDate="2026-10-06T14:03:11-04:00"
   xmp:CreatorTool="Adobe Photoshop Lightroom Classic 14.0 (Macintosh)"
   xmp:Rating="2"
   xmp:Label="Yellow"
   aux:Lens="NIKKOR Z 70-200mm f/2.8 VR S"
   aux:SerialNumber="6012345"
   photoshop:DateCreated="2026-10-06T14:03:11.000-04:00"
   photoshop:Headline="100726 SPO BGLF WPIAL Championships"
   xmpMM:DocumentID="xmp.did:6c7a8f1e-1b2c-4d5e-8f90-123456789abc"
   crs:Version="17.0"
   crs:ProcessVersion="15.4"
   crs:WhiteBalance="As Shot"
   crs:Exposure2012="+0.35"
   crs:Contrast2012="+10"
   crs:ToneCurveName2012="Linear"
   crs:HasSettings="True"
   crs:HasCrop="False"
   myvendor:Secret="keep &amp; me">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">North Catholic's Christopher Faber tees off on No. 1 in the WPIAL Class 2A Boys Individual Golf Championship held Tuesday, Oct. 6, 2026, at Oakmont Country Club. Caleb McCartney/Special to the Eagle</rdf:li>
    </rdf:Alt>
   </dc:description>
   <dc:subject>
    <rdf:Bag>
     <rdf:li>WPIAL</rdf:li>
     <rdf:li>golf</rdf:li>
     <rdf:li>Oakmont Country Club</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:creator>
    <rdf:Seq>
     <rdf:li>Caleb McCartney</rdf:li>
    </rdf:Seq>
   </dc:creator>
   <xmpMM:History>
    <rdf:Seq>
     <stEvt:action>derived</stEvt:action>
    </rdf:Seq>
   </xmpMM:History>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    /// Strip only the properties we own, so "everything else" can be compared exactly.
    fn without_marks(t: &str) -> String {
        let re = Regex::new(r#"\s*(?:xmp|xap):(?:Rating|Label)="[^"]*"|\s*photomechanic:(?:ColorClass|Tagged)="[^"]*"|\s*xmlns:photomechanic="[^"]*""#).unwrap();
        re.replace_all(t, "").into_owned()
    }

    #[test]
    fn lightroom_sidecar_everything_else_is_byte_identical() {
        let m = XmpMarks {
            rating: Some(5),
            label: Some("Winner".into()),
            color_class: Some(1),
            tagged: Some(true),
        };
        let out = apply(LIGHTROOM, &m).unwrap();
        assert_eq!(
            without_marks(&out),
            without_marks(LIGHTROOM),
            "non-mark content must not change"
        );
        let back = parse(&out);
        assert_eq!(back, m);
        assert!(out.contains(r#"crs:Exposure2012="+0.35""#));
        assert!(out.contains("myvendor:Secret=\"keep &amp; me\""));
        assert!(out.contains("<rdf:li>Oakmont Country Club</rdf:li>"));
        assert_eq!(out.matches("xmlns:photomechanic=").count(), 1);
        // Idempotent: applying the same marks again changes nothing.
        assert_eq!(apply(&out, &m).unwrap(), out);
    }

    #[test]
    fn legacy_xap_prefix_is_updated_not_duplicated() {
        let t = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xap="http://ns.adobe.com/xap/1.0/" xap:Rating="1" xap:CreateDate="2020-01-01"></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let m = XmpMarks {
            rating: Some(4),
            label: Some("Winner".into()),
            color_class: None,
            tagged: None,
        };
        let out = apply(t, &m).unwrap();
        assert!(out.contains(r#"xap:Rating="4""#));
        assert!(!out.contains("xmp:Rating"), "{out}");
        assert!(
            out.contains(r#"xap:Label="Winner""#),
            "label must use the prefix bound to the xmp namespace: {out}"
        );
        assert!(
            !out.contains("xmlns:xmp="),
            "must not declare a second prefix for the same namespace"
        );
        assert_eq!(parse(&out).rating, Some(4));
        assert_eq!(parse(&out).label.as_deref(), Some("Winner"));
    }

    #[test]
    fn malformed_sidecar_without_description_is_refused() {
        let m = XmpMarks {
            rating: Some(3),
            label: None,
            color_class: None,
            tagged: None,
        };
        assert!(matches!(
            apply("<x:xmpmeta></x:xmpmeta>", &m),
            Err(XmpError::NoDescription)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn failed_write_leaves_original_untouched_and_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.xmp");
        fs::write(&p, PM_STYLE).unwrap();
        // Make the directory read-only so the temp file cannot be created.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let m = XmpMarks {
            rating: Some(3),
            label: None,
            color_class: None,
            tagged: None,
        };
        // Root (containers, some CI) ignores directory permissions; probe
        // rather than trusting $USER, which is often unset there.
        let enforced = fs::File::create(dir.path().join("probe")).is_err();
        let result = write_marks(&p, &m);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        if !enforced {
            eprintln!("SKIPPED: permissions are not enforced for this user");
            return;
        }
        assert!(matches!(result, Err(XmpError::Io(_))), "{result:?}");
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            PM_STYLE,
            "original must be untouched"
        );
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file left behind"
        );
    }

    #[test]
    fn embedded_jpeg_xmp_and_conflicts() {
        let packet = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="1" xmp:Label="Red"/></rdf:RDF></x:xmpmeta>"#;
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        jpg.extend_from_slice(&[0u8; 14]);
        let sig = b"http://ns.adobe.com/xap/1.0/\0";
        let len = 2 + sig.len() + packet.len();
        jpg.extend_from_slice(&[0xFF, 0xE1]);
        jpg.extend_from_slice(&(len as u16).to_be_bytes());
        jpg.extend_from_slice(sig);
        jpg.extend_from_slice(packet.as_bytes());
        jpg.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        let emb = parse(embedded_packet_in_jpeg(&jpg).unwrap());
        assert_eq!(emb.rating, Some(1));
        let side = XmpMarks {
            rating: Some(4),
            label: Some("Red".into()),
            color_class: Some(1),
            tagged: None,
        };
        let c = conflicts(&side, &emb);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].field, "rating");
        assert_eq!(conflicts(&side, &XmpMarks::default()), vec![]);
        assert!(embedded_packet_in_jpeg(b"II*\0").is_none());
    }

    /// Runs only when ExifTool is installed; validates the sidecar we create
    /// from scratch and the Lightroom one we edit, and checks ExifTool reads
    /// the Photo Mechanic properties back.
    #[test]
    fn exiftool_validates_and_reads_our_output() {
        let Ok(out) = std::process::Command::new("exiftool").arg("-ver").output() else {
            eprintln!("SKIPPED: exiftool not installed");
            return;
        };
        assert!(out.status.success());
        let dir = tempfile::tempdir().unwrap();
        let scheme = ColorClassScheme::default();
        let marks = Marks {
            rating: Rating(3),
            color_class: ColorClass(2),
            tagged: true,
        };
        let fresh = dir.path().join("fresh.xmp");
        write_marks(&fresh, &XmpMarks::from_marks(&marks, &scheme)).unwrap();
        let edited = dir.path().join("lr.xmp");
        fs::write(&edited, LIGHTROOM).unwrap();
        write_marks(&edited, &XmpMarks::from_marks(&marks, &scheme)).unwrap();
        for p in [&fresh, &edited] {
            let o = std::process::Command::new("exiftool")
                .args([
                    "-validate",
                    "-warning",
                    "-error",
                    "-a",
                    "-s",
                    "-Rating",
                    "-Label",
                    "-ColorClass",
                    "-Tagged",
                    "-Headline",
                ])
                .arg(p)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            assert!(o.status.success(), "{text}");
            assert!(!text.contains("Error"), "{text}");
            assert!(
                text.contains("Rating                          : 3"),
                "{text}"
            );
            assert!(
                text.contains("Label                           : Yellow"),
                "{text}"
            );
            assert!(
                text.contains("ColorClass                      : 2 (Winner alt)"),
                "{text}"
            );
            assert!(
                text.contains("Tagged                          : Yes"),
                "{text}"
            );
            if p == &edited {
                assert!(
                    text.contains(
                        "Headline                        : 100726 SPO BGLF WPIAL Championships"
                    ),
                    "{text}"
                );
            }
        }
    }
}
