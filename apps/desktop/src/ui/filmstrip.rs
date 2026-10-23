use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use super::tokens::{Theme, FILMSTRIP_H, S2, S3};
use crate::app::App;
use crate::commands::Command;

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    if app.view.is_empty() {
        return;
    }
    let mut cmds = Vec::new();
    let panel = egui::TopBottomPanel::bottom("filmstrip")
        .exact_height(FILMSTRIP_H)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                .stroke(egui::Stroke::NONE),
        )
        .show_animated(ctx, app.prefs.filmstrip_visible, |ui| {
            let frame_h = FILMSTRIP_H - S2 * 2.0;
            let frame_w = frame_h * 1.5;
            let count = ((ui.available_width() + S2) / (frame_w + S2))
                .floor()
                .max(1.0) as usize;
            let start = app
                .cursor
                .saturating_sub(count / 2)
                .min(app.items.len().saturating_sub(count));
            let end = (start + count).min(app.items.len());
            let mut requests = Vec::new();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S2;
                for idx in start..end {
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(frame_w, frame_h), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(rect, CornerRadius::ZERO, t.photo_cell);
                    if let Some(tex) = &app.thumbs[idx] {
                        let size = tex.size_vec2();
                        let s = (rect.width() / size.x).min(rect.height() / size.y);
                        let fitted = Rect::from_center_size(rect.center(), size * s);
                        let tint = if idx == app.cursor {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(170)
                        };
                        p.image(
                            tex.id(),
                            fitted,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            tint,
                        );
                    } else {
                        requests.push(idx);
                    }
                    if idx == app.cursor {
                        p.rect_stroke(
                            rect.shrink(1.0),
                            CornerRadius::ZERO,
                            Stroke::new(2.0, t.accent),
                            StrokeKind::Inside,
                        );
                    }
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            app.cell_accessible_name(idx),
                        )
                    });
                    if resp.clicked() {
                        cmds.push(Command::Click {
                            idx,
                            shift: false,
                            command: false,
                        });
                    }
                }
            });
            for idx in requests {
                app.request_thumb(idx, 5);
            }
        });
    // Hairline along the filmstrip's own top edge, only while it is shown
    // (a fixed-height, full-width line used to cross the side panels).
    if let Some(panel) = panel {
        let r = panel.response.rect;
        ctx.layer_painter(egui::LayerId::background()).hline(
            r.x_range(),
            r.top(),
            Stroke::new(1.0, t.line),
        );
    }
    for c in cmds {
        app.dispatch(ctx, c);
    }
}
