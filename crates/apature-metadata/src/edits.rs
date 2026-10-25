//! Non-destructive edits kept in the XMP sidecar: rotation and crop.
//!
//! Both use the fields Lightroom, Camera Raw and Photo Mechanic already read,
//! so an edit made here shows up there too:
//!
//! * rotation is `tiff:Orientation` (EXIF values 1–8). When present it
//!   overrides the orientation in the file; the RAW itself is never touched.
//! * crop is `crs:HasCrop` with `crs:CropTop/Left/Bottom/Right`, fractions of
//!   the image as stored (before the orientation is applied), and
//!   `crs:CropAngle` written as 0 since there is no straighten tool. A crop
//!   that carries a straighten angle (Lightroom, Camera Raw) is reported as
//!   [`Edits::straightened_crop`] and not applied, rather than applied
//!   without its angle and framed differently from the editor that made it.
//!
//! Properties are found under the usual prefixes (`tiff:`, `crs:`) and under
//! whatever other prefix a document binds to the same namespace URI.
//!
//! Editing follows the sidecar rules in [`crate::xmp`]: properties are
//! changed in place, everything else in the document is left alone, and the
//! file is replaced atomically only after it reads back correctly.

use std::path::Path;

use crate::xmp::{cached_re, prop_pattern, set_prop, value_of, write_sidecar_with, XmpError};

pub const NS_TIFF: &str = "http://ns.adobe.com/tiff/1.0/";
pub const NS_CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";

/// A crop as fractions (0..=1) of the stored image: `left < right`,
/// `top < bottom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl Crop {
    /// Clamp to the image and reject an empty or inverted rectangle.
    pub fn normalized(self) -> Option<Crop> {
        let c = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let r = Crop {
            top: c(self.top),
            left: c(self.left),
            bottom: c(self.bottom),
            right: c(self.right),
        };
        (r.right - r.left > 0.001 && r.bottom - r.top > 0.001).then_some(r)
    }

    /// True when the crop keeps the whole image.
    pub fn is_full(&self) -> bool {
        self.top <= 0.0005 && self.left <= 0.0005 && self.bottom >= 0.9995 && self.right >= 0.9995
    }

    /// Pixel rectangle `(x, y, w, h)` inside a `width` × `height` image.
    pub fn pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let x0 = (self.left * width as f32).round() as u32;
        let y0 = (self.top * height as f32).round() as u32;
        let x1 = ((self.right * width as f32).round() as u32).clamp(x0 + 1, width.max(1));
        let y1 = ((self.bottom * height as f32).round() as u32).clamp(y0 + 1, height.max(1));
        let x0 = x0.min(x1 - 1);
        let y0 = y0.min(y1 - 1);
        (x0, y0, x1 - x0, y1 - y0)
    }
}

/// Rotation and crop as found in a sidecar. `None` = not set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edits {
    pub orientation: Option<u8>,
    pub crop: Option<Crop>,
    /// The sidecar has a crop with a non-zero `crs:CropAngle`; `crop` is
    /// `None` because a straightened crop cannot be applied here.
    pub straightened_crop: bool,
}

impl Edits {
    /// True when there is a rotation or a crop to apply.
    pub fn is_edited(&self) -> bool {
        self.orientation.is_some() || self.crop.is_some()
    }

    /// The orientation to show and export: the sidecar's when set, else the
    /// file's own.
    pub fn orientation_or(&self, file: u32) -> u32 {
        self.orientation.map(u32::from).unwrap_or(file)
    }
}

/// EXIF orientation after a further quarter turn clockwise.
pub fn rotate_cw(o: u32) -> u32 {
    match o {
        1 => 6,
        6 => 3,
        3 => 8,
        8 => 1,
        2 => 7,
        7 => 4,
        4 => 5,
        5 => 2,
        _ => 6,
    }
}

/// EXIF orientation after a further quarter turn counter-clockwise.
pub fn rotate_ccw(o: u32) -> u32 {
    rotate_cw(rotate_cw(rotate_cw(o)))
}

/// The orientation that undoes `o` (only the two quarter turns differ from
/// their own inverse).
pub fn inverse(o: u32) -> u32 {
    match o {
        6 => 8,
        8 => 6,
        o => o,
    }
}

const CROP_NAMES: [&str; 4] = ["CropTop", "CropLeft", "CropBottom", "CropRight"];

/// The prefixes a property of `ns` may appear under in `text`: the usual
/// one, plus any other prefix the document binds to the same URI.
fn prefixes(text: &str, ns: &str, usual: &str) -> Vec<String> {
    let mut v = vec![usual.to_string()];
    if let Some(p) = crate::xmp::declared_prefix(text, ns) {
        if p != usual {
            v.push(p);
        }
    }
    v
}

/// Value of `prefix:name` under any of `prefixes`.
fn prop(text: &str, prefixes: &[String], name: &str) -> Option<String> {
    let alt = prefixes
        .iter()
        .map(|p| regex::escape(p))
        .collect::<Vec<_>>()
        .join("|");
    value_of(&cached_re(&prop_pattern(&alt, name)).ok()?, text)
}

/// Parse rotation and crop out of sidecar text.
pub fn parse(text: &str) -> Edits {
    let tiff = prefixes(text, NS_TIFF, "tiff");
    let crs = prefixes(text, NS_CRS, "crs");
    let orientation = prop(text, &tiff, "Orientation")
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|o| (1..=8).contains(o));
    let has_crop = prop(text, &crs, "HasCrop")
        .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "true" | "1"));
    let angle = prop(text, &crs, "CropAngle")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(0.0);
    let straightened_crop = has_crop && angle.abs() > 0.01;
    let crop = if has_crop && !straightened_crop {
        let v: Vec<Option<f32>> = CROP_NAMES
            .iter()
            .map(|n| prop(text, &crs, n).and_then(|v| v.trim().parse::<f32>().ok()))
            .collect();
        match v[..] {
            [Some(top), Some(left), Some(bottom), Some(right)] => Crop {
                top,
                left,
                bottom,
                right,
            }
            .normalized(),
            _ => None,
        }
    } else {
        None
    };
    Edits {
        orientation,
        crop,
        straightened_crop,
    }
}

/// What to change in a sidecar. Each field: `None` leaves it alone,
/// `Some(None)` clears it, `Some(Some(v))` sets it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Change {
    pub orientation: Option<Option<u8>>,
    pub crop: Option<Option<Crop>>,
}

/// Apply `change` to sidecar text (an empty text starts a new sidecar).
pub fn apply(text: &str, change: &Change) -> Result<String, XmpError> {
    let mut out = if text.trim().is_empty() {
        crate::xmp::empty_sidecar()
    } else {
        text.to_string()
    };
    // Edit a property where it already is, whatever prefix the document
    // binds its namespace to, so it is never written twice.
    let tiff = prefixes(&out, NS_TIFF, "tiff");
    let tiff: Vec<&str> = tiff.iter().map(String::as_str).collect();
    let crs = prefixes(&out, NS_CRS, "crs");
    let crs: Vec<&str> = crs.iter().map(String::as_str).collect();
    if let Some(o) = change.orientation {
        let v = o.filter(|o| (1..=8).contains(o)).map(|o| o.to_string());
        set_prop(&mut out, &tiff, NS_TIFF, "Orientation", v.as_deref())?;
    }
    if let Some(crop) = change.crop {
        match crop.and_then(Crop::normalized).filter(|c| !c.is_full()) {
            Some(c) => {
                set_prop(&mut out, &crs, NS_CRS, "HasCrop", Some("True"))?;
                for (name, v) in CROP_NAMES.iter().zip([c.top, c.left, c.bottom, c.right]) {
                    set_prop(&mut out, &crs, NS_CRS, name, Some(&format!("{v:.6}")))?;
                }
                set_prop(&mut out, &crs, NS_CRS, "CropAngle", Some("0"))?;
            }
            None => {
                // Lightroom reads HasCrop="False" as "no crop"; drop the numbers
                // so nothing stale is left behind.
                set_prop(&mut out, &crs, NS_CRS, "HasCrop", Some("False"))?;
                for name in CROP_NAMES {
                    set_prop(&mut out, &crs, NS_CRS, name, None)?;
                }
            }
        }
    }
    Ok(out)
}

fn satisfied(change: &Change, got: &Edits) -> bool {
    let o_ok = match change.orientation {
        None => true,
        Some(want) => want.filter(|o| (1..=8).contains(o)) == got.orientation,
    };
    let c_ok = match change.crop {
        None => true,
        Some(want) => {
            let want = want.and_then(Crop::normalized).filter(|c| !c.is_full());
            match (want, got.crop) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    (a.top - b.top).abs() < 1e-4
                        && (a.left - b.left).abs() < 1e-4
                        && (a.bottom - b.bottom).abs() < 1e-4
                        && (a.right - b.right).abs() < 1e-4
                }
                _ => false,
            }
        }
    };
    o_ok && c_ok
}

/// Read the edits in the sidecar at `path` (none when it does not exist).
pub fn read(path: &Path) -> Result<Edits, XmpError> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(parse(&t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Edits::default()),
        Err(e) => Err(e.into()),
    }
}

/// Write `change` into the sidecar at `path`, creating it when needed.
/// Atomic, and only after the edited text reads back as intended.
pub fn write(path: &Path, change: &Change) -> Result<(), XmpError> {
    write_sidecar_with(path, |t| apply(t, change), |t| satisfied(change, &parse(t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmp:Rating="3"
    crs:Exposure2012="+0.35"
    crs:HasCrop="True"
    crs:CropTop="0.1"
    crs:CropLeft="0.2"
    crs:CropBottom="0.9"
    crs:CropRight="0.8"
    crs:CropAngle="0">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    #[test]
    fn quarter_turns_compose_and_undo() {
        for o in 1..=8 {
            assert_eq!(rotate_ccw(rotate_cw(o)), o);
            assert_eq!(rotate_cw(rotate_cw(rotate_cw(rotate_cw(o)))), o);
            assert_eq!(inverse(inverse(o)), o);
        }
        assert_eq!(rotate_cw(1), 6);
        assert_eq!(rotate_ccw(1), 8);
        assert_eq!(rotate_cw(6), 3);
    }

    #[test]
    fn reads_a_lightroom_crop_and_keeps_its_other_settings() {
        let e = parse(LR);
        assert_eq!(e.orientation, None);
        let c = e.crop.unwrap();
        assert_eq!((c.top, c.left, c.bottom, c.right), (0.1, 0.2, 0.9, 0.8));
        assert_eq!(c.pixels(1000, 500), (200, 50, 600, 400));

        let rotated = apply(
            LR,
            &Change {
                orientation: Some(Some(6)),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(rotated.contains("crs:Exposure2012=\"+0.35\""));
        assert!(rotated.contains("xmlns:tiff=\"http://ns.adobe.com/tiff/1.0/\""));
        assert_eq!(parse(&rotated).orientation, Some(6));
        assert_eq!(parse(&rotated).crop, e.crop);

        let cleared = apply(
            &rotated,
            &Change {
                crop: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parse(&cleared).crop, None);
        assert!(cleared.contains("crs:HasCrop=\"False\""));
        assert!(!cleared.contains("CropTop"));
        assert!(cleared.contains("crs:Exposure2012"));
    }

    #[test]
    fn a_straightened_lightroom_crop_is_reported_not_applied() {
        let t = LR.replace("crs:CropAngle=\"0\"", "crs:CropAngle=\"2.5\"");
        let e = parse(&t);
        assert_eq!(e.crop, None);
        assert!(e.straightened_crop);
        assert!(!e.is_edited());
        assert!(!parse(LR).straightened_crop);
    }

    #[test]
    fn any_prefix_bound_to_the_namespace_is_read_and_edited_in_place() {
        let t = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:foo="http://ns.adobe.com/tiff/1.0/"
    xmlns:cr="http://ns.adobe.com/camera-raw-settings/1.0/"
    foo:Orientation="6"
    cr:HasCrop="True" cr:CropTop="0.1" cr:CropLeft="0.1" cr:CropBottom="0.9" cr:CropRight="0.9">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let e = parse(t);
        assert_eq!(e.orientation, Some(6));
        assert!(e.crop.is_some());
        let out = apply(
            t,
            &Change {
                orientation: Some(Some(8)),
                crop: Some(None),
            },
        )
        .unwrap();
        assert_eq!(out.matches("Orientation=").count(), 1, "{out}");
        assert!(out.contains("foo:Orientation=\"8\""));
        assert_eq!(parse(&out).orientation, Some(8));
        assert_eq!(parse(&out).crop, None);
        assert!(!out.contains("CropTop"));
        // write() verifies by reading back, which now succeeds too.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.xmp");
        std::fs::write(&p, t).unwrap();
        write(
            &p,
            &Change {
                orientation: Some(Some(3)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read(&p).unwrap().orientation, Some(3));
    }

    #[test]
    fn a_full_or_empty_crop_is_no_crop() {
        let full = Crop {
            top: 0.0,
            left: 0.0,
            bottom: 1.0,
            right: 1.0,
        };
        let t = apply(
            "",
            &Change {
                crop: Some(Some(full)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parse(&t).crop, None);
        let inverted = Crop {
            top: 0.5,
            left: 0.5,
            bottom: 0.4,
            right: 0.9,
        };
        assert_eq!(inverted.normalized(), None);
    }

    #[test]
    fn write_creates_then_edits_a_sidecar_atomically() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("DSC_0001.xmp");
        let crop = Crop {
            top: 0.25,
            left: 0.0,
            bottom: 0.75,
            right: 0.5,
        };
        write(
            &p,
            &Change {
                orientation: Some(Some(8)),
                crop: Some(Some(crop)),
            },
        )
        .unwrap();
        let e = read(&p).unwrap();
        assert_eq!(e.orientation, Some(8));
        assert_eq!(e.crop, Some(crop));
        // Marks written later leave the edits alone.
        crate::xmp::write_marks(
            &p,
            &crate::XmpMarks {
                rating: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read(&p).unwrap(), e);
        // Back to the file's own orientation.
        write(
            &p,
            &Change {
                orientation: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read(&p).unwrap().orientation, None);
        assert_eq!(read(&d.path().join("none.xmp")).unwrap(), Edits::default());
        let leftovers: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
