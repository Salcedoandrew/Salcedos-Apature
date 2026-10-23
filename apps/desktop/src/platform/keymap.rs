//! The menu bar's key equivalents for each shortcut layout, as one table:
//! the macOS menu is built from it and the tests check it for collisions.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use crate::prefs::ShortcutLayout;

pub const CMD: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const CTRL: u8 = 8;

/// A key with its modifiers. `key` is the key cap: "T", "1", "[", "F5", "Backspace".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Accel {
    pub mods: u8,
    pub key: &'static str,
}

const fn a(mods: u8, key: &'static str) -> Option<Accel> {
    Some(Accel { mods, key })
}

const DIGITS: [&str; 9] = ["0", "1", "2", "3", "4", "5", "6", "7", "8"];
/// Table names of the Colour Class items (their titles carry the scheme's names).
pub const CLASS: [&str; 9] = [
    "Class None",
    "Class 1",
    "Class 2",
    "Class 3",
    "Class 4",
    "Class 5",
    "Class 6",
    "Class 7",
    "Class 8",
];
/// Table names of the Rating items; 0 is Unrated.
pub const RATING: [&str; 6] = [
    "Unrated", "Rating 1", "Rating 2", "Rating 3", "Rating 4", "Rating 5",
];

/// Table names of Edit › Select by Color Class: none, 1–8, any.
pub const SELECT_CLASS: [&str; 10] = [
    "Select No Class",
    "Select Class 1",
    "Select Class 2",
    "Select Class 3",
    "Select Class 4",
    "Select Class 5",
    "Select Class 6",
    "Select Class 7",
    "Select Class 8",
    "Select Any Class",
];
/// Table names of Edit › Select by Rating: unrated, 1–5, any.
pub const SELECT_RATING: [&str; 7] = [
    "Select Unrated",
    "Select Rating 1",
    "Select Rating 2",
    "Select Rating 3",
    "Select Rating 4",
    "Select Rating 5",
    "Select Any Rating",
];

#[cfg(test)]
/// Keys the Photo Mechanic layout handles outside the menu (the egui key
/// handler), which no menu item may also take without modifiers.
pub const PM_BARE_KEYS: [&str; 13] = [
    "[", "]", "M", "I", "0", "1", "2", "3", "4", "5", "6", "7", "8",
];

/// Bare keys the egui handler owns only while Preview is open, in both
/// layouts: G closes Preview (in the crop tool G cycles the guides instead).
/// No menu item may take them bare, or the press would do two things.
pub const PREVIEW_BARE_KEYS: [&str; 1] = ["G"];

/// Bare Tab shows or hides the sidebar in the catalog and in Preview. The
/// egui input hook owns it (so it can stand aside while a text field has
/// focus or a dialog is open); the menu keeps ⌃⌘S, because a bare Tab menu
/// key would take Tab away from text fields too. No menu item may take it.
#[cfg(test)]
pub const SIDEBAR_BARE_KEY: &str = "Tab";

/// Does a bare G press close Preview? Only in Preview, outside the crop
/// tool (where G is next guides), and without ⇧. Elsewhere G does nothing
/// bare (⌘G is Ingest… in the Photo Mechanic layout, the menu's job).
pub fn g_closes_preview(in_preview: bool, crop_active: bool, shift: bool) -> bool {
    in_preview && !crop_active && !shift
}

/// Every menu item with a key in either layout, by title, with its key in
/// `layout` (None: no key). The items macOS supplies (Hide, Quit, Cut, Copy,
/// Paste, Full Screen) are listed so a collision with them shows up too.
pub fn menu_keys(layout: ShortcutLayout) -> Vec<(&'static str, Option<Accel>)> {
    let pm = layout == ShortcutLayout::PhotoMechanic;
    let pick = |p: Option<Accel>, c: Option<Accel>| if pm { p } else { c };
    let mut v = vec![
        // Apature
        ("Settings…", a(CMD, ",")),
        ("Hide Apature", a(CMD, "H")),
        ("Hide Others", a(CMD | ALT, "H")),
        ("Quit Apature", a(CMD, "Q")),
        // File
        ("Open Folder in New Tab…", a(CMD, "O")),
        ("New Tab…", pick(a(CMD, "N"), a(CMD, "T"))),
        ("Close Tab", a(CMD, "W")),
        ("Add Folder to Favorites", None),
        ("Ingest…", pick(a(CMD, "G"), a(CMD | SHIFT, "I"))),
        ("Export…", pick(a(CMD | CTRL, "X"), a(CMD, "E"))),
        ("Rename Photos…", pick(a(CMD, "M"), a(CMD | SHIFT, "R"))),
        // Photo Mechanic's Copy items key (File menu); free in both layouts.
        ("Copy Photos…", a(CMD, "Y")),
        ("Move to Trash", a(CMD, "Backspace")),
        ("Reveal in Finder", pick(None, a(CMD, "R"))),
        ("Live Ingest", pick(a(CMD | SHIFT, "G"), None)),
        ("Close Window", pick(a(CMD | SHIFT, "W"), a(CMD, "W"))),
        // Edit
        ("Undo", a(CMD, "Z")),
        ("Redo", a(CMD | SHIFT, "Z")),
        ("Cut", a(CMD, "X")),
        ("Copy", a(CMD, "C")),
        ("Paste", a(CMD, "V")),
        ("Copy Metadata", a(CMD | ALT, "C")),
        ("Paste Metadata", a(CMD | ALT, "V")),
        ("Select All", a(CMD, "A")),
        ("Deselect All", pick(a(CMD, "D"), a(CMD | SHIFT, "A"))),
        // Select by: Photo Mechanic's Edit menu keys; classic has none.
        ("Select Tagged", pick(a(CMD, "T"), None)),
        ("Select Untagged", None),
        ("Select Cropped", pick(a(CMD | SHIFT, "C"), None)),
        ("Select RAW+JPEG Pairs", None),
        // View
        ("Contact Sheet", a(CMD | SHIFT, "1")),
        ("Preview", pick(a(CMD, "R"), a(CMD | SHIFT, "2"))),
        ("Caption", a(CMD | SHIFT, "3")),
        ("Ingest", a(CMD | SHIFT, "4")),
        ("Export", a(CMD | SHIFT, "5")),
        ("Show Sidebar", a(CMD | CTRL, "S")),
        ("Show Inspector", a(CMD | ALT, "I")),
        ("Show Filmstrip", a(SHIFT, "F")),
        ("Refresh Folder", pick(a(0, "F5"), a(CMD, "R"))),
        ("Clear Filters", a(CMD | ALT, "F")),
        ("Larger Thumbnails", a(CMD, "=")),
        ("Smaller Thumbnails", a(CMD, "-")),
        ("Zoom 1:1 / Fit", a(0, "Z")),
        ("Compare Selected", pick(a(0, "V"), a(0, "C"))),
        ("Enter Full Screen", a(CMD | CTRL, "F")),
        // Image
        ("Reject", a(0, "X")),
        ("Toggle Tag", a(0, "T")),
        ("Rotate Left", a(CMD, "[")),
        ("Rotate Right", a(CMD, "]")),
        (
            "Apply Assignment Pad to Selection",
            pick(a(CMD | SHIFT, "I"), None),
        ),
        ("Crop…", pick(a(0, "C"), a(0, "K"))),
        ("Remove Crop", pick(a(ALT, "C"), None)),
        ("Loupe", a(0, "L")),
        ("Brighten Preview (view only)", a(0, "B")),
        ("Expand or Collapse Burst", a(0, "S")),
        // Window
        ("Minimize", pick(None, a(CMD, "M"))),
        ("Show Next Tab", a(CMD | SHIFT, "]")),
        ("Show Previous Tab", a(CMD | SHIFT, "[")),
    ];
    // Photo Mechanic: ⌘ + digit colour class, ⌃ + digit rating. Classic, as
    // it always was on the Mac: bare digit colour class, ⌘ + digit rating.
    let (class_mods, rating_mods) = if pm { (CMD, CTRL) } else { (0, CMD) };
    for (n, name) in CLASS.iter().enumerate() {
        v.push((name, a(class_mods, DIGITS[n])));
    }
    for (n, name) in RATING.iter().enumerate() {
        v.push((name, a(rating_mods, DIGITS[n])));
    }
    // Photo Mechanic: ⌘⌃ + digit selects a colour class (9 any, 0 none),
    // ⌥⌘ + digit a star rating (6 any, 0 none).
    for (n, name) in SELECT_CLASS.iter().enumerate() {
        v.push((
            name,
            pick(
                a(
                    CMD | CTRL,
                    ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"][n],
                ),
                None,
            ),
        ));
    }
    for (n, name) in SELECT_RATING.iter().enumerate() {
        v.push((
            name,
            pick(a(CMD | ALT, ["0", "1", "2", "3", "4", "5", "6"][n]), None),
        ));
    }
    v
}

/// The key for one menu item in `layout`; titles not in the table have none.
pub fn key(layout: ShortcutLayout, title: &str) -> Option<Accel> {
    menu_keys(layout)
        .into_iter()
        .find(|(t, _)| *t == title)
        .and_then(|(_, k)| k)
}

#[cfg(test)]
/// Keys used by more than one item, with the items' titles.
pub fn duplicates(layout: ShortcutLayout) -> Vec<(Accel, Vec<&'static str>)> {
    let mut seen: Vec<(Accel, Vec<&'static str>)> = Vec::new();
    for (title, k) in menu_keys(layout) {
        let Some(k) = k else { continue };
        match seen.iter_mut().find(|(s, _)| *s == k) {
            Some((_, titles)) => titles.push(title),
            None => seen.push((k, vec![title])),
        }
    }
    seen.retain(|(_, t)| t.len() > 1);
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_mechanic_layout_has_no_duplicate_keys() {
        assert_eq!(duplicates(ShortcutLayout::PhotoMechanic), vec![]);
        // Nor does the menu take a bare key the egui handler owns in this layout.
        for (title, k) in menu_keys(ShortcutLayout::PhotoMechanic) {
            if let Some(k) = k {
                assert!(
                    !(k.mods == 0 && PM_BARE_KEYS.contains(&k.key)),
                    "{title} takes bare {}",
                    k.key
                );
            }
        }
    }

    #[test]
    fn classic_layout_has_no_new_duplicate_keys() {
        // Classic is kept exactly as it shipped, including its two old pairs.
        let mut d: Vec<Vec<&str>> = duplicates(ShortcutLayout::Classic)
            .into_iter()
            .map(|(_, t)| t)
            .collect();
        d.sort();
        assert_eq!(
            d,
            vec![
                vec!["Close Tab", "Close Window"],
                vec!["Reveal in Finder", "Refresh Folder"],
            ]
        );
    }

    #[test]
    fn preview_bare_keys_are_free_in_both_layouts() {
        for layout in [ShortcutLayout::PhotoMechanic, ShortcutLayout::Classic] {
            for (title, k) in menu_keys(layout) {
                if let Some(k) = k {
                    assert!(
                        !(k.mods == 0 && PREVIEW_BARE_KEYS.contains(&k.key)),
                        "{title} takes bare {} ({layout:?})",
                        k.key
                    );
                }
            }
        }
        // ⌘G stays Ingest… in Photo Mechanic's layout.
        assert_eq!(key(ShortcutLayout::PhotoMechanic, "Ingest…"), a(CMD, "G"));
    }

    #[test]
    fn tab_is_free_for_the_sidebar_in_both_layouts() {
        for layout in [ShortcutLayout::PhotoMechanic, ShortcutLayout::Classic] {
            for (title, k) in menu_keys(layout) {
                if let Some(k) = k {
                    assert!(
                        !(k.mods == 0 && k.key == SIDEBAR_BARE_KEY),
                        "{title} takes bare Tab ({layout:?})"
                    );
                }
            }
            assert_eq!(key(layout, "Show Sidebar"), a(CMD | CTRL, "S"));
        }
    }

    #[test]
    fn g_closes_preview_only_in_preview_outside_crop() {
        assert!(g_closes_preview(true, false, false));
        assert!(!g_closes_preview(false, false, false));
        assert!(!g_closes_preview(true, true, false));
        assert!(!g_closes_preview(true, false, true));
    }

    #[test]
    fn titles_are_unique() {
        let keys = menu_keys(ShortcutLayout::PhotoMechanic);
        for (i, (t, _)) in keys.iter().enumerate() {
            assert!(keys[i + 1..].iter().all(|(u, _)| u != t), "{t} twice");
        }
    }

    #[test]
    fn photo_mechanic_keys_where_they_belong() {
        let pm = ShortcutLayout::PhotoMechanic;
        assert_eq!(key(pm, "Toggle Tag"), a(0, "T"));
        assert_eq!(key(pm, "Class 1"), a(CMD, "1"));
        assert_eq!(key(pm, "Rating 5"), a(CTRL, "5"));
        assert_eq!(key(pm, "Ingest…"), a(CMD, "G"));
        assert_eq!(key(pm, "Compare Selected"), a(0, "V"));
        assert_eq!(key(pm, "Add Folder to Favorites"), None);
        assert_eq!(key(pm, "Reveal in Finder"), None);
        assert_eq!(key(pm, "Select Tagged"), a(CMD, "T"));
        assert_eq!(key(pm, "Select Cropped"), a(CMD | SHIFT, "C"));
        assert_eq!(key(pm, "Select Class 3"), a(CMD | CTRL, "3"));
        assert_eq!(key(pm, "Select Any Rating"), a(CMD | ALT, "6"));
        assert_eq!(key(pm, "Copy Photos…"), a(CMD, "Y"));
        let classic = ShortcutLayout::Classic;
        assert_eq!(key(classic, "Select Tagged"), None);
        assert_eq!(key(classic, "Select Class 3"), None);
        assert_eq!(key(classic, "Copy Photos…"), a(CMD, "Y"));
    }
}
