//! Native menu bar via `muda`, mapped onto `Command`s.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::commands::{Command, Workspace};
use crate::platform::keymap::{self, Accel};
use crate::prefs::ShortcutLayout;

pub struct MenuBar {
    menu: Menu,
    window: Submenu,
    help: Submenu,
    installed: bool,
    sidebar: CheckMenuItem,
    advance: CheckMenuItem,
    inspector: CheckMenuItem,
    filmstrip: CheckMenuItem,
    class_items: Vec<MenuItem>,
    pub layout: ShortcutLayout,
}

/// A table key as a muda accelerator; None for a key the menu cannot show.
fn to_muda(k: Accel) -> Option<Accelerator> {
    let mut m = Modifiers::empty();
    for (bit, md) in [
        (keymap::CMD, Modifiers::META),
        (keymap::SHIFT, Modifiers::SHIFT),
        (keymap::ALT, Modifiers::ALT),
        (keymap::CTRL, Modifiers::CONTROL),
    ] {
        if k.mods & bit != 0 {
            m |= md;
        }
    }
    let code = match k.key {
        "A" => Code::KeyA,
        "B" => Code::KeyB,
        "C" => Code::KeyC,
        "D" => Code::KeyD,
        "E" => Code::KeyE,
        "F" => Code::KeyF,
        "G" => Code::KeyG,
        "H" => Code::KeyH,
        "I" => Code::KeyI,
        "K" => Code::KeyK,
        "L" => Code::KeyL,
        "M" => Code::KeyM,
        "N" => Code::KeyN,
        "O" => Code::KeyO,
        "Q" => Code::KeyQ,
        "R" => Code::KeyR,
        "S" => Code::KeyS,
        "T" => Code::KeyT,
        "V" => Code::KeyV,
        "W" => Code::KeyW,
        "X" => Code::KeyX,
        "Z" => Code::KeyZ,
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "," => Code::Comma,
        "=" => Code::Equal,
        "-" => Code::Minus,
        "Backspace" => Code::Backspace,
        "F5" => Code::F5,
        _ => return None,
    };
    Some(Accelerator::new(m, code))
}

impl MenuBar {
    /// Build the menu for a layout. Every key comes from `keymap`, so a
    /// layout change rebuilds the menu rather than editing keys in place
    /// (muda cannot take a key away from an item on macOS).
    pub fn install(layout: ShortcutLayout) -> muda::Result<Self> {
        let pm = layout == ShortcutLayout::PhotoMechanic;
        let k = |title: &str| keymap::key(layout, title).and_then(to_muda);
        // An item whose table title is its menu title.
        let item =
            |cmd: Command, title: &str| MenuItem::with_id(cmd.menu_id(), title, true, k(title));
        let menu = Menu::new();

        let app = Submenu::new("Apature", true);
        app.append_items(&[
            &MenuItem::with_id(Command::OpenAbout.menu_id(), "About Apature", true, None),
            &MenuItem::with_id(
                Command::CheckUpdates.menu_id(),
                "Check for Updates…",
                true,
                None,
            ),
            &PredefinedMenuItem::separator(),
            &item(Command::OpenSettings, "Settings…"),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])?;

        // Photo Mechanic gives ⌘M to Rename and ⌘W to the tab, so Close
        // Window (⌘⇧W) and Minimize (no key) are Apature's own items there;
        // classic keeps the system ones.
        let close_window: Box<dyn muda::IsMenuItem> = if pm {
            Box::new(item(Command::CloseWindow, "Close Window"))
        } else {
            Box::new(PredefinedMenuItem::close_window(Some("Close Window")))
        };
        let minimize: Box<dyn muda::IsMenuItem> = if pm {
            Box::new(item(Command::MinimizeWindow, "Minimize"))
        } else {
            Box::new(PredefinedMenuItem::minimize(None))
        };

        let file = Submenu::new("File", true);
        file.append_items(&[
            &item(Command::PickFolder, "Open Folder in New Tab…"),
            &item(Command::PickFolder, "New Tab…"),
            &item(Command::CloseTab, "Close Tab"),
            &item(Command::AddFavorite, "Add Folder to Favorites"),
            &PredefinedMenuItem::separator(),
            &item(Command::LoadAssignment, "Load Assignment…"),
            &item(Command::LoadCodes, "Load Code Replacements…"),
            &PredefinedMenuItem::separator(),
            &item(Command::IngestStart, "Ingest…"),
            &item(Command::ExportStart, "Export…"),
            &PredefinedMenuItem::separator(),
            &item(Command::RenamePhotos, "Rename Photos…"),
            &item(Command::MoveToTrash, "Move to Trash"),
            &PredefinedMenuItem::separator(),
            &item(Command::RevealInFinder, "Reveal in Finder"),
            &PredefinedMenuItem::separator(),
            &item(Command::DeliverLast, "Send Last Export…"),
            &item(Command::DeliverResendFailed, "Resend Failed Files"),
            &item(Command::LiveToggle, "Live Ingest"),
            &PredefinedMenuItem::separator(),
            close_window.as_ref(),
        ])?;

        let edit = Submenu::new("Edit", true);
        edit.append_items(&[
            &item(Command::Undo, "Undo"),
            &item(Command::Redo, "Redo"),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::separator(),
            &item(Command::CopyMetadata, "Copy Metadata"),
            &item(Command::PasteMetadata, "Paste Metadata"),
            &PredefinedMenuItem::separator(),
            &item(Command::SelectAll, "Select All"),
            &item(Command::SelectNone, "Deselect All"),
        ])?;

        let check = |cmd: Command, title: &str, on: bool| {
            CheckMenuItem::with_id(cmd.menu_id(), title, true, on, k(title))
        };
        let sidebar = check(Command::ToggleSidebar, "Show Sidebar", true);
        let inspector = check(Command::ToggleInspector, "Show Inspector", true);
        let filmstrip = check(Command::ToggleFilmstrip, "Show Filmstrip", false);
        let sort = Submenu::new("Sort By", true);
        for so in crate::filter::Sort::ALL {
            sort.append(&item(Command::SetSort(so), &capitalize(so.label())))?;
        }
        sort.append_items(&[
            &PredefinedMenuItem::separator(),
            &item(Command::ToggleSortReversed, "Reverse Order"),
        ])?;
        let view = Submenu::new("View", true);
        view.append_items(&[
            &item(Command::SetWorkspace(Workspace::Sheet), "Contact Sheet"),
            &item(Command::SetWorkspace(Workspace::Preview), "Preview"),
            &item(Command::SetWorkspace(Workspace::Caption), "Caption"),
            &item(Command::SetWorkspace(Workspace::Ingest), "Ingest"),
            &item(Command::SetWorkspace(Workspace::Export), "Export"),
            &PredefinedMenuItem::separator(),
            &sidebar,
            &inspector,
            &filmstrip,
            &item(Command::RefreshFolder, "Refresh Folder"),
            &PredefinedMenuItem::separator(),
            &sort,
            &item(Command::ClearFilter(None), "Clear Filters"),
            &PredefinedMenuItem::separator(),
            &item(Command::ZoomIn, "Larger Thumbnails"),
            &item(Command::ZoomOut, "Smaller Thumbnails"),
            &PredefinedMenuItem::separator(),
            &item(Command::ToggleZoom, "Zoom 1:1 / Fit"),
            &item(Command::ToggleCompare, "Compare Selected"),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::fullscreen(None),
        ])?;

        let class = Submenu::new("Color Class", true);
        let names = apature_core::ColorClassScheme::default().names;
        let mut class_items = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let it = MenuItem::with_id(
                Command::SetClass(i as u8 + 1).menu_id(),
                format!("{} {n}", i + 1),
                true,
                k(keymap::CLASS[i + 1]),
            );
            class.append(&it)?;
            class_items.push(it);
        }
        class.append_items(&[
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                Command::SetClass(0).menu_id(),
                "None",
                true,
                k(keymap::CLASS[0]),
            ),
        ])?;
        let rating = Submenu::new("Rating", true);
        for r in (1..=5).rev() {
            rating.append(&MenuItem::with_id(
                Command::SetRating(r).menu_id(),
                format!(
                    "{} {}",
                    "★".repeat(r as usize),
                    if r == 1 { "Star" } else { "Stars" }
                ),
                true,
                k(keymap::RATING[r as usize]),
            ))?;
        }
        rating.append_items(&[
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                Command::SetRating(0).menu_id(),
                "Unrated",
                true,
                k(keymap::RATING[0]),
            ),
            &item(Command::SetRating(-1), "Reject"),
        ])?;
        let image = Submenu::new("Image", true);
        let advance = check(
            Command::ToggleAdvanceAfterMark,
            "Advance After Marking",
            true,
        );
        image.append_items(&[
            &class,
            &rating,
            &PredefinedMenuItem::separator(),
            &item(Command::ToggleTag, "Toggle Tag"),
            &PredefinedMenuItem::separator(),
            &item(Command::Rotate(false), "Rotate Left"),
            &item(Command::Rotate(true), "Rotate Right"),
            &PredefinedMenuItem::separator(),
            &item(
                Command::ApplyStationery,
                "Apply Assignment Pad to Selection",
            ),
            &item(Command::SaveCaption, "Save Caption"),
            &PredefinedMenuItem::separator(),
            &item(Command::CropToggle, "Crop…"),
            &item(Command::CropClear, "Remove Crop"),
            &item(Command::ToggleLoupe, "Loupe"),
            &item(Command::CyclePreviewBoost, "Brighten Preview (view only)"),
            &item(Command::ToggleStack, "Expand or Collapse Burst"),
            &PredefinedMenuItem::separator(),
            &advance,
        ])?;

        let window = Submenu::new("Window", true);
        window.append_items(&[
            minimize.as_ref(),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &item(Command::NextTab, "Show Next Tab"),
            &item(Command::PrevTab, "Show Previous Tab"),
        ])?;
        let help = Submenu::new("Help", true);
        help.append_items(&[
            &item(Command::OpenShortcuts, "Keyboard Shortcuts"),
            &item(Command::RevealLog, "Reveal Log in Finder"),
            &item(Command::OpenRepo, "Apature on GitHub"),
        ])?;

        menu.append_items(&[&app, &file, &edit, &view, &image, &window, &help])?;
        Ok(MenuBar {
            menu,
            window,
            help,
            installed: false,
            sidebar,
            inspector,
            filmstrip,
            advance,
            class_items,
            layout,
        })
    }

    /// Attach to NSApp. Must run after the application has finished launching,
    /// or winit's default menu replaces it; the app calls this on its first frame.
    pub fn install_if_needed(&mut self) {
        if self.installed {
            return;
        }
        self.menu.init_for_nsapp();
        self.window.set_as_windows_menu_for_nsapp();
        self.help.set_as_help_menu_for_nsapp();
        self.installed = true;
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    pub fn set_class_names(&self, names: &[String; 8]) {
        for (i, it) in self.class_items.iter().enumerate() {
            it.set_text(format!("{} {}", i + 1, names[i]));
        }
    }

    pub fn drain(&self) -> Vec<Command> {
        let mut out = Vec::new();
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if let Some(c) = Command::from_menu_id(ev.id().as_ref()) {
                out.push(c);
            }
        }
        out
    }

    pub fn sync_toggles(&self, sidebar: bool, inspector: bool, filmstrip: bool, advance: bool) {
        self.sidebar.set_checked(sidebar);
        self.inspector.set_checked(inspector);
        self.filmstrip.set_checked(filmstrip);
        self.advance.set_checked(advance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_key_has_a_menu_key() {
        for layout in [ShortcutLayout::PhotoMechanic, ShortcutLayout::Classic] {
            for (title, k) in keymap::menu_keys(layout) {
                if let Some(k) = k {
                    assert!(to_muda(k).is_some(), "{title}: {k:?}");
                }
            }
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `defaults read com.apple.universalaccess reduceMotion`, once at startup.
pub fn reduce_motion() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

/// The menu bar extra at the right of the Mac's menu bar (HIG: a template
/// image, status in a short title, a short menu). Shows what Apature is
/// doing while its window is behind something else: an ingest, a delivery,
/// Live Ingest watching.
pub struct StatusItem {
    tray: tray_icon::TrayIcon,
    live: CheckMenuItem,
    last_title: std::cell::RefCell<String>,
}

impl StatusItem {
    pub fn install() -> Result<Self, String> {
        let menu = Menu::new();
        let open = MenuItem::with_id("app.show", "Open Apature", true, None);
        let live = CheckMenuItem::with_id(
            Command::LiveToggle.menu_id(),
            "Live Ingest",
            true,
            false,
            None,
        );
        menu.append_items(&[
            &open,
            &PredefinedMenuItem::separator(),
            &live,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some("Quit Apature")),
        ])
        .map_err(|e| e.to_string())?;
        let tray = tray_icon::TrayIconBuilder::new()
            .with_icon_templated(template_icon())
            .with_menu(Box::new(menu))
            .with_tooltip("Apature")
            .build()
            .map_err(|e| e.to_string())?;
        Ok(StatusItem {
            tray,
            live,
            last_title: std::cell::RefCell::new(String::new()),
        })
    }

    /// Short text beside the icon; empty hides it. Only touched on change.
    pub fn set_title(&self, text: &str) {
        if *self.last_title.borrow() != text {
            self.tray.set_title(if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            });
            *self.last_title.borrow_mut() = text.to_string();
        }
    }

    pub fn set_live(&self, on: bool) {
        self.live.set_checked(on);
    }
}

/// An 18 pt aperture ring drawn in alpha only, so macOS tints it for the
/// menu bar's light or dark appearance (a template image).
fn template_icon() -> tray_icon::Icon {
    let n = 36u32; // 18 pt at 2x
    let mut rgba = vec![0u8; (n * n * 4) as usize];
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let r = (dx * dx + dy * dy).sqrt();
            // Ring from radius 10 to 15, with a gap at the top right like a shutter blade edge.
            let ang = dy.atan2(dx);
            let in_ring = (10.0..=15.0).contains(&r) && !((-1.2..-0.5).contains(&ang));
            let dot = r <= 4.0;
            let a = if in_ring || dot { 255 } else { 0 };
            let i = ((y * n + x) * 4) as usize;
            rgba[i] = 0;
            rgba[i + 1] = 0;
            rgba[i + 2] = 0;
            rgba[i + 3] = a;
        }
    }
    tray_icon::Icon::from_rgba(rgba, n, n).expect("icon")
}
