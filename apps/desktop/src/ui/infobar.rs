//! The folder strip under the tabs, where Photo Mechanic keeps its summary:
//! folder path, how many photos (and how many shown when a filter is on),
//! the selection with its size, and on the right the focused photo. The
//! bottom bar keeps transient messages and timings.

use egui::{Align, Layout, RichText};

use super::tokens::{Theme, S2, S4};
use crate::app::App;

pub const INFOBAR_H: f32 = 24.0;

fn human(b: u64) -> String {
    if b >= 1_000_000_000 {
        format!("{:.1} GB", b as f64 / 1e9)
    } else {
        format!("{:.0} MB", b as f64 / 1e6)
    }
}

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    egui::TopBottomPanel::top("infobar")
        .exact_height(INFOBAR_H)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin::symmetric(S4 as i8, 0))
                .stroke(egui::Stroke::NONE),
        )
        .show(ctx, |ui| {
            let Some(folder) = app.folder.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label(super::mono("No folder open · ⌘O", 10.5, t.muted));
                });
                return;
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = S2;
                // Right: the focused photo and its exposure.
                if let Some(it) = app.items.get(app.cursor) {
                    let mut right = format!(
                        "{}/{}  {}",
                        app.cursor_pos() + 1,
                        app.view.len(),
                        it.display_name()
                    );
                    if let Some(e) = &app.exif[app.cursor] {
                        let ex = e.exposure_line();
                        if !ex.is_empty() {
                            right = format!("{right}  ·  {ex}");
                        }
                    }
                    ui.label(super::mono(right, 10.5, t.text2));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    let name = folder
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let parent = folder
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    ui.label(RichText::new(name).size(12.5).color(t.text));
                    ui.add(egui::Label::new(super::mono(parent, 10.0, t.muted)).truncate());
                    let count = if app.filter.is_active() {
                        format!("{} of {} shown", app.view.len(), app.items.len())
                    } else {
                        format!("{} photos", app.items.len())
                    };
                    ui.label(super::mono(format!("·  {count}"), 10.5, t.text2));
                    let mut subfolders = app.prefs.scan_subfolders;
                    if ui
                        .checkbox(
                            &mut subfolders,
                            RichText::new("Include subfolders").size(11.0),
                        )
                        .on_hover_text(
                            "List the photos in this folder's subfolders too. Off: this folder only.",
                        )
                        .changed()
                    {
                        app.prefs.scan_subfolders = subfolders;
                        app.prefs_dirty = true;
                        app.refresh_folder();
                    }
                    let sel = app.selection.len();
                    if sel > 1 {
                        let bytes: u64 = app
                            .selection
                            .targets(app.cursor)
                            .iter()
                            .filter_map(|&i| app.items.get(i))
                            .map(|it| it.variants().map(|v| v.size).sum::<u64>())
                            .sum();
                        ui.label(super::mono(
                            format!("·  {sel} selected ({})", human(bytes)),
                            10.5,
                            t.accent,
                        ));
                    }
                });
            });
        });
}
