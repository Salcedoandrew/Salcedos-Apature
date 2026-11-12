//! The contact sheet: a virtual grid of accessible cells over the view.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use super::tokens::{Theme, LABEL_H, S1, S3};
use crate::app::App;
use crate::commands::{Command, Workspace};

pub fn show(app: &mut App, ui: &mut egui::Ui, t: &Theme) {
    let gap = if t.dark { S3 } else { S3 + S1 };
    let pad = gap;
    let cell = app.prefs.cell;
    let avail_w = ui.available_width();
    let avail_h = ui.available_height();
    app.cols = (((avail_w - pad * 2.0 + gap) / (cell + gap)).floor() as usize).max(1);
    let rows = app.row_count();
    let row_h = cell + LABEL_H + gap;

    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if app.scroll_to_cursor && app.cols > 0 {
        let row = app.cursor_pos() / app.cols;
        let vis = &app.last_visible_rows;
        if row < vis.start || row + 1 > vis.end.saturating_sub(1) {
            let target = if row < vis.start {
                row as f32 * row_h
            } else {
                ((row + 1) as f32 * row_h - avail_h + pad).max(0.0)
            };
            area = area.vertical_scroll_offset(target);
        }
        app.scroll_to_cursor = false;
    }

    let mut cmds: Vec<Command> = Vec::new();
    let mut visible = 0..0;
    let cols = app.cols;
    let n = app.view.len();
    let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));

    area.show_rows(ui, row_h, rows, |ui, range| {
        visible = range.clone();
        ui.add_space(if range.start == 0 { pad } else { 0.0 });
        for row in range {
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.spacing_mut().item_spacing.x = gap;
                for col in 0..cols {
                    let p = row * cols + col;
                    if p >= n {
                        break;
                    }
                    let idx = app.view[p];
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(cell, cell + LABEL_H), Sense::click());
                    draw_cell(app, ui, t, rect, idx);
                    if idx == app.cursor {
                        app.cursor_cell_rect =
                            Some(Rect::from_min_size(rect.min, Vec2::splat(cell)));
                    }
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            app.cell_accessible_name(idx),
                        )
                    });
                    if resp.double_clicked() {
                        cmds.push(Command::Click {
                            idx,
                            shift: false,
                            command: false,
                        });
                        cmds.push(Command::SetWorkspace(Workspace::Preview));
                    } else if resp.clicked() {
                        cmds.push(Command::Click {
                            idx,
                            shift,
                            command,
                        });
                    }
                }
            });
            ui.add_space(gap);
        }
    });

    app.last_visible_rows = visible.clone();
    for c in cmds {
        app.dispatch(ui.ctx(), c);
    }
    app.schedule_thumbs(visible);
}

fn draw_cell(app: &App, ui: &egui::Ui, t: &Theme, rect: Rect, idx: usize) {
    let p = ui.painter();
    let cell = app.prefs.cell;
    let focused = idx == app.cursor;
    let selected = app.selection.contains(idx);
    let img_rect = Rect::from_min_size(rect.min, Vec2::splat(cell));
    p.rect_filled(img_rect, CornerRadius::ZERO, t.photo_cell);

    // A thumbnail that arrives after the cell was first drawn fades in
    // (120 ms); one already present on the first draw shows at once.
    let cell_id = egui::Id::new(("cell", idx)).with(app.folder.as_ref());
    let arrived = ui.ctx().animate_bool_with_time(
        cell_id.with("thumb"),
        app.thumbs[idx].is_some(),
        app.anim(0.12),
    );
    if let Some(tex) = &app.thumbs[idx] {
        let size = tex.size_vec2();
        let s = (cell / size.x).min(cell / size.y);
        let fitted = Rect::from_center_size(img_rect.center(), size * s);
        // Tab switch: the new sheet fades in over 160 ms.
        let a = super::motion::progress(app.sheet_fade_since, app.anim(0.16)) * arrived;
        p.image(
            tex.id(),
            fitted,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(a),
        );
    } else if app.thumb_err[idx].is_some() {
        p.text(
            img_rect.center(),
            Align2::CENTER_CENTER,
            "no preview",
            FontId::proportional(12.0),
            t.muted,
        );
    }

    let m = &app.marks[idx];
    if let Some(c) = t.class_color(&app.scheme, m.color_class) {
        let bar = Rect::from_min_max(
            Pos2::new(img_rect.left(), img_rect.bottom() - 3.0),
            img_rect.right_bottom(),
        );
        p.rect_filled(bar, CornerRadius::ZERO, c);
    }
    // Selection ring fades in and out (120 ms).
    let sel = ui.ctx().animate_bool_with_time(
        cell_id.with("sel"),
        selected && app.selection.len() > 1,
        app.anim(0.12),
    );
    if sel > 0.01 {
        p.rect_stroke(
            img_rect.shrink(0.5),
            CornerRadius::ZERO,
            Stroke::new(1.0_f32, t.select_photo.gamma_multiply(sel)),
            StrokeKind::Inside,
        );
    }
    if focused {
        // After a keyboard move the ring slides from the old cell (160 ms),
        // unless the old cell is far away (a scroll jump), then it cuts.
        let mut ring = img_rect.shrink(1.0);
        if let (Some(from), Some(since)) = (app.focus_from, app.focus_since) {
            let near = (from.center() - img_rect.center()).length() < cell * 4.0;
            if near && from != img_rect {
                let k = super::motion::progress(Some(since), app.anim(0.16));
                ring = super::motion::lerp_rect(from.shrink(1.0), ring, k);
            }
        }
        p.rect_stroke(
            ring,
            CornerRadius::ZERO,
            Stroke::new(2.0_f32, t.accent),
            StrokeKind::Inside,
        );
    }
    if let Some(members) = app.stacks.get(&idx) {
        let open = app.stacks_open.contains(&idx);
        let n = members.len() + 1;
        let label = if open {
            format!("⌄ {n}")
        } else {
            format!("× {n}")
        };
        let galley =
            ui.fonts_mut(|f| f.layout_no_wrap(label, FontId::monospace(10.5), Color32::WHITE));
        let r = Rect::from_min_size(
            Pos2::new(img_rect.left() + 6.0, img_rect.bottom() - 24.0),
            galley.size() + Vec2::new(10.0, 6.0),
        );
        p.rect_filled(
            r,
            CornerRadius::same(4),
            if open {
                t.accent
            } else {
                Color32::from_black_alpha(170)
            },
        );
        p.galley(r.min + Vec2::new(5.0, 3.0), galley, Color32::WHITE);
        if !open {
            // A stacked-paper hint: two offset edges behind the cell.
            p.rect_stroke(
                img_rect.translate(Vec2::new(3.0, -3.0)),
                CornerRadius::same(2),
                Stroke::new(1.0_f32, t.line_strong),
                StrokeKind::Outside,
            );
        }
    }
    if m.tagged {
        // Big enough to see from across the desk (a top Photo Mechanic request).
        let r = Rect::from_min_size(
            Pos2::new(img_rect.right() - 30.0, img_rect.top() + 6.0),
            Vec2::new(24.0, 24.0),
        );
        p.rect_filled(r, CornerRadius::same(5), t.tag);
        p.rect_stroke(
            r,
            CornerRadius::same(5),
            Stroke::new(1.0_f32, Color32::WHITE.gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            "T",
            FontId::proportional(14.0),
            Color32::WHITE,
        );
    }
    if app.items[idx].is_pair() {
        p.text(
            Pos2::new(img_rect.left() + 6.0, img_rect.top() + 5.0),
            Align2::LEFT_TOP,
            "R+J",
            FontId::monospace(9.0),
            t.text2,
        );
    }

    let name = app.items[idx].display_name();
    let y1 = img_rect.bottom() + S1 + 2.0;
    let name_color = if m.rating.is_rejected() {
        t.muted
    } else if selected {
        t.text
    } else {
        t.text2
    };
    let galley =
        ui.fonts_mut(|f| f.layout_no_wrap(name.clone(), FontId::monospace(10.5), name_color));
    let shown = if galley.size().x > cell {
        ellipsize(&name, cell, 6.3)
    } else {
        name
    };
    p.text(
        Pos2::new(rect.left(), y1),
        Align2::LEFT_TOP,
        &shown,
        FontId::monospace(10.5),
        name_color,
    );
    if m.rating.is_rejected() {
        let w = (shown.chars().count() as f32 * 6.3).min(cell);
        p.line_segment(
            [
                Pos2::new(rect.left(), y1 + 7.0),
                Pos2::new(rect.left() + w, y1 + 7.0),
            ],
            Stroke::new(1.0_f32, t.muted),
        );
    }
    let y2 = y1 + 15.0;
    let stars = super::rating_text(m.rating);
    if !stars.is_empty() {
        let c = if m.rating.is_rejected() {
            t.reject
        } else {
            t.star
        };
        p.text(
            Pos2::new(rect.left(), y2),
            Align2::LEFT_TOP,
            stars,
            FontId::monospace(10.0),
            c,
        );
    }
    if let (Some(c), Some(name)) = (
        t.class_color(&app.scheme, m.color_class),
        app.scheme.name(m.color_class),
    ) {
        let label = if cell >= 200.0 {
            format!("{} {name}", m.color_class.0)
        } else {
            m.color_class.0.to_string()
        };
        let g = ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), FontId::monospace(9.5), t.text));
        let w = g.size().x + 6.0 + 7.0 + 5.0 + 6.0;
        let chip = Rect::from_min_size(Pos2::new(rect.right() - w, y2 - 2.0), Vec2::new(w, 15.0));
        p.rect_filled(chip, CornerRadius::same(4), t.raised);
        p.circle_filled(Pos2::new(chip.left() + 6.0 + 3.5, chip.center().y), 3.5, c);
        p.galley(
            Pos2::new(
                chip.left() + 6.0 + 7.0 + 5.0,
                chip.center().y - g.size().y / 2.0,
            ),
            g,
            t.text,
        );
    }
}

fn ellipsize(s: &str, width: f32, char_w: f32) -> String {
    let n = ((width / char_w) as usize).saturating_sub(1).max(4);
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let keep = n - 1;
        let head = keep * 2 / 3;
        let tail = keep - head;
        let chars: Vec<char> = s.chars().collect();
        format!(
            "{}…{}",
            chars[..head].iter().collect::<String>(),
            chars[chars.len() - tail..].iter().collect::<String>()
        )
    }
}
