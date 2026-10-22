//! The thin native layer. Everything egui cannot do itself lives here:
//! the menu bar and system accessibility preferences. Nothing in `ui/`
//! or the core crates imports from this module except through `Platform`.

use crate::commands::Command;

pub mod keymap;
#[cfg(target_os = "macos")]
mod macos;

pub struct Platform {
    #[cfg(target_os = "macos")]
    menu: Option<macos::MenuBar>,
    #[cfg(target_os = "macos")]
    status: Option<macos::StatusItem>,
}

impl Platform {
    /// The menu bar is built for the saved shortcut layout from the start.
    pub fn new(layout: crate::prefs::ShortcutLayout) -> Self {
        #[cfg(not(target_os = "macos"))]
        let _ = layout;
        Platform {
            #[cfg(target_os = "macos")]
            menu: macos::MenuBar::install(layout)
                .map_err(|e| log::warn!("native menu unavailable: {e}"))
                .ok(),
            #[cfg(target_os = "macos")]
            status: None,
        }
    }

    /// True when the native menu bar owns the ⌘ shortcuts, so the egui key
    /// handler must leave command-modified keys alone.
    pub fn has_menu(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.menu.is_some()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Finish native setup that must wait for the app to be fully launched.
    pub fn on_first_frame(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(m) = &mut self.menu {
            m.install_if_needed();
        }
        #[cfg(target_os = "macos")]
        if self.status.is_none() {
            self.status = macos::StatusItem::install()
                .map_err(|e| log::warn!("menu bar extra unavailable: {e}"))
                .ok();
        }
    }

    /// Menu bar extra title (empty hides it) and the Live Ingest check.
    pub fn status_item(&self, title: &str, live: bool) {
        #[cfg(target_os = "macos")]
        if let Some(s) = &self.status {
            s.set_title(title);
            s.set_live(live);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (title, live);
        }
    }

    pub fn drain_menu(&self) -> Vec<Command> {
        #[cfg(target_os = "macos")]
        {
            self.menu.as_ref().map(|m| m.drain()).unwrap_or_default()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }

    /// Reflect the state of toggles in the menu (checkmarks).
    pub fn sync_toggles(&self, sidebar: bool, inspector: bool, filmstrip: bool, advance: bool) {
        #[cfg(target_os = "macos")]
        if let Some(m) = &self.menu {
            m.sync_toggles(sidebar, inspector, filmstrip, advance);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (sidebar, inspector, filmstrip, advance);
    }

    /// Rebuild the menu bar when the shortcut layout changed. True when it
    /// did, so the caller can put the checkmarks and class names back.
    pub fn set_shortcuts(&mut self, layout: crate::prefs::ShortcutLayout) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(old) = &self.menu {
            if old.layout == layout {
                return false;
            }
            let installed = old.installed();
            match macos::MenuBar::install(layout) {
                Ok(mut m) => {
                    if installed {
                        m.install_if_needed();
                    }
                    self.menu = Some(m);
                    return true;
                }
                Err(e) => log::warn!("menu rebuild failed: {e}"),
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = layout;
        false
    }

    pub fn set_class_names(&self, names: &[String; 8]) {
        #[cfg(target_os = "macos")]
        if let Some(m) = &self.menu {
            m.set_class_names(names);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = names;
    }

    /// System "Reduce Motion", read once at startup.
    pub fn reduce_motion() -> bool {
        #[cfg(target_os = "macos")]
        {
            macos::reduce_motion()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Unmount the volume that holds `path` (macOS: `diskutil unmount`),
    /// off the UI thread; only volumes under /Volumes qualify. Returns
    /// the volume root that was asked to unmount, if any.
    pub fn eject(path: &std::path::Path) -> Option<std::path::PathBuf> {
        let mut comps = path.components();
        let root: std::path::PathBuf = match (comps.next(), comps.next(), comps.next()) {
            (Some(std::path::Component::RootDir), Some(v), Some(name))
                if v.as_os_str() == "Volumes" =>
            {
                std::path::PathBuf::from("/Volumes").join(name.as_os_str())
            }
            _ => return None,
        };
        #[cfg(target_os = "macos")]
        {
            let r = root.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("diskutil")
                    .arg("unmount")
                    .arg(&r)
                    .output();
            });
        }
        Some(root)
    }

    pub fn reveal(path: &std::path::Path) {
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = path;
    }
}

/// What a mounted volume is, for the icon and for hiding system volumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeKind {
    /// Has a DCIM folder: a memory card or a tethered camera.
    Camera,
    /// Removable or external disk (USB, Thunderbolt, SD without DCIM).
    External,
    /// The Mac's own disk.
    Internal,
    /// Recovery and other system volumes: never an ingest source.
    System,
}

impl Platform {
    /// Classify a volume root. DCIM wins outright (a camera over USB reports
    /// the same bus as an SSD, so the bus alone cannot tell them apart);
    /// otherwise macOS `diskutil info -plist` says whether the disk is
    /// internal. One process per new mount, cached by the caller.
    pub fn volume_kind(path: &std::path::Path) -> VolumeKind {
        if path.join("DCIM").is_dir() {
            return VolumeKind::Camera;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name == "recovery"
            || name.starts_with("com.apple")
            || name == "preboot"
            || name == "vm"
            || name == "update"
        {
            return VolumeKind::System;
        }
        #[cfg(target_os = "macos")]
        {
            if let Ok(out) = std::process::Command::new("/usr/sbin/diskutil")
                .args(["info", "-plist"])
                .arg(path)
                .output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                // <key>Internal</key> <true/> ... without pulling in a plist crate.
                if let Some(i) = text.find("<key>Internal</key>") {
                    let tail = &text[i..];
                    if tail
                        .trim_start_matches("<key>Internal</key>")
                        .trim_start()
                        .starts_with("<true/>")
                    {
                        return VolumeKind::Internal;
                    }
                }
            }
        }
        VolumeKind::External
    }
}

/// Keychain service name for delivery passwords (account = target name).
const SECRET_SERVICE: &str = "dev.mccal.Apature.delivery";

impl Platform {
    /// Read a delivery password from the macOS Keychain. Elsewhere: none.
    pub fn secret_get(account: &str) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            security_framework::passwords::get_generic_password(SECRET_SERVICE, account)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
            None
        }
    }

    /// Store (or replace) a delivery password in the macOS Keychain.
    pub fn secret_set(account: &str, secret: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            security_framework::passwords::set_generic_password(
                SECRET_SERVICE,
                account,
                secret.as_bytes(),
            )
            .map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (account, secret);
            Err("no keychain on this platform".into())
        }
    }

    pub fn secret_delete(account: &str) {
        #[cfg(target_os = "macos")]
        {
            let _ = security_framework::passwords::delete_generic_password(SECRET_SERVICE, account);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
        }
    }
}
