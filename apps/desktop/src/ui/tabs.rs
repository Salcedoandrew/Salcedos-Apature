//! Tab strip: one contact sheet per tab, Photo Mechanic style.

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::tokens::{Theme, S2, S3, S4};
use crate::app::App;
use crate::commands::Command;

pub const TABS_H: f32 = 32.0;

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    if app.tabs.is_empty() {
        return;
    }
    let mut cmds: Vec<Command> = Vec::new();
    egui::TopBottomPanel::top("tabs")
        .exact_height(TABS_H)
        .frame(
            egui::Frame::NONE
                .fill(t.window)
                .inner_margin(egui::Margin::symmetric(S3 as i8, 0))
                .stroke(egui::Stroke::NONE),
        )
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = S2;
                let n = app.tabs.len();
                let avail = ui.available_width() - 36.0;
                let tab_w = ((avail - S2 * (n as f32 - 1.0)) / n as f32).clamp(90.0, 220.0);
                for i in 0..n {
                    let active = i == app.active;
                    let name = app.tabs[i]
                        .folder
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| app.tabs[i].folder.display().to_string());
                    let count = if active {
                        app.items.len()
                    } else {
                        app.tabs[i].count
                    };
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(tab_w, TABS_H - 8.0), Sense::click());
                    let p = ui.painter();
                    if active {
                        // The highlight slides to the active tab; nothing else moves.
                        let x = ctx.animate_value_with_time(
                            egui::Id::new("tab_highlight_x"),
                            rect.left(),
                            app.anim(0.16),
                        );
                        let hl = Rect::from_min_size(Pos2::new(x, rect.top()), rect.size());
                        p.rect_filled(hl, CornerRadius::same(6), t.panel);
                        p.rect_stroke(
                            hl,
                            CornerRadius::same(6),
                            Stroke::new(1.0, t.line_strong),
                            egui::StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        p.rect_filled(rect, CornerRadius::same(6), t.raised);
                    }
                    let fg = if active { t.text } else { t.text2 };
                    // Close box on the left, like Safari.
                    let close = Rect::from_center_size(
                        Pos2::new(rect.left() + 14.0, rect.center().y),
                        Vec2::splat(14.0),
                    );
                    let close_resp = ui.interact(close, resp.id.with("close"), Sense::click());
                    if active || resp.hovered() || close_resp.hovered() {
                        let c = if close_resp.hovered() {
                            t.text
                        } else {
                            t.muted
                        };
                        p.text(
                            close.center(),
                            Align2::CENTER_CENTER,
                            "×",
                            FontId::proportional(14.0),
                            c,
                        );
                    }
                    let label = format!("{name}  {count}");
                    let max_w = rect.width() - 30.0 - S2;
                    let galley =
                        ui.fonts_mut(|f| f.layout_no_wrap(label, FontId::proportional(12.5), fg));
                    let shown = if galley.size().x > max_w {
                        ellipsize(&name, max_w)
                    } else {
                        format!("{name}  {count}")
                    };
                    p.text(
                        Pos2::new(rect.left() + 28.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        shown,
                        FontId::proportional(12.5),
                        fg,
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("tab {name}, {count} photos"),
                        )
                    });
                    if close_resp.clicked() {
                        cmds.push(Command::SelectTab(i));
                        cmds.push(Command::CloseTab);
                    } else if resp.clicked() {
                        cmds.push(Command::SelectTab(i));
                    }
                    let _ = S4;
                }
                let plus = ui.add(egui::Button::new("+").frame(false)).on_hover_text(
                    if app.prefs.shortcut_layout == crate::prefs::ShortcutLayout::PhotoMechanic {
                        "Open a folder in a new tab (⌘N)"
                    } else {
                        "Open a folder in a new tab (⌘T)"
                    },
                );
                if plus.clicked() {
                    cmds.push(Command::PickFolder);
                }
            });
        });
    for c in cmds {
        app.dispatch(ctx, c);
    }
}

fn ellipsize(s: &str, max_w: f32) -> String {
    let n = ((max_w / 7.0) as usize).saturating_sub(1).max(4);
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}
