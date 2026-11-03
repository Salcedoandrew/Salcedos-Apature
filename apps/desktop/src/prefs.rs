//! Per-user preferences and panel memory, stored as JSON under the app's
//! Application Support directory. Everything here is a convenience; losing
//! the file loses nothing photographic.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current saved-format version of prefs.json.
pub const PREFS_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Dark,
    Light,
}

/// The accent colour, in a soft palette that stays readable on both
/// appearances. Each name maps to a pair (dark appearance, light appearance)
/// in `ui::tokens::Theme::with_accent`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Lavender,
    Sage,
    Sky,
    Peach,
    Rose,
    Mist,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Lavender,
        Accent::Sage,
        Accent::Sky,
        Accent::Peach,
        Accent::Rose,
        Accent::Mist,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Lavender => "Lavender",
            Accent::Sage => "Sage",
            Accent::Sky => "Sky",
            Accent::Peach => "Peach",
            Accent::Rose => "Rose",
            Accent::Mist => "Mist",
        }
    }
}

/// Export workspace defaults (the desk's delivery spec).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportPrefs {
    /// Long side in pixels, 0 = full size.
    pub long_side: u32,
    pub quality: u8,
    pub dest: Option<PathBuf>,
    /// Naming template; empty keeps the source names.
    pub rename: String,
    pub keep_exif: bool,
    pub write_iim: bool,
    pub open_after: bool,
    /// Render RAW-only photos with the system RAW engine (macOS) instead of the preview.
    pub decode_raw: bool,
    /// Write JPEG XL instead of JPEG (lossless transcode of the JPEG via cjxl).
    pub jxl: bool,
}

impl Default for ExportPrefs {
    fn default() -> Self {
        ExportPrefs {
            long_side: 2048,
            quality: 90,
            dest: None,
            rename: String::new(),
            keep_exif: true,
            write_iim: true,
            open_after: true,
            decode_raw: false,
            jxl: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Prefs {
    /// Saved-format version (PREFS_VERSION). A missing key is version 0, the
    /// unversioned files written before 2026-10-20. Bumped only when a field
    /// is renamed or its meaning changes; new optional fields need no bump.
    pub version: u32,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub inspector_visible: bool,
    pub inspector_width: f32,
    pub filmstrip_visible: bool,
    pub cell: f32,
    pub favorites: Vec<PathBuf>,
    pub recent: Vec<PathBuf>,
    pub sort: crate::filter::Sort,
    pub sort_reversed: bool,
    pub advance_after_mark: bool,
    /// Also write marks and captions inside JPEG files (Photo Mechanic's
    /// embed habit). Off: JPEGs are never rewritten; marks go to a sidecar.
    pub embed_jpeg_marks: bool,
    /// Group frames shot within a second of the previous one behind one
    /// thumbnail (bursts); S expands or collapses the focused stack.
    pub stack_bursts: bool,
    /// Preview opens in its own window (Photo Mechanic style) instead of
    /// replacing the sheet; Enter and Esc still toggle it.
    pub preview_in_window: bool,
    /// What the bare 1–8 keys set: colour class (Photo Mechanic) or star rating.
    pub number_keys: NumberKeys,
    /// Photo Mechanic's macOS keys (the default, also for prefs saved before
    /// this existed) or Apature's original set.
    pub shortcut_layout: ShortcutLayout,
    pub assignment_file: Option<PathBuf>,
    pub codes_file: Option<PathBuf>,
    pub ingest: Option<apature_ingest::Options>,
    pub scheme: apature_core::ColorClassScheme,
    pub appearance: Appearance,
    /// Accent colour; `Accent::Lavender` is the Apature default.
    #[serde(default)]
    pub accent: Accent,
    pub open_tabs: Vec<PathBuf>,
    pub active_tab: usize,
    pub code_delimiter: char,
    /// Expand \code\ pairs while typing captions (off = type delimiters literally).
    pub codes_enabled: bool,
    pub open_after_ingest: bool,
    /// When a volume with a DCIM folder mounts, switch to Ingest with it as the source.
    pub open_ingest_on_card: bool,
    /// `{seqn}` keeps counting across ingests with the same headline
    /// (Photo Mechanic's persistent sequence variable, one counter per slug).
    pub seq_continue: bool,
    /// Next sequence number per headline key (lower-cased, trimmed; "" = none).
    pub seq_counters: BTreeMap<String, u32>,
    /// Recent values per caption field (headline, credit, city…), newest first,
    /// offered from the ▾ next to the field like Photo Mechanic's autocomplete.
    pub caption_history: BTreeMap<String, Vec<String>>,
    /// Accessibility: whole-UI scale (text and controls), 1.0 = default.
    pub ui_scale: f32,
    /// Accessibility: follow the system Reduce Motion setting, or force it.
    pub motion: Motion,
    /// Accessibility: stronger text and borders.
    pub high_contrast: bool,
    /// Show scan, frame and thumbnail timings at the right of the status bar.
    pub show_perf_stats: bool,
    /// Ask GitHub Releases once a day whether a newer Apature exists.
    pub check_updates: bool,
    /// Include pre-releases (betas) in that check.
    pub beta_updates: bool,
    /// Unix seconds of the last check.
    pub last_update_check: u64,
    /// A version the person chose to skip.
    pub skipped_update: Option<String>,
    pub export: ExportPrefs,
    /// Export presets per outlet: size, quality, naming, format and the
    /// delivery target in one chip.
    pub export_presets: Vec<ExportPreset>,
    /// The preset last applied (by name); cleared when the values drift.
    pub export_preset: Option<String>,
    /// Delivery targets (FTP, FTPS, SFTP). Passwords live in the Keychain.
    pub delivery: Vec<apature_deliver::Target>,
    /// Send every finished export to this target (by name) automatically.
    pub send_after_export: Option<String>,
    /// Last template used in Rename Photos.
    pub rename_template: String,
    /// Copy Photos… choices: destination, naming, partners, sidecars,
    /// collisions, date subfolders.
    pub copy: apature_ingest::CopyOptions,
    /// Extract JPEG Previews from RAW Photos… checkboxes.
    pub extract: apature_export::extract::ExtractOptions,
    /// Eject the card after an ingest where every file verified.
    pub eject_after_ingest: bool,
    /// Folders open in the sidebar's folder tree.
    pub tree_expanded: Vec<std::path::PathBuf>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            version: PREFS_VERSION,
            sidebar_visible: true,
            sidebar_width: 240.0,
            inspector_visible: true,
            inspector_width: 320.0,
            filmstrip_visible: false,
            cell: 180.0,
            favorites: Vec::new(),
            recent: Vec::new(),
            sort: crate::filter::Sort::Name,
            sort_reversed: false,
            advance_after_mark: true,
            embed_jpeg_marks: false,
            stack_bursts: false,
            preview_in_window: false,
            number_keys: NumberKeys::Rating,
            shortcut_layout: ShortcutLayout::PhotoMechanic,
            assignment_file: None,
            codes_file: None,
            ingest: None,
            scheme: apature_core::ColorClassScheme::default(),
            appearance: Appearance::System,
            open_tabs: Vec::new(),
            active_tab: 0,
            code_delimiter: '\\',
            codes_enabled: true,
            open_after_ingest: true,
            open_ingest_on_card: true,
            seq_continue: true,
            seq_counters: BTreeMap::new(),
            caption_history: BTreeMap::new(),
            ui_scale: 1.0,
            motion: Motion::System,
            high_contrast: false,
            accent: Accent::default(),
            show_perf_stats: false,
            check_updates: true,
            beta_updates: false,
            last_update_check: 0,
            skipped_update: None,
            export: ExportPrefs::default(),
            export_presets: Vec::new(),
            export_preset: None,
            delivery: Vec::new(),
            send_after_export: None,
            rename_template: String::new(),
            copy: apature_ingest::CopyOptions {
                seq_start: 1,
                partners: true,
                sidecars: true,
                ..Default::default()
            },
            eject_after_ingest: false,
            tree_expanded: Vec::new(),
            extract: Default::default(),
        }
    }
}

/// The app's ingest defaults: the owner's TribLive convention without the
/// section prefix, one folder per shoot and files `Headline001-MMDDYY.ext`.
pub fn default_ingest_options() -> apature_ingest::Options {
    apature_ingest::Options {
        folder_template: "{year4}/{month0} {monthname}/{mnn3} {day0} {headline}".into(),
        rename_template: "{headline}{seqn:3}-{month0}{day0}{yr2}".into(),
        ..Default::default()
    }
}

impl Prefs {
    /// Where marks for JPEG files go, from the "Write marks into JPEG files" setting.
    pub fn jpeg_marks(&self) -> apature_metadata::items::JpegMarks {
        if self.embed_jpeg_marks {
            apature_metadata::items::JpegMarks::Embed
        } else {
            apature_metadata::items::JpegMarks::Sidecar
        }
    }

    pub fn path() -> Option<PathBuf> {
        apature_core::project_dirs().map(|d| d.config_dir().join("prefs.json"))
    }

    /// Load, and say what happened: a damaged file is kept beside the live
    /// one as `prefs.damaged.json` and the app starts from defaults, visibly;
    /// a file from an older version is backed up once as
    /// `prefs.json.bak-v<old>` before the first save rewrites it.
    pub fn load_report() -> (Prefs, LoadOutcome) {
        let Some(p) = Self::path() else {
            return (Prefs::default(), LoadOutcome::Fresh);
        };
        let Ok(text) = fs::read_to_string(&p) else {
            return (Prefs::default(), LoadOutcome::Fresh);
        };
        match Self::parse(&text) {
            Ok((prefs, from)) => {
                if from < PREFS_VERSION {
                    let bak = p.with_extension(format!("json.bak-v{from}"));
                    if !bak.exists() {
                        let _ = fs::copy(&p, &bak);
                    }
                    (prefs, LoadOutcome::Migrated { from })
                } else if from > PREFS_VERSION {
                    // Written by a newer Apature: keep it untouched and run
                    // read-only on it, like a layout from the future.
                    (prefs, LoadOutcome::Newer { version: from })
                } else {
                    (prefs, LoadOutcome::Ok)
                }
            }
            Err(e) => {
                let damaged = p.with_file_name("prefs.damaged.json");
                let _ = fs::rename(&p, &damaged);
                (
                    Prefs::default(),
                    LoadOutcome::Damaged {
                        error: e,
                        kept: damaged,
                    },
                )
            }
        }
    }

    /// Parse the saved text and migrate it forward step by step; returns the
    /// version the file was written with. Migrations are pure functions of
    /// the JSON so each step has a test (`load_tests`).
    fn parse(text: &str) -> Result<(Prefs, u32), String> {
        let mut v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if !v.is_object() {
            return Err("prefs is not a JSON object".into());
        }
        let from = v.get("version").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let mut at = from;
        while at < PREFS_VERSION {
            migrate_step(&mut v, at);
            at += 1;
        }
        let mut prefs: Prefs = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if from <= PREFS_VERSION {
            prefs.version = PREFS_VERSION;
        }
        Ok((prefs, from))
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        if self.version > PREFS_VERSION {
            // A newer Apature wrote this file; this build does not rewrite it.
            log::warn!(
                "prefs.json is version {} (this build writes {PREFS_VERSION}); not saving",
                self.version
            );
            return;
        }
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(tmp, p);
            }
        }
    }

    pub fn note_recent(&mut self, folder: &Path) {
        self.recent.retain(|r| r != folder);
        self.recent.insert(0, folder.to_path_buf());
        self.recent.truncate(10);
    }
}

/// Motion preference: follow macOS Reduce Motion, or override it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Motion {
    #[default]
    System,
    Reduce,
    Full,
}

/// Key for the per-headline sequence counter.
pub fn seq_key(headline: Option<&str>) -> String {
    headline.unwrap_or("").trim().to_lowercase()
}

/// What the bare number keys do on the sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberKeys {
    /// 1–5 set the star rating, 0 clears it; ⌘1–⌘8 set the colour class.
    #[default]
    Rating,
    /// 1–8 set the colour class, 0 clears it; ⌘0–⌘5 set the rating (Photo Mechanic).
    ColorClass,
}

/// Which keys drive the menus: Photo Mechanic's macOS shortcuts or Apature's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShortcutLayout {
    /// ⌘1–⌘8 colour class, ⌃1–⌃5 rating, ⌘G ingest, ⌘M rename, ⌘D deselect, C crop.
    #[default]
    PhotoMechanic,
    /// Apature's original keys: ⌘⇧I ingest, ⌘⇧R rename, ⌘⇧A deselect, K crop.
    Classic,
}

impl Prefs {
    /// Remember a field value for the ▾ menu: newest first, no duplicates, 12 kept.
    pub fn remember_caption_value(&mut self, field: &str, value: &str) {
        let v = value.trim();
        if v.is_empty() {
            return;
        }
        let list = self.caption_history.entry(field.to_string()).or_default();
        list.retain(|x| x != v);
        list.insert(0, v.to_string());
        list.truncate(12);
    }
}

/// One outlet's export spec. `dest` is kept too, so a preset can point at a
/// fixed folder (an Eagle outbox) or leave the last folder alone (`None`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub name: String,
    pub export: ExportPrefs,
    pub send_to: Option<String>,
}

impl Prefs {
    /// True when the current export settings still match the named preset.
    pub fn export_matches_preset(&self, name: &str) -> bool {
        self.export_presets
            .iter()
            .find(|p| p.name == name)
            .is_some_and(|p| {
                let mut e = self.export.clone();
                if p.export.dest.is_none() {
                    e.dest = None;
                }
                e == p.export && self.send_after_export == p.send_to
            })
    }

    pub fn apply_export_preset(&mut self, name: &str) {
        if let Some(p) = self.export_presets.iter().find(|p| p.name == name).cloned() {
            let keep_dest = self.export.dest.clone();
            self.export = p.export.clone();
            if self.export.dest.is_none() {
                self.export.dest = keep_dest;
            }
            self.send_after_export = p.send_to.clone();
            self.export_preset = Some(name.to_string());
        }
    }
}

/// What `Prefs::load_report` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No file yet: defaults.
    Fresh,
    Ok,
    /// Read and migrated from an older format; the old file is kept as a backup.
    Migrated {
        from: u32,
    },
    /// Written by a newer Apature; loaded as far as this build understands.
    Newer {
        version: u32,
    },
    /// Could not be read; moved aside and defaults loaded.
    Damaged {
        error: String,
        kept: PathBuf,
    },
}

/// One migration step from `from` to `from + 1`, on the raw JSON so unknown
/// keys survive. Version 0 → 1 only adds the version key: every field name
/// is unchanged, so the step is a no-op by design and exists as the pattern.
fn migrate_step(v: &mut serde_json::Value, from: u32) {
    if let Some(obj) = v.as_object_mut() {
        if from == 0 {
            obj.insert("version".into(), serde_json::Value::from(1u32));
        }
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;

    #[test]
    fn version_0_file_loads_and_reports_migration() {
        // A real pre-versioning file: no version key, a few fields set.
        let text = r#"{"sidebar_visible":false,"cell":200.0,"code_delimiter":"\\","export":{"long_side":3000}}"#;
        let (p, from) = Prefs::parse(text).unwrap();
        assert_eq!(from, 0);
        assert_eq!(p.version, PREFS_VERSION);
        assert!(!p.sidebar_visible);
        assert_eq!(p.cell, 200.0);
        assert_eq!(p.export.long_side, 3000);
        assert_eq!(
            p.export.quality, 90,
            "fields the old file lacks take defaults"
        );
    }

    #[test]
    fn current_version_round_trips() {
        let mut p = Prefs {
            advance_after_mark: false,
            ..Default::default()
        };
        p.delivery.push(apature_deliver::Target {
            name: "Desk".into(),
            kind: apature_deliver::Kind::Sftp,
            host: "h".into(),
            port: 22,
            user: "u".into(),
            dir: "in".into(),
            key_file: None,
        });
        let text = serde_json::to_string(&p).unwrap();
        let (back, from) = Prefs::parse(&text).unwrap();
        assert_eq!(from, PREFS_VERSION);
        assert_eq!(back, p);
    }

    #[test]
    fn newer_file_is_read_not_rewritten_in_place() {
        let text = format!(
            r#"{{"version":{},"sidebar_visible":false,"future_key":{{"x":1}}}}"#,
            PREFS_VERSION + 5
        );
        let (p, from) = Prefs::parse(&text).unwrap();
        assert_eq!(from, PREFS_VERSION + 5);
        assert!(!p.sidebar_visible);
        assert_eq!(
            p.version,
            PREFS_VERSION + 5,
            "the newer version number is kept so save() can refuse to downgrade"
        );
    }

    #[test]
    fn file_without_a_layout_gets_photo_mechanic_keys() {
        let (p, _) = Prefs::parse(r#"{"version":1,"number_keys":"ColorClass"}"#).unwrap();
        assert_eq!(p.shortcut_layout, ShortcutLayout::PhotoMechanic);
        assert_eq!(p.number_keys, NumberKeys::ColorClass);
        let (p, _) = Prefs::parse(r#"{"version":1,"shortcut_layout":"Classic"}"#).unwrap();
        assert_eq!(p.shortcut_layout, ShortcutLayout::Classic);
    }

    #[test]
    fn damaged_text_is_an_error_not_defaults() {
        assert!(Prefs::parse("{not json").is_err());
        assert!(Prefs::parse("[1,2,3]").is_err());
    }
}
