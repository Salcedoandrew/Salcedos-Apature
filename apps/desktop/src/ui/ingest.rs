//! Ingest workspace: source, destinations, naming, the plan, then progress
//! as a list that never hides a failure.

use egui::{Color32, CornerRadius, RichText};

use apature_ingest::FileFilter;

use super::tokens::{Theme, S1, S2, S3, S4, S5};
use crate::app::App;
use crate::commands::Command;
use crate::ingest_job::FileStatus;

fn labeled(ui: &mut egui::Ui, t: &Theme, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(
        RichText::new(label.to_uppercase())
            .monospace()
            .size(9.5)
            .color(t.muted),
    );
    add(ui);
    ui.add_space(S1);
}

fn human_bytes(b: u64) -> String {
    const GB: f64 = 1e9;
    const MB: f64 = 1e6;
    let f = b as f64;
    if f >= GB {
        format!("{:.1} GB", f / GB)
    } else {
        format!("{:.0} MB", f / MB)
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui, t: &Theme) {
    let mut cmds: Vec<Command> = Vec::new();
    let mut opts_changed = false;
    let running = app.ingest.running.is_some()
        && app
            .ingest
            .running
            .as_ref()
            .is_some_and(|r| r.done.is_none());

    egui::Frame::NONE.fill(t.photo).inner_margin(egui::Margin::same(S5 as i8)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = S2;
        ui.add_enabled_ui(!running, |ui| {
            ui.columns(2, |cols| {
                // ---- Source ----
                let ui = &mut cols[0];
                egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0_f32, t.line)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(S4 as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    super::eyebrow(ui, t, "Source");
                    ui.add_space(S2);
                    ui.horizontal_wrapped(|ui| {
                        for (name, path) in app.volumes.clone() {
                            let on = app.ingest.opts.source == path;
                            let kind = app.volume_kinds.get(&path).copied().unwrap_or(crate::platform::VolumeKind::External);
                            if super::icons::volume_chip(ui, t, kind, &name, on).clicked() {
                                app.ingest.opts.source = path;
                                opts_changed = true;
                            }
                        }
                        if ui.button("Folder…").clicked() {
                            cmds.push(Command::IngestChooseSource);
                        }
                    });
                    let src = app.ingest.opts.source.display().to_string();
                    ui.label(RichText::new(if src.is_empty() { "no source chosen".into() } else { src }).monospace().size(10.5).color(t.text2));
                    ui.add_space(S2);
                    if ui.checkbox(&mut app.ingest.opts.include_subfolders, "Include subfolders (DCIM/100NIKON…)").changed() {
                        opts_changed = true;
                    }
                    if ui.checkbox(&mut app.ingest.opts.only_new, "Only photos not ingested before").changed() {
                        opts_changed = true;
                    }
                    if ui.checkbox(&mut app.ingest.opts.preserve_folders, "Keep the card's folders (DCIM/100NIKON…)").changed() {
                        opts_changed = true;
                    }
                    if ui.checkbox(&mut app.ingest.opts.copy_companions, "Copy voice memos and sidecars with their photos").changed() {
                        opts_changed = true;
                    }
                    let mut suffix = app.ingest.opts.on_collision == apature_ingest::CollisionPolicy::Suffix;
                    if ui.checkbox(&mut suffix, "Add -1, -2… when a name is taken (never overwrite)").changed() {
                        app.ingest.opts.on_collision = if suffix { apature_ingest::CollisionPolicy::Suffix } else { apature_ingest::CollisionPolicy::Refuse };
                        opts_changed = true;
                    }
                    if ui.checkbox(&mut app.prefs.eject_after_ingest, "Eject the card when every file has verified").changed() {
                        app.prefs_dirty = true;
                    }
                    // Days found on the card (from the last plan): pick one to ingest only that shoot.
                    let days: Vec<((i32, u32, u32), usize)> = app.ingest.plan.as_ref().map(|p| p.days.clone()).unwrap_or_default();
                    if days.len() > 1 || (app.ingest.opts.day.is_some() && !days.is_empty()) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("DAYS").monospace().size(9.5).color(t.muted));
                            let total: usize = days.iter().map(|(_, n)| n).sum();
                            if super::icons::chip(ui, t, &format!("all · {total}"), app.ingest.opts.day.is_none()).clicked() {
                                app.ingest.opts.day = None;
                                opts_changed = true;
                            }
                            const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                            for ((y, m, d), n) in days {
                                let label = format!("{} {d:02} · {n}", MON[(m.clamp(1, 12) - 1) as usize]);
                                if super::icons::chip(ui, t, &label, app.ingest.opts.day == Some((y, m, d))).on_hover_text(format!("{y}-{m:02}-{d:02}")).clicked() {
                                    app.ingest.opts.day = Some((y, m, d));
                                    opts_changed = true;
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("FILES").monospace().size(9.5).color(t.muted));
                        for (f, label) in [(FileFilter::All, "all"), (FileFilter::RawOnly, "RAW only"), (FileFilter::JpegOnly, "JPEG only")] {
                            if super::icons::chip(ui, t, label, app.ingest.opts.filter == f).clicked() {
                                app.ingest.opts.filter = f;
                                opts_changed = true;
                            }
                        }
                    });
                });

                // ---- Destinations ----
                let ui = &mut cols[1];
                egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0_f32, t.line)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(S4 as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    super::eyebrow(ui, t, "Destinations");
                    ui.add_space(S2);
                    labeled(ui, t, "Primary", |ui| {
                        ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Choose…").clicked() {
                                cmds.push(Command::IngestChoosePrimary);
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let p = app.ingest.opts.primary.display().to_string();
                                ui.add(egui::Label::new(RichText::new(if p.is_empty() { "choose a folder".into() } else { p }).monospace().size(10.5).color(t.text2)).truncate());
                                });
                            });
                        });
                    });
                    labeled(ui, t, "Secondary (backup)", |ui| {
                        ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if app.ingest.opts.secondary.is_some() && ui.button("×").on_hover_text("No secondary").clicked() {
                                app.ingest.opts.secondary = None;
                                opts_changed = true;
                            }
                            if ui.button("Choose…").clicked() {
                                cmds.push(Command::IngestChooseSecondary);
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let p = app.ingest.opts.secondary.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".into());
                                ui.add(egui::Label::new(RichText::new(p).monospace().size(10.5).color(t.text2)).truncate());
                                });
                            });
                        });
                    });
                    labeled(ui, t, "Folder template", |ui| {
                        if ui.add(egui::TextEdit::singleline(&mut app.ingest.opts.folder_template).desired_width(f32::INFINITY).hint_text("{date}-{headline}  or  {year}/{month}/{day}  (empty = none)")).changed() {
                            opts_changed = true;
                        }
                    });
                    labeled(ui, t, "Rename template", |ui| {
                        if ui.add(egui::TextEdit::singleline(&mut app.ingest.opts.rename_template).desired_width(f32::INFINITY).hint_text("{headline}-{seqn}  (empty = keep names)")).changed() {
                            opts_changed = true;
                        }
                    });
                    labeled(ui, t, "Headline / slug", |ui| {
                            let mut h = app.ingest.opts.headline.clone().unwrap_or_default();
                            if ui.add(egui::TextEdit::singleline(&mut h).desired_width(f32::INFINITY)).changed() {
                                app.ingest.opts.headline = if h.trim().is_empty() { None } else { Some(h) };
                                if app.prefs.seq_continue {
                                    app.ingest.opts.seq_start = app.seq_start_for_headline();
                                }
                                opts_changed = true;
                            }
                        });
                    labeled(ui, t, "GPS for every photo (optional)", |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut app.ingest.gps_on, "");
                            ui.add_enabled(app.ingest.gps_on, egui::TextEdit::singleline(&mut app.ingest.gps).desired_width(220.0).hint_text("40.4461, -79.8955"));
                            if app.ingest.gps_on {
                                let ok = crate::app::parse_latlon(&app.ingest.gps).is_some();
                                ui.label(RichText::new(if ok { "written to each sidecar after the copy verifies" } else { "lat, lon in decimal degrees" }).size(10.5).color(if ok { t.muted } else { t.reject }));
                            }
                        });
                    });
                    labeled(ui, t, "Seq. start", |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.add(egui::DragValue::new(&mut app.ingest.opts.seq_start).range(1..=99999)).changed() {
                                opts_changed = true;
                            }
                            if app.prefs.seq_continue {
                                let remembered = app.seq_start_for_headline();
                                if remembered > 1 {
                                    ui.label(RichText::new(format!("continues from the last ingest ({remembered})")).size(11.0).color(t.muted));
                                    if ui.small_button("Start over at 1").on_hover_text("Forget this headline's counter").clicked() {
                                        let key = crate::prefs::seq_key(app.ingest.opts.headline.as_deref());
                                        app.prefs.seq_counters.remove(&key);
                                        app.prefs_dirty = true;
                                        app.ingest.opts.seq_start = 1;
                                        opts_changed = true;
                                    }
                                }
                            }
                        });
                    });
                    // Live example: what the first photo would be called with these
                    // templates. Stem/ext from the plan's first entry when there is one,
                    // date from the chosen day, else the card's first day, else today.
                    {
                        let o = &app.ingest.opts;
                        let first = app.ingest.plan.as_ref().and_then(|p| p.entries.first());
                        let stem = first
                            .and_then(|e| e.source.file_stem().map(|s| s.to_string_lossy().into_owned()))
                            .unwrap_or_else(|| "DSC_0001".into());
                        let ext = first
                            .and_then(|e| e.source.extension().map(|s| s.to_string_lossy().into_owned()))
                            .unwrap_or_else(|| "NEF".into());
                        let date = o
                            .day
                            .or_else(|| app.ingest.plan.as_ref().and_then(|p| p.days.first().map(|(d, _)| *d)))
                            .unwrap_or_else(apature_ingest::today);
                        let facts = apature_ingest::Facts {
                            stem,
                            ext,
                            date: Some(date),
                            time: Some((19, 4, 0)),
                            folder: Some("100NIKON".into()),
                            camera: Some("NIKON Z 6_2".into()),
                            iso: Some(800),
                            f_number: Some(2.8),
                            exposure: Some((1, 250)),
                            focal_mm: Some(70.0),
                            today: Some(apature_ingest::today()),
                            ..Default::default()
                        };
                        let line = match apature_ingest::example_names(o, &facts) {
                            Ok((folder, name)) if folder.is_empty() => name,
                            Ok((folder, name)) => format!("{folder}/{name}"),
                            Err(_) => "the templates leave the name empty — add a token or a headline".into(),
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("E.G.").monospace().size(9.5).color(t.muted));
                            ui.add(egui::Label::new(RichText::new(line).monospace().size(10.5).color(t.text2)).wrap());
                        });
                        ui.add_space(S1);
                    }
                    if app.assignment.is_some() {
                        ui.checkbox(&mut app.ingest.apply_pad, "Apply the loaded assignment pad to the copies");
                    }
                    ui.add(egui::Label::new(RichText::new("tokens: {filename} {ext} {frame} {seqn:3} {headline} {date} {datesort} {yymmdd} {year4} {yr2} {month0} {monthname} {mnn3} {day0} {dow3} {time} {hour} {hour12}{ampm} {camera} {lens} {iso} {aperture} {shutter} {focal} {folder} {today} {todaysort}").monospace().size(9.5).color(t.muted)).wrap());
                });
            });
        });

        ui.add_space(S3);
        // ---- Plan ----
        egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0_f32, t.line)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(S4 as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            super::eyebrow(ui, t, "Plan");
            ui.add_space(S2);
            // Live Ingest: the receiving end of a camera or FTP drop folder.
            ui.horizontal_wrapped(|ui| {
                let on = app.ingest.live.on;
                let label = if on { "Live Ingest on" } else { "Live Ingest" };
                if super::icons::chip(ui, t, label, on)
                    .on_hover_text("Watch the source folder and ingest every photo that lands, once its size has held for two polls (about 4 s). Naming, verification, the ledger and the sequence counter apply as usual.")
                    .clicked()
                {
                    cmds.push(Command::LiveToggle);
                }
                if on {
                    let mins = app.ingest.live.since.map(|s| s.elapsed().as_secs() / 60).unwrap_or(0);
                    let waiting = app.ingest.live.pending.len();
                    ui.label(RichText::new(format!(
                        "watching {} · {} ingested · {} settling · {mins} min",
                        app.ingest.opts.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        app.ingest.live.ingested,
                        waiting
                    )).size(11.5).color(t.text2));
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                }
            });
            ui.add_space(S2);
            if app.ingest.planning {
                ui.label(RichText::new("reading the card…").size(12.0).color(t.muted));
            }
            if let Some(err) = &app.ingest.plan_error {
                ui.label(RichText::new(err).size(12.0).color(t.reject));
            }
            match &app.ingest.plan {
                None if !app.ingest.planning && app.ingest.plan_error.is_none() => {
                    ui.label(RichText::new("Choose a source and a primary destination.").size(12.0).color(t.muted));
                }
                Some(plan) => {
                    // The plan card fades in when the plan lands (160 ms).
                    let k = super::motion::progress(app.ingest.plan_since, app.anim(0.16));
                    ui.set_opacity(k);
                    let dests = 1 + app.ingest.opts.secondary.is_some() as usize;
                    let mut summary = format!("{} files in {} photos · {} · {} destination{}", plan.entries.len(), plan.items, human_bytes(plan.total_bytes), dests, if dests == 1 { "" } else { "s" });
                    if !plan.skipped.is_empty() {
                        summary.push_str(&format!(" · {} skipped", plan.skipped.len()));
                    }
                    if plan.companions > 0 {
                        summary.push_str(&format!(" · {} memo{} and sidecar{}", plan.companions, if plan.companions == 1 { "" } else { "s" }, if plan.companions == 1 { "" } else { "s" }));
                    }
                    if plan.suffixed > 0 {
                        summary.push_str(&format!(" · {} renamed with a suffix", plan.suffixed));
                    }
                    if plan.other_days > 0 {
                        summary.push_str(&format!(" · {} from other days left out", plan.other_days));
                    }
                    ui.label(RichText::new(summary).size(13.0).color(t.text));
                    if let Some(first) = plan.entries.first() {
                        ui.label(RichText::new(format!("first copy: {}", first.primary.display())).monospace().size(10.5).color(t.text2));
                    }
                    for c in plan.collisions.iter().take(8) {
                        ui.label(RichText::new(format!("✕ {} — {}", c.destination.display(), c.reason)).monospace().size(10.5).color(t.reject));
                    }
                    if plan.collisions.len() > 8 {
                        ui.label(RichText::new(format!("… and {} more collisions", plan.collisions.len() - 8)).size(10.5).color(t.reject));
                    }
                    ui.add_space(S2);
                    ui.horizontal(|ui| {
                        let can = plan.can_run() && !running;
                        let start = ui.add_enabled(can, egui::Button::new(RichText::new("Start ingest").strong())).on_disabled_hover_text(if plan.collisions.is_empty() { "Nothing to copy" } else { "Resolve the collisions first" });
                        if start.clicked() {
                            cmds.push(Command::IngestStart);
                        }
                        if running && ui.button("Cancel after this file").clicked() {
                            cmds.push(Command::IngestCancel);
                        }
                    });
                }
                _ => {}
            }
        });

        // ---- Progress ----
        if let (Some(r), Some(plan)) = (&app.ingest.running, &app.ingest.plan) {
            ui.add_space(S3);
            // Finished: the card's border flashes accent and settles (600 ms).
            let flash = 1.0 - super::motion::progress(r.done_since, app.anim(0.6));
            let border = super::motion::lerp_color(t.line, t.accent, flash);
            egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0_f32, border)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(S4 as i8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                super::eyebrow(ui, t, "Progress");
                let total = plan.entries.len().max(1);
                let done = r.statuses.iter().filter(|s| matches!(s, FileStatus::Verified | FileStatus::Failed)).count();
                // The bar follows bytes, not files, so one big NEF does not stall it.
                let dests = 1 + plan.entries.first().is_some_and(|e| e.secondary.is_some()) as u64;
                let frac = if r.done.is_some() {
                    1.0
                } else {
                    (r.bytes_done as f64 / (plan.total_bytes * dests).max(1) as f64) as f32
                };
                let frac = ui.ctx().animate_value_with_time(egui::Id::new("ingest_progress"), frac, app.anim(0.16));
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(3), t.raised);
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, 6.0)), CornerRadius::same(3), t.accent);
                let failed = r.statuses.iter().filter(|s| **s == FileStatus::Failed).count();
                // Speed = bytes verified per second since the run started (copy +
                // read-back hash, so it is the real end-to-end rate), with a rough
                // time left from the bytes still to go.
                let total_bytes = plan.total_bytes * dests;
                let mut line = format!("{done} of {total} files · {failed} failed");
                if r.done.is_none() {
                    let secs = r.started.elapsed().as_secs_f64();
                    if r.bytes_done > 0 && secs > 0.5 {
                        let rate = r.bytes_done as f64 / secs;
                        let left = total_bytes.saturating_sub(r.bytes_done) as f64 / rate;
                        let left = if left < 60.0 {
                            format!("about {} s left", (left.ceil() as u64).max(1))
                        } else {
                            format!("about {} min left", (left / 60.0).ceil() as u64)
                        };
                        line = format!(
                            "{line} · {} of {} · {:.0} MB/s · {left}",
                            human_bytes(r.bytes_done),
                            human_bytes(total_bytes),
                            rate / 1e6
                        );
                    } else {
                        line = format!("{line} · {} of {}", human_bytes(r.bytes_done), human_bytes(total_bytes));
                    }
                }
                ui.label(RichText::new(line).size(12.0).color(t.text2));
                if let Some(rep) = &r.done {
                    let ok = rep.all_verified(plan);
                    let msg = if rep.cancelled {
                        format!("Cancelled after {} files.", rep.outcomes.len())
                    } else if ok {
                        format!(
                            "Every file verified at every destination · {} in {:.0} s · {:.0} MB/s",
                            human_bytes(rep.bytes_copied),
                            rep.elapsed_secs,
                            rep.bytes_copied as f64 / rep.elapsed_secs.max(0.001) / 1e6
                        )
                    } else {
                        format!("{} file{} failed. The card has not been changed; fix the destination and ingest again (only new photos will copy).", rep.files_failed(), if rep.files_failed() == 1 { "" } else { "s" })
                    };
                    ui.label(RichText::new(msg).size(13.0).color(if ok { t.text } else { t.reject }));
                    if let Some((files, failed)) = r.pad {
                        ui.label(RichText::new(format!("assignment pad applied → {files} files{}", if failed > 0 { format!(", {failed} failed") } else { String::new() })).size(11.0).color(t.text2));
                    }
                    ui.horizontal(|ui| {
                        if ok && ui.button("Open the ingested folder").clicked() {
                            cmds.push(Command::IngestOpenResult);
                        }
                        if ok && app.ingest.opts.source.starts_with("/Volumes") && ui.button("Eject the card").clicked() {
                            cmds.push(Command::IngestEject);
                        }
                    });
                }
                ui.add_space(S2);
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                    let green = Color32::from_rgb(0x5F, 0xBF, 0x8A);
                    for (i, e) in plan.entries.iter().enumerate() {
                        // Rows arrive staggered when the run starts; the ✓ fades
                        // from the copying colour to green when a file verifies.
                        let row_k = super::motion::stagger(Some(r.started), i, 0.015, app.anim(0.16), 20);
                        let ok = ui.ctx().animate_bool_with_time(
                            egui::Id::new(("ingest-ok", i)),
                            r.statuses[i] == FileStatus::Verified,
                            app.anim(0.16),
                        );
                        let (glyph, color) = match r.statuses[i] {
                            FileStatus::Pending => ("·", t.muted),
                            FileStatus::Copying => ("→", t.accent),
                            FileStatus::Verified => ("✓", super::motion::lerp_color(t.accent, green, ok)),
                            FileStatus::Failed => ("✕", t.reject),
                        };
                        let color = color.gamma_multiply(row_k);
                        let name = e.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let line = match &r.errors[i] {
                            Some(err) => format!("{glyph} {name}   {err}"),
                            None => format!("{glyph} {name}"),
                        };
                        ui.label(RichText::new(line).monospace().size(10.5).color(color));
                    }
                });
            });
        }
    });

    if opts_changed {
        app.ingest_options_changed();
    }
    for c in cmds {
        app.dispatch(ui.ctx(), c);
    }
}
