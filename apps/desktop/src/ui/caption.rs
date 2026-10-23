//! Caption workspace: the photograph beside an IPTC editor, with stationery
//! and code replacements. Text fields own the keyboard; ⌘⏎ saves and advances.

use egui::{Color32, CornerRadius, Key, Pos2, Rect, RichText, Vec2};

use apature_metadata::codes::{self, Vars};
use apature_metadata::Iptc;

use super::tokens::{Theme, S1, S2, S3, S4, S5};
use crate::app::App;
use crate::commands::Command;

pub const FIELDS_W: f32 = 380.0;

/// Editable strings for the fields we show. Converted to an `Iptc` of only
/// the changed fields on save, so untouched metadata is never rewritten.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftFields {
    pub caption: String,
    pub headline: String,
    pub keywords: String,
    pub sublocation: String,
    pub city: String,
    pub state: String,
    pub country: String,
    pub creator: String,
    pub credit: String,
    pub copyright: String,
    pub source: String,
    pub title: String,
    pub caption_writer: String,
    pub instructions: String,
    pub transmission_reference: String,
    /// Decimal degrees as typed; empty = none.
    pub gps_lat: String,
    pub gps_lon: String,
}

impl DraftFields {
    pub fn from_iptc(i: &Iptc) -> Self {
        let s = |o: &Option<String>| o.clone().unwrap_or_default();
        DraftFields {
            caption: s(&i.description),
            headline: s(&i.headline),
            keywords: i
                .keywords
                .as_ref()
                .map(|k| k.join("; "))
                .unwrap_or_default(),
            sublocation: s(&i.sublocation),
            city: s(&i.city),
            state: s(&i.state),
            country: s(&i.country),
            creator: i.creator.as_ref().map(|c| c.join("; ")).unwrap_or_default(),
            credit: s(&i.credit),
            copyright: s(&i.copyright),
            source: s(&i.source),
            title: s(&i.title),
            caption_writer: s(&i.caption_writer),
            instructions: s(&i.instructions),
            transmission_reference: s(&i.transmission_reference),
            gps_lat: i
                .gps_decimal()
                .map(|(la, _)| format!("{la:.6}"))
                .unwrap_or_default(),
            gps_lon: i
                .gps_decimal()
                .map(|(_, lo)| format!("{lo:.6}"))
                .unwrap_or_default(),
        }
    }

    /// Only the fields that differ from `original` become `Some`.
    pub fn changes_from(&self, original: &DraftFields) -> Iptc {
        let d = |a: &String, b: &String| {
            if a != b {
                Some(a.trim().to_string())
            } else {
                None
            }
        };
        let list = |a: &String, b: &String| {
            if a != b {
                Some(
                    a.split(';')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        };
        let mut out = Iptc {
            description: d(&self.caption, &original.caption),
            headline: d(&self.headline, &original.headline),
            keywords: list(&self.keywords, &original.keywords),
            sublocation: d(&self.sublocation, &original.sublocation),
            city: d(&self.city, &original.city),
            state: d(&self.state, &original.state),
            country: d(&self.country, &original.country),
            creator: list(&self.creator, &original.creator),
            credit: d(&self.credit, &original.credit),
            copyright: d(&self.copyright, &original.copyright),
            source: d(&self.source, &original.source),
            title: d(&self.title, &original.title),
            caption_writer: d(&self.caption_writer, &original.caption_writer),
            instructions: d(&self.instructions, &original.instructions),
            transmission_reference: d(
                &self.transmission_reference,
                &original.transmission_reference,
            ),
            ..Default::default()
        };
        if self.gps_lat != original.gps_lat || self.gps_lon != original.gps_lon {
            match (
                self.gps_lat.trim().parse::<f64>(),
                self.gps_lon.trim().parse::<f64>(),
            ) {
                (Ok(la), Ok(lo)) => out.set_gps(la, lo),
                _ => {
                    out.gps_latitude = Some(String::new());
                    out.gps_longitude = Some(String::new());
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub idx: usize,
    pub fields: DraftFields,
    pub original: DraftFields,
    pub date_created: Option<String>,
}

impl Draft {
    pub fn dirty(&self) -> bool {
        self.fields != self.original
    }
}

fn field(
    ui: &mut egui::Ui,
    t: &Theme,
    label: &str,
    value: &mut String,
    multiline: bool,
) -> egui::Response {
    field_h(ui, t, label, value, multiline, &[])
}

/// A field with a ▾ menu of recent values (newest first) when there are any.
fn field_h(
    ui: &mut egui::Ui,
    t: &Theme,
    label: &str,
    value: &mut String,
    multiline: bool,
    history: &[String],
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label.to_uppercase())
                .monospace()
                .size(9.5)
                .color(t.muted),
        );
        if !history.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(RichText::new("▾ recent").size(10.0).color(t.text2), |ui| {
                    ui.set_min_width(220.0);
                    for h in history {
                        if ui.button(RichText::new(h).size(12.0)).clicked() {
                            *value = h.clone();
                            ui.close();
                        }
                    }
                });
            });
        }
    });
    let edit = if multiline {
        egui::TextEdit::multiline(value)
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Body)
    } else {
        egui::TextEdit::singleline(value).desired_width(f32::INFINITY)
    };
    let r = ui.add(edit);
    ui.add_space(S1);
    r
}

/// Draw the photo on the left; the fields panel is drawn by `show_fields`.
pub fn show_photo(app: &mut App, ui: &mut egui::Ui, t: &Theme) {
    let rect = ui.available_rect_before_wrap();
    let ppp = ui.ctx().pixels_per_point();
    let long_side = ((rect.width().max(rect.height())) * ppp).ceil() as u32;
    app.request_preview(app.cursor, long_side, 0);
    if let Some(n) = app.view_neighbor(1) {
        app.request_preview(n, long_side, 1);
    }
    let idx = app.cursor;
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, t.photo);
    let tex = app
        .previews
        .get(&idx)
        .map(|(h, ..)| h)
        .or(app.thumbs[idx].as_ref());
    if let Some(tex) = tex {
        let size = tex.size_vec2();
        let inner = rect.shrink(S5);
        let s = (inner.width() / size.x)
            .min(inner.height() / size.y)
            .min(1.0);
        let fitted = Rect::from_center_size(rect.center(), size * s);
        p.image(
            tex.id(),
            fitted,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let m = app.marks[idx];
    let mut line = vec![format!(
        "{}   {} / {}",
        app.items[idx].display_name(),
        app.cursor_pos() + 1,
        app.view.len()
    )];
    let stars = super::rating_text(m.rating);
    if !stars.is_empty() {
        line.push(stars);
    }
    if let Some(n) = app.scheme.name(m.color_class) {
        line.push(format!("{} {n}", m.color_class.0));
    }
    p.text(
        rect.left_top() + Vec2::new(S3 + 2.0, S3),
        egui::Align2::LEFT_TOP,
        line.join("   "),
        egui::FontId::monospace(12.0),
        t.text2,
    );
    if let Some(c) = t.class_color(&app.scheme, m.color_class) {
        p.rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(4.0, rect.height())),
            CornerRadius::ZERO,
            c,
        );
    }
    if app.cols > 0 {
        let row = app.cursor_pos() / app.cols;
        let rows = app.row_count();
        app.schedule_thumbs(row.saturating_sub(1)..(row + 2).min(rows));
    }
}

pub fn show_fields(app: &mut App, ctx: &egui::Context, t: &Theme) {
    let mut cmds: Vec<Command> = Vec::new();
    egui::SidePanel::right("caption_fields")
        .resizable(true)
        .default_width(FIELDS_W)
        .width_range(320.0..=520.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::same(S4 as i8)).stroke(egui::Stroke::NONE))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = S2;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // Assignment header + stationery actions.
                match &app.assignment {
                    Some(a) => {
                        ui.label(RichText::new(&a.slug).size(15.0).color(t.text));
                        if let Some(d) = a.text_date() {
                            ui.label(RichText::new(d).size(11.0).color(t.muted));
                        }
                    }
                    None => {
                        ui.label(RichText::new("No assignment loaded").size(13.0).color(t.text2));
                        ui.label(RichText::new("Load an assignment file for the pad and codes.").size(11.0).color(t.muted));
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Load assignment…").clicked() {
                        cmds.push(Command::LoadAssignment);
                    }
                    let n = app.selection.targets(app.cursor).len();
                    let apply = ui.add_enabled(app.assignment.is_some(), egui::Button::new(format!("Apply pad to {n}"))).on_disabled_hover_text("Load an assignment first");
                    if apply.clicked() {
                        cmds.push(Command::ApplyStationery);
                    }
                    let tpl = ui.add_enabled(app.assignment.is_some(), egui::Button::new("Insert template")).on_disabled_hover_text("Load an assignment first");
                    if tpl.clicked() {
                        cmds.push(Command::InsertCaptionTemplate);
                    }
                });
                ui.add_space(S3);
                ui.separator();
                ui.add_space(S2);

                let Some(draft) = app.caption_draft.as_mut() else {
                    ui.label(RichText::new("No photo focused").color(t.muted));
                    return;
                };
                let before = draft.fields.caption.clone();
                let cap = field(ui, t, "Caption", &mut draft.fields.caption, true);
                // Expand \code\ pairs as soon as the closing delimiter is typed.
                let delim = app.prefs.code_delimiter;
                if app.prefs.codes_enabled && draft.fields.caption != before && draft.fields.caption.matches(delim).count() % 2 == 0 && draft.fields.caption.contains(delim) {
                    let vars = Vars {
                        date: draft.date_created.as_deref().and_then(codes::parse_date).or_else(|| app.exif[draft.idx].as_ref().and_then(|e| e.date_time_original.as_deref()).and_then(codes::parse_date)),
                        credit: if draft.fields.credit.is_empty() { app.assignment.as_ref().and_then(|a| a.credit_line()) } else { Some(draft.fields.credit.clone()) },
                        headline: if draft.fields.headline.is_empty() { None } else { Some(draft.fields.headline.clone()) },
                    };
                    let expanded = codes::expand(&draft.fields.caption, &app.codes, &vars, delim);
                    if expanded != draft.fields.caption {
                        draft.fields.caption = expanded;
                        // Keep the caret at the end after an expansion.
                        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), cap.id) {
                            let end = egui::text::CCursor::new(draft.fields.caption.chars().count());
                            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                            st.store(ui.ctx(), cap.id);
                        }
                    }
                }
                // Autocomplete hint: codes that start with what follows the last open delimiter.
                let delim = app.prefs.code_delimiter;
                if let Some(open) = draft.fields.caption.rfind(delim) {
                    if draft.fields.caption.matches(delim).count() % 2 == 1 {
                        let prefix = &draft.fields.caption[open + 1..];
                        let hits: Vec<String> = app.codes.starting_with(prefix).take(6).map(|(c, r)| format!("{c} → {}", r.chars().take(28).collect::<String>())).collect();
                        if !hits.is_empty() {
                            ui.label(RichText::new(hits.join("   ")).monospace().size(10.0).color(t.accent));
                        }
                    }
                }
                let hist = |k: &str| -> Vec<String> { app.prefs.caption_history.get(k).cloned().unwrap_or_default() };
                field_h(ui, t, "Headline", &mut draft.fields.headline, false, &hist("headline"));
                // Keywords: chips with ×, an add box, and recent keywords to click in.
                ui.label(RichText::new("KEYWORDS").monospace().size(9.5).color(t.muted));
                let mut kws: Vec<String> = draft.fields.keywords.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    let mut remove: Option<usize> = None;
                    for (i, k) in kws.iter().enumerate() {
                        if super::icons::chip(ui, t, &format!("{k}  ×"), false).on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                    }
                    if let Some(i) = remove {
                        kws.remove(i);
                        changed = true;
                    }
                    let add = ui.add(egui::TextEdit::singleline(&mut app.caption_kw_draft).desired_width(140.0).hint_text("add keyword ⏎"));
                    if add.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        for k in app.caption_kw_draft.split([';', ',']).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                            if !kws.iter().any(|x| x.eq_ignore_ascii_case(k)) {
                                kws.push(k.to_string());
                                changed = true;
                            }
                        }
                        app.caption_kw_draft.clear();
                        add.request_focus();
                    }
                });
                let recent: Vec<String> = hist("keywords")
                    .iter()
                    .flat_map(|h| h.split(';').map(|s| s.trim().to_string()).collect::<Vec<_>>())
                    .filter(|k| !k.is_empty() && !kws.iter().any(|x| x.eq_ignore_ascii_case(k)))
                    .fold(Vec::new(), |mut acc: Vec<String>, k| { if !acc.contains(&k) { acc.push(k); } acc })
                    .into_iter()
                    .take(12)
                    .collect();
                if !recent.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("recent").size(10.5).color(t.muted));
                        for k in recent {
                            if ui.small_button(&k).clicked() {
                                kws.push(k.clone());
                                changed = true;
                            }
                        }
                    });
                }
                if changed {
                    draft.fields.keywords = kws.join("; ");
                }
                ui.add_space(S1);
                ui.columns(2, |cols| {
                    field_h(&mut cols[0], t, "Sublocation", &mut draft.fields.sublocation, false, &hist("sublocation"));
                    field_h(&mut cols[1], t, "City", &mut draft.fields.city, false, &hist("city"));
                });
                ui.columns(2, |cols| {
                    field_h(&mut cols[0], t, "State", &mut draft.fields.state, false, &hist("state"));
                    field_h(&mut cols[1], t, "Country", &mut draft.fields.country, false, &hist("country"));
                });
                field_h(ui, t, "Creator", &mut draft.fields.creator, false, &hist("creator"));
                field_h(ui, t, "Credit", &mut draft.fields.credit, false, &hist("credit"));
                ui.columns(2, |cols| {
                    field_h(&mut cols[0], t, "Copyright", &mut draft.fields.copyright, false, &hist("copyright"));
                    field_h(&mut cols[1], t, "Source", &mut draft.fields.source, false, &hist("source"));
                });
                // The rest of Photo Mechanic's IPTC Info dialog, folded away
                // until a desk asks for them.
                let more_used = !(draft.fields.title.is_empty()
                    && draft.fields.caption_writer.is_empty()
                    && draft.fields.instructions.is_empty()
                    && draft.fields.transmission_reference.is_empty());
                egui::CollapsingHeader::new(RichText::new("More fields").size(11.0).color(t.text2))
                    .default_open(more_used)
                    .show(ui, |ui| {
                        ui.columns(2, |cols| {
                            field_h(&mut cols[0], t, "Title", &mut draft.fields.title, false, &hist("title"));
                            field_h(&mut cols[1], t, "Caption writer", &mut draft.fields.caption_writer, false, &hist("caption_writer"));
                        });
                        field_h(ui, t, "Instructions", &mut draft.fields.instructions, false, &hist("instructions"));
                        field_h(ui, t, "Transmission reference", &mut draft.fields.transmission_reference, false, &hist("transmission_reference"));
                        ui.label(RichText::new("GPS (DECIMAL DEGREES)").monospace().size(9.5).color(t.muted));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut draft.fields.gps_lat).desired_width(120.0).hint_text("40.4461"));
                            ui.add(egui::TextEdit::singleline(&mut draft.fields.gps_lon).desired_width(120.0).hint_text("-79.8955"));
                            if (!draft.fields.gps_lat.is_empty() || !draft.fields.gps_lon.is_empty()) && ui.small_button("Clear").clicked() {
                                draft.fields.gps_lat.clear();
                                draft.fields.gps_lon.clear();
                            }
                        });
                        ui.label(RichText::new("Written as exif:GPSLatitude/Longitude in the sidecar; Lightroom and PM read it. Both fields or neither.").size(10.5).color(t.muted));
                    });
                let delim = app.prefs.code_delimiter;
                if !app.codes.codes.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if ui.checkbox(&mut app.prefs.codes_enabled, "Expand codes").on_hover_text("Off: delimiters are typed literally, nothing expands").changed() {
                            app.prefs_dirty = true;
                        }
                        let list: Vec<String> = app.codes.codes.iter().take(12).map(|(c, _)| format!("{delim}{c}{delim}")).collect();
                        ui.label(RichText::new(list.join(" ")).monospace().size(10.0).color(if app.prefs.codes_enabled { t.muted } else { t.line_strong }));
                    });
                }
                ui.add_space(S2);
                ui.horizontal(|ui| {
                    let dirty = draft.dirty();
                    let save = ui.add_enabled(dirty, egui::Button::new("Save & next  ⌘⏎"));
                    if save.clicked() {
                        cmds.push(Command::SaveCaptionAndNext);
                    }
                    if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                        cmds.push(Command::SaveCaption);
                    }
                    let n = app.selection.targets(app.cursor).len();
                    if n > 1
                        && ui
                            .add_enabled(dirty, egui::Button::new(format!("Save to {n} selected")))
                            .on_hover_text("Writes only the fields you changed to every selected photo")
                            .clicked()
                    {
                        cmds.push(Command::SaveCaptionToSelection);
                    }
                    if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                        cmds.push(Command::RevertCaption);
                    }
                    if dirty {
                        ui.label(RichText::new("unsaved").size(10.5).color(t.star));
                    }
                });
            });
        });
    // ⌘⏎ anywhere in the fields saves and advances; Esc leaves the field.
    let (save_next, esc) = ctx.input(|i| {
        (
            i.modifiers.command && i.key_pressed(Key::Enter),
            i.key_pressed(Key::Escape),
        )
    });
    // Not while a dialog's text field above the caption has the keys.
    if save_next && app.bug_dialog.is_none() {
        cmds.push(Command::SaveCaptionAndNext);
    }
    if esc && ctx.wants_keyboard_input() {
        ctx.memory_mut(|m| m.stop_text_input());
    }
    for c in cmds {
        app.dispatch(ctx, c);
    }
}
