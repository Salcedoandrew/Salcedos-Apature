use std::path::{Path, PathBuf};
use std::time::Instant;

use egui::{Align2, FontId, Key, RichText, Sense};

use super::tokens::{Theme, ROW_H, S1, S2, S3, SIDEBAR_RANGE};
use crate::app::App;
use crate::commands::Command;
use crate::folder_tree::{FolderTree, RowInfo};

/// The folder tree's keyboard-focus id. Arrow keys and Return belong to the
/// tree only while it has focus (after a click in it); otherwise they cull.
pub fn tree_id() -> egui::Id {
    egui::Id::new("sidebar-folder-tree")
}

pub fn tree_has_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.has_focus(tree_id()))
}

/// Sections (which top-level tree a row is under) are a hash of the root;
/// favorites carry this bit.
const FAV_BIT: u64 = 1 << 63;

fn section_of(root: &Path, favorite: bool) -> u64 {
    let h = egui::Id::new(("tree-root", root)).value();
    if favorite {
        h | FAV_BIT
    } else {
        h & !FAV_BIT
    }
}

/// The Favorites section's empty-state hint: only while there are none.
pub fn favorites_hint(favorites: &[PathBuf]) -> Option<&'static str> {
    favorites
        .is_empty()
        .then_some("Add the open folder with File › Add Folder to Favorites")
}

/// Indent per tree level.
const INDENT: f32 = 12.0;

#[derive(Clone, Copy)]
enum Glyph {
    Box,
    Folder,
    Home,
    Volume(crate::platform::VolumeKind),
}

#[derive(Clone, Copy, PartialEq)]
enum Disclosure {
    None,
    Closed,
    Open,
}

fn row(
    ui: &mut egui::Ui,
    t: &Theme,
    label: &str,
    count: Option<usize>,
    selected: bool,
) -> egui::Response {
    let id = ui.next_auto_id();
    draw_row(ui, t, id, label, count, selected, Glyph::Box, None, false).0
}

/// One sidebar row. With `tree` = Some((depth, disclosure)) it is a tree row:
/// indented, with a disclosure triangle. Returns the response and whether the
/// press landed on the triangle.
#[allow(clippy::too_many_arguments)]
fn draw_row(
    ui: &mut egui::Ui,
    t: &Theme,
    id: egui::Id,
    label: &str,
    count: Option<usize>,
    selected: bool,
    glyph_kind: Glyph,
    tree: Option<(usize, Disclosure)>,
    cursor_ring: bool,
) -> (egui::Response, bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    super::icons::focus_ring(p, rect, t, &resp);
    if selected {
        p.rect_filled(rect, egui::CornerRadius::same(6), t.accent_wash);
    } else if resp.hovered() {
        p.rect_filled(rect, egui::CornerRadius::same(6), t.raised);
    }
    if cursor_ring {
        p.rect_stroke(
            rect.shrink(0.5),
            egui::CornerRadius::same(6),
            egui::Stroke::new(1.5_f32, t.accent),
            egui::StrokeKind::Inside,
        );
    }
    let fg = if selected { t.accent } else { t.text2 };
    let mut x = rect.left() + S2;
    let mut on_triangle = false;
    if let Some((depth, disc)) = tree {
        x = rect.left() + S1 + depth as f32 * INDENT;
        let tri = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(12.0, ROW_H));
        if disc != Disclosure::None {
            let c = tri.center();
            let col = if selected { t.accent } else { t.muted };
            let pts = if disc == Disclosure::Open {
                vec![
                    egui::pos2(c.x - 4.0, c.y - 2.0),
                    egui::pos2(c.x + 4.0, c.y - 2.0),
                    egui::pos2(c.x, c.y + 3.0),
                ]
            } else {
                vec![
                    egui::pos2(c.x - 2.0, c.y - 4.0),
                    egui::pos2(c.x + 3.0, c.y),
                    egui::pos2(c.x - 2.0, c.y + 4.0),
                ]
            };
            p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
            on_triangle = resp
                .interact_pointer_pos()
                .is_some_and(|pos| pos.x <= tri.right() + 2.0);
        }
        x = tri.right() + 2.0;
    }
    let glyph =
        egui::Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), egui::vec2(12.0, 12.0));
    match glyph_kind {
        Glyph::Volume(kind) => super::icons::volume_glyph(p, glyph.center(), kind, fg),
        Glyph::Box => {
            p.rect_stroke(
                glyph,
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.5_f32, fg),
                egui::StrokeKind::Inside,
            );
        }
        Glyph::Folder => folder_glyph(p, glyph, fg),
        Glyph::Home => home_glyph(p, glyph, fg),
    }
    let text_x = x + 12.0 + S2;
    let mut max_w = rect.right() - S2 - text_x;
    if let Some(n) = count {
        let cs = p.text(
            egui::pos2(rect.right() - S2, rect.center().y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            FontId::monospace(10.0),
            t.muted,
        );
        max_w -= cs.width() + S2;
    }
    let galley = ui.fonts_mut(|f| {
        f.layout(
            label.to_string(),
            FontId::proportional(13.0),
            fg,
            max_w.max(20.0),
        )
    });
    let galley = if galley.rows.len() > 1 {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(
                truncate(label, (max_w / 7.0) as usize),
                FontId::proportional(13.0),
                fg,
            )
        })
    } else {
        galley
    };
    p.galley(
        egui::pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    (resp.on_hover_text(label), on_triangle)
}

fn folder_glyph(p: &egui::Painter, r: egui::Rect, c: egui::Color32) {
    let s = egui::Stroke::new(1.4_f32, c);
    let body = egui::Rect::from_min_max(
        egui::pos2(r.left(), r.top() + 3.0),
        r.right_bottom() - egui::vec2(0.0, 1.0),
    );
    p.rect_stroke(
        body,
        egui::CornerRadius::same(2),
        s,
        egui::StrokeKind::Inside,
    );
    p.line_segment(
        [
            egui::pos2(r.left() + 1.0, r.top() + 3.5),
            egui::pos2(r.left() + 2.0, r.top() + 1.0),
        ],
        s,
    );
    p.line_segment(
        [
            egui::pos2(r.left() + 2.0, r.top() + 1.0),
            egui::pos2(r.left() + 5.5, r.top() + 1.0),
        ],
        s,
    );
    p.line_segment(
        [
            egui::pos2(r.left() + 5.5, r.top() + 1.0),
            egui::pos2(r.left() + 6.5, r.top() + 3.5),
        ],
        s,
    );
}

fn home_glyph(p: &egui::Painter, r: egui::Rect, c: egui::Color32) {
    let s = egui::Stroke::new(1.4_f32, c);
    let roof = [
        egui::pos2(r.left(), r.top() + 6.0),
        egui::pos2(r.center().x, r.top() + 0.5),
        egui::pos2(r.right(), r.top() + 6.0),
    ];
    p.line_segment([roof[0], roof[1]], s);
    p.line_segment([roof[1], roof[2]], s);
    let walls = egui::Rect::from_min_max(
        egui::pos2(r.left() + 2.0, r.top() + 5.0),
        egui::pos2(r.right() - 2.0, r.bottom() - 0.5),
    );
    p.line_segment([walls.left_top(), walls.left_bottom()], s);
    p.line_segment([walls.right_top(), walls.right_bottom()], s);
    p.line_segment([walls.left_bottom(), walls.right_bottom()], s);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n.max(4) {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars()
                .take(n.saturating_sub(1).max(3))
                .collect::<String>()
        )
    }
}

fn name_of(p: &std::path::Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// The home folder ($HOME), the first Folders root.
pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

/// Drawing state for one frame of the tree.
struct Pass<'a> {
    tree: &'a mut FolderTree,
    ctx: egui::Context,
    t: &'a Theme,
    current: Option<PathBuf>,
    favorites: Vec<PathBuf>,
    focused: bool,
    rows: Vec<RowInfo>,
    cmds: Vec<Command>,
    clicked_in_tree: bool,
    expanded_changed: bool,
}

impl Pass<'_> {
    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        ui: &mut egui::Ui,
        section: u64,
        path: &Path,
        label: &str,
        glyph: Glyph,
        depth: usize,
        parent: Option<usize>,
        has_children: Option<bool>,
        connected: bool,
    ) {
        let expanded = self.tree.is_expanded(path);
        // A favorite on a drive that isn't mounted is never read: the read
        // would fail (or list an empty mount point) and stick.
        if connected && expanded && self.tree.node(path).is_none() {
            self.tree.request(path, Some(&self.ctx));
        }
        let kids: Option<Vec<crate::folder_tree::Child>> = if connected {
            self.tree.children(path).map(|c| c.to_vec())
        } else {
            None
        };
        let failed = match self.tree.node(path) {
            Some(crate::folder_tree::Node::Failed(e)) if connected => Some(e.clone()),
            _ => None,
        };
        // A triangle unless the folder is known to have no subfolders; an
        // expanded row always keeps its (open) triangle so it can be closed.
        let expandable = connected
            && match (&kids, has_children) {
                (Some(k), _) if !self.tree.is_loading(path) => !k.is_empty(),
                (_, Some(h)) => h,
                _ => true,
            };
        let disc = match (expandable || failed.is_some(), expanded) {
            (_, true) => Disclosure::Open,
            (false, false) => Disclosure::None,
            (true, false) => Disclosure::Closed,
        };
        let selected = self.current.as_deref() == Some(path);
        let is_cursor = self
            .tree
            .cursor
            .as_ref()
            .is_some_and(|(s, p)| *s == section && p == path);
        let id = egui::Id::new(("tree-row", section, path));
        let (r, on_tri) = draw_row(
            ui,
            self.t,
            id,
            label,
            None,
            selected,
            glyph,
            Some((depth, disc)),
            self.focused && is_cursor,
        );
        let me = self.rows.len();
        self.rows.push(RowInfo {
            section,
            path: path.to_path_buf(),
            expandable: disc != Disclosure::None,
            expanded,
            parent,
        });
        if selected && self.tree.reveal_since.is_some() {
            r.scroll_to_me(Some(egui::Align::Center));
            self.tree.reveal_since = None;
        }
        if is_cursor && self.tree.scroll_cursor {
            r.scroll_to_me(None);
            self.tree.scroll_cursor = false;
        }
        if r.clicked() || r.double_clicked() || r.secondary_clicked() {
            self.clicked_in_tree = true;
            self.tree.cursor = Some((section, path.to_path_buf()));
        }
        if r.clicked() && on_tri && disc != Disclosure::None {
            self.toggle(path);
        } else if r.double_clicked() {
            self.tree.pending_click = None;
            self.cmds.push(Command::OpenFolder(path.to_path_buf()));
        } else if r.clicked() {
            self.tree.pending_click = Some((path.to_path_buf(), Instant::now()));
        }
        let is_fav_root = depth == 0 && section & FAV_BIT != 0;
        let already_fav = self.favorites.iter().any(|f| f == path);
        r.context_menu(|ui| {
            if ui.button("Open in New Tab").clicked() {
                self.cmds.push(Command::OpenFolder(path.to_path_buf()));
                ui.close();
            }
            if ui.button("Reveal in Finder").clicked() {
                self.cmds.push(Command::RevealFolder(path.to_path_buf()));
                ui.close();
            }
            if is_fav_root {
                if ui.button("Remove from Favorites").clicked() {
                    self.cmds.push(Command::RemoveFavorite(path.to_path_buf()));
                    ui.close();
                }
            } else if ui
                .add_enabled(!already_fav, egui::Button::new("Add to Favorites"))
                .clicked()
            {
                self.cmds.push(Command::AddFavoritePath(path.to_path_buf()));
                ui.close();
            }
        });
        if !expanded {
            return;
        }
        // Loading, empty, unreadable or not connected: one dim line says so,
        // never a bare open triangle.
        let note = crate::folder_tree::note_for(self.tree.node(path), connected);
        for k in kids.unwrap_or_default() {
            self.node(
                ui,
                section,
                &k.path,
                &k.name,
                Glyph::Folder,
                depth + 1,
                Some(me),
                Some(k.has_children),
                true,
            );
        }
        if let Some(n) = note {
            self.note(ui, depth + 1, &n);
        }
    }

    fn note(&self, ui: &mut egui::Ui, depth: usize, text: &str) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_H - 6.0),
            Sense::hover(),
        );
        ui.painter().text(
            egui::pos2(
                rect.left() + S1 + depth as f32 * INDENT + 14.0,
                rect.center().y,
            ),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(11.5),
            self.t.muted,
        );
    }

    fn toggle(&mut self, path: &Path) {
        if self.tree.is_expanded(path) {
            self.tree.collapse(path);
        } else {
            self.tree.expand(path, Some(&self.ctx));
        }
        self.expanded_changed = true;
    }
}

/// Arrow keys move the tree's cursor while it has focus: ↑↓ by row, → opens
/// (or steps into) a folder, ← closes it (or steps out), Return opens the
/// folder in this tab. Uses last frame's rows.
fn tree_keys(app: &mut App, ctx: &egui::Context, cmds: &mut Vec<Command>) -> bool {
    let rows = app.tree.rows.clone();
    if rows.is_empty() {
        return false;
    }
    let (up, down, left, right, enter) = ctx.input_mut(|i| {
        let m = egui::Modifiers::NONE;
        (
            i.consume_key(m, Key::ArrowUp),
            i.consume_key(m, Key::ArrowDown),
            i.consume_key(m, Key::ArrowLeft),
            i.consume_key(m, Key::ArrowRight),
            i.consume_key(m, Key::Enter),
        )
    });
    if !(up || down || left || right || enter) {
        return false;
    }
    let at = app
        .tree
        .cursor
        .as_ref()
        .and_then(|(s, p)| rows.iter().position(|r| r.section == *s && &r.path == p));
    let Some(at) = at.or_else(|| {
        app.folder
            .as_ref()
            .and_then(|f| rows.iter().position(|r| &r.path == f))
    }) else {
        app.tree.cursor = Some((rows[0].section, rows[0].path.clone()));
        return false;
    };
    let mut to = at;
    let mut changed = false;
    let r = &rows[at];
    if up {
        to = at.saturating_sub(1);
    } else if down {
        to = (at + 1).min(rows.len() - 1);
    } else if right && r.expandable {
        if r.expanded {
            to = (at + 1).min(rows.len() - 1);
        } else {
            app.tree.expand(&r.path, Some(ctx));
            changed = true;
        }
    } else if left {
        if r.expanded {
            app.tree.collapse(&r.path);
            changed = true;
        } else if let Some(p) = r.parent {
            to = p;
        }
    } else if enter {
        app.tree.pending_click = None;
        cmds.push(Command::OpenFolderHere(r.path.clone()));
    }
    app.tree.cursor = Some((rows[to].section, rows[to].path.clone()));
    app.tree.scroll_cursor = true;
    changed
}

pub fn show(app: &mut App, ctx: &egui::Context, t: &Theme) {
    let mut cmds: Vec<Command> = Vec::new();
    let current = app.folder.clone();

    // Roots: connected favorites, then Home and each mounted volume, with
    // device ids so a drive mounting is a new root (see `tree_roots`).
    let home = home_dir();
    let mounted: Vec<PathBuf> = app.volumes.iter().map(|(_, p)| p.clone()).collect();
    let mounted_ids: Vec<crate::folder_tree::RootId> = mounted
        .iter()
        .map(|p| (p.clone(), app.volume_devs.get(p).copied().unwrap_or(0)))
        .collect();
    let roots = crate::folder_tree::tree_roots(&app.prefs.favorites, home.as_deref(), &mounted_ids);
    app.tree.sync_root_ids(&roots);
    app.tree.poll();
    let mut expanded_changed = false;

    // Show the current tab's folder: open its parents, scroll to it.
    if app.tree.revealed_for != current {
        app.tree.revealed_for = current.clone();
        if let Some(f) = &current {
            expanded_changed |= app.tree.expand_to(f);
            app.tree.reveal_since = Some(Instant::now());
            app.tree.cursor = None;
        }
    }
    if app
        .tree
        .reveal_since
        .is_some_and(|s| s.elapsed().as_secs_f32() > 3.0)
    {
        app.tree.reveal_since = None;
    }

    // A single click waits out the double-click interval, so a double-click
    // (new tab) doesn't also replace this tab's folder.
    let delay = ctx.options(|o| o.input_options.max_double_click_delay) as f32 + 0.05;
    if let Some((p, at)) = app.tree.pending_click.clone() {
        let waited = at.elapsed().as_secs_f32();
        if waited >= delay {
            app.tree.pending_click = None;
            cmds.push(Command::OpenFolderHere(p));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(delay - waited));
        }
    }

    let shown = app.sidebar_shown();
    let focused = shown && tree_has_focus(ctx);
    if focused {
        expanded_changed |= tree_keys(app, ctx, &mut cmds);
    }

    let favorites = app.prefs.favorites.clone();
    let volumes = app.volumes.clone();
    let kinds = app.volume_kinds.clone();
    let recent = app.prefs.recent.clone();
    let mut pass = Pass {
        tree: &mut app.tree,
        ctx: ctx.clone(),
        t,
        current: current.clone(),
        favorites: favorites.clone(),
        focused,
        rows: Vec::new(),
        cmds: Vec::new(),
        clicked_in_tree: false,
        expanded_changed: false,
    };
    let resp = egui::SidePanel::left("sidebar")
        .resizable(true)
        .default_width(app.prefs.sidebar_width)
        .width_range(SIDEBAR_RANGE)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin::symmetric(S2 as i8, S3 as i8))
                .stroke(egui::Stroke::NONE),
        )
        .show_animated(ctx, shown, |ui| {
            ui.spacing_mut().item_spacing.y = S1;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    super::eyebrow(ui, t, "Favorites");
                    if let Some(hint) = favorites_hint(&favorites) {
                        ui.label(RichText::new(hint).size(11.0).color(t.muted));
                    }
                    for f in favorites.iter() {
                        pass.node(
                            ui,
                            section_of(f, true),
                            f,
                            &name_of(f),
                            Glyph::Folder,
                            0,
                            None,
                            None,
                            crate::folder_tree::favorite_connected(f, &mounted),
                        );
                    }
                    ui.add_space(S3);
                    super::eyebrow(ui, t, "Recent");
                    for f in recent {
                        if row(
                            ui,
                            t,
                            &name_of(&f),
                            None,
                            current.as_deref() == Some(f.as_path()),
                        )
                        .clicked()
                        {
                            pass.cmds.push(Command::OpenFolder(f));
                        }
                    }
                    ui.add_space(S3);
                    super::eyebrow(ui, t, "Folders");
                    if let Some(h) = &home {
                        pass.node(
                            ui,
                            section_of(h, false),
                            h,
                            &name_of(h),
                            Glyph::Home,
                            0,
                            None,
                            None,
                            true,
                        );
                    }
                    for (name, path) in volumes.iter() {
                        let glyph = kinds
                            .get(path)
                            .map(|k| Glyph::Volume(*k))
                            .unwrap_or(Glyph::Folder);
                        pass.node(
                            ui,
                            section_of(path, false),
                            path,
                            name,
                            glyph,
                            0,
                            None,
                            None,
                            true,
                        );
                    }
                    // One focus target for the whole tree, so egui's own
                    // arrow-key focus moves don't fight the tree's.
                    let area = ui.min_rect();
                    let f = ui.interact(area, tree_id(), Sense::focusable_noninteractive());
                    let _ = f;
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            tree_id(),
                            egui::EventFilter {
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                ..Default::default()
                            },
                        );
                        if pass.clicked_in_tree {
                            m.request_focus(tree_id());
                        }
                    });
                });
        });
    let Pass {
        rows,
        cmds: tree_cmds,
        expanded_changed: toggled,
        ..
    } = pass;
    if resp.is_some() {
        app.tree.rows = rows;
    }
    cmds.extend(tree_cmds);
    if expanded_changed || toggled {
        app.prefs.tree_expanded = app.tree.expanded_list();
        app.prefs_dirty = true;
    }
    let Some(resp) = resp else {
        for c in cmds {
            app.dispatch(ctx, c);
        }
        return;
    };
    let w = resp.response.rect.width();
    if app.sidebar_shown()
        && (w - app.prefs.sidebar_width).abs() > 0.5
        && SIDEBAR_RANGE.contains(&w)
    {
        app.prefs.sidebar_width = w;
        app.prefs_dirty = true;
    }
    // Divider.
    let r = resp.response.rect;
    ctx.layer_painter(egui::LayerId::background()).line_segment(
        [r.right_top(), r.right_bottom()],
        egui::Stroke::new(1.0, t.line),
    );
    for c in cmds {
        app.dispatch(ctx, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorites_hint_only_without_favorites() {
        assert!(favorites_hint(&[]).is_some());
        assert_eq!(
            favorites_hint(&[PathBuf::from("/Volumes/Work Drive")]),
            None
        );
    }
}
