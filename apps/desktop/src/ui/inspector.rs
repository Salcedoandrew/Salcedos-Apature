//! Right-side inspector: preview on top, then collapsible sections.

use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Vec2};

use apature_core::{ColorClass, Kind, Rating};

use super::tokens::{Theme, INSPECTOR_RANGE, S1, S2, S3, S4};
use crate::app::App;
use crate::commands::Command;

fn kv(ui: &mut egui::Ui, t: &Theme, rows: &[(&str, String)]) {
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .spacing([S2, S1])
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.add_sized(
                    [72.0, 14.0],
                    egui::Label::new(
                        RichText::new(k.to_uppercase())
                            .monospace()
                            .size(9.5)
                            .color(t.muted),
                    )
                    .halign(egui::Align::LEFT),
                );
                ui.add(
                    egui::Label::new(RichText::new(v).monospace().size(11.0).color(t.text2)).wrap(),
                );
                ui.end_row();
            }
        });
}

fn section<R>(
    ui: &mut egui::Ui,
    t: &Theme,
    title: &str,
    trailing: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) {
    let id = ui.make_persistent_id(title);
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let header = state.show_header(ui, |ui| {
        super::eyebrow(ui, t, title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(trailing).monospace().size(9.5).color(t.muted));
        });
    });
    header.body(|ui| {
        ui.add_space(S1);
        add(ui);
    });
    ui.add_space(S2);
    // Hairline between sections, like the sheet frames.
    let r = ui.available_rect_before_wrap();
    ui.painter()
        .hline(r.x_range(), r.top(), egui::Stroke::new(1.0, t.line));
    ui.add_space(S2);
}

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    let mut cmds: Vec<Command> = Vec::new();
    let resp = egui::SidePanel::right("inspector")
        .resizable(true)
        .default_width(app.prefs.inspector_width)
        .width_range(INSPECTOR_RANGE)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin::same(S3 as i8))
                .stroke(egui::Stroke::NONE),
        )
        .show_animated(ctx, app.prefs.inspector_visible, |ui| {
            ui.spacing_mut().item_spacing.y = S2;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let Some(idx) = app.items.get(app.cursor).map(|_| app.cursor) else {
                        ui.label(RichText::new("No folder open").color(t.muted));
                        return;
                    };
                    // Preview on top.
                    let w = ui.available_width();
                    // A sharp preview for the inspector: decode at panel size × scale.
                    let want = (w * ui.ctx().pixels_per_point()).ceil() as u32;
                    app.request_preview(idx, want, 1);
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(w, w * 2.0 / 3.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::ZERO, t.photo_cell);
                    let tex = app
                        .previews
                        .get(&idx)
                        .map(|(h, ..)| h)
                        .or(app.thumbs[idx].as_ref());
                    let mut label = String::from("loading…");
                    if let Some(tex) = tex {
                        let size = tex.size_vec2();
                        let s = (rect.width() / size.x).min(rect.height() / size.y);
                        let fitted = Rect::from_center_size(rect.center(), size * s);
                        ui.painter().image(
                            tex.id(),
                            fitted,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        label = match app.previews.get(&idx) {
                            Some((_, ls, _)) => format!("{ls} px preview"),
                            None => "thumbnail".into(),
                        };
                    }
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(super::mono(label, 10.0, t.muted));
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(super::mono(
                                            app.items[idx].display_name(),
                                            10.0,
                                            t.muted,
                                        ))
                                        .truncate(),
                                    );
                                },
                            );
                        });
                    });
                    ui.add_space(S4 - S2);

                    section(ui, t, "Info", "", |ui| {
                        // One grid for every row, so the gaps are even: EXIF,
                        // then camera details, then the files.
                        let mut rows: Vec<(&str, String)> = Vec::new();
                        match &app.exif[idx] {
                            Some(e) => {
                                let cam = e.camera_line();
                                if !cam.is_empty() {
                                    rows.push(("Camera", cam));
                                }
                                if let Some(l) = &e.lens {
                                    rows.push(("Lens", l.clone()));
                                }
                                let ex = e.exposure_line();
                                if !ex.is_empty() {
                                    rows.push(("Exposure", ex));
                                }
                                if let Some(d) = &e.date_time_original {
                                    rows.push(("Taken", d.clone()));
                                }
                                rows.extend(e.details.rows());
                                if rows.is_empty() {
                                    ui.label(
                                        RichText::new("No EXIF in this file")
                                            .size(11.0)
                                            .color(t.muted),
                                    );
                                }
                            }
                            None => {
                                ui.label(RichText::new("reading…").size(11.0).color(t.muted));
                            }
                        }
                        // File rows (formerly the Files section).
                        let it = &app.items[idx];
                        for v in it.variants() {
                            let k = match v.kind {
                                Kind::Raw => "RAW",
                                Kind::Jpeg => "JPEG",
                                Kind::Heic => "HEIC",
                            };
                            rows.push((
                                k,
                                format!("{} · {:.1} MB", v.file_name(), v.size as f64 / 1e6),
                            ));
                        }
                        let sc = it.sidecar_path();
                        rows.push((
                            "Sidecar",
                            if sc.is_file() {
                                format!(
                                    "{} · present",
                                    sc.file_name()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_default()
                                )
                            } else {
                                "none yet".into()
                            },
                        ));
                        if !app.conflicts[idx].is_empty() {
                            rows.push(("Conflict", app.conflicts[idx].join("; ")));
                        }
                        if let Some(s) = app.deliver.sent.get(it.path()) {
                            rows.push(("Sent", format!("{} · {}", s.target, s.at)));
                        }
                        if sc.is_file() {
                            if let Ok(e) = apature_metadata::edits::read(&sc) {
                                let mut parts = Vec::new();
                                if let Some(o) = e.orientation {
                                    parts.push(match o {
                                        6 => "turned right".to_string(),
                                        8 => "turned left".to_string(),
                                        3 => "upside down".to_string(),
                                        other => format!("orientation {other}"),
                                    });
                                }
                                if e.crop.as_ref().is_some_and(|c| !c.is_full()) {
                                    parts.push("cropped".into());
                                }
                                if e.straightened_crop {
                                    parts.push("straightened (export only)".into());
                                }
                                if !parts.is_empty() {
                                    rows.push(("Edits", parts.join(" · ")));
                                }
                            }
                        }
                        kv(ui, t, &rows);
                        let _ = Rating(0);
                    });

                    let targets = app.selection.targets(idx);
                    let mixed = |f: &dyn Fn(&apature_core::Marks) -> i32| {
                        targets
                            .iter()
                            .any(|&i| f(&app.marks[i]) != f(&app.marks[idx]))
                    };
                    let trailing = if targets.len() > 1 {
                        format!("{} selected", targets.len())
                    } else {
                        String::new()
                    };
                    section(ui, t, "Marks", &trailing, |ui| {
                        let m = app.marks[idx];
                        let rating_mixed = mixed(&|m| m.rating.0 as i32);
                        let class_mixed = mixed(&|m| m.color_class.0 as i32);
                        let tag_mixed = mixed(&|m| m.tagged as i32);
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [64.0, 18.0],
                                egui::Label::new(
                                    RichText::new("RATING").monospace().size(9.5).color(t.muted),
                                )
                                .halign(egui::Align::LEFT),
                            );
                            for r in 1..=5i8 {
                                let on = m.rating.0 >= r;
                                let txt = RichText::new(if on { "★" } else { "☆" })
                                    .size(15.0)
                                    .color(if on { t.star } else { t.muted });
                                let b = ui.add(egui::Button::new(txt).frame(false)).on_hover_text(
                                    format!("{r} star{}", if r == 1 { "" } else { "s" }),
                                );
                                if b.clicked() {
                                    cmds.push(Command::SetRating(if m.rating.0 == r {
                                        0
                                    } else {
                                        r
                                    }));
                                }
                            }
                            let rj = ui
                                .add(
                                    egui::Button::new(RichText::new("✕").size(13.0).color(
                                        if m.rating.is_rejected() {
                                            t.reject
                                        } else {
                                            t.muted
                                        },
                                    ))
                                    .frame(false),
                                )
                                .on_hover_text("Reject (X)");
                            if rj.clicked() {
                                cmds.push(Command::SetRating(if m.rating.is_rejected() {
                                    0
                                } else {
                                    -1
                                }));
                            }
                            if rating_mixed {
                                ui.label(
                                    RichText::new("mixed").size(10.0).italics().color(t.muted),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [64.0, 18.0],
                                egui::Label::new(
                                    RichText::new("CLASS").monospace().size(9.5).color(t.muted),
                                )
                                .halign(egui::Align::LEFT),
                            );
                            let current = if class_mixed {
                                "mixed".to_string()
                            } else {
                                app.scheme
                                    .name(m.color_class)
                                    .map(|n| format!("{} {n}", m.color_class.0))
                                    .unwrap_or_else(|| "None".into())
                            };
                            egui::ComboBox::from_id_salt("class")
                                .selected_text(current)
                                .width(140.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(!m.color_class.is_set(), "None")
                                        .clicked()
                                    {
                                        cmds.push(Command::SetClass(0));
                                    }
                                    for i in 1..=8u8 {
                                        let c = ColorClass(i);
                                        let name = app.scheme.name(c).unwrap_or("");
                                        let col = t.class_color(&app.scheme, c).unwrap_or(t.muted);
                                        let txt = RichText::new(format!("● {i} {name}")).color(col);
                                        if ui.selectable_label(m.color_class == c, txt).clicked() {
                                            cmds.push(Command::SetClass(i));
                                        }
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [64.0, 18.0],
                                egui::Label::new(
                                    RichText::new("TAGGED").monospace().size(9.5).color(t.muted),
                                )
                                .halign(egui::Align::LEFT),
                            );
                            let mut tagged = m.tagged;
                            if ui
                                .checkbox(&mut tagged, if tag_mixed { "mixed" } else { "" })
                                .on_hover_text("Toggle tag (T)")
                                .changed()
                            {
                                cmds.push(Command::ToggleTag);
                            }
                        });
                    });

                    let needs = app
                        .inspector_iptc
                        .as_ref()
                        .map(|(i, _)| *i != idx)
                        .unwrap_or(true);
                    if needs {
                        app.inspector_iptc =
                            Some((idx, apature_metadata::items::read_iptc(&app.items[idx])));
                    }
                    let iptc = app
                        .inspector_iptc
                        .as_ref()
                        .map(|(_, i)| i.clone())
                        .unwrap_or_default();
                    section(ui, t, "Caption", "⌘⇧3", |ui| {
                        if let Some(h) = &iptc.headline {
                            ui.label(RichText::new(h).size(12.0).color(t.text));
                        }
                        match &iptc.description {
                            Some(d) if !d.is_empty() => {
                                ui.add(
                                    egui::Label::new(RichText::new(d).size(11.5).color(t.text2))
                                        .wrap(),
                                );
                            }
                            _ => {
                                ui.label(
                                    RichText::new(
                                        "No caption yet. ⌘⇧3 opens the Caption workspace.",
                                    )
                                    .size(11.0)
                                    .color(t.muted),
                                );
                            }
                        }
                        let mut small = Vec::new();
                        if let Some(c) = &iptc.credit {
                            small.push(c.clone());
                        }
                        if let Some(k) = &iptc.keywords {
                            if !k.is_empty() {
                                small.push(format!(
                                    "{} keyword{}",
                                    k.len(),
                                    if k.len() == 1 { "" } else { "s" }
                                ));
                            }
                        }
                        if !small.is_empty() {
                            ui.label(
                                RichText::new(small.join(" · "))
                                    .monospace()
                                    .size(10.0)
                                    .color(t.muted),
                            );
                        }
                    });
                });
        });
    let Some(resp) = resp else {
        for c in cmds {
            app.dispatch(ctx, c);
        }
        return;
    };
    let w = resp.response.rect.width();
    if app.prefs.inspector_visible
        && (w - app.prefs.inspector_width).abs() > 0.5
        && INSPECTOR_RANGE.contains(&w)
    {
        app.prefs.inspector_width = w;
        app.prefs_dirty = true;
    }
    let r = resp.response.rect;
    ctx.layer_painter(egui::LayerId::background()).line_segment(
        [r.left_top(), r.left_bottom()],
        egui::Stroke::new(1.0, t.line),
    );
    for c in cmds {
        app.dispatch(ctx, c);
    }
}
