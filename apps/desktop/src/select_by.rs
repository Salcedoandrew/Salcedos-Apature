//! Edit › Select Tagged, Untagged, by Rating, by Color Class, Cropped and
//! RAW+JPEG pairs, like Photo Mechanic's Edit menu. They pick from the
//! current tab's visible photos (the filtered, sorted view) and replace the
//! selection.

use apature_core::{Marks, PhotoItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectBy {
    Tagged,
    Untagged,
    /// Exactly this many stars (1–5), or 0 for no stars (rejects excluded).
    Rating(u8),
    /// This many stars or more (1–5).
    RatingAtLeast(u8),
    /// Colour class 1–8, or 0 for none.
    Class(u8),
    /// Any colour class at all.
    AnyClass,
    /// A crop is stored in the sidecar.
    Cropped,
    /// A RAW with a JPEG (or HEIC) beside it.
    Pairs,
}

impl SelectBy {
    pub fn menu_id(self) -> String {
        match self {
            SelectBy::Tagged => "edit.select.tagged".into(),
            SelectBy::Untagged => "edit.select.untagged".into(),
            SelectBy::Rating(n) => format!("edit.select.rating.{n}"),
            SelectBy::RatingAtLeast(n) => format!("edit.select.atleast.{n}"),
            SelectBy::Class(n) => format!("edit.select.class.{n}"),
            SelectBy::AnyClass => "edit.select.anyclass".into(),
            SelectBy::Cropped => "edit.select.cropped".into(),
            SelectBy::Pairs => "edit.select.pairs".into(),
        }
    }

    pub fn from_menu_id(id: &str) -> Option<SelectBy> {
        let rest = id.strip_prefix("edit.select.")?;
        let num = |s: &str, max: u8| s.parse::<u8>().ok().filter(|n| *n <= max);
        Some(match rest {
            "tagged" => SelectBy::Tagged,
            "untagged" => SelectBy::Untagged,
            "cropped" => SelectBy::Cropped,
            "pairs" => SelectBy::Pairs,
            "anyclass" => SelectBy::AnyClass,
            _ => {
                if let Some(n) = rest.strip_prefix("rating.") {
                    SelectBy::Rating(num(n, 5)?)
                } else if let Some(n) = rest.strip_prefix("atleast.") {
                    SelectBy::RatingAtLeast(num(n, 5).filter(|n| *n >= 1)?)
                } else {
                    SelectBy::Class(num(rest.strip_prefix("class.")?, 8)?)
                }
            }
        })
    }

    /// For the status line: "tagged", "3-star", "Red" …
    pub fn describe(self, class_names: &[String; 8]) -> String {
        match self {
            SelectBy::Tagged => "tagged".into(),
            SelectBy::Untagged => "untagged".into(),
            SelectBy::Rating(0) => "unrated".into(),
            SelectBy::Rating(n) => format!("{n}-star"),
            SelectBy::RatingAtLeast(n) => format!("{n}-star or better"),
            SelectBy::Class(0) => "unclassed".into(),
            SelectBy::AnyClass => "colour-classed".into(),
            SelectBy::Class(n) => format!(
                "{} (class {n})",
                class_names.get(n as usize - 1).cloned().unwrap_or_default()
            ),
            SelectBy::Cropped => "cropped".into(),
            SelectBy::Pairs => "RAW+JPEG".into(),
        }
    }

    /// Does one photo qualify? `cropped` is only asked for [`SelectBy::Cropped`].
    pub fn matches(self, marks: &Marks, item: &PhotoItem, cropped: impl FnOnce() -> bool) -> bool {
        let stars = marks.rating.0;
        match self {
            SelectBy::Tagged => marks.tagged,
            SelectBy::Untagged => !marks.tagged,
            SelectBy::Rating(n) => stars == n as i8,
            SelectBy::RatingAtLeast(n) => stars >= n as i8,
            SelectBy::Class(n) => marks.color_class.0 == n,
            SelectBy::AnyClass => marks.color_class.0 != 0,
            SelectBy::Cropped => cropped(),
            SelectBy::Pairs => {
                item.primary.kind == apature_core::Kind::Raw
                    && item
                        .secondary
                        .iter()
                        .any(|v| v.kind != apature_core::Kind::Raw)
            }
        }
    }
}

/// The visible photos (`view`, in view order) that match `by`.
pub fn pick(
    by: SelectBy,
    view: &[usize],
    marks: &[Marks],
    items: &[PhotoItem],
    cropped: impl Fn(usize) -> bool,
) -> Vec<usize> {
    view.iter()
        .copied()
        .filter(|&i| {
            let m = marks.get(i).copied().unwrap_or_default();
            items
                .get(i)
                .is_some_and(|it| by.matches(&m, it, || cropped(i)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::{ColorClass, Kind, Rating, Variant};
    use std::path::PathBuf;

    fn item(id: usize, kinds: &[Kind]) -> PhotoItem {
        let v = |k: &Kind| Variant {
            path: PathBuf::from(format!(
                "/p/{id}.{}",
                match k {
                    Kind::Raw => "NEF",
                    Kind::Jpeg => "JPG",
                    Kind::Heic => "HEIC",
                }
            )),
            kind: *k,
            size: 1,
            modified: None,
        };
        PhotoItem {
            id,
            stem: id.to_string(),
            primary: v(&kinds[0]),
            secondary: kinds[1..].iter().map(v).collect(),
        }
    }

    fn marks(rating: i8, class: u8, tagged: bool) -> Marks {
        Marks {
            rating: Rating(rating),
            color_class: ColorClass(class),
            tagged,
        }
    }

    #[test]
    fn picks_by_mark_from_the_visible_view_only() {
        let items: Vec<PhotoItem> = (0..5).map(|i| item(i, &[Kind::Raw])).collect();
        let m = vec![
            marks(3, 2, true),
            marks(5, 0, false),
            marks(-1, 2, true),
            marks(0, 0, false),
            marks(4, 1, true),
        ];
        // Item 4 is filtered out; the view is in sort order.
        let view = [3, 2, 1, 0];
        let p = |by| pick(by, &view, &m, &items, |_| false);
        assert_eq!(p(SelectBy::Tagged), [2, 0]);
        assert_eq!(p(SelectBy::Untagged), [3, 1]);
        assert_eq!(p(SelectBy::Rating(3)), [0]);
        // No stars: not the reject.
        assert_eq!(p(SelectBy::Rating(0)), [3]);
        assert_eq!(p(SelectBy::AnyClass), [2, 0]);
        assert_eq!(p(SelectBy::RatingAtLeast(3)), [1, 0]);
        // A reject is never "at least one star".
        assert_eq!(p(SelectBy::RatingAtLeast(1)), [1, 0]);
        assert_eq!(p(SelectBy::Class(2)), [2, 0]);
        assert_eq!(p(SelectBy::Class(0)), [3, 1]);
    }

    #[test]
    fn cropped_and_pairs() {
        let items = vec![
            item(0, &[Kind::Raw, Kind::Jpeg]),
            item(1, &[Kind::Raw]),
            item(2, &[Kind::Jpeg]),
            item(3, &[Kind::Raw, Kind::Heic]),
        ];
        let m = vec![Marks::default(); 4];
        let view = [0, 1, 2, 3];
        assert_eq!(pick(SelectBy::Pairs, &view, &m, &items, |_| false), [0, 3]);
        assert_eq!(pick(SelectBy::Cropped, &view, &m, &items, |i| i == 2), [2]);
    }

    #[test]
    fn menu_ids_round_trip() {
        let all = [
            SelectBy::Tagged,
            SelectBy::Untagged,
            SelectBy::Rating(0),
            SelectBy::Rating(1),
            SelectBy::Rating(5),
            SelectBy::RatingAtLeast(2),
            SelectBy::Class(0),
            SelectBy::Class(8),
            SelectBy::AnyClass,
            SelectBy::Cropped,
            SelectBy::Pairs,
        ];
        for by in all {
            assert_eq!(SelectBy::from_menu_id(&by.menu_id()), Some(by));
        }
        assert_eq!(SelectBy::from_menu_id("edit.select.rating.6"), None);
        assert_eq!(SelectBy::from_menu_id("edit.select.atleast.0"), None);
        assert_eq!(SelectBy::from_menu_id("edit.select.class.9"), None);
        assert_eq!(SelectBy::from_menu_id("edit.selectall"), None);
    }
}
