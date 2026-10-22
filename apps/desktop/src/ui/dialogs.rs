//! Modal dialogs for file operations: Rename Photos and Move to Trash.
//! Both show exactly what will happen before anything moves.

use egui::RichText;

use super::tokens::{Theme, S1, S2, S3, S4};
use crate::app::App;
use crate::commands::Command;
use apature_ingest::{DateFolders, OnCollision};

const THIRD_PARTY: &str = include_str!("../../../../docs/THIRD_PARTY.md");

/// About: name, version and build, what it is, where it lives, and the
/// third-party notices the licences require us to show.
fn about(app: &mut App, ctx: &egui::Context, t: &Theme) {
    if !app.about_open {
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.vertical_centered(|ui| {
            ui.add_space(S3);
            ui.label(RichText::new("Apature").size(22.0).strong().color(t.text));
            ui.label(
                RichText::new(format!(
                    "Version {} · build {}",
                    env!("CARGO_PKG_VERSION"),
                    option_env!("APATURE_BUILD").unwrap_or("dev")
                ))
                .monospace()
                .size(11.0)
                .color(t.text2),
            );
            ui.add_space(S2);
            ui.label(RichText::new("Keyboard-first contact sheet, culling, metadata and delivery for photojournalists.").size(12.5).color(t.text2));
            ui.add_space(S2);
            ui.horizontal(|ui| {
                ui.hyperlink_to("Apature on GitHub", "https://github.com/McCal-Codes/apature");
                ui.label(RichText::new("·  © 2026 Caleb McCartney  ·  MIT or Apache-2.0").size(11.0).color(t.muted));
            });
        });
        ui.add_space(S3);
        super::eyebrow(ui, t, "Third-party notices");
        ui.add_space(S1);
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            // The notices file is a Markdown table; show it as plain lines.
            let plain: String = THIRD_PARTY
                .lines()
                .filter(|l| !l.starts_with("|---"))
                .map(|l| l.trim_start_matches('#').replace(" | ", "   ").trim_matches(|c| c == '|' || c == ' ').to_string())
                .collect::<Vec<_>>()
                .join("\n");
            ui.add(egui::Label::new(RichText::new(plain).monospace().size(10.0).color(t.text2)).wrap());
        });
        ui.add_space(S3);
        ui.vertical_centered(|ui| {
            if ui.button("Close").clicked() {
                app.about_open = false;
            }
        });
    });
    if modal.should_close() {
        app.about_open = false;
    }
}

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    about(app, ctx, t);
    let mut cmds: Vec<Command> = Vec::new();
    copy_photos(app, ctx, t, &mut cmds);

    if let Some(d) = app.rename_dialog.as_mut() {
        let n = d.targets.len();
        let mut changed = false;
        let modal = egui::Modal::new(egui::Id::new("rename-dialog")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.spacing_mut().item_spacing.y = S2;
            ui.label(RichText::new(format!("Rename {n} photo{}", if n == 1 { "" } else { "s" })).size(16.0).strong());
            ui.label(RichText::new("Every file of a photo moves together: RAW, JPEG and the XMP sidecar. Nothing is overwritten.").size(11.5).color(t.text2));
            ui.add_space(S2);
            ui.label(RichText::new("NEW NAME").monospace().size(9.5).color(t.muted));
            let r = ui.add(egui::TextEdit::singleline(&mut d.template).desired_width(f32::INFINITY).hint_text("100726-SPO-BGLF-{seqn:3}   tokens: {filename} {frame} {seqn} {headline} {date} {yymmdd} {year} {month} {day} {time}"));
            changed |= r.changed();
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && d.plan.as_ref().is_some_and(|p| p.can_run()) {
                cmds.push(Command::RenameApply);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("SEQUENCE STARTS AT").monospace().size(9.5).color(t.muted));
                changed |= ui.add(egui::DragValue::new(&mut d.seq_start).range(1..=99999)).changed();
            });
            ui.add_space(S2);
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).size(12.0).color(t.reject));
            }
            if let Some(plan) = &d.plan {
                let files = plan.moves.len();
                ui.label(RichText::new(format!("{files} file{} will be renamed{}", if files == 1 { "" } else { "s" }, if plan.unchanged > 0 { format!(" · {} unchanged", plan.unchanged) } else { String::new() })).size(12.0).color(t.text));
                egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (from, to) in plan.moves.iter().take(40) {
                        let f = from.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        let g = to.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.label(RichText::new(format!("{f}  →  {g}")).monospace().size(10.5).color(t.text2));
                    }
                    if plan.moves.len() > 40 {
                        ui.label(RichText::new(format!("… and {} more", plan.moves.len() - 40)).size(10.5).color(t.muted));
                    }
                    for (p, why) in &plan.collisions {
                        ui.label(RichText::new(format!("✕ {}  {why}", p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default())).monospace().size(10.5).color(t.reject));
                    }
                });
            }
            ui.add_space(S3);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can = d.plan.as_ref().is_some_and(|p| p.can_run());
                    if ui.add_enabled(can, egui::Button::new(RichText::new("Rename").strong())).clicked() {
                        cmds.push(Command::RenameApply);
                    }
                    if ui.button("Cancel").clicked() {
                        cmds.push(Command::CloseDialog);
                    }
                });
            });
        });
        if modal.should_close() {
            cmds.push(Command::CloseDialog);
        }
        if changed {
            app.rename_recompute();
        }
    }

    if let Some(targets) = app.trash_dialog.clone() {
        let n = targets.len();
        let files: usize = targets
            .iter()
            .map(|&i| apature_core::fileops::item_files(&app.items[i]).len())
            .sum();
        let modal = egui::Modal::new(egui::Id::new("trash-dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.spacing_mut().item_spacing.y = S2;
            ui.label(RichText::new(format!("Move {n} photo{} to the Trash?", if n == 1 { "" } else { "s" })).size(16.0).strong());
            ui.label(RichText::new(format!("{files} file{} including RAW, JPEG and sidecars. Apature never deletes: they stay in the Trash until you empty it.", if files == 1 { "" } else { "s" })).size(11.5).color(t.text2));
            egui::ScrollArea::vertical().max_height(120.0).auto_shrink([false, true]).show(ui, |ui| {
                for &i in targets.iter().take(20) {
                    ui.label(RichText::new(app.items[i].display_name()).monospace().size(10.5).color(t.text2));
                }
                if n > 20 {
                    ui.label(RichText::new(format!("… and {} more", n - 20)).size(10.5).color(t.muted));
                }
            });
            ui.add_space(S4);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Move to Trash").strong().color(t.reject))).clicked() {
                        cmds.push(Command::TrashConfirm);
                    }
                    if ui.button("Cancel").clicked() {
                        cmds.push(Command::CloseDialog);
                    }
                });
            });
        });
        if modal.should_close() {
            cmds.push(Command::CloseDialog);
        }
    }

    for c in cmds {
        app.dispatch(ctx, c);
    }
}

fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

/// Copy Photos…: where, how to name, what travels along, what to do on a
/// name clash; then the copy's progress with Cancel.
fn copy_photos(app: &mut App, ctx: &egui::Context, t: &Theme, cmds: &mut Vec<Command>) {
    if app.copy_dialog.is_none() {
        return;
    }
    let has_pad = app.assignment.is_some();
    let mut changed = false;
    let modal = egui::Modal::new(egui::Id::new("copy-dialog")).show(ctx, |ui| {
        ui.set_width(580.0);
        ui.spacing_mut().item_spacing.y = S2;
        if let Some(r) = &app.copy_running {
            let n = r.photos;
            ui.label(RichText::new(format!("Copying {n} photo{} to {}", if n == 1 { "" } else { "s" }, r.dest.display())).size(16.0).strong());
            ui.add(egui::ProgressBar::new(match &r.done {
                // A cancelled copy shows how far it got, not 100 %.
                Some(d) if !d.cancelled => 1.0,
                _ => r.fraction(),
            }).show_percentage());
            if let Some(p) = &r.progress {
                let cur = p.current.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("{} of {} files · {} of {}  {cur}", p.files_done, p.files_total, human_bytes(p.bytes_done), human_bytes(p.bytes_total))).monospace().size(10.5).color(t.text2));
            }
            if let Some(o) = &r.done {
                let mut line = format!("{} file{} copied", o.copied.len(), if o.copied.len() == 1 { "" } else { "s" });
                if o.cancelled {
                    line += " · cancelled";
                }
                if o.pad_written > 0 {
                    line += &format!(" · caption written to {} sidecar{}", o.pad_written, if o.pad_written == 1 { "" } else { "s" });
                }
                ui.label(RichText::new(line).size(12.0).color(t.text));
                for (p, why) in o.failed.iter().take(10) {
                    ui.label(RichText::new(format!("✕ {}  {why}", p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default())).monospace().size(10.5).color(t.reject));
                }
            }
            ui.add_space(S3);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if r.done.is_some() {
                        if ui.button(RichText::new("Close").strong()).clicked() {
                            cmds.push(Command::CloseDialog);
                        }
                        if !r.done.as_ref().is_some_and(|o| o.copied.is_empty()) && ui.button("Show in Finder").clicked() {
                            cmds.push(Command::CopyReveal);
                        }
                    } else {
                        if ui.button("Hide").on_hover_text("Keep copying in the background").clicked() {
                            cmds.push(Command::CloseDialog);
                        }
                        if ui.button(RichText::new("Cancel Copy").color(t.reject)).clicked() {
                            cmds.push(Command::CopyCancel);
                        }
                    }
                });
            });
            return;
        }
        let Some(d) = app.copy_dialog.as_mut() else { return };
        let n = d.targets.len();
        ui.label(RichText::new(format!("Copy {n} photo{}", if n == 1 { "" } else { "s" })).size(16.0).strong());
        ui.label(RichText::new("Files are copied byte for byte with their dates; nothing is re-encoded and the originals are not touched.").size(11.5).color(t.text2));
        ui.add_space(S1);
        ui.label(RichText::new("DESTINATION").monospace().size(9.5).color(t.muted));
        ui.horizontal(|ui| {
            let shown = if d.opts.dest.as_os_str().is_empty() { "No folder chosen".to_string() } else { d.opts.dest.display().to_string() };
            if ui.button("Choose…").clicked() {
                cmds.push(Command::CopyChooseDest);
            }
            ui.label(RichText::new(shown).monospace().size(11.0).color(t.text));
        });
        ui.label(RichText::new("RENAME (EMPTY KEEPS THE NAMES)").monospace().size(9.5).color(t.muted));
        let r = ui.add(egui::TextEdit::singleline(&mut d.opts.rename_template).desired_width(f32::INFINITY).hint_text("{headline}-{seqn:3}   tokens: {filename} {frame} {seqn} {headline} {date} {yymmdd} {time}"));
        changed |= r.changed();
        ui.horizontal(|ui| {
            ui.label(RichText::new("SEQUENCE STARTS AT").monospace().size(9.5).color(t.muted));
            changed |= ui.add(egui::DragValue::new(&mut d.opts.seq_start).range(1..=99999)).changed();
        });
        changed |= ui.checkbox(&mut d.opts.partners, "Copy RAW+JPEG partners").changed();
        changed |= ui.checkbox(&mut d.opts.sidecars, "Copy XMP sidecars").changed();
        ui.horizontal(|ui| {
            ui.label(RichText::new("SUBFOLDERS BY DATE").monospace().size(9.5).color(t.muted));
            for (v, label) in [(DateFolders::None, "None"), (DateFolders::Day, "2026-10-22"), (DateFolders::YearMonthDay, "2026/10/22")] {
                changed |= ui.radio_value(&mut d.opts.date_folders, v, label).changed();
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("IF A NAME IS TAKEN").monospace().size(9.5).color(t.muted));
            for (v, label) in [(OnCollision::Skip, "Skip"), (OnCollision::AddNumber, "Add a number"), (OnCollision::Replace, "Replace")] {
                changed |= ui.radio_value(&mut d.opts.on_collision, v, label).changed();
            }
        });
        ui.add_enabled(has_pad, egui::Checkbox::new(&mut d.apply_pad, "Apply the assignment pad to the copies (XMP sidecars only)"))
            .on_disabled_hover_text("Load an assignment first (File › Load Assignment…)");
        ui.add_space(S1);
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).size(12.0).color(t.reject));
        }
        if let Some(plan) = &d.plan {
            let files = plan.entries.len();
            let mut line = format!("{} photo{} · {files} file{} · {}", plan.photos, if plan.photos == 1 { "" } else { "s" }, if files == 1 { "" } else { "s" }, human_bytes(plan.total_bytes));
            if plan.numbered > 0 {
                line += &format!(" · {} numbered", plan.numbered);
            }
            if plan.replacing > 0 {
                line += &format!(" · {} replace existing files", plan.replacing);
            }
            if !plan.skipped.is_empty() {
                line += &format!(" · {} skipped", plan.skipped.len());
            }
            ui.label(RichText::new(line).size(12.0).color(if plan.replacing > 0 { t.reject } else { t.text }));
            egui::ScrollArea::vertical().max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
                let root = &d.opts.dest;
                for e in plan.entries.iter().take(40) {
                    let f = e.source.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                    let g = e.destination.strip_prefix(root).unwrap_or(&e.destination).display().to_string();
                    ui.label(RichText::new(format!("{f}  →  {g}{}", if e.replace { "  (replaces)" } else { "" })).monospace().size(10.5).color(t.text2));
                }
                if plan.entries.len() > 40 {
                    ui.label(RichText::new(format!("… and {} more", plan.entries.len() - 40)).size(10.5).color(t.muted));
                }
                for s in plan.skipped.iter().take(20) {
                    ui.label(RichText::new(format!("– {}  {}", s.source.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), s.reason)).monospace().size(10.5).color(t.muted));
                }
            });
        }
        ui.add_space(S3);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can = d.plan.as_ref().is_some_and(|p| !p.entries.is_empty());
                if ui.add_enabled(can, egui::Button::new(RichText::new("Copy").strong())).clicked() {
                    cmds.push(Command::CopyStart);
                }
                if ui.button("Cancel").clicked() {
                    cmds.push(Command::CloseDialog);
                }
            });
        });
    });
    if modal.should_close() {
        cmds.push(Command::CloseDialog);
    }
    if changed {
        app.copy_recompute();
    }
}
