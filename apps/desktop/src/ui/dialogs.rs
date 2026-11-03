//! Modal dialogs: About, Report a Bug, and the file operations (Rename
//! Photos, Move to Trash, Copy Photos, Extract Previews), which show exactly
//! what will happen before anything moves.

use egui::RichText;

use super::tokens::{Theme, S1, S2, S3, S4};
use crate::app::App;
use crate::commands::Command;
use apature_ingest::{DateFolders, DestMode, OnCollision};

const THIRD_PARTY: &str = include_str!("../../../../docs/THIRD_PARTY.md");

/// About: name, version and build, what it is, who made it (the original
/// project and this edition's changes, see `credits`), and the
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
                ui.label(RichText::new("Apature, originally created by").size(12.0).color(t.text));
                ui.hyperlink_to(crate::credits::UPSTREAM_NAME, crate::credits::UPSTREAM_URL);
            });
            ui.horizontal(|ui| {
                ui.hyperlink_to("Apature on GitHub", crate::credits::UPSTREAM_URL);
                ui.label(RichText::new("·  © 2026 Caleb McCartney  ·  MIT or Apache-2.0").size(11.0).color(t.muted));
            });
            ui.add_space(S2);
            ui.horizontal(|ui| {
                ui.label(RichText::new("This edition by").size(12.0).color(t.text));
                ui.hyperlink_to(crate::credits::FORK_AUTHOR, crate::credits::FORK_URL);
            });
        });
        ui.add_space(S1);
        super::eyebrow(ui, t, "Changes in this edition");
        ui.add_space(S1);
        for c in crate::credits::FORK_CHANGES {
            ui.label(RichText::new(format!("•  {c}")).size(11.5).color(t.text2));
        }
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
    extract_previews(app, ctx, t, &mut cmds);
    report_bug(app, ctx, t, &mut cmds);

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

/// Tools › Extract JPEG Previews from RAW Photos… (Photo Mechanic's dialog),
/// then its progress with Cancel and a summary.
fn extract_previews(app: &mut App, ctx: &egui::Context, t: &Theme, cmds: &mut Vec<Command>) {
    if app.extract_dialog.is_none() {
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("extract-dialog")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.spacing_mut().item_spacing.y = S2;
        if let Some(r) = &app.extract_running {
            ui.label(RichText::new(format!("Extracting JPEG previews from {} photo{}", r.photos, if r.photos == 1 { "" } else { "s" })).size(16.0).strong());
            ui.add(egui::ProgressBar::new(match &r.done {
                Some(o) if !o.cancelled => 1.0,
                _ => r.fraction(),
            }).show_percentage());
            if let (Some(p), None) = (&r.progress, &r.done) {
                let cur = p.current.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("{} of {}  {cur}", p.done, p.total)).monospace().size(10.5).color(t.text2));
            }
            if let Some(o) = &r.done {
                let n = o.extracted.len();
                let mut line = format!("{n} preview{} extracted", if n == 1 { "" } else { "s" });
                if !o.skipped.is_empty() {
                    line += &format!(" · {} skipped", o.skipped.len());
                }
                if o.cancelled {
                    line += " · cancelled";
                }
                ui.label(RichText::new(line).size(12.0).color(t.text));
                egui::ScrollArea::vertical().max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (src, why) in o.skipped.iter().take(40) {
                        let f = src.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.label(RichText::new(format!("– {f}  {why}")).monospace().size(10.5).color(t.muted));
                    }
                    for (raw, jpg) in o.extracted.iter().take(40) {
                        let f = raw.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        let g = jpg.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.label(RichText::new(format!("{f}  →  {g}")).monospace().size(10.5).color(t.text2));
                    }
                });
            }
            ui.add_space(S3);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match &r.done {
                        Some(o) => {
                            if ui.button(RichText::new("Close").strong()).clicked() {
                                cmds.push(Command::CloseDialog);
                            }
                            if !o.extracted.is_empty() && ui.button("Show in Finder").clicked() {
                                cmds.push(Command::ExtractReveal);
                            }
                        }
                        None => {
                            if ui.button("Hide").on_hover_text("Keep extracting in the background").clicked() {
                                cmds.push(Command::CloseDialog);
                            }
                            if ui.button(RichText::new("Cancel").color(t.reject)).clicked() {
                                cmds.push(Command::ExtractCancel);
                            }
                        }
                    }
                });
            });
            return;
        }
        let Some(d) = app.extract_dialog.as_ref() else { return };
        let o = &mut app.prefs.extract;
        let before = *o;
        ui.label(RichText::new("Extract JPEG Previews from RAW photos").size(16.0).strong());
        ui.label("This tool extracts JPEG previews found within certain RAW photos. A JPEG photo will be created with the same base name as the original RAW photo.");
        ui.add_space(S1);
        ui.label("When extracting previews:");
        ui.indent("extract-opts", |ui| {
            ui.checkbox(&mut o.copy_iptc, "Copy IPTC/XMP Info");
            ui.checkbox(&mut o.keep_exif, "Preserve Exif data where possible");
            ui.checkbox(&mut o.keep_marks, "Preserve image preferences (Tag and Color Class)");
            ui.checkbox(&mut o.embed_icc, "Embed ICC Profile");
        });
        ui.add_space(S3);
        let can = d.raws > 0;
        ui.horizontal(|ui| {
            ui.checkbox(&mut o.to_source, "Extract to source folder");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(can, egui::Button::new(RichText::new("Extract").strong())).clicked() {
                    cmds.push(Command::ExtractStart);
                }
                if ui.button("Cancel").clicked() {
                    cmds.push(Command::CloseDialog);
                }
            });
        });
        if *o != before {
            app.prefs_dirty = true;
        }
    });
    if modal.should_close() {
        cmds.push(Command::CloseDialog);
    }
}

/// White field with dark text (as in Photo Mechanic) for a dialog input.
fn light<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let dark = egui::Color32::from_gray(24);
        let v = ui.visuals_mut();
        v.text_edit_bg_color = Some(egui::Color32::WHITE);
        v.extreme_bg_color = egui::Color32::WHITE;
        v.override_text_color = Some(dark);
        v.text_cursor.stroke.color = dark;
        v.selection.bg_fill = egui::Color32::from_rgb(179, 207, 255);
        v.selection.stroke.color = egui::Color32::from_rgb(40, 110, 220);
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
        ] {
            w.bg_fill = egui::Color32::WHITE;
            w.weak_bg_fill = egui::Color32::WHITE;
            w.fg_stroke.color = dark;
        }
        add(ui)
    })
    .inner
}

/// `~/x` typed into a path box means the home folder.
fn expand_home(text: &str) -> std::path::PathBuf {
    match (text.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::PathBuf::from(home).join(rest),
        _ => std::path::PathBuf::from(text),
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
            // Time and speed, like Photo Mechanic's stats table: live while
            // copying (average so far), final once done.
            let elapsed = r.elapsed();
            egui::Grid::new("copy-stats").num_columns(2).spacing([S3, 2.0]).show(ui, |ui| {
                ui.label(RichText::new("Time").size(11.5).color(t.text2));
                ui.label(RichText::new(crate::copy_job::format_elapsed(elapsed)).monospace().size(11.5).color(t.text));
                ui.end_row();
                ui.label(RichText::new("Speed").size(11.5).color(t.text2));
                ui.label(RichText::new(crate::copy_job::format_speed(crate::copy_job::mb_per_sec(r.bytes_copied(), elapsed))).monospace().size(11.5).color(t.text));
                ui.end_row();
            });
            if r.done.is_none() {
                // Keep the clock ticking between progress reports (e.g. one big file).
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
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
        changed |= ui
            .radio_value(&mut d.opts.dest_mode, DestMode::OriginalFolder, "Original folder")
            .on_hover_text("Each copy goes next to its original. A name already there gets a number; nothing is replaced.")
            .changed();
        ui.horizontal(|ui| {
            changed |= ui.radio_value(&mut d.opts.dest_mode, DestMode::Subfolder, "Create subfolder").changed();
            let r = light(ui, |ui| ui.add(egui::TextEdit::singleline(&mut d.opts.subfolder).desired_width(180.0)))
                .on_hover_text("A folder of this name inside each original's folder");
            if r.changed() {
                d.opts.dest_mode = DestMode::Subfolder;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            changed |= ui.radio_value(&mut d.opts.dest_mode, DestMode::ThisFolder, "This folder:").changed();
            let mut text = d.opts.dest.display().to_string();
            let w = (ui.available_width() - 34.0).max(120.0);
            let r = light(ui, |ui| ui.add(egui::TextEdit::singleline(&mut text).desired_width(w)));
            if r.changed() {
                d.opts.dest = expand_home(text.trim());
                d.opts.dest_mode = DestMode::ThisFolder;
                changed = true;
            }
            if ui.button("…").on_hover_text("Choose a folder").clicked() {
                cmds.push(Command::CopyChooseDest);
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing + 64.0);
            ui.label(RichText::new("+").strong().color(t.text2));
            let r = light(ui, |ui| ui.add(egui::TextEdit::singleline(&mut d.opts.dest_extra).desired_width(220.0)))
                .on_hover_text("Optional sub-path under this folder, e.g. Smith/2nd");
            if r.changed() {
                d.opts.dest_mode = DestMode::ThisFolder;
                changed = true;
            }
        });
        if d.opts.dest_mode == DestMode::ThisFolder && !d.opts.dest.as_os_str().is_empty() && !d.opts.dest.is_dir() {
            ui.label(RichText::new("This folder doesn't exist yet; it will be created.").size(11.0).color(t.text2));
        }
        changed |= ui
            .radio_value(&mut d.opts.dest_mode, DestMode::AlwaysPick, "Always pick destination")
            .on_hover_text("Shows the folder picker when you press Copy")
            .changed();
        changed |= ui.checkbox(&mut d.opts.open_after, "Open destination as a Contact Sheet").changed();
        ui.add_space(S1);
        ui.horizontal(|ui| {
            ui.label(RichText::new("RENAME").monospace().size(9.5).color(t.muted));
            ui.label(RichText::new("empty keeps the names").size(10.5).color(t.muted));
        });
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 90.0).max(120.0);
            changed |= light(ui, |ui| ui.add(egui::TextEdit::singleline(&mut d.opts.rename_template).desired_width(w))).changed();
            ui.menu_button("Variables…", |ui| {
                for (tok, what) in [
                    ("{filename}", "original name"),
                    ("{frame}", "frame number"),
                    ("{seqn}", "sequence"),
                    ("{seqn:3}", "sequence, 3 digits"),
                    ("{headline}", "assignment headline"),
                    ("{date}", "2026-10-22"),
                    ("{yymmdd}", "261022"),
                    ("{year}", "2026"),
                    ("{month}", "10"),
                    ("{day}", "22"),
                    ("{time}", "143005"),
                ] {
                    if ui.button(RichText::new(format!("{tok}   {what}")).monospace().size(11.0)).clicked() {
                        d.opts.rename_template.push_str(tok);
                        changed = true;
                        ui.close();
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("SEQUENCE STARTS AT").monospace().size(9.5).color(t.muted));
            changed |= light(ui, |ui| ui.add(egui::DragValue::new(&mut d.opts.seq_start).range(1..=99999))).changed();
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
            let beside = d.opts.dest_mode == DestMode::OriginalFolder;
            for (v, label) in [(OnCollision::Skip, "Skip"), (OnCollision::AddNumber, "Add a number"), (OnCollision::Replace, "Replace")] {
                let on = d.opts.on_collision == v || (beside && v == OnCollision::AddNumber && d.opts.on_collision == OnCollision::Replace);
                let r = ui
                    .add_enabled(!(beside && v == OnCollision::Replace), egui::RadioButton::new(on, label))
                    .on_disabled_hover_text("Next to the originals a taken name always gets a number");
                if r.clicked() && d.opts.on_collision != v {
                    d.opts.on_collision = v;
                    changed = true;
                }
            }
        });
        ui.add_enabled(has_pad, egui::Checkbox::new(&mut d.apply_pad, "Apply the assignment pad to the copies (XMP sidecars only)"))
            .on_disabled_hover_text("Load an assignment first (File › Load Assignment…)");
        ui.add_space(S1);
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).size(12.0).color(t.reject));
        }
        if d.plan.is_none() && d.error.is_none() {
            let why = match d.opts.dest_mode {
                DestMode::AlwaysPick => "You'll pick the folder when you press Copy.",
                DestMode::Subfolder => "Name the subfolder.",
                _ => "Choose a folder.",
            };
            ui.label(RichText::new(why).size(12.0).color(t.text2));
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
                let per_original = matches!(d.opts.dest_mode, DestMode::OriginalFolder | DestMode::Subfolder);
                for e in plan.entries.iter().take(40) {
                    let f = e.source.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                    let base = if per_original { e.source.parent().unwrap_or(root) } else { root.as_path() };
                    let g = e.destination.strip_prefix(base).unwrap_or(&e.destination).display().to_string();
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
                // Always pick plans once the folder is picked.
                let can = d.plan.as_ref().is_some_and(|p| !p.entries.is_empty()) || (d.opts.dest_mode == DestMode::AlwaysPick && d.plan.is_none() && d.error.is_none() && !d.targets.is_empty());
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

/// One labelled white field of the bug report. `select_all` applies a ⌘A
/// that the menu bar caught (see `App::bug_dialog`) to this field when it
/// has focus, instead of selecting every photo.
fn bug_field(
    ui: &mut egui::Ui,
    t: &Theme,
    label: &str,
    text: &mut String,
    rows: usize,
    hint: &str,
    select_all: bool,
) -> egui::Response {
    ui.label(RichText::new(label).monospace().size(9.5).color(t.muted));
    let r = light(ui, |ui| {
        let edit = if rows <= 1 {
            egui::TextEdit::singleline(text)
        } else {
            egui::TextEdit::multiline(text).desired_rows(rows)
        };
        ui.add(edit.desired_width(f32::INFINITY).hint_text(hint))
    });
    if select_all && r.has_focus() {
        if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let n = text.chars().count();
            s.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(n),
            )));
            s.store(ui.ctx(), r.id);
        }
    }
    r
}

/// Help › Report a Bug…: a title, three optional notes and, if wanted, the
/// system info, opened as a prefilled GitHub issue in the browser (no token
/// in the app; the tester signs in on GitHub) or copied as Markdown.
fn report_bug(app: &mut App, ctx: &egui::Context, t: &Theme, cmds: &mut Vec<Command>) {
    use crate::bug_report;
    let mut status: Option<String> = None;
    let Some(d) = app.bug_dialog.as_mut() else {
        return;
    };
    let modal = egui::Modal::new(egui::Id::new("bug-dialog")).show(ctx, |ui| {
        ui.set_width(560.0);
        ui.spacing_mut().item_spacing.y = S2;
        ui.label(RichText::new("Report a Bug").size(16.0).strong());
        ui.label(RichText::new("Opens GitHub in your browser; you'll need a GitHub account to submit.").size(11.5).color(t.text2));
        ui.add_space(S1);
        let all = std::mem::take(&mut d.select_all);
        let r = bug_field(ui, t, "TITLE (REQUIRED)", &mut d.report.title, 1, "A short summary, e.g. Crop guides vanish after rotating", all);
        if std::mem::take(&mut d.focus_title) {
            r.request_focus();
        }
        bug_field(ui, t, "WHAT HAPPENED", &mut d.report.happened, 3, "What went wrong?", all);
        bug_field(ui, t, "STEPS TO REPRODUCE", &mut d.report.steps, 3, "1. Open a folder of NEFs\n2. …", all);
        bug_field(ui, t, "WHAT YOU EXPECTED", &mut d.report.expected, 2, "What should have happened?", all);
        ui.add_space(S1);
        ui.checkbox(&mut d.include_info, "Include system info");
        if d.include_info {
            egui::Frame::NONE
                .fill(t.raised)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(S2 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (k, v) in d.info.lines() {
                        ui.label(RichText::new(format!("{k}: {v}")).monospace().size(10.5).color(t.text2));
                    }
                    ui.label(RichText::new("Only these lines are added: no file names, folders or photo metadata.").size(10.5).color(t.muted));
                });
        }
        if let Some(n) = &d.notice {
            ui.label(RichText::new(n).size(11.5).color(t.text));
        }
        ui.add_space(S2);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let info = d.include_info.then_some(&d.info);
                let submit = ui
                    .add_enabled(d.report.can_submit(), egui::Button::new(RichText::new("Submit on GitHub…").strong()))
                    .on_disabled_hover_text("Give the bug a title first");
                if submit.clicked() {
                    let link = bug_report::issue_link(&d.report, info);
                    match bug_report::open_in_browser(&link.url) {
                        Ok(()) if link.trimmed => {
                            ui.ctx().copy_text(bug_report::clipboard_text(&d.report, info));
                            d.notice = Some(format!(
                                "The report is too long for a GitHub link (over {} characters), so the page opens with a trimmed copy. The full report is on the clipboard: paste it over the trimmed text before you submit.",
                                bug_report::MAX_URL_LEN
                            ));
                        }
                        Ok(()) => {
                            status = Some("Bug report opened in your browser".into());
                            cmds.push(Command::CloseDialog);
                        }
                        Err(e) => d.notice = Some(format!("Couldn't open the browser ({e}). Use Copy to Clipboard instead.")),
                    }
                }
                if ui.button("Copy to Clipboard").on_hover_text("The whole report as Markdown").clicked() {
                    ui.ctx().copy_text(bug_report::clipboard_text(&d.report, info));
                    d.notice = Some("Copied the report as Markdown.".into());
                }
                if ui.button(if d.notice.is_some() { "Close" } else { "Cancel" }).clicked() {
                    cmds.push(Command::CloseDialog);
                }
            });
        });
    });
    // Esc closes; a stray click beside the dialog doesn't throw the report away.
    if modal.should_close() && !modal.backdrop_response.clicked() {
        cmds.push(Command::CloseDialog);
    }
    if let Some(s) = status {
        app.status = s;
    }
}
