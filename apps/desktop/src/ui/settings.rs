//! Settings, in its own window (⌘,), as macOS expects.

use egui::{Color32, RichText};

use apature_core::ColorClassScheme;

use super::tokens::{Theme, S1, S2, S3, S4};
use crate::app::App;
use crate::commands::Command;
use crate::prefs::{Appearance, NumberKeys, ShortcutLayout};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Classes,
    Metadata,
    Ingest,
    Accessibility,
    Delivery,
    Cache,
    Shortcuts,
}

const PAGES: [(Page, &str); 8] = [
    (Page::General, "General"),
    (Page::Classes, "Color classes"),
    (Page::Metadata, "Metadata"),
    (Page::Ingest, "Ingest"),
    (Page::Accessibility, "Accessibility"),
    (Page::Delivery, "Delivery"),
    (Page::Cache, "Cache & diagnostics"),
    (Page::Shortcuts, "Shortcuts"),
];

fn heading(ui: &mut egui::Ui, t: &Theme, text: &str) {
    ui.label(RichText::new(text).size(15.0).strong().color(t.text));
    ui.add_space(S2);
}

fn hint(ui: &mut egui::Ui, t: &Theme, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.0).color(t.muted)).wrap());
}

/// A section inside a page: the same eyebrow the sheets and inspector use,
/// with a hairline above so pages read as a list of groups.
fn section(ui: &mut egui::Ui, t: &Theme, title: &str) {
    ui.add_space(S2);
    let r = ui.available_rect_before_wrap();
    ui.painter()
        .hline(r.x_range(), r.top(), egui::Stroke::new(1.0, t.line));
    ui.add_space(S3);
    super::eyebrow(ui, t, title);
    ui.add_space(S1);
}

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    if !app.settings_open {
        return;
    }
    let mut cmds: Vec<Command> = Vec::new();
    let mut close = false;
    let t = *t;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("apature-settings"),
        egui::ViewportBuilder::default()
            .with_title("Apature Settings")
            .with_inner_size([680.0, 560.0])
            .with_min_inner_size([560.0, 420.0]),
        |ctx, _class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            // Screenshot support for this window (see main.rs --settings). The
            // viewport only receives the Screenshot event on a frame it paints,
            // so keep it painting while a capture is pending.
            if let Some(path) = app.screenshot_path() {
                ctx.request_repaint();
                let shot = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = shot {
                    let [w, h] = img.size;
                    if let Some(buf) =
                        image::RgbaImage::from_raw(w as u32, h as u32, img.as_raw().to_vec())
                    {
                        let _ = buf.save(&path);
                    }
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                }
            }
            body(app, ctx, &t, &mut cmds);
        },
    );
    if close {
        app.settings_open = false;
    }
    for c in cmds {
        app.dispatch(ctx, c);
    }
}

/// Apature classic keys (Settings › Shortcuts › Apature classic).
const SHORTCUTS: &[(&str, &str)] = &[
    (
        "← → ↑ ↓",
        "Move focus (⇧ extends the selection, ⌘ keeps it)",
    ),
    ("X · T", "Reject · toggle tag"),
    ("⏎ / Space · Esc", "Preview · back"),
    (
        "Z · C",
        "Preview: 1:1 at the pointer · compare the selection",
    ),
    (
        "L · ⌘scroll",
        "Preview: loupe under the pointer · magnification 100–400 %",
    ),
    (
        "K · R · G",
        "Preview: crop tool · next ratio · next guides (⏎ apply, ⌫ remove, Esc cancel)",
    ),
    ("B", "Preview: brighten +1/+2 stops, view only"),
    ("⌘[ · ⌘]", "Rotate left · right (sidecar only)"),
    ("S", "Expand or collapse the focused burst stack"),
    ("⌘R", "Refresh the folder, keeping cursor and selection"),
    ("⌘A · ⌘⇧A", "Select all · none"),
    ("⌘Z · ⌘⇧Z", "Undo · redo"),
    ("⌘⇧1 … ⌘⇧5", "Sheet · Preview · Caption · Ingest · Export"),
    ("⌘E", "Export…"),
    ("⌘= · ⌘-", "Larger · smaller thumbnails"),
    ("⌃⌘S · ⌥⌘I · ⇧F", "Sidebar · inspector · filmstrip"),
    ("⌥⌘F", "Clear filters"),
    ("⌘O / ⌘T · ⌘W", "Open folder in a new tab · close tab"),
    ("⌘⇧] · ⌘⇧[", "Next · previous tab"),
    ("⌘⏎", "Save caption and advance"),
    ("⌘,", "Settings"),
];

/// Photo Mechanic's macOS keys, for the actions Apature has (the default).
const SHORTCUTS_PM: &[(&str, &str)] = &[
    (
        "← → ↑ ↓",
        "Move focus (⇧ extends the selection, ⌘ keeps it)",
    ),
    ("X · T", "Reject · toggle tag"),
    ("⏎ / Space · Esc", "Preview · back"),
    (
        "Z · V",
        "Preview: 1:1 at the pointer · compare the selection",
    ),
    (
        "L · ⌘scroll",
        "Preview: loupe under the pointer · magnification 100–400 %",
    ),
    (
        "C · R · G",
        "Preview: crop tool · next ratio · next guides (⏎ apply, ⌥C or ⌫ remove, Esc cancel)",
    ),
    ("B", "Preview: brighten +1/+2 stops, view only"),
    ("[ · ]  or  ⌘[ · ⌘]", "Rotate left · right (sidecar only)"),
    ("S", "Expand or collapse the focused burst stack"),
    ("M or ⌘M", "Rename photos"),
    ("I · ⌘⇧I", "Caption the photo · apply the assignment pad"),
    ("F5", "Refresh the folder, keeping cursor and selection"),
    ("⌘A · ⌘D", "Select all · none"),
    ("⌘Z · ⌘⇧Z", "Undo · redo"),
    ("⌘⌫", "Move to Trash"),
    ("⌥⌘C · ⌥⌘V", "Copy · paste metadata"),
    ("⌘R", "Preview"),
    ("⌘⇧1 · ⌘⇧3 … ⌘⇧5", "Sheet · Caption · Ingest · Export"),
    ("⌘G · ⌘⇧G · ⌃⌘X", "Ingest… · Live Ingest · Export…"),
    ("⌘= · ⌘-", "Larger · smaller thumbnails"),
    ("⌃⌘S · ⌥⌘I · ⇧F", "Sidebar · inspector · filmstrip"),
    ("⌥⌘F", "Clear filters"),
    ("⌘O / ⌘N · ⌘W", "Open folder in a new tab · close tab"),
    ("⌘⇧W", "Close the window"),
    ("⌘⇧] · ⌘⇧[", "Next · previous tab"),
    ("⌘⏎", "Save caption and advance"),
    ("⌘,", "Settings"),
];

fn dir_size(dir: &std::path::Path) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    for e in walkdir::WalkDir::new(dir).into_iter().flatten() {
        if e.file_type().is_file() {
            files += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (files, bytes)
}

/// The window's content: nav on the left, one page on the right. Also drawn
/// inline on the main window by `show_inline` for headless screenshots.
fn body(app: &mut App, ctx: &egui::Context, t: &Theme, cmds: &mut Vec<Command>) {
    egui::SidePanel::left("settings_nav")
        .exact_width(150.0)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin::same(S3 as i8)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (page, label) in PAGES {
                let on = app.settings_page == page;
                let r = ui.add_sized(
                    [ui.available_width(), 26.0],
                    egui::Button::new(RichText::new(label).color(if on {
                        t.accent
                    } else {
                        t.text2
                    }))
                    .fill(if on {
                        t.accent_wash
                    } else {
                        Color32::TRANSPARENT
                    })
                    .frame(false),
                );
                if r.clicked() {
                    app.settings_page = page;
                }
            }
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.window).inner_margin(egui::Margin::same(S4 as i8))).show(ctx, |ui| {
        ui.spacing_mut().item_spacing.y = S2;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.settings_page {
            Page::General => {
                heading(ui, t, "General");
                section(ui, t, "Appearance");
                ui.horizontal(|ui| {
                    for (a, label) in [(Appearance::System, "System"), (Appearance::Dark, "Apature Dark"), (Appearance::Light, "Apature Light")] {
                        if super::icons::chip(ui, t, label, app.prefs.appearance == a).clicked() {
                            app.prefs.appearance = a;
                            app.prefs_dirty = true;
                        }
                    }
                });
                hint(ui, t, "System follows System Settings › Appearance.");
                ui.add_space(S2);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Accent").color(t.text2));
                    for a in crate::prefs::Accent::ALL {
                        let swatch = Theme::for_ctx(ui.ctx()).with_accent(a);
                        let selected = app.prefs.accent == a;
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        let resp = resp.on_hover_text(a.label());
                        let p = ui.painter();
                        p.circle_filled(rect.center(), 9.0, swatch.accent);
                        if selected {
                            p.circle_stroke(rect.center(), 11.0, egui::Stroke::new(2.0, t.text));
                        } else if resp.hovered() {
                            p.circle_stroke(rect.center(), 11.0, egui::Stroke::new(1.0, t.line_strong));
                        }
                        if resp.clicked() {
                            app.prefs.accent = a;
                            app.prefs_dirty = true;
                        }
                    }
                    ui.label(egui::RichText::new(app.prefs.accent.label()).color(t.muted));
                });
                hint(ui, t, "Selection, links and focus take the accent. Lavender is the Apature default.");
                ui.add_space(S3);
                section(ui, t, "Behaviour");
                if ui.checkbox(&mut app.prefs.advance_after_mark, "Advance to the next photo after marking one").changed() {
                    app.prefs_dirty = true;
                }
                if ui.checkbox(&mut app.prefs.check_updates, "Check for updates once a day (one request to GitHub; nothing is sent)").changed() {
                    app.prefs_dirty = true;
                }
                if app.prefs.check_updates
                    && ui.checkbox(&mut app.prefs.beta_updates, "Include beta releases").changed()
                {
                    app.prefs_dirty = true;
                }
                if ui.checkbox(&mut app.prefs.preview_in_window, "Open Preview in its own window (the sheet stays; good for two screens)").changed() {
                    app.prefs_dirty = true;
                }
                if ui.checkbox(&mut app.prefs.stack_bursts, "Stack bursts: frames shot within a second sit behind one thumbnail (S expands)").changed() {
                    app.prefs_dirty = true;
                    app.rebuild_view();
                }
                if ui.checkbox(&mut app.prefs.open_after_ingest, "Open the ingested folder when an ingest finishes").changed() {
                    app.prefs_dirty = true;
                }
                ui.add_space(S3);
                section(ui, t, "Default thumbnail size");
                let mut cell = app.prefs.cell;
                if ui.add(egui::Slider::new(&mut cell, 80.0..=600.0).suffix(" pt")).changed() {
                    app.set_cell(cell);
                }
                section(ui, t, "Files Apature keeps");
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Reveal settings in Finder").clicked() {
                        if let Some(p) = crate::prefs::Prefs::path() {
                            let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
                        }
                    }
                    if ui.small_button("Reveal log").on_hover_text("apature.log: every ingest, export, send and error").clicked() {
                        if let Some(d) = directories::ProjectDirs::from("dev", "mccal", "Apature") {
                            let _ = std::process::Command::new("open").arg("-R").arg(d.data_dir().join("apature.log")).spawn();
                        }
                    }
                    if ui.small_button("Reset all settings").on_hover_text("Keeps a copy as prefs.json.bak-reset; delivery passwords stay in the Keychain").clicked() {
                        if let Some(p) = crate::prefs::Prefs::path() {
                            let _ = std::fs::copy(&p, p.with_extension("json.bak-reset"));
                        }
                        app.prefs = crate::prefs::Prefs::default();
                        app.prefs_dirty = true;
                    }
                });
                hint(ui, t, &format!("Settings file version {} · {}", crate::prefs::PREFS_VERSION, crate::prefs::Prefs::path().map(|p| p.display().to_string()).unwrap_or_default()));
            }
            Page::Classes => {
                heading(ui, t, "Color classes");
                hint(ui, t, "Names are written into xmp:Label, so they should match your Photo Mechanic preferences for labels to round-trip. The numeric class is always written too.");
                ui.add_space(S2);
                let mut changed = false;
                egui::Grid::new("classes").num_columns(3).spacing([S3, S2]).show(ui, |ui| {
                    for i in 0..8 {
                        ui.label(RichText::new(format!("{}", i + 1)).monospace().color(t.muted));
                        let mut rgb = app.prefs.scheme.colors[i];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            app.prefs.scheme.colors[i] = rgb;
                            changed = true;
                        }
                        if ui.add(egui::TextEdit::singleline(&mut app.prefs.scheme.names[i]).desired_width(220.0)).changed() {
                            changed = true;
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(S2);
                if ui.button("Reset to Photo Mechanic names and colors").clicked() {
                    app.prefs.scheme = ColorClassScheme::default();
                    changed = true;
                }
                if changed {
                    app.scheme_changed();
                }
            }
            Page::Metadata => {
                heading(ui, t, "Metadata");
                section(ui, t, "Assignment file");
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(app.prefs.assignment_file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".into())).monospace().size(10.5).color(t.text2)).truncate());
                    if ui.button("Choose…").clicked() {
                        cmds.push(Command::LoadAssignment);
                    }
                    if app.prefs.assignment_file.is_some() && ui.button("Clear").clicked() {
                        app.prefs.assignment_file = None;
                        app.assignment = None;
                        app.prefs_dirty = true;
                    }
                });
                section(ui, t, "Code replacement file");
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(app.prefs.codes_file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".into())).monospace().size(10.5).color(t.text2)).truncate());
                    if ui.button("Choose…").clicked() {
                        cmds.push(Command::LoadCodes);
                    }
                    if app.prefs.codes_file.is_some() && ui.button("Clear").clicked() {
                        app.prefs.codes_file = None;
                        app.prefs_dirty = true;
                    }
                });
                ui.add_space(S2);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Code delimiter").size(12.0).color(t.text2));
                    let mut d = app.prefs.code_delimiter.to_string();
                    if ui.add(egui::TextEdit::singleline(&mut d).desired_width(28.0).char_limit(1)).changed() {
                        if let Some(c) = d.chars().next() {
                            app.prefs.code_delimiter = c;
                            app.prefs_dirty = true;
                        }
                    }
                    hint(ui, t, "Type \\cf\\ (or =cf= in Photo Mechanic's default) to expand a code.");
                });
                hint(ui, t, &format!("{} codes loaded.", app.codes.codes.len()));
            }
            Page::Ingest => {
                heading(ui, t, "Ingest defaults");
                let mut o = app.prefs.ingest.clone().unwrap_or_else(crate::prefs::default_ingest_options);
                let mut changed = false;
                if ui.checkbox(&mut app.prefs.seq_continue, "Keep {seqn} counting across ingests with the same headline").changed() {
                    app.prefs_dirty = true;
                }
                if !app.prefs.seq_counters.is_empty() {
                    hint(ui, t, &format!("{} headline counter(s) remembered.", app.prefs.seq_counters.len()));
                    if ui.small_button("Forget all counters").clicked() {
                        app.prefs.seq_counters.clear();
                        app.prefs_dirty = true;
                    }
                }
                ui.add_space(S3);
                section(ui, t, "Folder template");
                changed |= ui.add(egui::TextEdit::singleline(&mut o.folder_template).desired_width(f32::INFINITY)).changed();
                section(ui, t, "Rename template");
                changed |= ui.add(egui::TextEdit::singleline(&mut o.rename_template).desired_width(f32::INFINITY)).changed();
                changed |= ui.checkbox(&mut o.only_new, "Only photos not ingested before").changed();
                if ui.button("Reset to defaults  ({year4}/{month0} {monthname}/{mnn3} {day0} {headline} · {headline}{seqn:3}-{month0}{day0}{yr2})").clicked() {
                    let keep = (o.source.clone(), o.primary.clone(), o.secondary.clone());
                    o = apature_ingest::Options { source: keep.0, primary: keep.1, secondary: keep.2, ..crate::prefs::default_ingest_options() };
                    changed = true;
                }
                changed |= ui.checkbox(&mut o.include_subfolders, "Include subfolders").changed();
                changed |= ui.checkbox(&mut o.preserve_folders, "Keep the card's folders").changed();
                changed |= ui.checkbox(&mut o.copy_companions, "Copy voice memos and sidecars with their photos").changed();
                if ui.checkbox(&mut app.prefs.open_ingest_on_card, "When a camera card mounts, open Ingest with it as the source").changed() {
                    app.prefs_dirty = true;
                }
                hint(ui, t, "tokens: {filename} {ext} {frame} {seqn} {seqn:4} {headline} {date} {yymmdd} {year} {month} {day} {time}");
                if changed {
                    app.prefs.ingest = Some(o.clone());
                    app.ingest.opts = o;
                    app.prefs_dirty = true;
                }
            }
            Page::Accessibility => {
                heading(ui, t, "Accessibility");
                section(ui, t, "Text and control size");
                let mut pct = (app.prefs.ui_scale * 100.0).round();
                if ui.add(egui::Slider::new(&mut pct, 80.0..=180.0).step_by(5.0).suffix(" %")).changed() {
                    app.prefs.ui_scale = pct / 100.0;
                    app.prefs_dirty = true;
                }
                hint(ui, t, "Scales every panel, label and control. ⌘+ and ⌘− do the same while the app is open; 100 % is the default.");
                ui.add_space(S3);
                section(ui, t, "Motion");
                ui.horizontal(|ui| {
                    for (m, label) in [(crate::prefs::Motion::System, "Follow macOS Reduce Motion"), (crate::prefs::Motion::Reduce, "Reduce"), (crate::prefs::Motion::Full, "Full")] {
                        if super::icons::chip(ui, t, label, app.prefs.motion == m).clicked() {
                            app.prefs.motion = m;
                            app.prefs_dirty = true;
                        }
                    }
                });
                hint(ui, t, "Reduce turns fades, slides and the ingest stagger into instant changes. Nothing in Apature loops or autoplays either way.");
                ui.add_space(S3);
                if ui.checkbox(&mut app.prefs.high_contrast, "Higher contrast text and borders").changed() {
                    app.prefs_dirty = true;
                }
                hint(ui, t, "Lifts the quiet grey labels and strengthens panel borders in both appearances.");
                ui.add_space(S3);
                hint(ui, t, "Keyboard: every workspace is reachable with ⌘⇧1–5, the sheet with arrows, marks with the number keys (see Shortcuts). Screen reader labels are still being filled in (see docs/ACCESSIBILITY.md).");
            }
            Page::Delivery => {
                heading(ui, t, "Delivery");
                hint(ui, t, "Where finished exports go. Passwords are kept in the macOS Keychain, never in the prefs file. Each file lands as a hidden .part name and is renamed when complete.");
                ui.add_space(S2);
                let mut remove: Option<usize> = None;
                let mut rename: Option<(String, String)> = None;
                let n = app.prefs.delivery.len();
                for i in 0..n {
                    let (old_name, title) = {
                        let d = &app.prefs.delivery[i];
                        (d.name.clone(), format!("{}  ·  {:?} {}:{}", d.name, d.kind, d.host, d.port))
                    };
                    egui::CollapsingHeader::new(RichText::new(title).size(12.5).color(t.text)).id_salt(("deliv", i)).default_open(n == 1).show(ui, |ui| {
                        let d = &mut app.prefs.delivery[i];
                        egui::Grid::new(("deliv_grid", i)).num_columns(2).spacing([S3, S2]).show(ui, |ui| {
                            ui.label(RichText::new("Name").size(12.0).color(t.text2));
                            let mut nm = d.name.clone();
                            if ui.add(egui::TextEdit::singleline(&mut nm).desired_width(260.0)).lost_focus() && nm.trim() != d.name && !nm.trim().is_empty() {
                                rename = Some((d.name.clone(), nm.trim().to_string()));
                            }
                            ui.end_row();
                            ui.label(RichText::new("Protocol").size(12.0).color(t.text2));
                            ui.horizontal(|ui| {
                                for (k, label) in [(apature_deliver::Kind::Folder, "Folder"), (apature_deliver::Kind::Ftp, "FTP"), (apature_deliver::Kind::Ftps, "FTPS (TLS)"), (apature_deliver::Kind::Sftp, "SFTP")] {
                                    if super::icons::chip(ui, t, label, d.kind == k).clicked() {
                                        d.kind = k;
                                        d.port = apature_deliver::Target::default_port(k);
                                        app.prefs_dirty = true;
                                    }
                                }
                            });
                            ui.end_row();
                            let is_folder = d.kind == apature_deliver::Kind::Folder;
                            if !is_folder {
                                ui.label(RichText::new("Host").size(12.0).color(t.text2));
                                if ui.add(egui::TextEdit::singleline(&mut d.host).desired_width(260.0).hint_text("ftp.example.com or 192.168.1.20")).changed() { app.prefs_dirty = true; }
                                ui.end_row();
                                ui.label(RichText::new("Port").size(12.0).color(t.text2));
                                if ui.add(egui::DragValue::new(&mut d.port).range(1..=65535)).changed() { app.prefs_dirty = true; }
                                ui.end_row();
                                ui.label(RichText::new("User").size(12.0).color(t.text2));
                                if ui.add(egui::TextEdit::singleline(&mut d.user).desired_width(260.0)).changed() { app.prefs_dirty = true; }
                                ui.end_row();
                            }
                            ui.label(RichText::new("Folder").size(12.0).color(t.text2));
                            ui.horizontal(|ui| {
                                let hint = if is_folder { "a mounted share, or the Google Drive / OneDrive / Dropbox sync folder" } else { "incoming/photos (empty = login folder)" };
                                if ui.add(egui::TextEdit::singleline(&mut d.dir).desired_width(if is_folder { 200.0 } else { 260.0 }).hint_text(hint)).changed() { app.prefs_dirty = true; }
                                if is_folder && ui.small_button("Choose…").clicked() {
                                    if let Some(p) = rfd::FileDialog::new().pick_folder() { d.dir = p.display().to_string(); app.prefs_dirty = true; }
                                }
                            });
                            ui.end_row();
                            if is_folder {
                                ui.label(RichText::new("").size(12.0));
                                ui.label(RichText::new("No sign-in needed: the Google Drive or OneDrive desktop app syncs this folder to the cloud.").size(11.0).color(t.muted));
                                ui.end_row();
                            }
                            if d.kind == apature_deliver::Kind::Sftp {
                                ui.label(RichText::new("Key file").size(12.0).color(t.text2));
                                ui.horizontal(|ui| {
                                    let shown = d.key_file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "password login".into());
                                    ui.label(RichText::new(shown).monospace().size(10.5).color(t.text2));
                                    if ui.small_button("Choose…").clicked() {
                                        if let Some(p) = rfd::FileDialog::new().pick_file() { d.key_file = Some(p); app.prefs_dirty = true; }
                                    }
                                    if d.key_file.is_some() && ui.small_button("Use password").clicked() { d.key_file = None; app.prefs_dirty = true; }
                                });
                                ui.end_row();
                            }
                            if !is_folder {
                            ui.label(RichText::new(if d.key_file.is_some() { "Passphrase" } else { "Password" }).size(12.0).color(t.text2));
                            ui.horizontal(|ui| {
                                let draft = app.deliver.pw_drafts.entry(old_name.clone()).or_default();
                                ui.add(egui::TextEdit::singleline(draft).password(true).desired_width(180.0));
                                let has = crate::platform::Platform::secret_get(&old_name).is_some();
                                if ui.small_button("Save to Keychain").clicked() {
                                    match crate::platform::Platform::secret_set(&old_name, draft) {
                                        Ok(()) => { draft.clear(); app.deliver.test_result = Some(format!("{old_name}: password saved")); }
                                        Err(e) => app.deliver.test_result = Some(format!("{old_name}: {e}")),
                                    }
                                }
                                ui.label(RichText::new(if has { "saved" } else { "none saved" }).size(10.5).color(t.muted));
                            });
                            ui.end_row();
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Test connection").clicked() {
                                cmds.push(Command::DeliverTest(old_name.clone()));
                            }
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some((from, to)) = rename {
                    if let Some(d) = app.prefs.delivery.iter_mut().find(|d| d.name == from) { d.name = to.clone(); }
                    if let Some(pw) = crate::platform::Platform::secret_get(&from) {
                        let _ = crate::platform::Platform::secret_set(&to, &pw);
                        crate::platform::Platform::secret_delete(&from);
                    }
                    if app.prefs.send_after_export.as_deref() == Some(from.as_str()) { app.prefs.send_after_export = Some(to); }
                    app.prefs_dirty = true;
                }
                if let Some(i) = remove {
                    let d = app.prefs.delivery.remove(i);
                    crate::platform::Platform::secret_delete(&d.name);
                    if app.prefs.send_after_export.as_deref() == Some(d.name.as_str()) { app.prefs.send_after_export = None; }
                    app.prefs_dirty = true;
                }
                if ui.button("Add a target").clicked() {
                    let k = apature_deliver::Kind::Sftp;
                    app.prefs.delivery.push(apature_deliver::Target { name: format!("Desk {}", app.prefs.delivery.len() + 1), kind: k, host: String::new(), port: apature_deliver::Target::default_port(k), user: String::new(), dir: String::new(), key_file: None });
                    app.prefs_dirty = true;
                }
                if let Some(r) = &app.deliver.test_result {
                    ui.add_space(S2);
                    ui.label(RichText::new(r).size(12.0).color(if r.contains("connected") || r.contains("saved") { t.text } else { t.reject }));
                }
            }
            Page::Cache => {
                heading(ui, t, "Cache & diagnostics");
                section(ui, t, "Diagnostics");
                if ui.checkbox(&mut app.prefs.show_perf_stats, "Show timings in the status bar (scan, frame, thumbnails)").changed() {
                    app.prefs_dirty = true;
                }
                hint(ui, t, "Off by default. The same numbers go to apature.log either way.");
                section(ui, t, "Thumbnail cache");
                let dir = app.cache_dir();
                ui.add(egui::Label::new(RichText::new(dir.display().to_string()).monospace().size(10.5).color(t.text2)).truncate());
                ui.horizontal(|ui| {
                    if ui.button("Calculate size").clicked() {
                        app.cache_size = Some(dir_size(&dir));
                    }
                    if let Some((files, bytes)) = app.cache_size {
                        ui.label(RichText::new(format!("{files} thumbnails · {:.0} MB", bytes as f64 / 1e6)).size(12.0).color(t.text2));
                    }
                });
                if ui.button("Clear thumbnail cache").on_hover_text("Thumbnails are regenerated from the photos as needed").clicked() {
                    cmds.push(Command::ClearThumbCache);
                }
                hint(ui, t, "Originals are never touched; the cache only holds 320 px JPEG thumbnails.");
            }
            Page::Shortcuts => {
                heading(ui, t, "Shortcuts");
                section(ui, t, "Shortcut layout");
                ui.horizontal_wrapped(|ui| {
                    for (l, label) in [
                        (ShortcutLayout::PhotoMechanic, "Photo Mechanic"),
                        (ShortcutLayout::Classic, "Apature classic"),
                    ] {
                        if super::icons::chip(ui, t, label, app.prefs.shortcut_layout == l).clicked() {
                            app.prefs.shortcut_layout = l;
                            app.prefs_dirty = true;
                        }
                    }
                });
                let pm = app.prefs.shortcut_layout == ShortcutLayout::PhotoMechanic;
                hint(ui, t, if pm {
                    "Photo Mechanic's macOS keys for everything Apature does. Keys it has no match for keep Apature's."
                } else {
                    "Apature's original keys."
                });
                section(ui, t, "Number keys 1–8");
                ui.horizontal_wrapped(|ui| {
                    for (m, label) in [
                        (NumberKeys::Rating, "Stars: 1–5 rate, 0 clears"),
                        (NumberKeys::ColorClass, "Colour class: 1–8, 0 clears (Photo Mechanic)"),
                    ] {
                        if super::icons::chip(ui, t, label, app.prefs.number_keys == m).clicked() {
                            app.prefs.number_keys = m;
                            app.prefs_dirty = true;
                        }
                    }
                });
                hint(ui, t, match (pm, app.prefs.number_keys) {
                    (true, _) => "⌘1–⌘8 always set the colour class and ⌃1–⌃5 the rating, as in Photo Mechanic.",
                    (false, NumberKeys::Rating) => "⌘1–⌘8 set the colour class, ⌘0 clears it.",
                    (false, NumberKeys::ColorClass) => "⌘0–⌘5 set the rating.",
                });
                ui.add_space(S3);
                hint(ui, t, "Editable profiles arrive later.");
                ui.add_space(S2);
                let bare: (&str, &str) = match app.prefs.number_keys {
                    NumberKeys::Rating => ("1–5 · 0", "Rating · clear"),
                    NumberKeys::ColorClass => ("1–8 · 0", "Colour class · clear"),
                };
                let mut rows: Vec<(&str, &str)> = vec![bare];
                if pm {
                    rows.push(("⌘1–⌘8 · ⌘0", "Colour class · clear"));
                    rows.push(("⌃1–⌃5 · ⌃0", "Rating · clear"));
                } else if app.prefs.number_keys == NumberKeys::Rating {
                    rows.push(("⌘1–⌘8 · ⌘0", "Colour class · clear"));
                } else {
                    rows.push(("⌘0–⌘5", "Rating · clear"));
                }
                rows.extend_from_slice(if pm { SHORTCUTS_PM } else { SHORTCUTS });
                egui::Grid::new("keys").num_columns(2).spacing([S4, S2]).striped(true).show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(k).monospace().size(11.5).color(t.text));
                        ui.label(RichText::new(v).size(12.0).color(t.text2));
                        ui.end_row();
                    }
                });
            }
        });
    });
}

/// Headless screenshot path: draw the Settings content on the root window
/// (the separate viewport cannot be captured by the harness).
pub fn show_inline(app: &mut App, ctx: &egui::Context, t: &Theme) {
    let mut cmds: Vec<Command> = Vec::new();
    let t = *t;
    body(app, ctx, &t, &mut cmds);
    for c in cmds {
        app.dispatch(ctx, c);
    }
}
