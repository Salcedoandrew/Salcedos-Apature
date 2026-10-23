use egui::{Align, Layout, RichText};

use super::tokens::{Theme, S2, S3, S4, TOOLBAR_H};
use crate::app::App;
use crate::commands::{Command, Workspace};
use crate::filter::{Filter, Sort};

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    let mut cmds = Vec::new();
    egui::TopBottomPanel::top("toolbar")
        .exact_height(TOOLBAR_H)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin::symmetric(S4 as i8, 0))
                .stroke(egui::Stroke::NONE),
        )
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = S3;
                let sb = super::icons::button(
                    ui,
                    t,
                    super::icons::Icon::Sidebar,
                    app.sidebar_shown(),
                    "Show or hide the sidebar (⌃⌘S; Tab shows or hides both side panels)",
                );
                if sb.clicked() {
                    cmds.push(Command::ToggleSidebar);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for w in [
                        Workspace::Sheet,
                        Workspace::Preview,
                        Workspace::Caption,
                        Workspace::Ingest,
                        Workspace::Export,
                    ] {
                        let selected =
                            app.workspace == w || (w == Workspace::Ingest && app.ingest_window);
                        match w.available() {
                            Ok(()) => {
                                if super::icons::workspace_button(ui, t, w, selected, true)
                                    .clicked()
                                {
                                    cmds.push(Command::SetWorkspace(w));
                                }
                            }
                            Err(why) => {
                                super::icons::workspace_button(ui, t, w, false, false)
                                    .on_hover_text(why);
                            }
                        }
                    }
                });

                // Filter button + popup, then one chip per active clause.
                let active = app.filter.is_active();
                let fb = super::icons::button(
                    ui,
                    t,
                    super::icons::Icon::Funnel,
                    active,
                    "Filter by rating, class, tag (⌥⌘F clears)",
                );
                if fb.clicked() {
                    app.filter_popup_open = !app.filter_popup_open;
                }
                if app.filter_popup_open {
                    let mut draft = app.filter.clone();
                    let mut changed = false;
                    let area = egui::Area::new(egui::Id::new("filter_popup"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(fb.rect.left_bottom() + egui::vec2(0.0, S2))
                        .show(ctx, |ui| {
                            egui::Frame::popup(ui.style())
                                .fill(t.raised)
                                .show(ui, |ui| {
                                    ui.set_width(260.0);
                                    ui.spacing_mut().item_spacing.y = S2;
                                    super::eyebrow(ui, t, "Rating at least");
                                    ui.horizontal(|ui| {
                                        for r in 0..=5u8 {
                                            let on = draft.min_rating.unwrap_or(0) == r;
                                            let label = if r == 0 {
                                                "any".to_string()
                                            } else {
                                                "★".repeat(r as usize)
                                            };
                                            if ui.selectable_label(on, label).clicked() {
                                                draft.min_rating =
                                                    if r == 0 { None } else { Some(r) };
                                                changed = true;
                                            }
                                        }
                                    });
                                    if ui
                                        .checkbox(&mut draft.hide_rejected, "Hide rejected")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    ui.add_space(S2);
                                    super::eyebrow(ui, t, "Color class");
                                    ui.horizontal_wrapped(|ui| {
                                        for c in 0..=8u8 {
                                            let on = draft.classes.contains(&c);
                                            let (label, color) = if c == 0 {
                                                ("none".to_string(), t.muted)
                                            } else {
                                                let cc = apature_core::ColorClass(c);
                                                (
                                                    format!(
                                                        "{c} {}",
                                                        app.scheme.name(cc).unwrap_or("")
                                                    ),
                                                    t.class_color(&app.scheme, cc)
                                                        .unwrap_or(t.muted),
                                                )
                                            };
                                            if ui
                                                .selectable_label(
                                                    on,
                                                    RichText::new(format!("● {label}"))
                                                        .color(color),
                                                )
                                                .clicked()
                                            {
                                                if on {
                                                    draft.classes.remove(&c);
                                                } else {
                                                    draft.classes.insert(c);
                                                }
                                                changed = true;
                                            }
                                        }
                                    });
                                    ui.add_space(S2);
                                    super::eyebrow(ui, t, "Tag");
                                    ui.horizontal(|ui| {
                                        for (v, label) in [
                                            (None, "any"),
                                            (Some(true), "tagged"),
                                            (Some(false), "untagged"),
                                        ] {
                                            if ui
                                                .selectable_label(draft.tagged == v, label)
                                                .clicked()
                                            {
                                                draft.tagged = v;
                                                changed = true;
                                            }
                                        }
                                    });
                                    if ui
                                        .checkbox(&mut draft.pairs_only, "RAW+JPEG pairs only")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    ui.add_space(S2);
                                    ui.horizontal(|ui| {
                                        if ui.button("Clear").clicked() {
                                            draft = Filter::default();
                                            changed = true;
                                        }
                                        if ui.button("Done").clicked() {
                                            app.filter_popup_open = false;
                                        }
                                    });
                                });
                        });
                    if changed {
                        cmds.push(Command::SetFilter(draft));
                    }
                    if ctx.input(|i| i.pointer.any_click())
                        && !area.response.hovered()
                        && !fb.hovered()
                    {
                        app.filter_popup_open = false;
                    }
                }
                for (label, clause) in app.filter.chips(&app.scheme) {
                    let a = ctx.animate_bool_with_time(
                        egui::Id::new(("chip", &label)),
                        true,
                        app.anim(0.12),
                    );
                    let chip = ui.add(
                        egui::Button::new(
                            RichText::new(format!("{label}  ×"))
                                .size(12.0)
                                .color(t.accent.gamma_multiply(a)),
                        )
                        .fill(t.accent_wash.gamma_multiply(a))
                        .stroke(egui::Stroke::new(1.0, t.accent.gamma_multiply(a))),
                    );
                    if chip.on_hover_text("Remove this filter").clicked() {
                        cmds.push(Command::ClearFilter(Some(clause)));
                    }
                }

                // Sort.
                let sort_label = format!(
                    "Sort: {}{}",
                    app.prefs.sort.label(),
                    if app.prefs.sort_reversed { " ↓" } else { "" }
                );
                egui::ComboBox::from_id_salt("sort")
                    .selected_text(sort_label)
                    .show_ui(ui, |ui| {
                        for s in Sort::ALL {
                            if ui
                                .selectable_label(app.prefs.sort == s, s.label())
                                .clicked()
                            {
                                cmds.push(Command::SetSort(s));
                            }
                        }
                        ui.separator();
                        if ui
                            .selectable_label(app.prefs.sort_reversed, "reverse order")
                            .clicked()
                        {
                            cmds.push(Command::ToggleSortReversed);
                        }
                    });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = S3;
                    let ins = super::icons::button(
                        ui,
                        t,
                        super::icons::Icon::Inspector,
                        app.inspector_shown(),
                        "Show or hide the inspector (⌥⌘I)",
                    );
                    if ins.clicked() {
                        cmds.push(Command::ToggleInspector);
                    }
                    let pending = app.pool.pending();
                    if pending > 0 {
                        ui.label(super::mono(format!("{pending} decoding"), 11.0, t.muted));
                    }
                    let mut cell = app.prefs.cell;
                    let slider = ui
                        .add(egui::Slider::new(&mut cell, 80.0..=600.0).show_value(false))
                        .on_hover_text("Thumbnail size (⌘= / ⌘-)");
                    if slider.changed() {
                        app.set_cell(cell);
                    }
                });
            });
        });
    for c in cmds {
        app.dispatch(ctx, c);
    }
}
