//! Native menu bar via `muda`, mapped onto `Command`s.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::commands::{Command, Workspace};
use crate::prefs::{NumberKeys, ShortcutLayout};

/// Digit key codes, index = digit.
const DIGITS: [Code; 9] = [
    Code::Digit0,
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
];

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
    class_none: MenuItem,
    /// Rating items with their star count (0 = Unrated).
    rating_items: Vec<(i8, MenuItem)>,
    /// Items whose key differs by layout: (item, Photo Mechanic, classic).
    keyed: Vec<(MenuItem, Option<Accelerator>, Option<Accelerator>)>,
    applied: std::cell::Cell<Option<(ShortcutLayout, NumberKeys)>>,
}

fn item(cmd: Command, text: &str, acc: Option<Accelerator>) -> MenuItem {
    MenuItem::with_id(cmd.menu_id(), text, true, acc)
}

fn acc(mods: Modifiers, code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(mods, code))
}

type Keyed = Vec<(MenuItem, Option<Accelerator>, Option<Accelerator>)>;

/// An item whose key depends on the shortcut layout. It starts with the
/// classic key; `set_shortcuts` picks the one in use.
fn keyed(
    list: &mut Keyed,
    cmd: Command,
    text: &str,
    pm: Option<Accelerator>,
    classic: Option<Accelerator>,
) -> MenuItem {
    let it = item(cmd, text, classic);
    list.push((it.clone(), pm, classic));
    it
}

impl MenuBar {
    pub fn install() -> muda::Result<Self> {
        let menu = Menu::new();
        let cmd = Modifiers::META;
        let cmd_shift = Modifiers::META | Modifiers::SHIFT;
        let cmd_alt = Modifiers::META | Modifiers::ALT;
        let ctrl_cmd = Modifiers::META | Modifiers::CONTROL;
        let bare = Modifiers::empty();
        let mut k: Keyed = Vec::new();

        let app = Submenu::new("Apature", true);
        app.append_items(&[
            &item(Command::OpenAbout, "About Apature", None),
            &item(Command::CheckUpdates, "Check for Updates…", None),
            &PredefinedMenuItem::separator(),
            &item(Command::OpenSettings, "Settings…", acc(cmd, Code::Comma)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])?;

        let file = Submenu::new("File", true);
        file.append_items(&[
            &item(
                Command::PickFolder,
                "Open Folder in New Tab…",
                acc(cmd, Code::KeyO),
            ),
            &keyed(
                &mut k,
                Command::PickFolder,
                "New Tab…",
                acc(cmd, Code::KeyN),
                acc(cmd, Code::KeyT),
            ),
            &item(Command::CloseTab, "Close Tab", acc(cmd, Code::KeyW)),
            &item(Command::AddFavorite, "Add Folder to Favorites", None),
            &PredefinedMenuItem::separator(),
            &item(Command::LoadAssignment, "Load Assignment…", None),
            &item(Command::LoadCodes, "Load Code Replacements…", None),
            &PredefinedMenuItem::separator(),
            &keyed(
                &mut k,
                Command::IngestStart,
                "Ingest…",
                acc(cmd, Code::KeyG),
                acc(cmd_shift, Code::KeyI),
            ),
            &keyed(
                &mut k,
                Command::ExportStart,
                "Export…",
                acc(ctrl_cmd, Code::KeyX),
                acc(cmd, Code::KeyE),
            ),
            &PredefinedMenuItem::separator(),
            &keyed(
                &mut k,
                Command::RenamePhotos,
                "Rename Photos…",
                acc(cmd, Code::KeyM),
                acc(cmd_shift, Code::KeyR),
            ),
            &item(
                Command::MoveToTrash,
                "Move to Trash",
                acc(cmd, Code::Backspace),
            ),
            &PredefinedMenuItem::separator(),
            &keyed(
                &mut k,
                Command::RevealInFinder,
                "Reveal in Finder",
                None,
                acc(cmd, Code::KeyR),
            ),
            &PredefinedMenuItem::separator(),
            &item(Command::DeliverLast, "Send Last Export…", None),
            &item(Command::DeliverResendFailed, "Resend Failed Files", None),
            &keyed(
                &mut k,
                Command::LiveToggle,
                "Live Ingest",
                acc(cmd_shift, Code::KeyG),
                None,
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Close Window")),
        ])?;

        let edit = Submenu::new("Edit", true);
        edit.append_items(&[
            &item(Command::Undo, "Undo", acc(cmd, Code::KeyZ)),
            &item(Command::Redo, "Redo", acc(cmd_shift, Code::KeyZ)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::separator(),
            &item(
                Command::CopyMetadata,
                "Copy Metadata",
                acc(cmd_alt, Code::KeyC),
            ),
            &item(
                Command::PasteMetadata,
                "Paste Metadata",
                acc(cmd_alt, Code::KeyV),
            ),
            &PredefinedMenuItem::separator(),
            &item(Command::SelectAll, "Select All", acc(cmd, Code::KeyA)),
            &keyed(
                &mut k,
                Command::SelectNone,
                "Deselect All",
                acc(cmd, Code::KeyD),
                acc(cmd_shift, Code::KeyA),
            ),
        ])?;

        let sidebar = CheckMenuItem::with_id(
            Command::ToggleSidebar.menu_id(),
            "Show Sidebar",
            true,
            true,
            acc(ctrl_cmd, Code::KeyS),
        );
        let inspector = CheckMenuItem::with_id(
            Command::ToggleInspector.menu_id(),
            "Show Inspector",
            true,
            true,
            acc(cmd_alt, Code::KeyI),
        );
        let filmstrip = CheckMenuItem::with_id(
            Command::ToggleFilmstrip.menu_id(),
            "Show Filmstrip",
            true,
            false,
            acc(Modifiers::SHIFT, Code::KeyF),
        );
        let sort = Submenu::new("Sort By", true);
        for so in crate::filter::Sort::ALL {
            sort.append(&item(Command::SetSort(so), &capitalize(so.label()), None))?;
        }
        sort.append_items(&[
            &PredefinedMenuItem::separator(),
            &item(Command::ToggleSortReversed, "Reverse Order", None),
        ])?;
        let view = Submenu::new("View", true);
        view.append_items(&[
            &item(
                Command::SetWorkspace(Workspace::Sheet),
                "Contact Sheet",
                acc(cmd_shift, Code::Digit1),
            ),
            &keyed(
                &mut k,
                Command::SetWorkspace(Workspace::Preview),
                "Preview",
                acc(cmd, Code::KeyR),
                acc(cmd_shift, Code::Digit2),
            ),
            &item(
                Command::SetWorkspace(Workspace::Caption),
                "Caption",
                acc(cmd_shift, Code::Digit3),
            ),
            &item(
                Command::SetWorkspace(Workspace::Ingest),
                "Ingest",
                acc(cmd_shift, Code::Digit4),
            ),
            &item(
                Command::SetWorkspace(Workspace::Export),
                "Export",
                acc(cmd_shift, Code::Digit5),
            ),
            &PredefinedMenuItem::separator(),
            &sidebar,
            &inspector,
            &filmstrip,
            &keyed(
                &mut k,
                Command::RefreshFolder,
                "Refresh Folder",
                acc(bare, Code::F5),
                acc(Modifiers::META, Code::KeyR),
            ),
            &PredefinedMenuItem::separator(),
            &sort,
            &item(
                Command::ClearFilter(None),
                "Clear Filters",
                acc(cmd_alt, Code::KeyF),
            ),
            &PredefinedMenuItem::separator(),
            &item(Command::ZoomIn, "Larger Thumbnails", acc(cmd, Code::Equal)),
            &item(
                Command::ZoomOut,
                "Smaller Thumbnails",
                acc(cmd, Code::Minus),
            ),
            &PredefinedMenuItem::separator(),
            &item(
                Command::ToggleZoom,
                "Zoom 1:1 / Fit",
                Some(Accelerator::new(Modifiers::empty(), Code::KeyZ)),
            ),
            &keyed(
                &mut k,
                Command::ToggleCompare,
                "Compare Selected",
                acc(bare, Code::KeyV),
                acc(bare, Code::KeyC),
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::fullscreen(None),
        ])?;

        let class = Submenu::new("Color Class", true);
        let names = apature_core::ColorClassScheme::default().names;
        let mut class_items = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let it = item(
                Command::SetClass(i as u8 + 1),
                &format!("{} {n}", i + 1),
                acc(bare, DIGITS[i + 1]),
            );
            class.append(&it)?;
            class_items.push(it);
        }
        let class_none = item(Command::SetClass(0), "None", acc(bare, Code::Digit0));
        class.append_items(&[&PredefinedMenuItem::separator(), &class_none])?;
        let rating = Submenu::new("Rating", true);
        let mut rating_items = Vec::new();
        for r in (1..=5).rev() {
            let it = item(
                Command::SetRating(r),
                &format!(
                    "{} {}",
                    "★".repeat(r as usize),
                    if r == 1 { "Star" } else { "Stars" }
                ),
                acc(cmd, DIGITS[r as usize]),
            );
            rating.append(&it)?;
            rating_items.push((r, it));
        }
        let unrated = item(Command::SetRating(0), "Unrated", acc(cmd, Code::Digit0));
        rating_items.push((0, unrated.clone()));
        rating.append_items(&[
            &PredefinedMenuItem::separator(),
            &unrated,
            &item(
                Command::SetRating(-1),
                "Reject",
                Some(Accelerator::new(Modifiers::empty(), Code::KeyX)),
            ),
        ])?;
        let image = Submenu::new("Image", true);
        let advance = CheckMenuItem::with_id(
            Command::ToggleAdvanceAfterMark.menu_id(),
            "Advance After Marking",
            true,
            true,
            None,
        );
        image.append_items(&[
            &class,
            &rating,
            &PredefinedMenuItem::separator(),
            &item(
                Command::ToggleTag,
                "Toggle Tag",
                Some(Accelerator::new(Modifiers::empty(), Code::KeyT)),
            ),
            &PredefinedMenuItem::separator(),
            &item(
                Command::Rotate(false),
                "Rotate Left",
                acc(cmd, Code::BracketLeft),
            ),
            &item(
                Command::Rotate(true),
                "Rotate Right",
                acc(cmd, Code::BracketRight),
            ),
            &PredefinedMenuItem::separator(),
            &keyed(
                &mut k,
                Command::ApplyStationery,
                "Apply Assignment Pad to Selection",
                acc(cmd_shift, Code::KeyI),
                None,
            ),
            &item(Command::SaveCaption, "Save Caption", None),
            &PredefinedMenuItem::separator(),
            &keyed(
                &mut k,
                Command::CropToggle,
                "Crop…",
                acc(bare, Code::KeyC),
                acc(bare, Code::KeyK),
            ),
            &keyed(
                &mut k,
                Command::CropClear,
                "Remove Crop",
                acc(Modifiers::ALT, Code::KeyC),
                None,
            ),
            &item(
                Command::ToggleLoupe,
                "Loupe",
                Some(Accelerator::new(Modifiers::empty(), Code::KeyL)),
            ),
            &item(
                Command::CyclePreviewBoost,
                "Brighten Preview (view only)",
                Some(Accelerator::new(Modifiers::empty(), Code::KeyB)),
            ),
            &item(
                Command::ToggleStack,
                "Expand or Collapse Burst",
                Some(Accelerator::new(Modifiers::empty(), Code::KeyS)),
            ),
            &PredefinedMenuItem::separator(),
            &advance,
        ])?;

        let window = Submenu::new("Window", true);
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &item(
                Command::NextTab,
                "Show Next Tab",
                acc(cmd_shift, Code::BracketRight),
            ),
            &item(
                Command::PrevTab,
                "Show Previous Tab",
                acc(cmd_shift, Code::BracketLeft),
            ),
        ])?;
        let help = Submenu::new("Help", true);
        help.append_items(&[
            &item(Command::OpenShortcuts, "Keyboard Shortcuts", None),
            &item(Command::RevealLog, "Reveal Log in Finder", None),
            &item(Command::OpenRepo, "Apature on GitHub", None),
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
            class_none,
            rating_items,
            keyed: k,
            applied: std::cell::Cell::new(None),
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

    pub fn set_class_names(&self, names: &[String; 8]) {
        for (i, it) in self.class_items.iter().enumerate() {
            it.set_text(format!("{} {}", i + 1, names[i]));
        }
    }

    /// Key equivalents for a layout. Photo Mechanic: ⌘ + digit sets the
    /// colour class and ⌃ + digit the rating, and the bare digits are left to
    /// the egui key handler (Number keys setting). Classic: the bare digits do
    /// what the Number keys setting says and ⌘ + digit does the other.
    pub fn set_shortcuts(&self, layout: ShortcutLayout, numbers: NumberKeys) {
        if self.applied.get() == Some((layout, numbers)) {
            return;
        }
        self.applied.set(Some((layout, numbers)));
        let pm = layout == ShortcutLayout::PhotoMechanic;
        for (it, p, c) in &self.keyed {
            let _ = it.set_accelerator(if pm { *p } else { *c });
        }
        let (class_mods, rating_mods) = match (pm, numbers) {
            (true, _) => (Modifiers::META, Modifiers::CONTROL),
            (false, NumberKeys::ColorClass) => (Modifiers::empty(), Modifiers::META),
            (false, NumberKeys::Rating) => (Modifiers::META, Modifiers::empty()),
        };
        for (i, it) in self.class_items.iter().enumerate() {
            let _ = it.set_accelerator(acc(class_mods, DIGITS[i + 1]));
        }
        let _ = self.class_none.set_accelerator(acc(class_mods, DIGITS[0]));
        for (r, it) in &self.rating_items {
            let _ = it.set_accelerator(acc(rating_mods, DIGITS[*r as usize]));
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
