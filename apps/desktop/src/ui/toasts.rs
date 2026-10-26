//! Toasts: short notices stacked above the status bar. Info ones leave on
//! their own after a few seconds; warnings and errors stay until dismissed,
//! because an unplugged card or a failed delivery must not scroll away.
//! Under Reduce Motion they appear and vanish without the fade.

use std::time::Instant;

use egui::{Align2, CornerRadius, RichText, Stroke};

use super::tokens::{Theme, S2, S3, S4, STATUS_H};
use crate::app::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub kind: Kind,
    pub since: Instant,
    /// An optional button: label and the command it dispatches.
    pub action: Option<(String, crate::commands::Command)>,
    /// A running update's progress line: one at a time, its text replaced as
    /// the work advances instead of stacking a toast per chunk.
    pub progress: bool,
}

impl Toast {
    pub fn sticky(&self) -> bool {
        self.kind != Kind::Info
    }
}

const INFO_SECS: f32 = 6.0;

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    // Expire info toasts.
    app.toasts
        .retain(|x| x.sticky() || x.since.elapsed().as_secs_f32() < INFO_SECS);
    if app.toasts.is_empty() {
        return;
    }
    let reduce = app.reduce_motion();
    let mut dismiss: Option<usize> = None;
    let mut act: Option<(usize, crate::commands::Command)> = None;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-S4, -(STATUS_H + S3)))
        .order(egui::Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.spacing_mut().item_spacing.y = S2;
            for (i, toast) in app.toasts.iter().enumerate() {
                let age = toast.since.elapsed().as_secs_f32();
                // Fade in over 160 ms, out over the last 600 ms (info only).
                let fade_in = if reduce {
                    1.0
                } else {
                    (age / 0.16).clamp(0.0, 1.0)
                };
                let fade_out = if toast.sticky() || reduce {
                    1.0
                } else {
                    ((INFO_SECS - age) / 0.6).clamp(0.0, 1.0)
                };
                let a = fade_in.min(fade_out);
                if a < 1.0 {
                    ctx.request_repaint();
                }
                let (stroke, fg) = match toast.kind {
                    Kind::Info => (t.accent, t.text),
                    Kind::Warn => (t.star, t.text),
                    Kind::Error => (t.reject, t.text),
                };
                egui::Frame::NONE
                    .fill(t.raised.gamma_multiply(a))
                    .stroke(Stroke::new(1.0, stroke.gamma_multiply(a)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&toast.text)
                                        .size(12.5)
                                        .color(fg.gamma_multiply(a)),
                                )
                                .wrap(),
                            );
                            if let Some((label, cmd)) = &toast.action {
                                if ui.small_button(label).clicked() {
                                    act = Some((i, cmd.clone()));
                                }
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("×").size(14.0).color(t.text2))
                                        .frame(false),
                                )
                                .on_hover_text("Dismiss")
                                .clicked()
                            {
                                dismiss = Some(i);
                            }
                        });
                    });
            }
        });
    if let Some((i, cmd)) = act {
        app.toasts.remove(i);
        app.dispatch(ctx, cmd);
    }
    if let Some(i) = dismiss {
        app.toasts.remove(i);
    }
    if app.toasts.iter().any(|x| !x.sticky()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
