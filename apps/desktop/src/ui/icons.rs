//! Toolbar icons drawn with the painter, so they follow the tokens and need
//! no icon font. Each is a 16 pt glyph inside a 28×24 button.

use egui::{CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use super::tokens::Theme;
use crate::platform::VolumeKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sidebar,
    Inspector,
    Funnel,
}

pub fn button(
    ui: &mut egui::Ui,
    t: &Theme,
    icon: Icon,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 24.0), Sense::click());
    let p = ui.painter();
    focus_ring(p, rect, t, &resp);
    if selected {
        p.rect_filled(rect, CornerRadius::same(6), t.accent_wash);
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, t.accent),
            StrokeKind::Inside,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), t.raised);
    }
    let fg = if selected { t.accent } else { t.text2 };
    let s = Stroke::new(1.5_f32, fg);
    let g = Rect::from_center_size(rect.center(), Vec2::new(16.0, 13.0));
    match icon {
        Icon::Sidebar => {
            p.rect_stroke(g, CornerRadius::same(2), s, StrokeKind::Inside);
            let x = g.left() + 6.0;
            p.line_segment([Pos2::new(x, g.top()), Pos2::new(x, g.bottom())], s);
        }
        Icon::Inspector => {
            p.rect_stroke(g, CornerRadius::same(2), s, StrokeKind::Inside);
            let x = g.right() - 6.0;
            p.line_segment([Pos2::new(x, g.top()), Pos2::new(x, g.bottom())], s);
        }
        Icon::Funnel => {
            let top_l = Pos2::new(g.left(), g.top() + 1.0);
            let top_r = Pos2::new(g.right(), g.top() + 1.0);
            let mid_l = Pos2::new(g.center().x - 2.5, g.center().y + 1.0);
            let mid_r = Pos2::new(g.center().x + 2.5, g.center().y + 1.0);
            let bot_l = Pos2::new(g.center().x - 2.5, g.bottom());
            let bot_r = Pos2::new(g.center().x + 2.5, g.bottom() - 2.0);
            p.line_segment([top_l, top_r], s);
            p.line_segment([top_l, mid_l], s);
            p.line_segment([top_r, mid_r], s);
            p.line_segment([mid_l, bot_l], s);
            p.line_segment([mid_r, bot_r], s);
            p.line_segment([bot_l, bot_r], s);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip));
    resp.on_hover_text(tooltip)
}

/// A 16×12 pt drive glyph: SD card for cameras and cards, a drive with an
/// activity dot for external disks, a drive with a slot line for the Mac's own.
pub fn volume_glyph(p: &egui::Painter, center: Pos2, kind: VolumeKind, color: egui::Color32) {
    let s = Stroke::new(1.4_f32, color);
    match kind {
        VolumeKind::Camera => {
            // Card: 10×13, top-right corner cut, three contact ticks at the top.
            let r = Rect::from_center_size(center, Vec2::new(10.0, 13.0));
            let cut = 3.0;
            let pts = vec![
                Pos2::new(r.left(), r.top()),
                Pos2::new(r.right() - cut, r.top()),
                Pos2::new(r.right(), r.top() + cut),
                Pos2::new(r.right(), r.bottom()),
                Pos2::new(r.left(), r.bottom()),
            ];
            p.add(egui::Shape::closed_line(pts, s));
            for i in 0..3 {
                let x = r.left() + 2.5 + i as f32 * 2.5;
                p.line_segment(
                    [Pos2::new(x, r.top() + 2.0), Pos2::new(x, r.top() + 4.5)],
                    Stroke::new(1.0_f32, color),
                );
            }
        }
        VolumeKind::External | VolumeKind::System => {
            let r = Rect::from_center_size(center, Vec2::new(16.0, 9.0));
            p.rect_stroke(r, CornerRadius::same(2), s, StrokeKind::Inside);
            p.circle_filled(Pos2::new(r.right() - 3.5, r.center().y), 1.3, color);
        }
        VolumeKind::Internal => {
            let r = Rect::from_center_size(center, Vec2::new(16.0, 9.0));
            p.rect_stroke(r, CornerRadius::same(2), s, StrokeKind::Inside);
            p.line_segment(
                [
                    Pos2::new(r.left() + 3.0, r.center().y),
                    Pos2::new(r.right() - 6.0, r.center().y),
                ],
                Stroke::new(1.0_f32, color),
            );
            p.circle_filled(Pos2::new(r.right() - 3.5, r.center().y), 1.0, color);
        }
    }
}

/// A selectable chip with a drive glyph and the volume name (Ingest source row).
pub fn volume_chip(
    ui: &mut egui::Ui,
    t: &Theme,
    kind: VolumeKind,
    name: &str,
    selected: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(name.to_string(), font, t.text));
    let w = 8.0 + 16.0 + 6.0 + galley.size().x + 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
    let p = ui.painter();
    focus_ring(p, rect, t, &resp);
    if selected {
        p.rect_filled(rect, CornerRadius::same(6), t.accent_wash);
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, t.accent),
            StrokeKind::Inside,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), t.raised);
    } else {
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, t.line),
            StrokeKind::Inside,
        );
    }
    let fg = if selected { t.accent } else { t.text2 };
    volume_glyph(
        p,
        Pos2::new(rect.left() + 8.0 + 8.0, rect.center().y),
        kind,
        fg,
    );
    p.galley(
        Pos2::new(
            rect.left() + 8.0 + 16.0 + 6.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        if selected { t.text } else { t.text2 },
    );
    resp.on_hover_text(match kind {
        VolumeKind::Camera => "Camera or memory card",
        VolumeKind::External => "External drive",
        VolumeKind::Internal => "This Mac",
        VolumeKind::System => "System volume",
    })
}

/// A plain selectable chip (no glyph): the one style for DAYS, FILES, export
/// scope and size choices, matching `volume_chip`.
pub fn chip(ui: &mut egui::Ui, t: &Theme, label: &str, selected: bool) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font, t.text));
    let w = 10.0 + galley.size().x + 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
    let p = ui.painter();
    focus_ring(p, rect, t, &resp);
    if selected {
        p.rect_filled(rect, CornerRadius::same(6), t.accent_wash);
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, t.accent),
            StrokeKind::Inside,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), t.raised);
    } else {
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, t.line),
            StrokeKind::Inside,
        );
    }
    p.galley(
        Pos2::new(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        if selected { t.text } else { t.text2 },
    );
    resp
}

/// Workspace glyphs, 14×11 pt, drawn in the stroke style of the drive icons.
pub fn workspace_glyph(
    p: &egui::Painter,
    center: Pos2,
    w: crate::commands::Workspace,
    color: egui::Color32,
) {
    use crate::commands::Workspace;
    let s = Stroke::new(1.3_f32, color);
    let g = Rect::from_center_size(center, Vec2::new(14.0, 11.0));
    match w {
        Workspace::Sheet => {
            // 2×2 grid of cells
            let cw = (g.width() - 2.0) / 2.0;
            let ch = (g.height() - 2.0) / 2.0;
            for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let r = Rect::from_min_size(
                    Pos2::new(g.left() + i * (cw + 2.0), g.top() + j * (ch + 2.0)),
                    Vec2::new(cw, ch),
                );
                p.rect_stroke(r, CornerRadius::same(1), s, StrokeKind::Inside);
            }
        }
        Workspace::Preview => {
            // Frame with a mountain and a sun
            p.rect_stroke(g, CornerRadius::same(2), s, StrokeKind::Inside);
            p.line_segment(
                [
                    Pos2::new(g.left() + 2.0, g.bottom() - 2.0),
                    Pos2::new(g.left() + 6.0, g.center().y),
                ],
                s,
            );
            p.line_segment(
                [
                    Pos2::new(g.left() + 6.0, g.center().y),
                    Pos2::new(g.right() - 2.0, g.bottom() - 2.0),
                ],
                s,
            );
            p.circle_filled(Pos2::new(g.right() - 4.0, g.top() + 3.5), 1.2, color);
        }
        Workspace::Caption => {
            // Three text lines, the last one short
            for (k, frac) in [(0.0, 1.0), (1.0, 1.0), (2.0, 0.55)] {
                let y = g.top() + 1.5 + k * 4.0;
                p.line_segment(
                    [
                        Pos2::new(g.left(), y),
                        Pos2::new(g.left() + g.width() * frac, y),
                    ],
                    s,
                );
            }
        }
        Workspace::Ingest => {
            // Card with an arrow coming out to the right
            let card = Rect::from_min_size(g.left_top(), Vec2::new(8.0, g.height()));
            p.rect_stroke(card, CornerRadius::same(1), s, StrokeKind::Inside);
            let y = g.center().y;
            p.line_segment(
                [Pos2::new(card.right() + 1.5, y), Pos2::new(g.right(), y)],
                s,
            );
            p.line_segment(
                [Pos2::new(g.right() - 3.0, y - 3.0), Pos2::new(g.right(), y)],
                s,
            );
            p.line_segment(
                [Pos2::new(g.right() - 3.0, y + 3.0), Pos2::new(g.right(), y)],
                s,
            );
        }
        Workspace::Export => {
            // Tray with an arrow going up and out
            p.line_segment(
                [
                    Pos2::new(g.left(), g.bottom() - 3.0),
                    Pos2::new(g.left(), g.bottom()),
                ],
                s,
            );
            p.line_segment(
                [
                    Pos2::new(g.left(), g.bottom()),
                    Pos2::new(g.right(), g.bottom()),
                ],
                s,
            );
            p.line_segment(
                [
                    Pos2::new(g.right(), g.bottom()),
                    Pos2::new(g.right(), g.bottom() - 3.0),
                ],
                s,
            );
            let x = g.center().x;
            p.line_segment([Pos2::new(x, g.top()), Pos2::new(x, g.bottom() - 3.0)], s);
            p.line_segment(
                [Pos2::new(x - 3.0, g.top() + 3.0), Pos2::new(x, g.top())],
                s,
            );
            p.line_segment(
                [Pos2::new(x + 3.0, g.top() + 3.0), Pos2::new(x, g.top())],
                s,
            );
        }
    }
}

/// A workspace tab button: glyph + label, selected state in the accent wash.
pub fn workspace_button(
    ui: &mut egui::Ui,
    t: &Theme,
    w: crate::commands::Workspace,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(w.label().to_string(), font, t.text));
    let width = 8.0 + 14.0 + 5.0 + galley.size().x + 9.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 25.0), Sense::click());
    let p = ui.painter();
    focus_ring(p, rect, t, &resp);
    if selected {
        p.rect_filled(rect, CornerRadius::same(6), t.accent_wash);
        p.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, t.accent),
            StrokeKind::Inside,
        );
    } else if resp.hovered() && enabled {
        p.rect_filled(rect, CornerRadius::same(6), t.raised);
    }
    let fg = if !enabled {
        t.muted
    } else if selected {
        t.accent
    } else {
        t.text2
    };
    workspace_glyph(
        p,
        Pos2::new(rect.left() + 8.0 + 7.0, rect.center().y),
        w,
        fg,
    );
    p.galley(
        Pos2::new(
            rect.left() + 8.0 + 14.0 + 5.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        if !enabled {
            t.muted
        } else if selected {
            t.text
        } else {
            t.text2
        },
    );
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, w.label())
    });
    resp
}

/// Keyboard focus ring for painter-drawn controls: a 2 px accent outline,
/// drawn under the control's own strokes so selection still reads.
pub fn focus_ring(p: &egui::Painter, rect: Rect, t: &Theme, resp: &egui::Response) {
    if resp.has_focus() {
        p.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(8),
            Stroke::new(2.0_f32, t.accent),
            StrokeKind::Outside,
        );
    }
}
