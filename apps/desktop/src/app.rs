//! Application state and the single command dispatcher. Drawing lives in
//! `ui/`; native integration in `platform/`; decoding in `worker.rs`;
//! selection, filters and undo are pure modules with their own tests.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use egui::{Color32, ColorImage, Key, TextureHandle, TextureOptions};

use crate::ui::tokens::S4;
use rayon::prelude::*;

use apature_core::{ColorClass, ColorClassScheme, Marks, PhotoItem, Rating, ScanOptions};
use apature_metadata::edits;
use apature_metadata::items;
use apature_metadata::ExifSummary;
use apature_preview::{Rgb, ThumbCache};

use apature_metadata::codes::{self, CodeTable, Vars};
use apature_metadata::stationery::Assignment;

use crate::commands::{Command, Workspace};
use crate::deliver_job::{DeliverJobs, Msg as DMsg, Status as DStatus};
use crate::filter::{self, Filter};
use crate::ingest_job::{FileStatus, IngestJobs, Msg, Running};
use crate::platform::Platform;
use crate::prefs::{Prefs, ShortcutLayout};
use crate::selection::{Mods, Selection};
use crate::ui::caption::{Draft, DraftFields};
use crate::ui::{self, tokens::Theme};
use crate::undo::{Change, Entry, UndoStack};
use crate::worker::{Job, JobKind, Pool};
use crate::Launch;

const THUMB_PX: u32 = 320;
const PREFETCH_ROWS: usize = 3;
const KEEP_ROWS: usize = 12;
const PREVIEW_LRU: usize = 8;

/// Positions to preload around `pos` in a view of `len`, most urgent
/// first: two ahead in the direction of travel, one behind, a third ahead.
pub fn prefetch_plan(pos: usize, len: usize, dir: isize) -> Vec<usize> {
    let d = if dir < 0 { -1 } else { 1 };
    [d, 2 * d, -d, 3 * d]
        .iter()
        .filter_map(|&k| {
            let p = pos as isize + k;
            (p >= 0 && (p as usize) < len).then_some(p as usize)
        })
        .collect()
}

/// Which cached previews to drop, oldest first, so at most `max_n` stay and
/// their bytes fit in `budget`. Photos in `keep` (on screen, preloading)
/// are never dropped, even when that leaves the cache over budget.
pub fn preview_evictions(
    lru: &VecDeque<usize>,
    bytes: impl Fn(usize) -> usize,
    keep: &[usize],
    max_n: usize,
    budget: usize,
) -> Vec<usize> {
    let mut left: Vec<usize> = lru.iter().copied().collect();
    let mut total: usize = left.iter().map(|&i| bytes(i)).sum();
    let mut out = Vec::new();
    while left.len() > max_n || total > budget {
        let Some(p) = left.iter().position(|i| !keep.contains(i)) else {
            break;
        };
        let i = left.remove(p);
        total -= bytes(i);
        out.push(i);
    }
    out
}

/// Everything that belongs to one open folder. The active tab's state lives
/// directly in `App` (so the UI code reads `app.items` etc.); inactive tabs
/// are parked here and swapped in with `switch_tab`.
pub struct TabState {
    pub folder: Option<PathBuf>,
    pub items: Vec<PhotoItem>,
    pub marks: Vec<Marks>,
    pub conflicts: Vec<Vec<String>>,
    pub exif: Vec<Option<ExifSummary>>,
    pub thumbs: Vec<Option<TextureHandle>>,
    pub thumb_err: Vec<Option<String>>,
    /// Upright size of the largest image each photo can show, from the
    /// first decode of it (see `worker::Done::native`).
    pub native: Vec<Option<[u32; 2]>>,
    /// Preview textures: (texture, long side in px, is the full resolution).
    pub previews: HashMap<usize, (TextureHandle, u32, bool)>,
    pub preview_lru: VecDeque<usize>,
    pub generation: u64,
    pub cursor: usize,
    pub selection: Selection,
    pub view: Vec<usize>,
    /// Burst stacks: head item → members hidden behind it (when collapsed).
    pub stacks: HashMap<usize, Vec<usize>>,
    /// Stack heads currently expanded.
    pub stacks_open: HashSet<usize>,
    pub pos: Vec<Option<usize>>,
    pub filter: Filter,
    pub undo: UndoStack,
    pub cols: usize,
    pub last_visible_rows: Range<usize>,
    pub stats: Stats,
    pub caption_draft: Option<Draft>,
    pub inspector_iptc: Option<(usize, apature_metadata::Iptc)>,
}

/// A tab slot: the folder it shows and, when inactive, its parked state.
pub struct Tab {
    pub folder: PathBuf,
    pub state: Option<TabState>,
    /// Photo count for the strip while the state is parked.
    pub count: usize,
}

#[derive(Default)]
/// Live Ingest: the source folder is polled every two seconds; a new file is
/// copied once its size has held for two polls (the camera or FTP server
/// has finished writing it). Batches run through the normal ingest, so
/// verification, naming, the ledger and the sequence counter all apply.
pub struct LiveIngest {
    pub on: bool,
    /// Files seen but not yet stable: path → (size, polls at that size).
    pub pending: HashMap<PathBuf, (u64, u8)>,
    /// Files already handed to an ingest this session.
    pub handed: HashSet<PathBuf>,
    pub ingested: usize,
    pub since: Option<Instant>,
    pub last_batch: Option<Instant>,
}

#[derive(Default)]
pub struct IngestState {
    pub live: LiveIngest,
    pub opts: apature_ingest::Options,
    pub plan: Option<apature_ingest::Plan>,
    pub plan_error: Option<String>,
    /// When the current plan arrived (the plan card fades in).
    pub plan_since: Option<Instant>,
    pub planning: bool,
    pub generation: u64,
    pub running: Option<Running>,
    pub apply_pad: bool,
    /// Set GPS on ingest: "lat, lon" in decimal degrees, and the switch.
    pub gps: String,
    pub gps_on: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomMode {
    #[default]
    Fit,
    OneToOne,
}

/// Preview workspace view state: zoom, pan and the compare set. Shared by
/// every pane in compare so zoom and pan stay in sync.
pub struct PreviewView {
    pub mode: ZoomMode,
    /// Image point (0..1 × 0..1) shown at the pane centre when at 1:1.
    pub center: egui::Vec2,
    pub zoom_since: Option<Instant>,
    pub zoom_from: Option<egui::Rect>,
    /// Where the focused photo was last drawn (the zoom animation starts there).
    pub last_rect: Option<egui::Rect>,
    /// Item indices shown side by side, when comparing.
    pub compare: Option<Vec<usize>>,
    /// The crop tool, when open (Preview, K).
    pub crop: Option<CropEdit>,
    /// Loupe under the pointer (L) in fit mode.
    pub loupe: bool,
    /// One magnification for 1:1 and the loupe: 1.0 = one image pixel per
    /// screen pixel, ¼× to 8×. ⌘-scroll or the Zoom slider change it.
    pub zoom: f32,
    /// Lock scrolling (Zoom section): keep the pan when moving to another
    /// photo while zoomed; off re-centres each photo.
    pub lock_scroll: bool,
    /// Last view position drawn and the direction of travel (+1 / -1),
    /// for preloading ahead.
    pub last_pos: Option<usize>,
    pub dir: isize,
}

/// Crop ratios as width/height of the *displayed* frame; None = free.
pub const CROP_RATIOS: [(Option<f32>, &str); 7] = [
    (None, "free"),
    (Some(3.0 / 2.0), "3:2"),
    (Some(2.0 / 3.0), "2:3"),
    (Some(4.0 / 3.0), "4:3"),
    (Some(16.0 / 9.0), "16:9"),
    (Some(1.0), "1:1"),
    (Some(5.0 / 4.0), "5:4"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropOverlay {
    None,
    Thirds,
    Golden,
}

#[derive(Clone, Debug)]
pub struct CropEdit {
    /// Fractions of the displayed (already turned) image: l, t, r, b.
    pub rect: [f32; 4],
    pub ratio_i: usize,
    pub overlay: CropOverlay,
    /// Active drag: handle index 0..8 (corners 0-3, edges 4-7, 8 = move),
    /// or 9 = drawing a new rectangle from `anchor`.
    pub drag: Option<(u8, egui::Vec2)>,
}

impl CropEdit {
    pub fn ratio(&self) -> Option<f32> {
        CROP_RATIOS[self.ratio_i].0
    }
}

/// Stored (unturned) crop fractions ↔ displayed fractions for the four
/// orientations the camera and our rotate write (1, 3, 6, 8).
pub fn crop_display_to_stored(r: [f32; 4], o: u32) -> [f32; 4] {
    let [l, t, rr, b] = r;
    match o {
        3 => [1.0 - rr, 1.0 - b, 1.0 - l, 1.0 - t],
        6 => [t, 1.0 - rr, b, 1.0 - l],
        8 => [1.0 - b, l, 1.0 - t, rr],
        _ => r,
    }
}

pub fn crop_stored_to_display(s: [f32; 4], o: u32) -> [f32; 4] {
    let [l, t, rr, b] = s;
    match o {
        3 => [1.0 - rr, 1.0 - b, 1.0 - l, 1.0 - t],
        6 => [1.0 - b, l, 1.0 - t, rr],
        8 => [t, 1.0 - rr, b, 1.0 - l],
        _ => s,
    }
}

impl Default for PreviewView {
    fn default() -> Self {
        PreviewView {
            mode: ZoomMode::Fit,
            center: egui::Vec2::splat(0.5),
            crop: None,
            loupe: false,
            zoom: 1.0,
            lock_scroll: false,
            zoom_since: None,
            zoom_from: None,
            last_rect: None,
            compare: None,
            last_pos: None,
            dir: 1,
        }
    }
}

/// Rename Photos dialog state.
/// Copy Photos… (File menu): the photos, the choices and the live plan.
/// Extract JPEG Previews: the photos it will work on.
pub struct ExtractDialog {
    pub targets: Vec<usize>,
    /// How many of them have a RAW.
    pub raws: usize,
}

pub struct CopyDialog {
    pub targets: Vec<usize>,
    pub opts: apature_ingest::CopyOptions,
    /// Write the assignment pad into the copies' XMP sidecars.
    pub apply_pad: bool,
    pub plan: Option<apature_ingest::CopyPlan>,
    pub error: Option<String>,
}

/// Help › Report a Bug… (see `bug_report`).
pub struct BugDialog {
    pub report: crate::bug_report::BugReport,
    pub include_info: bool,
    /// Gathered when the dialog opens; shown read-only and sent only when
    /// `include_info` is on.
    pub info: crate::bug_report::SystemInfo,
    /// What the last Submit or Copy did.
    pub notice: Option<String>,
    /// ⌘A reached the menu bar (Edit › Select All) while the dialog was
    /// open: select the focused field's text, not every photo.
    pub select_all: bool,
    /// Put the cursor in Title on the first frame.
    pub focus_title: bool,
}

pub struct RenameDialog {
    pub template: String,
    pub seq_start: u32,
    pub targets: Vec<usize>,
    pub plan: Option<apature_core::fileops::RenamePlan>,
    pub error: Option<String>,
}

/// Export workspace state; settings live in `Prefs::export`.
/// "40.4461, -79.8955" (comma or space) → (lat, lon).
pub fn parse_latlon(s: &str) -> Option<(f64, f64)> {
    let mut it = s
        .split([',', ' '])
        .map(|p| p.trim())
        .filter(|p| !p.is_empty());
    let la: f64 = it.next()?.parse().ok()?;
    let lo: f64 = it.next()?.parse().ok()?;
    ((-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo)).then_some((la, lo))
}

#[derive(Default)]
pub struct DeliverState {
    pub running: Option<crate::deliver_job::Running>,
    /// Outputs of the last finished export, ready to send.
    pub last_outputs: Vec<PathBuf>,
    /// Result line for the Settings test button.
    pub test_result: Option<String>,
    /// Password drafts per target name (Settings), never saved to prefs.
    pub pw_drafts: HashMap<String, String>,
    /// Target picked on the Export sheet for a one-off send.
    pub pick: Option<String>,
    /// Every file ever sent, by source photo path (persisted to delivered.json).
    pub sent: HashMap<PathBuf, SentRecord>,
    /// Output path → source photo path for the last export, so sends can be
    /// recorded against the photo you see on the sheet.
    pub source_of: HashMap<PathBuf, PathBuf>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SentRecord {
    pub target: String,
    /// Local time as "YYYY-MM-DD HH:MM".
    pub at: String,
    pub output: PathBuf,
}

fn delivered_path() -> PathBuf {
    apature_core::project_dirs()
        .map(|d| d.data_dir().join("delivered.json"))
        .unwrap_or_else(|| PathBuf::from("delivered.json"))
}

fn load_delivered() -> HashMap<PathBuf, SentRecord> {
    std::fs::read_to_string(delivered_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_delivered(map: &HashMap<PathBuf, SentRecord>) {
    let p = delivered_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(t) = serde_json::to_string(map) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

fn local_stamp() -> String {
    // Civil time from the system clock without a date crate: UTC offset via
    // the C library's localtime is not available here, so use the date
    // command's format through std only when possible; fall back to UTC.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = {
        let z = secs.div_euclid(86_400) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (if m <= 2 { y + 1 } else { y }, m, d)
    };
    let t = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}Z",
        t / 3600,
        (t % 3600) / 60
    )
}

#[derive(Default)]
pub struct ExportState {
    /// Export everything in the view rather than the selection.
    pub scope_all: bool,
    pub running: Option<crate::export_job::Running>,
    /// Name typed for a new preset on the Export sheet.
    pub preset_draft: String,
    /// Planned (item index, output path) for the current scope and names.
    pub names: Vec<(usize, PathBuf)>,
    names_key: Option<(Vec<usize>, String, PathBuf, Option<String>)>,
}

#[derive(Default)]
pub struct Stats {
    pub thumbs_done: usize,
    pub cache_hits: usize,
    pub decode_ms_total: f32,
    pub scan_ms: f32,
    pub frame_ms_ema: f32,
    pub frame_ms_max: f32,
    pub textures_uploaded: usize,
}

/// The bare number keys, index = digit.
const NUM_KEYS: [Key; 9] = [
    Key::Num0,
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
];

/// The number keys pressed this frame as rating (0–5) or colour class (0–8) commands.
fn number_commands(i: &egui::InputState, rates: bool, out: &mut Vec<Command>) {
    for (n, k) in NUM_KEYS.iter().enumerate() {
        if i.key_pressed(*k) {
            if !rates {
                out.push(Command::SetClass(n as u8));
            } else if n <= 5 {
                out.push(Command::SetRating(n as i8));
            }
        }
    }
}

/// What one background poll sends back: the fresh volume list and, when
/// Live Ingest is on, its scan results.
struct PollResult {
    volumes: Vec<(String, PathBuf)>,
    volume_kinds: HashMap<PathBuf, crate::platform::VolumeKind>,
    volume_devs: HashMap<PathBuf, u64>,
    live: Option<LivePoll>,
}

/// The Live Ingest half of a background poll, computed on the worker.
struct LivePoll {
    /// The source folder that was scanned; results are dropped when the
    /// user has repointed Live Ingest since the poll launched.
    src: PathBuf,
    /// Pending size counters, advanced by the scan.
    pending: HashMap<PathBuf, (u64, u8)>,
    /// Files whose size held for two polls.
    ready: Vec<PathBuf>,
    /// The plan for `ready`, built on the worker from the tick's options
    /// snapshot; `None` when nothing was ready.
    plan: Option<Result<apature_ingest::Plan, String>>,
}

/// The filesystem half of a volume refresh: list `/Volumes`, classify each
/// entry and note its device id. Pure I/O against the passed-in kind cache,
/// so the 2 s poll can run it on a worker thread; `App::refresh_volumes`
/// (startup, folder open) runs it in place.
fn scan_volumes(
    kinds: &mut HashMap<PathBuf, crate::platform::VolumeKind>,
) -> (Vec<(String, PathBuf)>, HashMap<PathBuf, u64>) {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() && !name.starts_with('.') {
                let kind = *kinds
                    .entry(p.clone())
                    .or_insert_with(|| Platform::volume_kind(&p));
                if kind != crate::platform::VolumeKind::System {
                    v.push((name, p));
                }
            }
        }
    }
    v.sort();
    let present: Vec<PathBuf> = v.iter().map(|(_, p)| p.clone()).collect();
    let devs = present
        .iter()
        .map(|p| (p.clone(), crate::folder_tree::device_of(p)))
        .collect();
    kinds.retain(|p, _| present.contains(p) || p.join("DCIM").is_dir());
    (v, devs)
}

/// The filesystem half of one Live Ingest poll: scan the source folder,
/// stat each file not yet handed over and advance the pending counters.
/// Returns the updated counters and the files whose size held for two
/// polls; `None` when the source is gone or unreadable, in which case the
/// caller leaves Live Ingest state untouched, as the old tick did.
fn live_scan(
    src: &Path,
    recursive: bool,
    mut pending: HashMap<PathBuf, (u64, u8)>,
    handed: &HashSet<PathBuf>,
) -> Option<(HashMap<PathBuf, (u64, u8)>, Vec<PathBuf>)> {
    if !src.is_dir() {
        return None;
    }
    let items = apature_core::scan(src, &apature_core::ScanOptions { recursive }).ok()?;
    let mut ready: Vec<PathBuf> = Vec::new();
    for it in &items {
        for v in it.variants() {
            if handed.contains(&v.path) {
                continue;
            }
            let size = std::fs::metadata(&v.path).map(|m| m.len()).unwrap_or(0);
            let e = pending.entry(v.path.clone()).or_insert((size, 0));
            if e.0 == size {
                e.1 = e.1.saturating_add(1);
            } else {
                *e = (size, 0);
            }
            if e.1 >= 2 && size > 0 {
                ready.push(v.path.clone());
            }
        }
    }
    Some((pending, ready))
}

pub struct App {
    pub folder: Option<PathBuf>,
    pub items: Vec<PhotoItem>,
    pub marks: Vec<Marks>,
    pub conflicts: Vec<Vec<String>>,
    pub exif: Vec<Option<ExifSummary>>,
    pub thumbs: Vec<Option<TextureHandle>>,
    pub thumb_err: Vec<Option<String>>,
    /// Upright size of the largest image each photo can show, from the
    /// first decode of it (see `worker::Done::native`).
    pub native: Vec<Option<[u32; 2]>>,
    /// Preview textures: (texture, long side in px, is the full resolution).
    pub previews: HashMap<usize, (TextureHandle, u32, bool)>,
    preview_lru: VecDeque<usize>,
    pub pool: Pool,
    generation: u64,
    /// Focused item (an index into `items`).
    pub cursor: usize,
    pub selection: Selection,
    /// Visible, ordered item indices after filter and sort.
    pub view: Vec<usize>,
    pub stacks: HashMap<usize, Vec<usize>>,
    pub stacks_open: HashSet<usize>,
    /// Position of each item in `view`, `None` when filtered out.
    pub pos: Vec<Option<usize>>,
    pub filter: Filter,
    pub undo: UndoStack,
    pub workspace: Workspace,
    /// The Ingest window (a separate native window, like Photo Mechanic's
    /// ingest dialog) is open. Headless runs keep Ingest inline instead.
    pub ingest_window: bool,
    /// Verified count reported by the last ingest event, consumed after the drain
    /// (the event loop holds a borrow on `ingest.running`).
    ingest_sheet_pending: Option<usize>,
    pub cols: usize,
    pub last_visible_rows: Range<usize>,
    pub scroll_to_cursor: bool,
    pub scheme: ColorClassScheme,
    pub stats: Stats,
    pub status: String,
    pub prefs: Prefs,
    pub prefs_dirty: bool,
    prefs_saved_at: Instant,
    pub volumes: Vec<(String, PathBuf)>,
    /// Icon class per mounted volume, filled once per mount.
    pub volume_kinds: HashMap<PathBuf, crate::platform::VolumeKind>,
    /// Device id of each mounted volume, so the folder tree notices a drive
    /// mounting over a mount point it had already read (same path).
    pub volume_devs: HashMap<PathBuf, u64>,
    /// The sidebar's folder tree (lazily read subfolders, expanded rows).
    pub tree: crate::folder_tree::FolderTree,
    /// The open folder's volume vanished (name), until it comes back or the tab closes.
    pub folder_unplugged: Option<String>,
    /// Notices above the status bar (see ui/toasts.rs); at most four.
    pub toasts: Vec<ui::toasts::Toast>,
    /// View-only exposure boost for previews, in stops (0, 1, 2); never written anywhere.
    pub preview_boost: u8,
    window_title: String,
    /// Keyword being typed on the Caption workspace.
    pub caption_kw_draft: String,
    pub filter_popup_open: bool,
    pub caption_draft: Option<Draft>,
    pub assignment: Option<Assignment>,
    pub codes: CodeTable,
    pub ingest: IngestState,
    pub export: ExportState,
    pub deliver: DeliverState,
    export_jobs: crate::export_job::ExportJobs,
    deliver_jobs: DeliverJobs,
    pub pv: PreviewView,
    pub rename_dialog: Option<RenameDialog>,
    /// Internal metadata clipboard: marks plus IPTC of one photo.
    pub meta_clip: Option<(Marks, apature_metadata::Iptc)>,
    /// Items awaiting confirmation to go to the Trash.
    pub trash_dialog: Option<Vec<usize>>,
    pub copy_dialog: Option<CopyDialog>,
    /// Help › Report a Bug…. While it is open, menu keys never reach the
    /// photos (see `update`).
    pub bug_dialog: Option<BugDialog>,
    /// The copy in progress (or just finished), shown by the dialog or the status line.
    pub copy_running: Option<crate::copy_job::CopyRunning>,
    /// Open the destination as a contact sheet when the running copy ends.
    copy_open_after: bool,
    copy_jobs: crate::copy_job::CopyJobs,
    pub extract_dialog: Option<ExtractDialog>,
    pub extract_running: Option<crate::extract_job::ExtractRunning>,
    extract_jobs: crate::extract_job::ExtractJobs,
    /// One action per keypress across the menu and the key handler.
    key_once: crate::keyonce::KeyOnce,
    /// Writes marks on its own thread (see `mark_writer`).
    mark_writer: crate::mark_writer::MarkWriter,
    /// The newest single-photo mark and its status line, finished with the
    /// file count once the write lands.
    mark_status: Option<(u64, String)>,
    /// A bulk sidecar write (paste, caption-to-selection, stationery,
    /// rotate) running on the mark-writer thread; the final status is set
    /// when its last file reports in.
    bulk_write: Option<BulkWrite>,
    /// A single caption save running on the mark-writer thread. If it
    /// fails, the draft goes dirty again so the caption is not lost.
    caption_write: Option<CaptionWrite>,
    ingest_jobs: IngestJobs,
    /// Files currently being dragged over the window, for the drop overlay.
    pub drag_hover: usize,
    /// IPTC of the focused photo, read once per focus change for the inspector.
    pub inspector_iptc: Option<(usize, apature_metadata::Iptc)>,
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_generation: u64,
    pub settings_open: bool,
    pub about_open: bool,
    /// Preview is showing in its own window (prefs.preview_in_window).
    pub preview_window: bool,
    update_checker: Option<crate::updater::Checker>,
    /// A running update install: download, stage, swap.
    update_install: Option<crate::update_install::Installer>,
    /// True when the person asked (menu), so "up to date" is worth a toast.
    update_manual: bool,
    pub update_available: Option<crate::updater::Update>,
    pub settings_page: ui::settings::Page,
    pub cache_size: Option<(usize, u64)>,
    applied_appearance: Option<crate::prefs::Appearance>,
    dispatch_later: Option<Command>,
    platform: Platform,
    system_reduce_motion: bool,
    applied_ui_scale: Option<f32>,
    /// Motion state: when the sheet last changed tab, when the preview opened
    /// and from which cell, where the focused cell was drawn, when the status
    /// text last changed.
    pub sheet_fade_since: Option<Instant>,
    pub preview_since: Option<Instant>,
    /// Sidebar in Preview: hidden by default so the photo gets the room;
    /// the sidebar toggle shows it for this session. The catalog keeps
    /// `prefs.sidebar_visible`.
    pub preview_sidebar: bool,
    /// Inspector in Preview, once toggled there (Tab, ⌥⌘I): kept for the
    /// session only. None follows `prefs.inspector_visible`.
    pub preview_inspector: Option<bool>,
    /// A bare Tab taken from the main window's input (raw_input_hook):
    /// toggle both side panels this frame.
    tab_toggle: bool,
    pub preview_from: Option<egui::Rect>,
    pub cursor_cell_rect: Option<egui::Rect>,
    /// Keyboard focus moved: the ring slides from `focus_from` over 160 ms.
    pub focus_from: Option<egui::Rect>,
    pub focus_since: Option<Instant>,
    pub status_since: Option<Instant>,
    /// `/Volumes` is polled every 2 s so a card shows up when it mounts.
    volumes_checked: Instant,
    /// The in-flight background poll (volume list + Live Ingest scan), if
    /// any; a tick that finds one still running is skipped.
    poll_rx: Option<std::sync::mpsc::Receiver<PollResult>>,
    last_status: String,
    launch: Launch,
    quit_after_frame: bool,
    run_done: bool,
    frames: u64,
    screenshot_requested: bool,
}

/// What a bulk sidecar write was, for the final status line.
enum BulkKind {
    Paste,
    Caption,
    Stationery,
    Rotate { dir: &'static str },
}

/// A bulk sidecar write (paste, caption-to-selection, stationery, rotate)
/// running on the mark-writer thread. The UI stays responsive while the
/// files land; each file's result is recorded here, and the final status
/// is set when the last file reports in.
struct BulkWrite {
    /// Sequence numbers still to report in.
    remaining: HashSet<u64>,
    /// Photos whose write landed.
    ok: usize,
    /// Files written across the photos that landed.
    files: usize,
    /// Photos whose write failed.
    failed: usize,
    last_error: Option<String>,
    /// Photos the bulk was asked to write.
    total: usize,
    kind: BulkKind,
}

impl BulkWrite {
    fn new(seqs: Vec<u64>, total: usize, kind: BulkKind) -> Self {
        BulkWrite {
            remaining: seqs.into_iter().collect(),
            ok: 0,
            files: 0,
            failed: 0,
            last_error: None,
            total,
            kind,
        }
    }

    /// Record one file's result; returns the final status text when the
    /// last file reports in.
    fn record(&mut self, seq: u64, result: &Result<usize, String>) -> Option<String> {
        if !self.remaining.remove(&seq) {
            return None;
        }
        match result {
            Ok(n) => {
                self.ok += 1;
                self.files += n;
            }
            Err(e) => {
                self.failed += 1;
                self.last_error = Some(e.clone());
            }
        }
        if !self.remaining.is_empty() {
            return None;
        }
        Some(self.finish_text())
    }

    fn finish_text(&self) -> String {
        let s = if self.total == 1 { "" } else { "s" };
        match &self.kind {
            BulkKind::Paste => format!(
                "Pasted metadata onto {} photo{} ({} files{})",
                self.total,
                s,
                self.files,
                if self.failed > 0 {
                    format!(", {} failed", self.failed)
                } else {
                    String::new()
                }
            ),
            BulkKind::Caption => match &self.last_error {
                None => format!("Caption fields saved to {} photos", self.ok),
                Some(e) => format!(
                    "Saved to {} of {} photos; last error: {e}",
                    self.ok, self.total
                ),
            },
            BulkKind::Stationery => match &self.last_error {
                Some(e) => format!("Pad failed: {e}"),
                None => format!(
                    "Assignment pad applied to {} photo{} → {} files",
                    self.total, s, self.files
                ),
            },
            BulkKind::Rotate { dir } => match self.failed {
                0 => format!("Rotated {} photo{} {dir}", self.ok, s),
                f => format!(
                    "Rotated {} {dir}; {f} could not be written (see log)",
                    self.ok
                ),
            },
        }
    }
}

/// A single caption save running on the mark-writer thread. The draft was
/// marked clean when the save was queued; if the write fails, the draft's
/// original fields are restored here so the caption goes dirty again
/// instead of silently vanishing.
struct CaptionWrite {
    seq: u64,
    idx: usize,
    original: crate::ui::caption::DraftFields,
    announce: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let cache = apature_core::project_dirs()
            .and_then(|d| ThumbCache::new(d.cache_dir().join("thumbs")).ok());
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let threads = (threads.saturating_sub(1)).clamp(2, 12);
        let pool = Pool::new(cc.egui_ctx.clone(), cache, threads);
        let ledger_path = apature_core::project_dirs()
            .map(|d| d.data_dir().join("ingested.json"))
            .unwrap_or_else(|| PathBuf::from("ingested.json"));
        if let Some(theme) = launch.theme {
            cc.egui_ctx.set_theme(theme);
        }
        ui::tokens::install_fonts(&cc.egui_ctx);
        let (prefs, prefs_outcome) = Prefs::load_report();
        // Recent and favourite folders that no longer exist drop off the
        // sidebar (scratch folders, a renamed shoot); a folder on an unplugged
        // drive stays, since /Volumes/<name> comes back.
        let mut prefs = prefs;
        prefs
            .recent
            .retain(|p| p.exists() || p.starts_with("/Volumes"));
        prefs
            .favorites
            .retain(|p| p.exists() || p.starts_with("/Volumes"));
        prefs
            .tree_expanded
            .retain(|p| p.is_dir() || p.starts_with("/Volumes"));
        let tree = crate::folder_tree::FolderTree::new(prefs.tree_expanded.clone());
        let platform = Platform::new(prefs.shortcut_layout);
        let mut app = App {
            folder: None,
            items: Vec::new(),
            marks: Vec::new(),
            conflicts: Vec::new(),
            exif: Vec::new(),
            thumbs: Vec::new(),
            thumb_err: Vec::new(),
            native: Vec::new(),
            previews: HashMap::new(),
            preview_lru: VecDeque::new(),
            pool,
            generation: 0,
            cursor: 0,
            selection: Selection::default(),
            view: Vec::new(),
            stacks: HashMap::new(),
            stacks_open: HashSet::new(),
            pos: Vec::new(),
            filter: Filter::default(),
            undo: UndoStack::default(),
            workspace: Workspace::Sheet,
            ingest_window: false,
            ingest_sheet_pending: None,
            cols: 1,
            last_visible_rows: 0..0,
            scroll_to_cursor: false,
            scheme: prefs.scheme.clone(),
            stats: Stats::default(),
            status: String::new(),
            prefs,
            prefs_dirty: false,
            prefs_saved_at: Instant::now(),
            volumes: Vec::new(),
            volume_kinds: HashMap::new(),
            volume_devs: HashMap::new(),
            tree,
            folder_unplugged: None,
            toasts: Vec::new(),
            preview_boost: 0,
            window_title: String::new(),
            caption_kw_draft: String::new(),
            filter_popup_open: false,
            caption_draft: None,
            assignment: None,
            codes: CodeTable::default(),
            ingest: IngestState::default(),
            export: ExportState::default(),
            deliver: DeliverState {
                sent: load_delivered(),
                ..Default::default()
            },
            pv: PreviewView::default(),
            rename_dialog: None,
            meta_clip: None,
            trash_dialog: None,
            copy_dialog: None,
            bug_dialog: None,
            copy_running: None,
            copy_open_after: false,
            copy_jobs: crate::copy_job::CopyJobs::new(cc.egui_ctx.clone()),
            extract_dialog: None,
            extract_running: None,
            extract_jobs: crate::extract_job::ExtractJobs::new(cc.egui_ctx.clone()),
            key_once: Default::default(),
            mark_writer: Default::default(),
            mark_status: None,
            bulk_write: None,
            caption_write: None,
            export_jobs: crate::export_job::ExportJobs::new(cc.egui_ctx.clone()),
            deliver_jobs: DeliverJobs::new(cc.egui_ctx.clone()),
            ingest_jobs: IngestJobs::new(cc.egui_ctx.clone(), ledger_path),
            drag_hover: 0,
            inspector_iptc: None,
            tabs: Vec::new(),
            active: 0,
            next_generation: 1,
            settings_open: false,
            about_open: launch.about,
            preview_window: false,
            update_checker: None,
            update_install: None,
            update_manual: false,
            update_available: None,
            settings_page: ui::settings::Page::General,
            cache_size: None,
            applied_appearance: None,
            dispatch_later: None,
            platform,
            system_reduce_motion: Platform::reduce_motion(),
            applied_ui_scale: None,
            sheet_fade_since: None,
            preview_since: None,
            preview_sidebar: false,
            preview_inspector: None,
            tab_toggle: false,
            preview_from: None,
            cursor_cell_rect: None,
            focus_from: None,
            focus_since: None,
            volumes_checked: Instant::now(),
            poll_rx: None,
            status_since: None,
            last_status: String::new(),
            launch: launch.clone(),
            quit_after_frame: false,
            run_done: false,
            frames: 0,
            screenshot_requested: false,
        };
        app.sync_menu();
        app.refresh_volumes();
        if let Some(p) = app.prefs.assignment_file.clone() {
            app.load_assignment(&p);
        }
        if let Some(p) = app.prefs.codes_file.clone() {
            app.load_codes(&p);
        }
        if let Some(p) = launch.assignment.clone() {
            app.load_assignment(&p);
        }
        app.platform.set_class_names(&app.scheme.names);
        let restored: Vec<PathBuf> = app
            .prefs
            .open_tabs
            .clone()
            .into_iter()
            .filter(|f| f.is_dir())
            .collect();
        for f in &restored {
            app.open_folder(f);
        }
        if !restored.is_empty() {
            let want = app.prefs.active_tab.min(app.tabs.len().saturating_sub(1));
            app.switch_tab(want);
        }
        let first = launch.folder.clone().filter(|f| f.is_dir()).or_else(|| {
            if restored.is_empty() {
                app.prefs.recent.first().cloned().filter(|f| f.is_dir())
            } else {
                None
            }
        });
        if let Some(f) = first {
            app.open_folder(&f);
        }
        for f in launch.more_folders.iter().filter(|f| f.is_dir()) {
            app.open_folder(f);
        }
        if launch.settings {
            app.settings_open = true;
        }
        if launch.demo && !app.view.is_empty() {
            app.selection.all(&app.view);
            app.cursor = app.view[0];
            app.filter_popup_open = true;
        }
        if launch.caption && !app.view.is_empty() {
            app.workspace = Workspace::Caption;
        }
        app.ingest.opts = app
            .prefs
            .ingest
            .clone()
            .unwrap_or_else(crate::prefs::default_ingest_options);
        if let Some(s) = launch.ingest_source.clone() {
            app.ingest.opts.source = s;
        }
        if let Some(d) = launch.ingest_dest.clone() {
            app.ingest.opts.primary = d;
        }
        match prefs_outcome {
            crate::prefs::LoadOutcome::Damaged { error, kept } => {
                log::error!(
                    "prefs.json unreadable ({error}); kept as {}",
                    kept.display()
                );
                app.toast(
                    ui::toasts::Kind::Error,
                    format!(
                        "Settings could not be read and were reset. The old file is kept as {}.",
                        kept.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ),
                );
            }
            crate::prefs::LoadOutcome::Migrated { from } => {
                log::info!("prefs migrated from version {from}; backup kept");
            }
            crate::prefs::LoadOutcome::Newer { version } => {
                log::warn!("prefs.json is version {version}, newer than this build; running read-only on it");
                app.toast(
                    ui::toasts::Kind::Warn,
                    "Settings were saved by a newer Apature. This build reads them but will not change the file.",
                );
            }
            _ => {}
        }
        if launch.ingest {
            app.open_ingest(&cc.egui_ctx);
            app.ingest_options_changed();
            if launch.live {
                app.dispatch_later = Some(Command::LiveToggle);
            }
        }
        if launch.preview && !app.view.is_empty() {
            app.enter_preview();
            if launch.crop {
                app.dispatch_later = Some(Command::CropToggle);
            }
            app.preview_since = None;
            if launch.compare {
                let picks: Vec<usize> = app.view.iter().take(3).copied().collect();
                app.selection.only(picks[0]);
                for &i in &picks[1..] {
                    app.selection.click(
                        i,
                        Mods {
                            shift: false,
                            command: true,
                        },
                        &app.view,
                    );
                }
                app.toggle_compare();
            }
            if launch.zoom11 {
                app.toggle_zoom(Some(egui::Vec2::new(0.5, 0.35)), None);
                app.pv.zoom_since = None;
            }
        }
        if launch.rename_dialog && !app.view.is_empty() {
            app.selection.all(&app.view.clone());
            app.open_rename_dialog();
            if let Some(d) = app.rename_dialog.as_mut() {
                d.template = "100726-SPO-BGLF-{seqn:3}".into();
            }
            app.rename_recompute();
        }
        if launch.sidebar_shot {
            // Screenshot of the folder tree (scratch HOME, see main.rs): the
            // open folder's grandparent as a favorite, the tree focused.
            if app.prefs.favorites.is_empty() {
                if let Some(f) = app
                    .folder
                    .as_ref()
                    .and_then(|f| f.parent())
                    .and_then(|f| f.parent())
                {
                    app.prefs.favorites.push(f.to_path_buf());
                }
            }
            cc.egui_ctx
                .memory_mut(|m| m.request_focus(ui::sidebar::tree_id()));
        }
        if launch.extract_dialog && !app.view.is_empty() {
            app.open_extract_dialog();
        }
        if launch.copy_dialog && !app.view.is_empty() {
            app.selection.all(&app.view.clone());
            app.open_copy_dialog();
        }
        if launch.bug_dialog {
            app.open_bug_dialog();
            if let Some(d) = app.bug_dialog.as_mut() {
                d.report = crate::bug_report::BugReport {
                    title: "Crop guides vanish after rotating a photo".into(),
                    happened: "After rotating a NEF with ] the crop guides (G) no longer draw.".into(),
                    steps: "1. Open a folder of NEFs\n2. Press ] on a photo\n3. Press K, then G".into(),
                    expected: "The rule-of-thirds guides over the crop.".into(),
                };
            }
        }
        if launch.trash_dialog && !app.view.is_empty() {
            let picks: Vec<usize> = app.view.iter().take(2).copied().collect();
            app.trash_dialog = Some(picks);
        }
        if launch.export {
            app.workspace = Workspace::Export;
            app.export.scope_all = true;
            if let Some(d) = launch.export_dest.clone() {
                app.prefs.export.dest = Some(d);
            }
            if launch.export_run {
                app.dispatch_later = Some(Command::ExportStart);
            }
        }
        if let Some(image) = launch.install_update.clone() {
            // Hidden `--install-update IMAGE`: the whole swap from a local disk
            // image, so it can be checked without a release.
            if let Err(e) = app.install_from_image(image, &cc.egui_ctx) {
                eprintln!("install-update: {e}");
                app.quit_after_frame = true;
            }
        }
        app
    }

    // ----- file operations ---------------------------------------------------

    pub fn open_rename_dialog(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let mut targets = self.selection.targets(self.cursor);
        targets.sort_by_key(|&i| self.pos[i].unwrap_or(usize::MAX));
        self.rename_dialog = Some(RenameDialog {
            template: self.prefs.rename_template.clone(),
            seq_start: 1,
            targets,
            plan: None,
            error: None,
        });
        self.rename_recompute();
    }

    /// New stem for `idx` under `template` with sequence number `seq`.
    fn rename_stem(&self, idx: usize, template: &str, seq: u32) -> String {
        let it = &self.items[idx];
        let headline = self.assignment.as_ref().map(|a| a.slug.clone());
        let facts = apature_ingest::plan::facts_for(it, it.path(), headline.as_deref());
        let mut name = apature_ingest::template::expand(template, &facts, seq);
        // The template names the photo, not a file: drop a typed extension.
        for v in it.variants() {
            if let Some(ext) = v.path.extension().and_then(|e| e.to_str()) {
                let suffix = format!(".{ext}");
                if name.to_lowercase().ends_with(&suffix.to_lowercase()) {
                    name.truncate(name.len() - suffix.len());
                }
            }
        }
        name
    }

    pub fn rename_recompute(&mut self) {
        let Some(d) = self.rename_dialog.as_ref() else {
            return;
        };
        let template = d.template.trim().to_string();
        let seq_start = d.seq_start;
        let targets = d.targets.clone();
        if template.is_empty() {
            if let Some(d) = self.rename_dialog.as_mut() {
                d.plan = None;
                d.error = None;
            }
            return;
        }
        let stems: Vec<(usize, String)> = targets
            .iter()
            .enumerate()
            .map(|(i, &idx)| (idx, self.rename_stem(idx, &template, seq_start + i as u32)))
            .collect();
        let pairs: Vec<(&PhotoItem, String)> = stems
            .iter()
            .map(|(idx, s)| (&self.items[*idx], s.clone()))
            .collect();
        let result = apature_core::fileops::plan_rename(&pairs);
        if let Some(d) = self.rename_dialog.as_mut() {
            match result {
                Ok(p) => {
                    d.plan = Some(p);
                    d.error = None;
                }
                Err(e) => {
                    d.plan = None;
                    d.error = Some(e.to_string());
                }
            }
        }
    }

    fn rename_apply(&mut self) {
        let Some(d) = self.rename_dialog.as_ref() else {
            return;
        };
        let Some(plan) = d.plan.clone() else {
            return;
        };
        let template = d.template.trim().to_string();
        match apature_core::fileops::rename(&plan) {
            Ok(moves) => {
                let mut photos = 0;
                for it in self.items.iter_mut() {
                    let mut touched = false;
                    for v in std::iter::once(&mut it.primary).chain(it.secondary.iter_mut()) {
                        if let Some((_, to)) = moves.iter().find(|(from, _)| *from == v.path) {
                            v.path = to.clone();
                            touched = true;
                        }
                    }
                    if touched {
                        it.stem = it
                            .primary
                            .path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| it.stem.clone());
                        photos += 1;
                    }
                }
                self.prefs.rename_template = template;
                self.prefs_dirty = true;
                self.rename_dialog = None;
                self.rebuild_view();
                self.status = format!(
                    "Renamed {photos} photo{} ({} files)",
                    if photos == 1 { "" } else { "s" },
                    moves.len()
                );
            }
            Err(e) => {
                if let Some(d) = self.rename_dialog.as_mut() {
                    d.error = Some(e.to_string());
                }
            }
        }
    }

    // ----- Select by, Copy Photos -------------------------------------------

    /// Edit › Select Tagged / by Rating / by Color Class / Cropped / Pairs:
    /// replace the selection with the matching visible photos.
    fn select_by(&mut self, by: crate::select_by::SelectBy) {
        let items = &self.items;
        let picks = crate::select_by::pick(by, &self.view, &self.marks, items, |i| {
            edits::read(&items[i].sidecar_path())
                .ok()
                .and_then(|e| e.crop)
                .is_some_and(|c| !c.is_full())
        });
        let what = by.describe(&self.scheme.names);
        if picks.is_empty() {
            self.status = format!("No {what} photos in this tab");
            return;
        }
        self.selection.all(&picks);
        if !self.selection.contains(self.cursor) {
            self.cursor = picks[0];
            self.scroll_to_cursor = true;
        }
        self.status = format!(
            "Selected {} {what} photo{}",
            picks.len(),
            if picks.len() == 1 { "" } else { "s" }
        );
    }

    /// Help › Report a Bug…; an open dialog keeps what was typed.
    pub fn open_bug_dialog(&mut self) {
        if self.bug_dialog.is_some() {
            return;
        }
        let focused = self.items.get(self.cursor).map(|i| i.path().to_path_buf());
        self.bug_dialog = Some(BugDialog {
            report: Default::default(),
            include_info: true,
            info: crate::bug_report::SystemInfo::collect(self.prefs.shortcut_layout, focused.as_deref()),
            notice: None,
            select_all: false,
            focus_title: true,
        });
    }

    pub fn open_copy_dialog(&mut self) {
        if self.copy_running.as_ref().is_some_and(|r| r.done.is_none()) {
            // A copy is running: show its progress again.
            self.copy_dialog.get_or_insert(CopyDialog {
                targets: Vec::new(),
                opts: self.prefs.copy.clone(),
                apply_pad: false,
                plan: None,
                error: None,
            });
            return;
        }
        self.copy_running = None;
        if self.items.is_empty() {
            return;
        }
        let mut targets = self.selection.targets(self.cursor);
        targets.sort_by_key(|&i| self.pos[i].unwrap_or(usize::MAX));
        let mut opts = self.prefs.copy.clone();
        opts.headline = self.assignment.as_ref().map(|a| a.slug.clone());
        self.copy_dialog = Some(CopyDialog {
            targets,
            opts,
            apply_pad: false,
            plan: None,
            error: None,
        });
        self.copy_recompute();
    }

    /// Extract JPEG Previews from RAW Photos…: the selected photos, or every
    /// photo in the tab when none is selected.
    pub fn open_extract_dialog(&mut self) {
        if self
            .extract_running
            .as_ref()
            .is_some_and(|r| r.done.is_none())
        {
            self.extract_dialog.get_or_insert(ExtractDialog {
                targets: Vec::new(),
                raws: 0,
            });
            return;
        }
        self.extract_running = None;
        if self.view.is_empty() {
            return;
        }
        let mut targets: Vec<usize> = self
            .view
            .iter()
            .copied()
            .filter(|&i| self.selection.contains(i))
            .collect();
        if targets.is_empty() {
            targets = self.view.clone();
        }
        let raws = targets
            .iter()
            .filter(|&&i| apature_export::extract::raw_of(&self.items[i]).is_some())
            .count();
        self.extract_dialog = Some(ExtractDialog { targets, raws });
    }

    fn extract_start(&mut self) {
        let Some(d) = self.extract_dialog.as_ref() else {
            return;
        };
        if d.targets.is_empty() {
            return;
        }
        let opts = self.prefs.extract;
        let dest = if opts.to_source {
            None
        } else {
            let mut dlg = rfd::FileDialog::new().set_title("Extract JPEG Previews To");
            if let Some(f) = &self.folder {
                dlg = dlg.set_directory(f);
            }
            match dlg.pick_folder() {
                Some(dir) => Some(dir),
                None => return,
            }
        };
        let photos: Vec<PhotoItem> = d.targets.iter().map(|&i| self.items[i].clone()).collect();
        self.prefs_dirty = true;
        self.extract_running =
            Some(
                self.extract_jobs
                    .start(photos, dest, opts, self.scheme.clone()),
            );
    }

    fn drain_extract(&mut self) {
        while let Some(msg) = self.extract_jobs.try_recv() {
            let Some(r) = self.extract_running.as_mut() else {
                continue;
            };
            match msg {
                crate::extract_job::ExtractMsg::Progress(p) => {
                    if self.extract_dialog.is_none() && r.done.is_none() {
                        self.status = format!("Extracting previews: {} of {}…", p.done, p.total);
                    }
                    r.progress = Some(p);
                }
                crate::extract_job::ExtractMsg::Done(o) => {
                    let n = o.extracted.len();
                    let mut text = format!(
                        "Extracted {n} JPEG preview{}",
                        if n == 1 { "" } else { "s" }
                    );
                    if !o.skipped.is_empty() {
                        text += &format!(" · {} skipped", o.skipped.len());
                    }
                    if o.cancelled {
                        text += " · cancelled";
                    }
                    let into_open = self.folder.as_ref().is_some_and(|f| {
                        o.extracted
                            .iter()
                            .any(|(_, j)| j.parent() == Some(f.as_path()))
                    });
                    r.done = Some(o);
                    self.status = text.clone();
                    if self.extract_dialog.is_none() {
                        self.toast(ui::toasts::Kind::Info, text);
                    }
                    if into_open {
                        self.refresh_folder();
                    }
                }
            }
        }
    }

    /// Close the dialog; a running copy carries on and reports in the status line.
    fn close_copy_dialog(&mut self) {
        self.copy_dialog = None;
        if self.copy_running.as_ref().is_some_and(|r| r.done.is_some()) {
            self.copy_running = None;
        }
    }

    pub fn copy_recompute(&mut self) {
        let Some(d) = self.copy_dialog.as_ref() else {
            return;
        };
        let needs_folder = matches!(
            d.opts.dest_mode,
            apature_ingest::DestMode::ThisFolder | apature_ingest::DestMode::AlwaysPick
        );
        if needs_folder && d.opts.dest.as_os_str().is_empty() {
            if let Some(d) = self.copy_dialog.as_mut() {
                d.plan = None;
                d.error = None;
            }
            return;
        }
        let refs: Vec<&PhotoItem> = d.targets.iter().map(|&i| &self.items[i]).collect();
        let result = apature_ingest::plan_copy(&refs, &d.opts);
        if let Some(d) = self.copy_dialog.as_mut() {
            match result {
                Ok(p) => {
                    d.plan = Some(p);
                    d.error = None;
                }
                Err(e) => {
                    d.plan = None;
                    d.error = Some(e.to_string());
                }
            }
        }
    }

    fn copy_choose_dest(&mut self) {
        let Some(d) = self.copy_dialog.as_ref() else {
            return;
        };
        let mut dlg = rfd::FileDialog::new().set_title("Copy Photos To");
        if d.opts.dest.is_dir() {
            dlg = dlg.set_directory(&d.opts.dest);
        }
        if let Some(dir) = dlg.pick_folder() {
            if let Some(d) = self.copy_dialog.as_mut() {
                d.opts.dest = dir;
                if d.opts.dest_mode != apature_ingest::DestMode::AlwaysPick {
                    d.opts.dest_mode = apature_ingest::DestMode::ThisFolder;
                }
            }
            self.copy_recompute();
        }
    }

    fn copy_start(&mut self) {
        let Some(d) = self.copy_dialog.as_ref() else {
            return;
        };
        if d.opts.dest_mode == apature_ingest::DestMode::AlwaysPick {
            // Always pick destination: ask now, then plan against the pick.
            let mut dlg = rfd::FileDialog::new().set_title("Copy Photos To");
            if d.opts.dest.is_dir() {
                dlg = dlg.set_directory(&d.opts.dest);
            }
            let Some(dir) = dlg.pick_folder() else {
                return;
            };
            if let Some(d) = self.copy_dialog.as_mut() {
                d.opts.dest = dir;
            }
            self.copy_recompute();
        }
        let Some(d) = self.copy_dialog.as_ref() else {
            return;
        };
        let Some(plan) = d.plan.clone().filter(|p| !p.entries.is_empty()) else {
            return;
        };
        let pad = if d.apply_pad {
            self.assignment.as_ref().map(|a| a.iptc())
        } else {
            None
        };
        let mut keep = d.opts.clone();
        keep.headline = None;
        // Where the first photo goes (its own folder in the per-original modes).
        let dest = d
            .targets
            .first()
            .and_then(|&i| apature_ingest::dest_base(&d.opts, &self.items[i]))
            .unwrap_or_else(|| keep.dest.clone());
        self.copy_open_after = keep.open_after;
        self.prefs.copy = keep;
        self.prefs_dirty = true;
        self.copy_running = Some(self.copy_jobs.start(plan, pad, dest));
    }

    fn drain_copy(&mut self) {
        let mut open_after: Option<PathBuf> = None;
        while let Some(msg) = self.copy_jobs.try_recv() {
            let Some(r) = self.copy_running.as_mut() else {
                continue;
            };
            match msg {
                crate::copy_job::CopyMsg::Progress(p) => {
                    if self.copy_dialog.is_none() && r.done.is_none() {
                        self.status = format!(
                            "Copying photos: {} of {} files…",
                            p.files_done, p.files_total
                        );
                    }
                    r.progress = Some(p);
                }
                crate::copy_job::CopyMsg::Done(o) => {
                    r.finished.get_or_insert_with(std::time::Instant::now);
                    crate::copy_job::log_outcome(&o, r.elapsed());
                    let files = o.copied.len();
                    let text = if o.cancelled {
                        format!(
                            "Copy cancelled after {files} file{}",
                            if files == 1 { "" } else { "s" }
                        )
                    } else if o.failed.is_empty() {
                        format!(
                            "Copied {} photo{} ({files} files) to {}",
                            r.photos,
                            if r.photos == 1 { "" } else { "s" },
                            r.dest.display()
                        )
                    } else {
                        format!("Copied {files} files; {} failed", o.failed.len())
                    };
                    let kind = if o.failed.is_empty() {
                        ui::toasts::Kind::Info
                    } else {
                        ui::toasts::Kind::Error
                    };
                    let into_open = self.folder.as_ref().is_some_and(|f| {
                        r.dest.starts_with(f) || o.copied.iter().any(|(_, d)| d.starts_with(f))
                    });
                    if self.copy_open_after && !o.cancelled {
                        // Open as a Contact Sheet: the folder the first photo landed in.
                        open_after = o
                            .copied
                            .first()
                            .and_then(|(_, d)| d.parent().map(Path::to_path_buf));
                    }
                    r.done = Some(o);
                    self.status = text.clone();
                    if self.copy_dialog.is_none() {
                        self.toast(kind, text);
                    }
                    if into_open {
                        self.refresh_folder();
                    }
                }
            }
        }
        if let Some(dir) = open_after {
            self.close_copy_dialog();
            self.open_folder(&dir);
        }
    }

    fn trash_confirm(&mut self) {
        let Some(targets) = self.trash_dialog.take() else {
            return;
        };
        let files: Vec<PathBuf> = targets
            .iter()
            .flat_map(|&i| apature_core::fileops::item_files(&self.items[i]))
            .collect();
        let n = targets.len();
        match apature_core::fileops::trash(&files) {
            Ok(()) => {
                let pos = self.cursor_pos();
                if let Some(dir) = self.folder.clone() {
                    self.load_folder(&dir);
                }
                if !self.view.is_empty() {
                    let p = pos.min(self.view.len() - 1);
                    self.cursor = self.view[p];
                    self.selection.only(self.cursor);
                    self.scroll_to_cursor = true;
                }
                self.status = format!(
                    "Moved {n} photo{} ({} files) to the Trash",
                    if n == 1 { "" } else { "s" },
                    files.len()
                );
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    // ----- export ------------------------------------------------------------

    fn export_targets(&self) -> Vec<usize> {
        if self.export.scope_all {
            self.view.clone()
        } else {
            self.selection.targets(self.cursor)
        }
    }

    /// Planned output paths for the current scope, names and destination.
    /// Cached on those inputs so the workspace can show them every frame.
    pub fn export_names(&mut self) -> Vec<(usize, PathBuf)> {
        let Some(dest) = self.prefs.export.dest.clone() else {
            return Vec::new();
        };
        let targets = self.export_targets();
        let headline = self.assignment.as_ref().map(|a| a.slug.clone());
        let key = (
            targets.clone(),
            self.prefs.export.rename.clone(),
            dest.clone(),
            headline.clone(),
        );
        if self.export.names_key.as_ref() == Some(&key) {
            return self.export.names.clone();
        }
        let template = self.prefs.export.rename.trim().to_string();
        let mut names = Vec::with_capacity(targets.len());
        let mut used: HashSet<String> = HashSet::new();
        for (i, &idx) in targets.iter().enumerate() {
            let it = &self.items[idx];
            let mut facts = apature_ingest::plan::facts_for(it, it.path(), headline.as_deref());
            facts.ext = "jpg".into();
            let name = export_file_name(&template, &facts, (i + 1) as u32);
            let name = dedup_export_name(&name, &mut used);
            names.push((idx, dest.join(name)));
        }
        self.export.names = names.clone();
        self.export.names_key = Some(key);
        names
    }

    fn export_start(&mut self) {
        let Some(dest) = self.prefs.export.dest.clone() else {
            return;
        };
        if self.export.running.as_ref().is_some_and(|r| !r.finished) {
            return;
        }
        let names = self.export_names();
        if names.is_empty() {
            if self.launch.export_run {
                eprintln!("export-run: nothing to export");
                self.finish_headless_run();
            }
            return;
        }
        let pad = self.assignment.as_ref().map(|a| a.iptc());
        let jobs = names
            .iter()
            .map(|(idx, out)| crate::export_job::Job {
                item: self.items[*idx].clone(),
                out: out.clone(),
                pad: pad.clone(),
            })
            .collect();
        let settings = apature_export::Settings {
            long_side: self.prefs.export.long_side,
            quality: self.prefs.export.quality,
            keep_exif: self.prefs.export.keep_exif,
            write_iim: self.prefs.export.write_iim,
            raw: if self.prefs.export.decode_raw {
                apature_export::RawPixels::SystemDecode
            } else {
                apature_export::RawPixels::EmbeddedPreview
            },
            format: if self.prefs.export.jxl {
                apature_export::Format::JpegXl
            } else {
                apature_export::Format::Jpeg
            },
            ..Default::default()
        };
        if self.launch.export_run {
            eprintln!("export-run: {} files to {}", names.len(), dest.display());
        }
        self.export.running = Some(self.export_jobs.start(jobs, settings, dest));
        self.status = "Exporting…".into();
    }

    fn drain_export(&mut self) {
        use crate::export_job::{Msg as EMsg, Status as EStatus};
        while let Some(msg) = self.export_jobs.try_recv() {
            let finished = {
                let Some(r) = self.export.running.as_mut() else {
                    continue;
                };
                match msg {
                    EMsg::Started { index } => {
                        if let Some(s) = r.statuses.get_mut(index) {
                            *s = EStatus::Working;
                        }
                        None
                    }
                    EMsg::Done { index, result } => {
                        match result {
                            Ok(o) => {
                                r.ok += 1;
                                r.bytes += o.bytes;
                                r.outs[index] = Some(o.out);
                                r.statuses[index] = EStatus::Done;
                            }
                            Err(e) => {
                                r.failed += 1;
                                r.errors[index] = Some(e);
                                r.statuses[index] = EStatus::Failed;
                            }
                        }
                        None
                    }
                    EMsg::Finished => {
                        r.finished = true;
                        r.done_since = Some(Instant::now());
                        let errors: Vec<String> = r.errors.iter().flatten().cloned().collect();
                        let outs: Vec<PathBuf> = r.outs.iter().flatten().cloned().collect();
                        Some((r.ok, r.failed, r.dest.clone(), errors, outs))
                    }
                }
            };
            if let Some((ok, failed, dest, errors, outs)) = finished {
                self.deliver.last_outputs = outs;
                self.deliver.source_of = self
                    .export
                    .names
                    .iter()
                    .filter_map(|(i, out)| {
                        self.items
                            .get(*i)
                            .map(|it| (out.clone(), it.path().to_path_buf()))
                    })
                    .collect();
                if failed == 0 && !self.deliver.last_outputs.is_empty() {
                    if let Some(name) = self.prefs.send_after_export.clone() {
                        self.start_delivery(&name);
                    }
                }
                self.status = if failed == 0 {
                    format!(
                        "Export complete: {ok} JPEG{}",
                        if ok == 1 { "" } else { "s" }
                    )
                } else {
                    format!("Export finished with {failed} failed file(s)")
                };
                let msg = self.status.clone();
                self.toast(
                    if failed == 0 {
                        ui::toasts::Kind::Info
                    } else {
                        ui::toasts::Kind::Error
                    },
                    msg,
                );
                if self.launch.export_run {
                    eprintln!("export-run: {ok} ok, {failed} failed");
                    for e in &errors {
                        eprintln!("export-run: {e}");
                    }
                    self.finish_headless_run();
                } else if failed == 0 && self.prefs.export.open_after {
                    self.dispatch_later = Some(Command::OpenFolder(dest));
                }
            }
        }
    }

    // ----- ingest ------------------------------------------------------------

    pub const INGEST_VIEWPORT: &'static str = "apature-ingest";

    fn start_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.update_checker.is_some() || self.update_install.is_some() {
            return;
        }
        self.update_manual = manual;
        self.update_checker = Some(crate::updater::Checker::spawn(
            crate::updater::current_version(),
            self.prefs.beta_updates,
            ctx.clone(),
        ));
        self.prefs.last_update_check = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.prefs_dirty = true;
    }

    /// The one toast that shows an install's progress: its text is replaced as
    /// the work advances instead of stacking a toast per chunk.
    fn update_progress(&mut self, text: String) {
        match self.toasts.iter_mut().find(|t| t.progress) {
            Some(t) => t.text = text,
            None => self.toasts.push(ui::toasts::Toast {
                text,
                kind: ui::toasts::Kind::Warn,
                since: Instant::now(),
                action: None,
                progress: true,
            }),
        }
    }

    /// A long job that the quit an update needs must not cut off.
    fn busy_job(&self) -> Option<&'static str> {
        if self.ingest.running.is_some() {
            return Some("ingest");
        }
        if self.export.running.is_some() {
            return Some("export");
        }
        if self.deliver.running.is_some() {
            return Some("send");
        }
        if self.copy_running.as_ref().is_some_and(|r| r.done.is_none()) {
            return Some("copy");
        }
        if self
            .extract_running
            .as_ref()
            .is_some_and(|r| r.done.is_none())
        {
            return Some("preview extraction");
        }
        None
    }

    fn update_failed(&mut self, update: &crate::updater::Update, error: &str) {
        log::warn!("cannot install update {}: {error}", update.version);
        self.toasts.retain(|t| !t.progress);
        self.toasts.push(ui::toasts::Toast {
            text: format!("Could not install Apature {}: {error}", update.version),
            kind: ui::toasts::Kind::Error,
            since: Instant::now(),
            // A build that cannot replace itself still has the release page.
            action: Some(("Release page".into(), Command::OpenUpdatePage)),
            progress: false,
        });
    }

    /// Update now: download the release, stage it and swap it in for this
    /// copy. The app quits once the swap script is ready to run.
    fn start_update_install(&mut self, ctx: &egui::Context) {
        if self.update_install.is_some() {
            return;
        }
        let Some(update) = self.update_available.clone() else {
            self.status = "No update is ready to install.".into();
            return;
        };
        if let Some(job) = self.busy_job() {
            self.toast(
                ui::toasts::Kind::Warn,
                format!("Apature quits to install an update. Finish the {job} first."),
            );
            return;
        }
        match crate::update_install::Installer::spawn(&update, ctx.clone()) {
            Ok(installer) => {
                self.update_install = Some(installer);
                self.toasts.retain(|t| {
                    !t.progress && !(t.text.starts_with("Apature ") && t.text.contains("is available"))
                });
                log::info!("installing update {}", update.version);
                self.status = format!("Downloading Apature {}…", update.version);
                self.update_progress(self.status.clone());
            }
            Err(e) => self.update_failed(&update, &e),
        }
    }

    /// Drain the install thread: progress to the one toast, a failure to an
    /// error toast, and a quit once the new app is staged for the swap.
    fn update_install_tick(&mut self) {
        let Some(installer) = self.update_install.as_ref() else {
            return;
        };
        let events: Vec<crate::update_install::Progress> =
            std::iter::from_fn(|| installer.try_recv()).collect();
        if events.is_empty() {
            return;
        }
        let label = match self.update_available.as_ref() {
            Some(u) => format!("Apature {}", u.version),
            None => "Apature".to_string(),
        };
        let mut done = false;
        for event in events {
            match event {
                crate::update_install::Progress::Downloading { got, total } => {
                    let text = match total {
                        Some(total) if total > 0 => {
                            format!("Downloading {label}… {}%", got * 100 / total)
                        }
                        _ if got > 0 => {
                            format!("Downloading {label}… {} MB", got / (1024 * 1024))
                        }
                        _ => format!("Downloading {label}…"),
                    };
                    self.status = text.clone();
                    self.update_progress(text);
                }
                crate::update_install::Progress::Unpacking => {
                    let text = format!("Installing {label}…");
                    self.status = text.clone();
                    self.update_progress(text);
                }
                crate::update_install::Progress::Swapping => {
                    let text = format!("Replacing this copy of Apature with {label}…");
                    self.status = text.clone();
                    self.update_progress(text);
                }
                crate::update_install::Progress::Installed => {
                    log::info!("update staged; quitting so the swap can replace this copy");
                    self.status = format!("Restarting into {label}…");
                    // Everything queued reaches disk before the app goes.
                    self.flush_marks();
                    self.prefs.save();
                    self.quit_after_frame = true;
                    done = true;
                }
                crate::update_install::Progress::Failed(e) => {
                    match self.update_available.clone() {
                        Some(u) => self.update_failed(&u, &e),
                        None => self.toast(ui::toasts::Kind::Error, e.clone()),
                    }
                    self.status = format!("Update failed: {e}");
                    done = true;
                }
            }
        }
        if done {
            self.update_install = None;
        }
        if self.launch.install_update.is_some() {
            eprintln!("install-update: {}", self.status);
        }
    }

    /// Hidden `--install-update IMAGE|URL`: the whole install checked without
    /// a release, from a disk image on this Mac or one to download.
    fn install_from_image(&mut self, image: PathBuf, ctx: &egui::Context) -> Result<(), String> {
        let text = image.to_string_lossy().into_owned();
        self.update_install = Some(if text.starts_with("http://") || text.starts_with("https://") {
            crate::update_install::Installer::from_url(text, ctx.clone())?
        } else {
            crate::update_install::Installer::from_image(image, ctx.clone())?
        });
        self.status = "Installing an update from a disk image…".into();
        Ok(())
    }

    /// Daily check when enabled (never in headless runs unless asked), and
    /// delivery of the result as a toast with an Update button.
    fn update_tick(&mut self, ctx: &egui::Context) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let headless = self.launch.screenshot.is_some();
        if self.launch.check_updates
            && self.update_checker.is_none()
            && self.update_available.is_none()
            && !self.update_manual
            && self.prefs.last_update_check < now - 5
        {
            self.start_update_check(ctx, true);
        } else if !headless
            && self.prefs.check_updates
            && self.update_checker.is_none()
            && now.saturating_sub(self.prefs.last_update_check) >= 86_400
        {
            self.start_update_check(ctx, false);
        }
        let Some(result) = self.update_checker.as_ref().and_then(|c| c.try_recv()) else {
            return;
        };
        self.update_checker = None;
        match result {
            Ok(Some(u)) => {
                log::info!("update available: {} ({})", u.version, u.page);
                if self.launch.check_updates {
                    eprintln!("check-updates: {} available at {}", u.version, u.page);
                }
                let skipped = self.prefs.skipped_update.as_deref() == Some(u.version.as_str());
                if !skipped || self.update_manual {
                    let text = format!(
                        "Apature {} is available{}. You have {}.",
                        u.version,
                        if u.prerelease { " (beta)" } else { "" },
                        crate::updater::current_version()
                    );
                    self.toasts.retain(|t| {
                        !t.text.starts_with("Apature ") || !t.text.contains("is available")
                    });
                    self.toasts.push(ui::toasts::Toast {
                        text,
                        kind: ui::toasts::Kind::Warn,
                        since: Instant::now(),
                        action: Some(("Update".into(), Command::OpenUpdate)),
                        progress: false,
                    });
                }
                self.update_available = Some(u);
            }
            Ok(None) => {
                log::info!("update check: up to date");
                if self.launch.check_updates {
                    eprintln!(
                        "check-updates: up to date ({} from {})",
                        crate::updater::current_version(),
                        crate::updater::repo()
                    );
                }
                if self.update_manual {
                    self.toast(
                        ui::toasts::Kind::Info,
                        format!(
                            "Apature {} is the newest version.",
                            crate::updater::current_version()
                        ),
                    );
                }
            }
            Err(e) => {
                log::warn!("update check failed: {e}");
                if self.launch.check_updates {
                    eprintln!("check-updates: failed: {e}");
                }
                if self.update_manual {
                    self.toast(
                        ui::toasts::Kind::Error,
                        format!("Could not check for updates: {e}"),
                    );
                }
            }
        }
        self.update_manual = false;
        if self.launch.check_updates {
            self.finish_headless_run();
        }
    }

    /// Post a toast; identical text replaces the older one.
    pub fn toast(&mut self, kind: ui::toasts::Kind, text: impl Into<String>) {
        let text = text.into();
        self.toasts.retain(|x| x.text != text);
        self.toasts.push(ui::toasts::Toast {
            text,
            kind,
            since: Instant::now(),
            action: None,
            progress: false,
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// Reduce Motion: the system setting unless Settings › Accessibility overrides it.
    pub fn reduce_motion(&self) -> bool {
        match self.prefs.motion {
            crate::prefs::Motion::System => self.system_reduce_motion,
            crate::prefs::Motion::Reduce => true,
            crate::prefs::Motion::Full => false,
        }
    }

    /// Where `{seqn}` should start for the current headline when the counter
    /// is persistent: the number after the last verified ingest with it.
    pub fn seq_start_for_headline(&self) -> u32 {
        let key = crate::prefs::seq_key(self.ingest.opts.headline.as_deref());
        self.prefs.seq_counters.get(&key).copied().unwrap_or(1)
    }

    /// Open Ingest: a separate window in normal use (the contact sheet stays
    /// in the main window, so culling can go on while a card copies), inline
    /// as a workspace for headless screenshot and `--ingest-run` sessions,
    /// which capture the main viewport only.
    pub fn open_ingest(&mut self, ctx: &egui::Context) {
        if self.launch.screenshot.is_some() || self.launch.ingest_run {
            self.workspace = Workspace::Ingest;
            return;
        }
        if self.ingest_window {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::from_hash_of(Self::INGEST_VIEWPORT),
                egui::ViewportCommand::Focus,
            );
        }
        self.ingest_window = true;
        // Opening the window reads the card straight away when the sheet is
        // already set up, instead of waiting for an option to change.
        if self.ingest.plan.is_none() && !self.ingest.planning && self.ingest.running.is_none() {
            self.ingest_options_changed();
        }
    }

    fn show_ingest_window(&mut self, ctx: &egui::Context, t: &ui::tokens::Theme) {
        if !self.ingest_window {
            return;
        }
        let id = egui::ViewportId::from_hash_of(Self::INGEST_VIEWPORT);
        let builder = egui::ViewportBuilder::default()
            .with_title("Ingest")
            .with_app_id("dev.mccal.apature")
            .with_inner_size([1120.0, 800.0])
            .with_min_inner_size([820.0, 600.0]);
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            let close = ctx.input(|i| i.viewport().close_requested())
                || (ctx.input(|i| i.key_pressed(egui::Key::Escape))
                    && ctx.memory(|m| m.focused().is_none()));
            if close {
                self.ingest_window = false;
            }
            if class == egui::ViewportClass::Embedded {
                // No native child windows on this backend: fall back to a
                // floating panel inside the main window.
                egui::Window::new("Ingest")
                    .default_size([1080.0, 760.0])
                    .show(ctx, |ui| ui::ingest::show(self, ui, t));
            } else {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::NONE
                            .fill(t.photo)
                            .inner_margin(egui::Margin::same(0)),
                    )
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| ui::ingest::show(self, ui, t));
                    });
            }
        });
    }

    pub fn ingest_options_changed(&mut self) {
        if self.ingest.opts.headline.is_none() {
            self.ingest.opts.headline = self.assignment.as_ref().map(|a| a.slug.clone());
        }
        self.prefs.ingest = Some(self.ingest.opts.clone());
        self.prefs_dirty = true;
        self.ingest.plan = None;
        self.ingest.plan_error = None;
        if self.ingest.opts.source.as_os_str().is_empty()
            || self.ingest.opts.primary.as_os_str().is_empty()
        {
            self.ingest.planning = false;
            return;
        }
        self.ingest.generation += 1;
        self.ingest.planning = true;
        self.ingest_jobs
            .plan(self.ingest.opts.clone(), self.ingest.generation);
    }

    /// Apply a finished background Live Ingest poll: take over the
    /// pending counters the worker advanced, and hand any settled batch to
    /// a normal ingest. Results for a source the user has since repointed
    /// (or with Live Ingest switched off) are dropped. The plan was built
    /// on the worker from the tick's options snapshot, as the old
    /// synchronous tick built it; the only difference is that a batch
    /// planned while an ingest is running is discarded here instead of
    /// never being built.
    fn apply_live_poll(&mut self, live: LivePoll) {
        if !self.ingest.live.on || self.ingest.opts.source != live.src {
            return;
        }
        self.ingest.live.pending = live.pending;
        let ready = live.ready;
        if ready.is_empty()
            || self
                .ingest
                .running
                .as_ref()
                .is_some_and(|r| r.done.is_none())
        {
            return;
        }
        let Some(plan) = live.plan else { return };
        match plan {
            Ok(plan) if plan.can_run() && !plan.entries.is_empty() => {
                for p in &ready {
                    self.ingest.live.pending.remove(p);
                    self.ingest.live.handed.insert(p.clone());
                }
                let n = plan.items;
                self.ingest.live.ingested += n;
                self.ingest.live.last_batch = Some(Instant::now());
                log::info!("live ingest: batch of {n} photos");
                self.status = format!(
                    "Live Ingest: copying {n} new photo{}",
                    if n == 1 { "" } else { "s" }
                );
                let pad = if self.ingest.apply_pad {
                    self.assignment.as_ref().map(|a| a.iptc())
                } else {
                    None
                };
                self.ingest.plan = Some(plan.clone());
                self.ingest.running =
                    Some(self.ingest_jobs.start(plan, pad, self.prefs.jpeg_marks()));
            }
            Ok(plan) => {
                // Already in the ledger (or filtered out): forget them.
                for p in &ready {
                    self.ingest.live.pending.remove(p);
                    self.ingest.live.handed.insert(p.clone());
                }
                if !plan.collisions.is_empty() {
                    let msg = format!(
                        "Live Ingest: {} name collision(s); turn on the -1, -2 suffix option",
                        plan.collisions.len()
                    );
                    self.toast(ui::toasts::Kind::Error, msg);
                }
            }
            Err(e) => {
                self.status = format!("Live Ingest: {e}");
            }
        }
    }

    fn ledger_path(&self) -> PathBuf {
        apature_core::project_dirs()
            .map(|d| d.data_dir().join("ingested.json"))
            .unwrap_or_else(|| PathBuf::from("ingested.json"))
    }

    fn drain_ingest(&mut self) {
        while let Some(msg) = self.ingest_jobs.try_recv() {
            match msg {
                Msg::PlanReady { generation, result } => {
                    if generation != self.ingest.generation {
                        continue;
                    }
                    self.ingest.planning = false;
                    match result {
                        Ok(p) => {
                            self.ingest.plan = Some(p);
                            self.ingest.plan_since = Some(Instant::now());
                        }
                        Err(e) => self.ingest.plan_error = Some(e),
                    }
                    if self.launch.ingest_run && self.ingest.running.is_none() {
                        match &self.ingest.plan {
                            Some(p) if p.can_run() => {
                                eprintln!(
                                    "ingest-run: {} files planned, {} collisions, {} skipped",
                                    p.entries.len(),
                                    p.collisions.len(),
                                    p.skipped.len()
                                );
                                self.dispatch_later = Some(Command::IngestStart);
                            }
                            Some(p) => {
                                eprintln!(
                                    "ingest-run: cannot run ({} entries, {} collisions)",
                                    p.entries.len(),
                                    p.collisions.len()
                                );
                                self.finish_headless_run();
                            }
                            None => {
                                eprintln!(
                                    "ingest-run: plan failed: {}",
                                    self.ingest.plan_error.as_deref().unwrap_or("?")
                                );
                                self.finish_headless_run();
                            }
                        }
                    }
                }
                Msg::Event(ev) => {
                    if let Some(r) = self.ingest.running.as_mut() {
                        match ev {
                            apature_ingest::Event::Started { index, .. } => {
                                if let Some(s) = r.statuses.get_mut(index) {
                                    *s = FileStatus::Copying;
                                }
                            }
                            apature_ingest::Event::Verified { index, .. } => {
                                r.verified_count += 1;
                                let verified_now = r.verified_count;
                                self.ingest_sheet_pending = Some(verified_now);
                                r.bytes_done += self
                                    .ingest
                                    .plan
                                    .as_ref()
                                    .and_then(|p| p.entries.get(index))
                                    .map(|e| e.size)
                                    .unwrap_or(0);
                                if let Some(s) = r.statuses.get_mut(index) {
                                    if *s != FileStatus::Failed {
                                        *s = FileStatus::Verified;
                                    }
                                }
                            }
                            apature_ingest::Event::Failed {
                                index,
                                destination,
                                error,
                            } => {
                                log::warn!("ingest failed: {} — {error}", destination.display());
                                if let Some(s) = r.statuses.get_mut(index) {
                                    *s = FileStatus::Failed;
                                }
                                if let Some(e) = r.errors.get_mut(index) {
                                    *e = Some(format!("{}: {error}", destination.display()));
                                }
                            }
                            apature_ingest::Event::Finished => {}
                        }
                    }
                }
                Msg::Report(rep) => {
                    log::info!(
                        "ingest report: {} ok, {} failed, {} bytes, {:.1} s, cancelled={}",
                        rep.files_ok(),
                        rep.files_failed(),
                        rep.bytes_copied,
                        rep.elapsed_secs,
                        rep.cancelled
                    );
                    let ok = self
                        .ingest
                        .plan
                        .as_ref()
                        .map(|p| rep.all_verified(p))
                        .unwrap_or(false);
                    if let Some(e) = &rep.ledger_error {
                        log::warn!("ingest ledger not saved: {e}");
                        self.toast(
                            ui::toasts::Kind::Warn,
                            format!(
                                "Photos copied, but Apature couldn't save its list of imported photos ({e}); \
                                 \"Only new\" may offer them again"
                            ),
                        );
                    }
                    let open_after = ok && self.prefs.open_after_ingest && !self.ingest.apply_pad;
                    if let Some(r) = self.ingest.running.as_mut() {
                        self.status = if ok {
                            format!("Ingest complete: {} files verified", rep.files_ok())
                        } else {
                            format!("Ingest finished with {} failed file(s)", rep.files_failed())
                        };
                        r.done = Some(rep);
                        r.done_since = Some(Instant::now());
                        let msg = self.status.clone();
                        self.toast(
                            if ok {
                                ui::toasts::Kind::Info
                            } else {
                                ui::toasts::Kind::Error
                            },
                            msg,
                        );
                    }
                    if open_after {
                        self.dispatch_later = Some(Command::IngestOpenResult);
                    }
                    if ok {
                        self.ingest_sheet_progress(usize::MAX, true);
                        // Persistent sequence: remember where this headline's
                        // counter stops, so the next ingest continues it.
                        if let Some(next) = self.ingest.plan.as_ref().map(|p| p.next_seq) {
                            let key = crate::prefs::seq_key(self.ingest.opts.headline.as_deref());
                            self.prefs.seq_counters.insert(key, next);
                            self.prefs_dirty = true;
                            if self.prefs.seq_continue {
                                self.ingest.opts.seq_start = next;
                                self.prefs.ingest = Some(self.ingest.opts.clone());
                            }
                        }
                    }
                    if ok && self.prefs.eject_after_ingest {
                        let src = self.ingest.opts.source.clone();
                        if let Some(root) = Platform::eject(&src) {
                            self.status = format!("{} · ejecting {}", self.status, root.display());
                        }
                    }
                    if self.launch.ingest_run {
                        if let Some(r) = self.ingest.running.as_ref() {
                            if let Some(rep) = &r.done {
                                eprintln!(
                                    "ingest-run: {} ok, {} failed, {} bytes, {:.2} s, cancelled={}",
                                    rep.files_ok(),
                                    rep.files_failed(),
                                    rep.bytes_copied,
                                    rep.elapsed_secs,
                                    rep.cancelled
                                );
                                for (i, e) in r.errors.iter().enumerate() {
                                    if let Some(e) = e {
                                        eprintln!("ingest-run: file {i} failed: {e}");
                                    }
                                }
                            }
                        }
                        if !self.ingest.apply_pad {
                            self.finish_headless_run();
                        }
                    }
                }
                Msg::PadApplied { files, failed } => {
                    if let Some(r) = self.ingest.running.as_mut() {
                        r.pad = Some((files, failed));
                    }
                    if self.launch.ingest_run {
                        eprintln!("ingest-run: pad applied to {files} files, {failed} failed");
                        self.finish_headless_run();
                    }
                    if self.prefs.open_after_ingest {
                        self.dispatch_later = Some(Command::IngestOpenResult);
                    }
                }
            }
        }
        if let Some(n) = self.ingest_sheet_pending.take() {
            self.ingest_sheet_progress(n, false);
        }
    }

    /// `--ingest-run` is over: screenshot the finished workspace a moment
    /// later when asked for, otherwise quit now.
    fn finish_headless_run(&mut self) {
        self.run_done = true;
        if self.launch.screenshot.is_some() {
            self.frames = self.launch.screenshot_frame.saturating_sub(30);
            self.screenshot_requested = false;
        } else {
            self.quit_after_frame = true;
        }
    }

    /// Paste the internal metadata clipboard onto the selection: marks go
    /// through the undo stack; IPTC is written in place (not undoable, the
    /// status says how many files).
    fn paste_metadata(&mut self) {
        let Some((m, iptc)) = self.meta_clip.clone() else {
            self.status = "Nothing copied yet (⌥⌘C copies the focused photo's metadata)".into();
            return;
        };
        let targets = self.selection.targets(self.cursor);
        self.mark("paste marks", move |x| *x = m);
        // The IPTC half goes through the mark writer with the marks, in
        // queue order, so the window never blocks on a slow volume even
        // across hundreds of photos.
        let policy = self.prefs.jpeg_marks();
        let items: Vec<PhotoItem> = targets.iter().map(|&i| self.items[i].clone()).collect();
        let seqs: Vec<u64> = items
            .into_iter()
            .map(|item| self.mark_writer.submit_iptc(item, iptc.clone(), policy))
            .collect();
        let total = targets.len();
        self.bulk_write = Some(BulkWrite::new(seqs, total, BulkKind::Paste));
        self.mark_status = None;
        self.inspector_iptc = None;
        self.status = format!(
            "Pasting metadata onto {total} photo{}…",
            if total == 1 { "" } else { "s" }
        );
    }

    fn ingest_start(&mut self) {
        let Some(plan) = self.ingest.plan.clone() else {
            return;
        };
        if !plan.can_run() {
            return;
        }
        let mut pad = if self.ingest.apply_pad {
            self.assignment.as_ref().map(|a| a.iptc())
        } else {
            None
        };
        // Set GPS on ingest: one fixed location for the whole card (a stadium,
        // a venue), written into every copy's sidecar with the pad.
        if self.ingest.gps_on && !self.ingest.gps.trim().is_empty() {
            if let Some((la, lo)) = parse_latlon(&self.ingest.gps) {
                let mut p = pad.unwrap_or_default();
                p.set_gps(la, lo);
                pad = Some(p);
            }
        }
        self.ingest.running = Some(self.ingest_jobs.start(plan, pad, self.prefs.jpeg_marks()));
        self.status = "Ingesting…".into();
    }

    /// Drag-and-drop: a folder opens as a contact sheet; a file opens its
    /// folder with that photo focused, like dropping onto Photo Mechanic.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let (hover, dropped) =
            ctx.input(|i| (i.raw.hovered_files.len(), i.raw.dropped_files.clone()));
        self.drag_hover = hover;
        let Some(first) = dropped.into_iter().find_map(|f| f.path) else {
            return;
        };
        if first.is_dir() {
            self.open_folder(&first);
        } else if let Some(parent) = first.parent() {
            self.open_folder(parent);
            let stem = first
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase());
            if let Some(idx) = self
                .items
                .iter()
                .position(|it| Some(it.stem.to_lowercase()) == stem)
            {
                self.cursor = idx;
                self.selection.only(idx);
                self.scroll_to_cursor = true;
            }
        }
        self.workspace = Workspace::Sheet;
    }

    // ----- captions ----------------------------------------------------------

    fn load_assignment(&mut self, path: &Path) {
        match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| Assignment::parse(&t).map_err(|e| e.to_string()))
        {
            Ok(a) => {
                self.codes.merge(&a.codes());
                self.status = format!(
                    "Assignment loaded: {} ({} codes)",
                    a.slug,
                    self.codes.codes.len()
                );
                self.assignment = Some(a);
                self.prefs.assignment_file = Some(path.to_path_buf());
                self.prefs_dirty = true;
            }
            Err(e) => self.status = format!("Could not load assignment {}: {e}", path.display()),
        }
    }

    fn load_codes(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                let table = CodeTable::parse(&t);
                self.codes.merge(&table);
                self.status = format!(
                    "{} code replacements loaded from {}",
                    table.codes.len(),
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                );
                self.prefs.codes_file = Some(path.to_path_buf());
                self.prefs_dirty = true;
            }
            Err(e) => self.status = format!("Could not read {}: {e}", path.display()),
        }
    }

    fn caption_vars(&self, draft: &Draft) -> Vars {
        Vars {
            date: draft
                .date_created
                .as_deref()
                .and_then(codes::parse_date)
                .or_else(|| {
                    self.exif[draft.idx]
                        .as_ref()
                        .and_then(|e| e.date_time_original.as_deref())
                        .and_then(codes::parse_date)
                }),
            credit: if draft.fields.credit.is_empty() {
                self.assignment.as_ref().and_then(|a| a.credit_line())
            } else {
                Some(draft.fields.credit.clone())
            },
            headline: if draft.fields.headline.is_empty() {
                None
            } else {
                Some(draft.fields.headline.clone())
            },
        }
    }

    /// Keep the draft on the focused photo; save a dirty draft before moving on.
    fn sync_caption_draft(&mut self) {
        if self.items.is_empty() {
            self.caption_draft = None;
            return;
        }
        if self
            .caption_draft
            .as_ref()
            .is_some_and(|d| d.idx == self.cursor)
        {
            return;
        }
        self.save_caption(false);
        let it = &self.items[self.cursor];
        let iptc = items::read_iptc(it);
        let fields = DraftFields::from_iptc(&iptc);
        self.caption_draft = Some(Draft {
            idx: self.cursor,
            fields: fields.clone(),
            original: fields,
            date_created: iptc.date_created.clone(),
        });
    }

    /// Write the draft's changed fields. Returns true when something was written.
    fn save_caption(&mut self, announce: bool) -> bool {
        let Some(draft) = self.caption_draft.as_ref() else {
            return false;
        };
        if !draft.dirty() || draft.idx >= self.items.len() {
            return false;
        }
        let changes = draft.fields.changes_from(&draft.original);
        let idx = draft.idx;
        let original = draft.original.clone();
        // Queue the write instead of blocking on it: the file lands on the
        // writer thread, in order behind any pending marks. If it fails,
        // the draft goes dirty again (see poll_mark_writes).
        let seq = self.mark_writer.submit_iptc(
            self.items[idx].clone(),
            changes.clone(),
            self.prefs.jpeg_marks(),
        );
        self.caption_write = Some(CaptionWrite {
            seq,
            idx,
            original,
            announce,
        });
        self.remember_caption_history(&changes);
        if let Some(d) = self.caption_draft.as_mut() {
            d.original = d.fields.clone();
        }
        self.inspector_iptc = None;
        if announce {
            self.status = format!("Saving caption: {}…", self.items[idx].display_name());
        }
        true
    }

    /// Feed the ▾ recent-values menus from what was just written.
    fn remember_caption_history(&mut self, c: &apature_metadata::iptc::Iptc) {
        let pairs: [(&str, &Option<String>); 11] = [
            ("headline", &c.headline),
            ("sublocation", &c.sublocation),
            ("city", &c.city),
            ("state", &c.state),
            ("country", &c.country),
            ("credit", &c.credit),
            ("copyright", &c.copyright),
            ("source", &c.source),
            ("title", &c.title),
            ("caption_writer", &c.caption_writer),
            ("instructions", &c.instructions),
        ];
        let mut any = false;
        for (k, v) in pairs {
            if let Some(v) = v {
                self.prefs.remember_caption_value(k, v);
                any = true;
            }
        }
        if let Some(list) = &c.creator {
            self.prefs
                .remember_caption_value("creator", &list.join("; "));
            any = true;
        }
        if let Some(list) = &c.keywords {
            self.prefs
                .remember_caption_value("keywords", &list.join("; "));
            any = true;
        }
        if any {
            self.prefs_dirty = true;
        }
    }

    /// Photo Mechanic's "apply to all selected": only the fields that changed
    /// in the draft go to every selected photo; untouched fields stay as each
    /// photo had them.
    fn save_caption_to_selection(&mut self) {
        let Some(draft) = self.caption_draft.as_ref() else {
            return;
        };
        if !draft.dirty() {
            return;
        }
        let changes = draft.fields.changes_from(&draft.original);
        let targets = self.selection.targets(self.cursor);
        let policy = self.prefs.jpeg_marks();
        let items: Vec<PhotoItem> = targets.iter().map(|&i| self.items[i].clone()).collect();
        let seqs: Vec<u64> = items
            .into_iter()
            .map(|item| self.mark_writer.submit_iptc(item, changes.clone(), policy))
            .collect();
        let total = targets.len();
        self.bulk_write = Some(BulkWrite::new(seqs, total, BulkKind::Caption));
        self.mark_status = None;
        self.remember_caption_history(&changes);
        if let Some(d) = self.caption_draft.as_mut() {
            d.original = d.fields.clone();
        }
        self.inspector_iptc = None;
        self.status = format!(
            "Saving caption to {total} photo{}…",
            if total == 1 { "" } else { "s" }
        );
    }

    /// Send the last export's files to the named target on a worker thread.
    fn start_delivery(&mut self, name: &str) {
        if self
            .deliver
            .running
            .as_ref()
            .is_some_and(|r| r.done.is_none())
        {
            self.status = "A delivery is already running".into();
            return;
        }
        let Some(t) = self.prefs.delivery.iter().find(|t| t.name == name).cloned() else {
            self.status = format!("No delivery target named {name}");
            return;
        };
        if self.deliver.last_outputs.is_empty() {
            self.status = "Nothing to send: export first".into();
            return;
        }
        let secret = if t.kind == apature_deliver::Kind::Folder {
            String::new()
        } else {
            match Platform::secret_get(&t.name) {
                Some(s) => s,
                None => {
                    self.status = format!("No password saved for {}: Settings › Delivery", t.name);
                    return;
                }
            }
        };
        let files = self.deliver.last_outputs.clone();
        log::info!(
            "delivery start: {} files → {} ({})",
            files.len(),
            t.name,
            t.host
        );
        self.status = format!("Sending {} files to {}…", files.len(), t.name);
        self.deliver.running = Some(self.deliver_jobs.start(t, secret, files));
    }

    fn drain_deliver(&mut self) {
        while let Some(msg) = self.deliver_jobs.try_recv() {
            let Some(r) = self.deliver.running.as_mut() else {
                continue;
            };
            match msg {
                DMsg::Event(apature_deliver::Event::Started { index }) => {
                    if let Some(s) = r.statuses.get_mut(index) {
                        *s = DStatus::Sending;
                    }
                }
                DMsg::Event(apature_deliver::Event::Sent { index, bytes }) => {
                    r.bytes += bytes;
                    if let Some(s) = r.statuses.get_mut(index) {
                        *s = DStatus::Sent;
                    }
                    if let Some(out) = r.files.get(index).cloned() {
                        let src = self
                            .deliver
                            .source_of
                            .get(&out)
                            .cloned()
                            .unwrap_or_else(|| out.clone());
                        self.deliver.sent.insert(
                            src,
                            SentRecord {
                                target: r.target.clone(),
                                at: local_stamp(),
                                output: out,
                            },
                        );
                        save_delivered(&self.deliver.sent);
                    }
                }
                DMsg::Event(apature_deliver::Event::Failed { index, error }) => {
                    if let Some(s) = r.statuses.get_mut(index) {
                        *s = DStatus::Failed;
                    }
                    if let Some(e) = r.errors.get_mut(index) {
                        *e = Some(error);
                    }
                }
                DMsg::Event(apature_deliver::Event::Finished) => {}
                DMsg::Done(result) => {
                    self.status = match &result {
                        Ok(rep) if rep.failed.is_empty() => format!(
                            "Sent {} files to {} · {:.0} MB in {:.0} s",
                            rep.sent,
                            r.target,
                            rep.bytes as f64 / 1e6,
                            rep.elapsed_secs
                        ),
                        Ok(rep) => format!(
                            "Sent {} of {} to {}; {} failed",
                            rep.sent,
                            r.files.len(),
                            r.target,
                            rep.failed.len()
                        ),
                        Err(e) => format!("Delivery to {} failed: {e}", r.target),
                    };
                    let kind = if matches!(&result, Ok(rep) if rep.failed.is_empty()) {
                        ui::toasts::Kind::Info
                    } else {
                        ui::toasts::Kind::Error
                    };
                    r.done = Some(result);
                    let msg = self.status.clone();
                    self.toast(kind, msg);
                }
            }
        }
    }

    /// The orientation the photo is shown at: the sidecar override, else the file's.
    pub fn effective_orientation(&self, idx: usize) -> u32 {
        let file_o = self.exif[idx]
            .as_ref()
            .map(|e| e.orientation)
            .unwrap_or(1)
            .max(1);
        edits::read(&self.items[idx].sidecar_path())
            .map(|e| e.orientation_or(file_o))
            .unwrap_or(file_o)
    }

    /// Store (or remove) the crop in the sidecar; previews re-render.
    fn write_crop(&mut self, idx: usize, crop: Option<edits::Crop>) {
        let change = edits::Change {
            crop: Some(crop),
            ..Default::default()
        };
        // Queue the write: the crop lands on the writer thread, in order
        // behind any pending marks. A failure surfaces in poll_mark_writes.
        self.mark_writer
            .submit_edits(self.items[idx].clone(), change);
        self.inspector_iptc = None;
        self.status = match crop {
            Some(c) => format!(
                "Crop saved: {:.0}% × {:.0}% of the frame · applied on export",
                (c.right - c.left) * 100.0,
                (c.bottom - c.top) * 100.0
            ),
            None => "Crop removed".into(),
        };
        log::info!("crop {} → {:?}", self.items[idx].display_name(), crop);
    }

    fn apply_stationery(&mut self) {
        let Some(pad) = self.assignment.as_ref().map(|a| a.iptc()) else {
            return;
        };
        let targets = self.selection.targets(self.cursor);
        let policy = self.prefs.jpeg_marks();
        let items: Vec<PhotoItem> = targets.iter().map(|&i| self.items[i].clone()).collect();
        let seqs: Vec<u64> = items
            .into_iter()
            .map(|item| self.mark_writer.submit_iptc(item, pad.clone(), policy))
            .collect();
        let total = targets.len();
        self.bulk_write = Some(BulkWrite::new(seqs, total, BulkKind::Stationery));
        self.mark_status = None;
        self.status = format!(
            "Applying pad to {total} photo{}…",
            if total == 1 { "" } else { "s" }
        );
        self.caption_draft = None;
        self.inspector_iptc = None;
        self.sync_caption_draft();
    }

    fn insert_caption_template(&mut self) {
        let Some(tpl) = self.assignment.as_ref().map(|a| a.caption_template()) else {
            return;
        };
        let Some(draft) = self.caption_draft.as_ref() else {
            return;
        };
        let vars = self.caption_vars(draft);
        let text = codes::expand_variables(&tpl, &vars);
        if let Some(d) = self.caption_draft.as_mut() {
            d.fields.caption = text;
        }
    }

    fn sync_menu(&self) {
        self.platform.sync_toggles(
            self.sidebar_shown(),
            self.inspector_shown(),
            self.prefs.filmstrip_visible,
            self.prefs.advance_after_mark,
        );
    }

    // ----- folders -----------------------------------------------------------

    fn refresh_volumes(&mut self) {
        let (v, devs) = scan_volumes(&mut self.volume_kinds);
        self.volumes = v;
        self.volume_devs = devs;
    }

    /// Rescan the open folder and keep the cursor and selection on the same
    /// files (by path), so new arrivals from an ingest slot in without
    /// disturbing a cull in progress.
    pub fn refresh_folder(&mut self) {
        let Some(dir) = self.folder.clone() else {
            return;
        };
        self.save_caption(false);
        // The caption save is queued now: wait for it before the rescan
        // reads the sidecars, or the rescan would show the old caption.
        self.flush_marks();
        let cursor_path = self.items.get(self.cursor).map(|i| i.path().to_path_buf());
        let selected: Vec<PathBuf> = self
            .selection
            .targets(self.cursor)
            .iter()
            .filter_map(|&i| self.items.get(i).map(|it| it.path().to_path_buf()))
            .collect();
        let was_multi = self.selection.len() > 1;
        self.load_folder(&dir);
        let find = |p: &PathBuf| self.items.iter().position(|it| it.path() == p.as_path());
        if let Some(i) = cursor_path.as_ref().and_then(find) {
            self.cursor = i;
        }
        if was_multi {
            let idx: Vec<usize> = selected.iter().filter_map(find).collect();
            self.selection.all(&idx);
        } else if let Some(i) = cursor_path.as_ref().and_then(find) {
            self.selection.only(i);
        }
        self.scroll_to_cursor = true;
    }

    /// The folder the running (or last planned) ingest writes into.
    fn ingest_dest_dir(&self) -> Option<PathBuf> {
        self.ingest
            .plan
            .as_ref()
            .and_then(|p| p.entries.first())
            .and_then(|e| e.primary.parent().map(|d| d.to_path_buf()))
    }

    /// Sheet while ingesting: after the first verified file the destination
    /// opens as a tab (culling can start); it is refreshed every 25 files and
    /// at the end, cursor and selection kept.
    fn ingest_sheet_progress(&mut self, verified: usize, finished: bool) {
        if !self.prefs.open_after_ingest {
            return;
        }
        let Some(dest) = self.ingest_dest_dir() else {
            return;
        };
        let open = self.tabs.iter().position(|t| t.folder == dest);
        if verified == 1 && open.is_none() {
            // Open behind the Ingest window; the main window stays wherever it was.
            self.open_folder(&dest);
            self.status = "Ingesting · the sheet fills in as files verify (⌘R refreshes)".into();
            return;
        }
        if finished || verified.is_multiple_of(25) {
            if self.folder.as_deref() == Some(dest.as_path()) {
                self.refresh_folder();
            } else if let Some(i) = open {
                // Parked tab: drop its state so it rescans when shown.
                self.tabs[i].state = None;
            }
        }
    }

    /// The tab showing `dir`, if any: the same path, or another path to the
    /// same folder (a symlink such as /Volumes/Macintosh HD/…, different
    /// letter case on a case-insensitive disk).
    fn tab_for(&self, dir: &Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.folder == dir)
            .or_else(|| self.tabs.iter().position(|t| same_folder(&t.folder, dir)))
    }

    /// Open `dir` in a new tab, or switch to it if it is already open.
    pub fn open_folder(&mut self, dir: &Path) {
        self.flush_marks();
        if let Some(i) = self.tab_for(dir) {
            self.switch_tab(i);
            return;
        }
        if !self.tabs.is_empty() {
            let parked = self.take_tab();
            let count = parked.items.len();
            let cur = self.active;
            self.tabs[cur].state = Some(parked);
            self.tabs[cur].count = count;
        }
        self.tabs.push(Tab {
            folder: dir.to_path_buf(),
            state: None,
            count: 0,
        });
        self.active = self.tabs.len() - 1;
        self.load_folder(dir);
        self.remember_tabs();
    }

    /// Show `dir` in the current tab (the folder tree's single click),
    /// replacing what the tab showed. A folder already open in another tab
    /// switches to that tab instead; with no tabs it opens one.
    pub fn open_folder_here(&mut self, dir: &Path) {
        if let Some(i) = self.tab_for(dir) {
            self.switch_tab(i);
            return;
        }
        if self.tabs.is_empty() {
            self.open_folder(dir);
            return;
        }
        // Check first: a failed load would close the tab instead.
        if let Err(e) = std::fs::read_dir(dir) {
            self.status = format!("Could not open {}: {e}", dir.display());
            self.status_since = Some(Instant::now());
            return;
        }
        self.save_caption(false);
        let cur = self.active;
        self.tabs[cur].folder = dir.to_path_buf();
        self.tabs[cur].state = None;
        self.load_folder(dir);
        self.sheet_fade_since = Some(Instant::now());
        self.remember_tabs();
    }

    fn remember_tabs(&mut self) {
        self.prefs.open_tabs = self.tabs.iter().map(|t| t.folder.clone()).collect();
        self.prefs.active_tab = self.active;
        self.prefs_dirty = true;
    }

    /// Move the active tab's state out of `App`, leaving empty fields behind.
    fn take_tab(&mut self) -> TabState {
        self.next_generation += 1;
        TabState {
            folder: self.folder.take(),
            items: std::mem::take(&mut self.items),
            marks: std::mem::take(&mut self.marks),
            conflicts: std::mem::take(&mut self.conflicts),
            exif: std::mem::take(&mut self.exif),
            thumbs: std::mem::take(&mut self.thumbs),
            thumb_err: std::mem::take(&mut self.thumb_err),
            native: std::mem::take(&mut self.native),
            previews: std::mem::take(&mut self.previews),
            preview_lru: std::mem::take(&mut self.preview_lru),
            generation: std::mem::replace(&mut self.generation, self.next_generation),
            cursor: std::mem::take(&mut self.cursor),
            selection: std::mem::take(&mut self.selection),
            view: std::mem::take(&mut self.view),
            stacks: std::mem::take(&mut self.stacks),
            stacks_open: std::mem::take(&mut self.stacks_open),
            pos: std::mem::take(&mut self.pos),
            filter: std::mem::take(&mut self.filter),
            undo: std::mem::take(&mut self.undo),
            cols: std::mem::replace(&mut self.cols, 1),
            last_visible_rows: std::mem::replace(&mut self.last_visible_rows, 0..0),
            stats: std::mem::take(&mut self.stats),
            caption_draft: self.caption_draft.take(),
            inspector_iptc: self.inspector_iptc.take(),
        }
    }

    fn put_tab(&mut self, t: TabState) {
        self.folder = t.folder;
        self.items = t.items;
        self.marks = t.marks;
        self.conflicts = t.conflicts;
        self.exif = t.exif;
        self.thumbs = t.thumbs;
        self.thumb_err = t.thumb_err;
        self.native = t.native;
        self.previews = t.previews;
        self.preview_lru = t.preview_lru;
        self.generation = t.generation;
        self.cursor = t.cursor;
        self.selection = t.selection;
        self.view = t.view;
        self.stacks = t.stacks;
        self.stacks_open = t.stacks_open;
        self.pos = t.pos;
        self.filter = t.filter;
        self.undo = t.undo;
        self.cols = t.cols;
        self.last_visible_rows = t.last_visible_rows;
        self.stats = t.stats;
        self.caption_draft = t.caption_draft;
        self.inspector_iptc = t.inspector_iptc;
        self.scroll_to_cursor = true;
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active {
            return;
        }
        self.save_caption(false);
        // The caption save is queued now: wait for it before the tab
        // changes, so the parked tab's photo is on disk as saved.
        self.flush_marks();
        let parked = self.take_tab();
        let cur = self.active;
        self.tabs[cur].count = parked.items.len();
        self.tabs[cur].state = Some(parked);
        if let Some(next) = self.tabs[i].state.take() {
            self.put_tab(next);
        }
        self.active = i;
        self.pool.clear_pending();
        self.sheet_fade_since = Some(Instant::now());
        if matches!(self.workspace, Workspace::Preview | Workspace::Caption)
            && self.items.is_empty()
        {
            self.workspace = Workspace::Sheet;
        }
        self.remember_tabs();
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if i == self.active {
            self.save_caption(false);
            let _ = self.take_tab();
            self.pool.clear_pending();
            self.tabs.remove(i);
            if self.tabs.is_empty() {
                self.active = 0;
                self.workspace = Workspace::Sheet;
            } else {
                let next = i.min(self.tabs.len() - 1);
                if let Some(st) = self.tabs[next].state.take() {
                    self.put_tab(st);
                }
                self.active = next;
            }
        } else {
            self.tabs.remove(i);
            if i < self.active {
                self.active -= 1;
            }
        }
        self.remember_tabs();
    }

    /// Load `dir` into the active (empty or replaced) tab fields.
    fn load_folder(&mut self, dir: &Path) {
        log::info!("open folder {}", dir.display());
        self.folder_unplugged = None;
        let t0 = Instant::now();
        match apature_core::scan(dir, &ScanOptions::default()) {
            Ok(items) => {
                self.pool.clear_pending();
                self.next_generation += 1;
                self.generation = self.next_generation;
                let scheme = self.scheme.clone();
                let jpeg_policy = self.prefs.jpeg_marks();
                // Marks, conflicts and EXIF for every item up front: all three are
                // cheap (sidecars are tiny, EXIF is a few page faults via mmap) and
                // sorting by capture time needs them before the first frame.
                let read: Vec<(Marks, Vec<String>, Option<ExifSummary>)> = items
                    .par_iter()
                    .map(|it| {
                        let src = items::read(it);
                        let m = src.effective(it, &scheme);
                        let c = src
                            .conflicts(jpeg_policy)
                            .into_iter()
                            .map(|(p, c)| {
                                format!(
                                    "{}: sidecar {} vs {} {}",
                                    c.field,
                                    c.sidecar,
                                    p.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                    c.embedded
                                )
                            })
                            .collect();
                        let e = apature_metadata::exif::read(it.path()).ok();
                        (m, c, e)
                    })
                    .collect();
                let n = items.len();
                self.items = items;
                self.marks = read.iter().map(|(m, _, _)| *m).collect();
                self.conflicts = read.iter().map(|(_, c, _)| c.clone()).collect();
                self.exif = read.into_iter().map(|(_, _, e)| e).collect();
                self.thumbs = vec![None; n];
                self.thumb_err = vec![None; n];
                self.native = vec![None; n];
                self.previews.clear();
                self.preview_lru.clear();
                self.selection.clear();
                self.undo.clear();
                self.filter = Filter::default();
                self.rebuild_view();
                self.cursor = self.view.first().copied().unwrap_or(0);
                self.workspace = Workspace::Sheet;
                self.folder = Some(dir.to_path_buf());
                self.stats = Stats {
                    scan_ms: t0.elapsed().as_secs_f32() * 1000.0,
                    ..Default::default()
                };
                self.status = String::new();
                self.prefs.note_recent(dir);
                self.prefs_dirty = true;
                self.refresh_volumes();
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.count = n;
                }
            }
            Err(e) => {
                self.status = format!("Could not open {}: {e}", dir.display());
                if self.tabs.get(self.active).is_some_and(|t| t.folder == dir) {
                    self.tabs.remove(self.active);
                    self.active = self.active.min(self.tabs.len().saturating_sub(1));
                    // Back to the tab that was showing: bring its parked
                    // state back, or the next folder opened would park this
                    // empty one over it (a blank tab that can't be reopened).
                    if let Some(st) = self.tabs.get_mut(self.active).and_then(|t| t.state.take()) {
                        self.put_tab(st);
                    }
                }
            }
        }
    }

    /// The launch-time screenshot target, when one was requested.
    pub fn screenshot_path(&self) -> Option<PathBuf> {
        self.launch.screenshot.clone()
    }

    /// Animation duration honouring Reduce Motion (0 = instant).
    pub fn anim(&self, secs: f32) -> f32 {
        if self.reduce_motion() {
            0.0
        } else {
            secs
        }
    }

    /// Enter the Preview workspace with a grow-in from the focused cell.
    fn enter_preview(&mut self) {
        self.preview_from = self.cursor_cell_rect;
        self.preview_since = Some(Instant::now());
        if self.prefs.preview_in_window && self.launch.screenshot.is_none() {
            // Its own window; the sheet stays where it is.
            self.preview_window = true;
            return;
        }
        self.workspace = Workspace::Preview;
    }

    /// Preview in a separate window: keys are read from that window's own
    /// input, with the workspace set to Preview for the duration so every
    /// preview shortcut (Z, C, L, K, B, arrows) behaves as it does inline.
    fn show_preview_window(&mut self, ctx: &egui::Context, t: &ui::tokens::Theme) {
        if !self.preview_window {
            return;
        }
        let id = egui::ViewportId::from_hash_of("apature-preview");
        let builder = egui::ViewportBuilder::default()
            .with_title("Preview")
            .with_app_id("dev.mccal.apature")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([640.0, 480.0]);
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            let saved = self.workspace;
            self.workspace = Workspace::Preview;
            let cmds = self.keyboard_commands(ctx);
            let now = std::time::Instant::now();
            // The main window's raw_input_hook never sees this window's keys,
            // so bare Tab is taken here; it toggles the Preview sidebar.
            if self.tab_is_free(ctx)
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
            {
                // egui has already read this Tab as "focus the next widget";
                // cancel that, or a checkbox takes focus and eats the arrows.
                ctx.memory_mut(|m| {
                    m.move_focus(egui::FocusDirection::None);
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.toggle_side_panels();
            }
            for c in cmds {
                if self.key_once.admit(&c, crate::keyonce::Source::Keys, now) {
                    self.dispatch(ctx, c);
                }
            }
            let exited = self.workspace != Workspace::Preview
                || ctx.input(|i| i.viewport().close_requested());
            if !exited {
                if class == egui::ViewportClass::Embedded {
                    egui::Window::new("Preview")
                        .default_size([1200.0, 800.0])
                        .show(ctx, |ui| ui::preview::show(self, ui, t));
                } else {
                    // The inspector (Info, Zoom, Marks…) beside the photo, as in
                    // Photo Mechanic's Preview window.
                    ui::sidebar::show(self, ctx, t);
                    ui::inspector::show(self, ctx, t);
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(t.photo))
                        .show(ctx, |ui| ui::preview::show(self, ui, t));
                }
            }
            // The sheet in the main window stays on whatever it was showing.
            self.workspace = if saved == Workspace::Preview {
                Workspace::Sheet
            } else {
                saved
            };
            if exited {
                self.preview_window = false;
                self.scroll_to_cursor = true;
            }
        });
    }

    /// Bare Tab is ours (the sidebar) unless a text field has focus or a
    /// dialog is open; then it is left to move between fields.
    fn tab_is_free(&self, ctx: &egui::Context) -> bool {
        let text_focus = ctx
            .memory(|m| m.focused())
            .is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some());
        let dialog = self.bug_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.trash_dialog.is_some()
            || self.copy_dialog.is_some()
            || self.extract_dialog.is_some()
            || self.settings_open
            || self.about_open
            || self.filter_popup_open;
        !text_focus && !dialog
    }

    /// Whether the right inspector is shown: in Preview the session's own
    /// choice once toggled there; elsewhere the saved preference.
    pub fn inspector_shown(&self) -> bool {
        if self.workspace == Workspace::Preview {
            self.preview_inspector.unwrap_or(self.prefs.inspector_visible)
        } else {
            self.prefs.inspector_visible
        }
    }

    /// Tab, as in Photo Mechanic: hide both side panels if either is
    /// showing, else show both. Preview keeps it for the session; the
    /// catalog saves it.
    fn toggle_side_panels(&mut self) {
        let show = !(self.sidebar_shown() || self.inspector_shown());
        if self.workspace == Workspace::Preview {
            self.preview_sidebar = show;
            self.preview_inspector = Some(show);
        } else {
            self.prefs.sidebar_visible = show;
            self.prefs.inspector_visible = show;
            self.prefs_dirty = true;
        }
    }

    /// Whether the left sidebar is shown: hidden in Preview unless toggled
    /// on there; elsewhere the saved preference.
    pub fn sidebar_shown(&self) -> bool {
        if self.workspace == Workspace::Preview {
            self.preview_sidebar
        } else {
            self.prefs.sidebar_visible
        }
    }

    /// Zoom on or off at once, without the ease (Zoom section and click to
    /// zoom, as in Photo Mechanic). On keeps the last pan and level unless
    /// `at` (an image point 0..1) says where to centre.
    pub fn set_zoom(&mut self, on: bool, at: Option<egui::Vec2>) {
        if on {
            if let Some(a) = at {
                self.pv.center = egui::Vec2::new(a.x.clamp(0.0, 1.0), a.y.clamp(0.0, 1.0));
            }
            self.pv.mode = ZoomMode::OneToOne;
        } else {
            self.pv.mode = ZoomMode::Fit;
        }
        self.pv.zoom_since = None;
        self.pv.zoom_from = None;
    }

    /// Toggle 1:1 and fit. `at` is the image point (0..1) to zoom into (the
    /// pointer on a double-click, else the current centre); `from` is where
    /// the photo is drawn now, so the zoom eases from there.
    pub fn toggle_zoom(&mut self, at: Option<egui::Vec2>, from: Option<egui::Rect>) {
        if self.workspace != Workspace::Preview {
            if self.items.is_empty() {
                return;
            }
            self.enter_preview();
        }
        self.pv.mode = match self.pv.mode {
            ZoomMode::Fit => {
                if let Some(a) = at {
                    self.pv.center = egui::Vec2::new(a.x.clamp(0.0, 1.0), a.y.clamp(0.0, 1.0));
                }
                ZoomMode::OneToOne
            }
            ZoomMode::OneToOne => ZoomMode::Fit,
        };
        self.pv.zoom_from = from.or(self.pv.last_rect);
        self.pv.zoom_since = Some(Instant::now());
    }

    /// Compare the selection (two to four photos) side by side; a single
    /// photo is compared with the next one in the view. Toggles off.
    fn toggle_compare(&mut self) {
        if self.pv.compare.is_some() {
            self.pv.compare = None;
            self.status = String::new();
            return;
        }
        if self.items.is_empty() {
            return;
        }
        let mut set: Vec<usize> = self.selection.targets(self.cursor);
        set.sort_by_key(|&i| self.pos[i].unwrap_or(usize::MAX));
        set.truncate(4);
        if set.len() < 2 {
            if let Some(n) = self.view_neighbor(1).or_else(|| self.view_neighbor(-1)) {
                if !set.contains(&n) {
                    set.push(n);
                }
            }
        }
        if set.len() < 2 {
            self.status = "Select two to four photos to compare".into();
            return;
        }
        if !set.contains(&self.cursor) {
            self.cursor = set[0];
        }
        if self.workspace != Workspace::Preview {
            self.enter_preview();
        }
        let key = if self.prefs.shortcut_layout == ShortcutLayout::PhotoMechanic {
            "V"
        } else {
            "C"
        };
        self.status = format!("Comparing {} photos · {key} to stop", set.len());
        self.pv.compare = Some(set);
    }

    pub fn cache_dir(&self) -> PathBuf {
        apature_core::project_dirs()
            .map(|d| d.cache_dir().join("thumbs"))
            .unwrap_or_else(|| PathBuf::from("thumbs"))
    }

    /// The colour-class scheme changed in Settings: push it everywhere.
    pub fn scheme_changed(&mut self) {
        self.scheme = self.prefs.scheme.clone();
        self.prefs_dirty = true;
        self.platform.set_class_names(&self.scheme.names);
    }

    fn pick_folder(&mut self) {
        let mut dlg = rfd::FileDialog::new();
        if let Some(f) = &self.folder {
            dlg = dlg.set_directory(f);
        }
        if let Some(dir) = dlg.pick_folder() {
            self.open_folder(&dir);
        }
    }

    // ----- view (filter + sort) ----------------------------------------------

    pub fn rebuild_view(&mut self) {
        self.view = filter::build_view(
            &self.items,
            &self.marks,
            &self.exif,
            &self.filter,
            self.prefs.sort,
            self.prefs.sort_reversed,
        );
        self.stacks.clear();
        if self.prefs.stack_bursts {
            // Walk the view in capture order; a frame within a second of the
            // previous one joins its stack. Sorting by anything else still
            // stacks by capture time, so the groups never depend on the sort.
            let secs = |i: usize| -> Option<i64> {
                let d = self.exif[i].as_ref()?.date_time_original.as_deref()?;
                let (y, mo, da) = apature_metadata::codes::parse_date(d)?;
                let h: i64 = d.get(11..13)?.parse().ok()?;
                let m: i64 = d.get(14..16)?.parse().ok()?;
                let s: i64 = d.get(17..19)?.parse().ok()?;
                let days = i64::from(y) * 366 + i64::from(mo) * 31 + i64::from(da);
                Some(days * 86_400 + h * 3600 + m * 60 + s)
            };
            let mut by_time: Vec<(i64, usize)> = self
                .view
                .iter()
                .filter_map(|&i| secs(i).map(|t| (t, i)))
                .collect();
            by_time.sort();
            let mut head: Option<usize> = None;
            let mut last: Option<i64> = None;
            for (t, i) in by_time {
                if last.is_some_and(|l| t - l <= 1) {
                    if let Some(h) = head {
                        self.stacks.entry(h).or_default().push(i);
                    }
                } else {
                    head = Some(i);
                }
                last = Some(t);
            }
            // Hide members of collapsed stacks from the view.
            let hidden: HashSet<usize> = self
                .stacks
                .iter()
                .filter(|(h, _)| !self.stacks_open.contains(*h))
                .flat_map(|(_, m)| m.iter().copied())
                .collect();
            if !hidden.is_empty() {
                self.view.retain(|i| !hidden.contains(i));
            }
        }
        self.pos = vec![None; self.items.len()];
        for (p, &i) in self.view.iter().enumerate() {
            self.pos[i] = Some(p);
        }
        let visible: Vec<bool> = self.pos.iter().map(|p| p.is_some()).collect();
        self.selection.retain_visible(&visible);
        if self.pos.get(self.cursor).copied().flatten().is_none() {
            // Focused item got filtered out: move focus to the nearest visible one.
            if let Some(&first) = self.view.first() {
                self.cursor = first;
            }
        }
        self.scroll_to_cursor = true;
    }

    pub fn cursor_pos(&self) -> usize {
        self.pos.get(self.cursor).copied().flatten().unwrap_or(0)
    }

    // ----- background results ------------------------------------------------

    fn drain_results(&mut self, ctx: &egui::Context) {
        let mut n = 0;
        while let Some(done) = self.pool.try_recv() {
            // Results are keyed by folder generation and file path, not just
            // the slot: a late decode is dropped, never shown in place of the
            // photo now at that slot. Nothing here touches the focus or the
            // selection.
            let item_path = self.items.get(done.idx).map(|it| it.primary.path.as_path());
            if !done.belongs_to(self.generation, item_path) {
                continue;
            }
            if let (Some(e), None) = (done.exif, &self.exif[done.idx]) {
                self.exif[done.idx] = Some(e);
            }
            match done.result {
                Ok(rgb) => {
                    if let (Some(n), Some(slot)) = (done.native, self.native.get_mut(done.idx)) {
                        *slot = Some(n);
                    }
                    let gain = if done.kind == JobKind::Preview {
                        self.preview_boost
                    } else {
                        0
                    };
                    let tex = upload(ctx, &format!("{:?}{}", done.kind, done.idx), &rgb, gain);
                    self.stats.textures_uploaded += 1;
                    match done.kind {
                        JobKind::Thumb => {
                            self.thumbs[done.idx] = Some(tex);
                            self.thumb_err[done.idx] = None;
                            self.stats.thumbs_done += 1;
                            if done.cache_hit {
                                self.stats.cache_hits += 1;
                            }
                            self.stats.decode_ms_total += done.millis;
                        }
                        JobKind::Preview => {
                            self.previews
                                .insert(done.idx, (tex, rgb.long_side(), done.full));
                            self.preview_lru.retain(|&i| i != done.idx);
                            self.preview_lru.push_back(done.idx);
                            self.evict_previews();
                        }
                    }
                }
                Err(e) => {
                    if done.kind == JobKind::Thumb {
                        self.thumb_err[done.idx] = Some(e);
                    } else {
                        self.status = format!("Preview failed: {e}");
                    }
                }
            }
            n += 1;
            if n >= 48 {
                ctx.request_repaint();
                break;
            }
        }
    }

    /// Keep at most `PREVIEW_LRU` previews and about `PREVIEW_BUDGET` bytes
    /// of texture (a 45 MP 1:1 preview is 180 MB), never the focused photo,
    /// its neighbours or the compare set.
    fn evict_previews(&mut self) {
        const PREVIEW_BUDGET: usize = 768 << 20;
        // The focused photo and what is preloading around it in view order
        // (where the arrow keys go next), not in file order.
        let mut keep = self.preview_wanted();
        if let Some(c) = &self.pv.compare {
            keep.extend(c.iter().copied());
        }
        let bytes = |i: usize| {
            self.previews.get(&i).map_or(0, |(t, ..)| {
                let [w, h] = t.size();
                w * h * 4
            })
        };
        let drop = preview_evictions(&self.preview_lru, bytes, &keep, PREVIEW_LRU, PREVIEW_BUDGET);
        for idx in drop {
            self.preview_lru.retain(|&i| i != idx);
            self.previews.remove(&idx);
        }
    }

    /// Photos to preload in Preview, most urgent first (see `prefetch_plan`),
    /// in the direction the user last moved.
    pub fn prefetch_targets(&self) -> Vec<usize> {
        prefetch_plan(self.cursor_pos(), self.view.len(), self.pv.dir)
            .into_iter()
            .filter_map(|p| self.view.get(p).copied())
            .collect()
    }

    /// The focused photo followed by its prefetch targets.
    pub fn preview_wanted(&self) -> Vec<usize> {
        let mut v = vec![self.cursor];
        v.extend(self.prefetch_targets());
        v
    }

    pub fn request_thumb(&self, idx: usize, prio: u32) {
        if idx >= self.items.len() || self.thumbs[idx].is_some() || self.thumb_err[idx].is_some() {
            return;
        }
        let it = &self.items[idx];
        self.pool.request(
            Job {
                idx,
                kind: JobKind::Thumb,
                path: it.primary.path.clone(),
                size: it.primary.size,
                modified: it.primary.modified,
                long_side: THUMB_PX,
                generation: self.generation,
            },
            prio,
        );
    }

    pub fn request_preview(&self, idx: usize, long_side: u32, prio: u32) {
        if idx >= self.items.len() {
            return;
        }
        if let Some((_, have, full)) = self.previews.get(&idx) {
            // Already have it, it is the whole file, or the window only grew a little.
            if *full || *have >= long_side || (*have >= 1600 && long_side <= have * 5 / 4) {
                return;
            }
        }
        let it = &self.items[idx];
        self.pool.request(
            Job {
                idx,
                kind: JobKind::Preview,
                path: it.primary.path.clone(),
                size: it.primary.size,
                modified: it.primary.modified,
                long_side,
                generation: self.generation,
            },
            prio,
        );
    }

    /// Neighbour in view order, for preview prefetch.
    pub fn view_neighbor(&self, delta: isize) -> Option<usize> {
        let p = self.cursor_pos() as isize + delta;
        if p < 0 {
            return None;
        }
        self.view.get(p as usize).copied()
    }

    pub fn schedule_thumbs(&mut self, visible_rows: Range<usize>) {
        if self.view.is_empty() || self.cols == 0 {
            return;
        }
        let rows = self.row_count();
        let first = visible_rows.start.saturating_sub(PREFETCH_ROWS);
        let last = (visible_rows.end + PREFETCH_ROWS).min(rows);
        for row in first..last {
            let prio = if row < visible_rows.start {
                10 + (visible_rows.start - row) as u32
            } else if row >= visible_rows.end {
                10 + (row + 1 - visible_rows.end) as u32
            } else {
                (row - visible_rows.start) as u32
            };
            for p in row * self.cols..((row + 1) * self.cols).min(self.view.len()) {
                self.request_thumb(self.view[p], prio);
            }
        }
        let keep_first = visible_rows.start.saturating_sub(KEEP_ROWS);
        let keep_last = (visible_rows.end + KEEP_ROWS).min(rows);
        for row in (0..keep_first).chain(keep_last..rows) {
            for p in row * self.cols..((row + 1) * self.cols).min(self.view.len()) {
                let idx = self.view[p];
                if self.thumbs[idx].is_some() && idx != self.cursor {
                    self.thumbs[idx] = None;
                    self.stats.thumbs_done = self.stats.thumbs_done.saturating_sub(1);
                }
            }
        }
    }

    // ----- marks -------------------------------------------------------------

    /// Apply `edit` to every target, record one undo entry, write files.
    fn mark(&mut self, label: &str, edit: impl Fn(&mut Marks)) {
        if self.items.is_empty() {
            return;
        }
        let targets = self.selection.targets(self.cursor);
        let mut changes = Vec::with_capacity(targets.len());
        for &i in &targets {
            let before = self.marks[i];
            let mut after = before;
            edit(&mut after);
            if after != before {
                changes.push(Change {
                    idx: i,
                    before,
                    after,
                });
            }
        }
        if changes.is_empty() {
            return;
        }
        let apply: Vec<(usize, Marks)> = changes.iter().map(|c| (c.idx, c.after)).collect();
        self.undo.push(Entry {
            label: format!(
                "{label} ({} photo{})",
                changes.len(),
                if changes.len() == 1 { "" } else { "s" }
            ),
            changes,
        });
        self.apply_marks(&apply);
        if self.prefs.advance_after_mark
            && targets.len() == 1
            && self.workspace != Workspace::Caption
        {
            self.move_focus(1, Mods::default());
        }
    }

    /// Turn the targets a quarter turn by writing `tiff:Orientation` to each
    /// sidecar; the files themselves are never touched. Turning the other way
    /// undoes it.
    fn rotate(&mut self, cw: bool) {
        if self.items.is_empty() {
            return;
        }
        let targets = self.selection.targets(self.cursor);
        // Work out each photo's next orientation (reads the EXIF cache and
        // the sidecar), then queue the writes: the files land on the
        // writer thread, in order behind any pending marks.
        let mut jobs: Vec<(PhotoItem, edits::Change)> = Vec::with_capacity(targets.len());
        let (mut failed, mut last_error) = (0usize, None);
        for &i in &targets {
            let item = &self.items[i];
            let file_o = self.exif[i]
                .as_ref()
                .map(|e| e.orientation)
                .or_else(|| {
                    apature_metadata::exif::read(item.path())
                        .ok()
                        .map(|e| e.orientation)
                })
                .unwrap_or(1);
            match edits::read(&item.sidecar_path()) {
                Ok(e) => {
                    let now = e.orientation_or(file_o);
                    let next = if cw {
                        edits::rotate_cw(now)
                    } else {
                        edits::rotate_ccw(now)
                    };
                    // Back at the camera's own orientation: drop the override.
                    let value = (next != file_o).then_some(next as u8);
                    jobs.push((
                        item.clone(),
                        edits::Change {
                            orientation: Some(value),
                            ..Default::default()
                        },
                    ));
                }
                Err(e) => {
                    failed += 1;
                    last_error = Some(e.to_string());
                    log::warn!("rotate {}: {e}", item.display_name());
                }
            }
        }
        let total = targets.len();
        let dir = if cw { "right" } else { "left" };
        let seqs: Vec<u64> = jobs
            .into_iter()
            .map(|(item, change)| self.mark_writer.submit_edits(item, change))
            .collect();
        let mut bulk = BulkWrite::new(seqs, total, BulkKind::Rotate { dir });
        bulk.failed = failed;
        bulk.last_error = last_error;
        if bulk.remaining.is_empty() {
            self.status = bulk.finish_text();
            self.status_since = Some(Instant::now());
        } else {
            self.bulk_write = Some(bulk);
            self.mark_status = None;
            self.status = format!(
                "Rotating {total} photo{} {dir}…",
                if total == 1 { "" } else { "s" }
            );
        }
    }

    /// Set marks in memory and queue the file writes; never waits for disk.
    /// The view keeps its order (and the scroll its place) even when sorted
    /// or filtered by marks: the photo stays put until the next re-sort.
    fn apply_marks(&mut self, apply: &[(usize, Marks)]) {
        let policy = self.prefs.jpeg_marks();
        let mut last = 0;
        for &(i, m) in apply {
            self.marks[i] = m;
            last = self
                .mark_writer
                .submit(self.items[i].clone(), m, self.scheme.clone(), policy);
        }
        self.status = if apply.len() == 1 {
            let (i, m) = apply[0];
            format!(
                "{}: {} · {} · {}",
                self.items[i].display_name(),
                ui::rating_text(m.rating),
                self.scheme.name(m.color_class).unwrap_or("no class"),
                if m.tagged { "tagged" } else { "untagged" },
            )
        } else {
            format!("{} photos marked", apply.len())
        };
        self.mark_status = Some((last, self.status.clone()));
        // No `rebuild_view` here, even when sorted or filtered by marks:
        // re-sorting moved the photo away under the user (the sheet
        // scrolled after it; Preview landed at the end with nowhere to go).
    }

    /// Take in finished mark writes: clear conflicts, report failures.
    fn poll_mark_writes(&mut self) {
        for d in self.mark_writer.poll() {
            let idx = self.items.iter().position(|it| it.path() == d.path);
            if d.result.is_ok() {
                if let Some(i) = idx {
                    if let Some(c) = self.conflicts.get_mut(i) {
                        c.clear();
                    }
                }
            }
            // A bulk write finishing: a rotate re-renders each photo as
            // its file lands, and the final status is set when the last
            // file reports in.
            if self
                .bulk_write
                .as_ref()
                .is_some_and(|b| b.remaining.contains(&d.seq))
            {
                let is_rotate = self
                    .bulk_write
                    .as_ref()
                    .is_some_and(|b| matches!(b.kind, BulkKind::Rotate { .. }));
                if is_rotate && d.result.is_ok() {
                    if let Some(i) = idx {
                        self.invalidate_thumb(i);
                    }
                }
                let mut bulk = self.bulk_write.take().unwrap();
                if let Some(text) = bulk.record(d.seq, &d.result) {
                    self.status = text;
                    self.status_since = Some(Instant::now());
                } else {
                    self.bulk_write = Some(bulk);
                }
                continue;
            }
            // A single caption save finishing: on failure the draft goes
            // dirty again so the caption is not silently lost.
            if let Some(cw) = &self.caption_write {
                if cw.seq == d.seq {
                    let cw = self.caption_write.take().unwrap();
                    match &d.result {
                        Ok(n) => {
                            if cw.announce {
                                let name = self
                                    .items
                                    .get(cw.idx)
                                    .map(|it| it.display_name())
                                    .unwrap_or_default();
                                self.status = format!(
                                    "Caption saved: {name} → {n} file{}",
                                    if *n == 1 { "" } else { "s" }
                                );
                            }
                        }
                        Err(e) => {
                            if let Some(draft) = self.caption_draft.as_mut() {
                                if draft.idx == cw.idx {
                                    draft.original = cw.original;
                                }
                            }
                            self.status = format!("Could not save caption: {e}");
                            self.status_since = Some(Instant::now());
                        }
                    }
                    continue;
                }
            }
            match d.result {
                Ok(n) => {
                    if let Some((seq, text)) = &self.mark_status {
                        if *seq == d.seq && self.status == *text {
                            self.status =
                                format!("{text} → {n} file{}", if n == 1 { "" } else { "s" });
                        }
                    }
                }
                Err(e) => {
                    log::warn!("{} for {}: {e}", d.label, d.path.display());
                    if d.label == "edits" {
                        // The only single edits job is a crop save.
                        self.status = format!("Could not save crop: {e}");
                        self.toast(ui::toasts::Kind::Error, self.status.clone());
                    } else {
                        self.status = format!("Could not write {}: {e}", d.label);
                    }
                    self.status_since = Some(Instant::now());
                }
            }
        }
    }

    /// Drop a photo's decoded pixels so the next frame re-reads them with
    /// the sidecar's new orientation.
    fn invalidate_thumb(&mut self, i: usize) {
        self.thumbs[i] = None;
        self.thumb_err[i] = None;
        self.native[i] = None;
        self.previews.remove(&i);
        self.preview_lru.retain(|&p| p != i);
    }

    /// Wait for queued sidecar writes before anything reads a sidecar from
    /// disk (a rescan, opening a folder, a command that touches files) or
    /// before the app quits, so none of them reads stale data. Instant when
    /// nothing is queued. Writes themselves no longer need it: they go
    /// through the mark writer, which runs them one at a time in order.
    pub fn flush_marks(&mut self) {
        if self.mark_writer.pending() > 0
            && !self
                .mark_writer
                .wait_idle(std::time::Duration::from_secs(30))
        {
            log::warn!("mark writes still pending after 30 s");
        }
        self.poll_mark_writes();
    }

    // ----- navigation --------------------------------------------------------

    pub fn row_count(&self) -> usize {
        if self.cols == 0 {
            0
        } else {
            self.view.len().div_ceil(self.cols)
        }
    }

    /// Remember where the focus ring was so it can slide to the new cell.
    fn note_focus_move(&mut self, new_cursor: usize) {
        if new_cursor != self.cursor {
            self.focus_from = self.cursor_cell_rect;
            self.focus_since = Some(Instant::now());
        }
    }

    /// One 2 s poll, launched off the UI thread: the worker lists
    /// `/Volumes` and, when Live Ingest is on, scans its source folder and
    /// builds the plan for any settled files; `drain_poll` applies what it
    /// sends back. A tick that finds the previous poll still running is
    /// skipped, so scans never pile up on a slow card.
    fn poll_tick(&mut self, ctx: &egui::Context) {
        if self.poll_rx.is_some() {
            return;
        }
        let kinds = self.volume_kinds.clone();
        let live = self.ingest.live.on.then(|| {
            (
                self.ingest.opts.source.clone(),
                self.ingest.opts.include_subfolders,
                self.ingest.live.pending.clone(),
                self.ingest.live.handed.clone(),
                self.ingest.opts.clone(),
                self.ledger_path(),
            )
        });
        let (tx, rx) = std::sync::mpsc::channel();
        self.poll_rx = Some(rx);
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("volume-poll".into())
            .spawn(move || {
                let mut kinds = kinds;
                let (volumes, volume_devs) = scan_volumes(&mut kinds);
                let live = live.and_then(|(src, subfolders, pending, handed, opts, ledger_path)| {
                    let (pending, ready) = live_scan(&src, subfolders, pending, &handed)?;
                    let plan = if ready.is_empty() {
                        None
                    } else {
                        let mut opts = opts;
                        opts.only_paths = Some(ready.clone());
                        opts.day = None;
                        let ledger = apature_ingest::Ledger::load(&ledger_path);
                        Some(apature_ingest::build_plan(&opts, &ledger).map_err(|e| e.to_string()))
                    };
                    Some(LivePoll {
                        src,
                        pending,
                        ready,
                        plan,
                    })
                });
                let _ = tx.send(PollResult {
                    volumes,
                    volume_kinds: kinds,
                    volume_devs,
                    live,
                });
                ctx.request_repaint();
            });
    }

    /// Apply a finished background poll, volume changes first and Live
    /// Ingest second — the order the old synchronous tick ran them.
    fn drain_poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.poll_rx else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.poll_rx = None;
                let PollResult {
                    volumes,
                    volume_kinds,
                    volume_devs,
                    live,
                } = result;
                self.apply_volume_poll(ctx, volumes, volume_kinds, volume_devs);
                if let Some(live) = live {
                    self.apply_live_poll(live);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.poll_rx = None;
            }
        }
    }

    /// Apply a finished background volume poll: the worker's fresh list
    /// replaces the old one, then mounts and unplugs are announced and
    /// acted on exactly as the old synchronous poll did them.
    fn apply_volume_poll(
        &mut self,
        ctx: &egui::Context,
        volumes: Vec<(String, PathBuf)>,
        volume_kinds: HashMap<PathBuf, crate::platform::VolumeKind>,
        volume_devs: HashMap<PathBuf, u64>,
    ) {
        let before: Vec<(String, PathBuf)> = self.volumes.clone();
        self.volumes = volumes;
        self.volume_kinds = volume_kinds;
        self.volume_devs = volume_devs;
        // Anything that vanished: say so, and mark a tab whose folder went with it.
        let now: Vec<PathBuf> = self.volumes.iter().map(|(_, p)| p.clone()).collect();
        for (name, path) in before.iter().filter(|(_, p)| !now.contains(p)) {
            let was_card = self.volume_kinds.get(path)
                == Some(&crate::platform::VolumeKind::Camera)
                || path.join("DCIM").is_dir();
            let what = if was_card { "Card" } else { "Drive" };
            log::warn!("{what} unplugged: {name} ({})", path.display());
            let busy = self
                .ingest
                .running
                .as_ref()
                .is_some_and(|r| r.done.is_none())
                && (self.ingest.opts.source.starts_with(path)
                    || self.ingest.opts.primary.starts_with(path)
                    || self
                        .ingest
                        .opts
                        .secondary
                        .as_ref()
                        .is_some_and(|s| s.starts_with(path)));
            self.status = if busy {
                format!("{what} unplugged during the ingest: {name} · remaining files will fail; plug it back in and ingest again (only new photos copy)")
            } else {
                format!("{what} unplugged: {name}")
            };
            self.status_since = Some(Instant::now());
            let msg = self.status.clone();
            self.toast(ui::toasts::Kind::Warn, msg);
            if self.folder.as_ref().is_some_and(|f| f.starts_with(path)) {
                self.folder_unplugged = Some(name.clone());
            }
            if self.ingest.opts.source.starts_with(path) && !busy {
                self.ingest.plan = None;
                self.ingest.plan_error = Some(format!("{name} was unplugged"));
            }
        }
        // Back again?
        if let Some(name) = self.folder_unplugged.clone() {
            if self.folder.as_ref().is_some_and(|f| f.exists()) {
                self.folder_unplugged = None;
                self.status = format!("{name} is back");
                self.refresh_folder();
            }
        }
        let before: Vec<PathBuf> = before.into_iter().map(|(_, p)| p).collect();
        let new_cards: Vec<(String, PathBuf)> = self
            .volumes
            .iter()
            .filter(|(_, p)| !before.contains(p) && p.join("DCIM").is_dir())
            .cloned()
            .collect();
        if let Some((name, path)) = new_cards.first() {
            self.status = format!("Card mounted: {name}");
            if self.prefs.open_ingest_on_card && self.ingest.running.is_none() {
                self.ingest.opts.source = path.clone();
                self.ingest.opts.include_subfolders = true;
                self.open_ingest(ctx);
                self.ingest_options_changed();
            }
        }
    }

    fn move_focus(&mut self, delta: isize, mods: Mods) {
        if self.view.is_empty() {
            return;
        }
        let n = self.view.len() as isize;
        let p = (self.cursor_pos() as isize + delta).clamp(0, n - 1) as usize;
        self.note_focus_move(self.view[p]);
        self.cursor = self.view[p];
        self.selection.focus_moved(self.cursor, mods, &self.view);
        self.scroll_to_cursor = true;
    }

    fn focus_to_pos(&mut self, p: usize, mods: Mods) {
        if let Some(&idx) = self.view.get(p) {
            self.note_focus_move(idx);
            self.cursor = idx;
            self.selection.focus_moved(idx, mods, &self.view);
            self.scroll_to_cursor = true;
        }
    }

    pub fn set_cell(&mut self, cell: f32) {
        self.prefs.cell = cell.clamp(80.0, 600.0);
        self.prefs_dirty = true;
        self.scroll_to_cursor = true;
    }

    pub fn cell_accessible_name(&self, idx: usize) -> String {
        let m = self.marks[idx];
        let mut parts = vec![self.items[idx].display_name()];
        if m.rating.is_rejected() {
            parts.push("rejected".into());
        } else if m.rating.stars() > 0 {
            parts.push(format!("{} stars", m.rating.stars()));
        }
        if let Some(n) = self.scheme.name(m.color_class) {
            parts.push(format!("class {} {n}", m.color_class.0));
        }
        if m.tagged {
            parts.push("tagged".into());
        }
        if self.selection.contains(idx) {
            parts.push("selected".into());
        }
        parts.push(format!(
            "{} of {}",
            self.pos
                .get(idx)
                .copied()
                .flatten()
                .map(|p| p + 1)
                .unwrap_or(0),
            self.view.len()
        ));
        parts.join(", ")
    }

    /// Bottom-bar parts: the folder, count and focused photo moved to the
    /// info strip at the top (ui/infobar.rs); this keeps the marks and the
    /// transient message.
    pub fn status_parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(it) = self.items.get(self.cursor) {
            let _ = it;
            let m = self.marks[self.cursor];
            let stars = ui::rating_text(m.rating);
            if !stars.is_empty() {
                parts.push(stars);
            }
            if let Some(n) = self.scheme.name(m.color_class) {
                parts.push(format!("{} {n}", m.color_class.0));
            }
            if m.tagged {
                parts.push("tagged".into());
            }
        }
        parts
    }

    // ----- commands ----------------------------------------------------------

    pub fn dispatch(&mut self, ctx: &egui::Context, cmd: Command) {
        if !cmd.leaves_files_alone() {
            self.flush_marks();
        }
        match cmd {
            Command::OpenFolder(p) => self.open_folder(&p),
            Command::OpenFolderHere(p) => self.open_folder_here(&p),
            Command::RevealFolder(p) => Platform::reveal(&p),
            Command::AddFavoritePath(p) => {
                if !self.prefs.favorites.contains(&p) {
                    self.prefs.favorites.push(p);
                    self.prefs_dirty = true;
                }
            }
            Command::PickFolder => self.pick_folder(),
            Command::AddFavorite => {
                if let Some(f) = self.folder.clone() {
                    if !self.prefs.favorites.contains(&f) {
                        self.prefs.favorites.push(f);
                        self.prefs_dirty = true;
                    }
                }
            }
            Command::RemoveFavorite(p) => {
                self.prefs.favorites.retain(|f| f != &p);
                self.prefs_dirty = true;
            }
            Command::RevealInFinder => {
                if let Some(it) = self.items.get(self.cursor) {
                    Platform::reveal(it.path());
                }
            }
            Command::ToggleSidebar => {
                if self.workspace == Workspace::Preview {
                    self.preview_sidebar = !self.preview_sidebar;
                } else {
                    self.prefs.sidebar_visible = !self.prefs.sidebar_visible;
                    self.prefs_dirty = true;
                }
            }
            Command::ToggleInspector => {
                if self.workspace == Workspace::Preview {
                    self.preview_inspector = Some(!self.inspector_shown());
                } else {
                    self.prefs.inspector_visible = !self.prefs.inspector_visible;
                    self.prefs_dirty = true;
                }
            }
            Command::ToggleFilmstrip => {
                self.prefs.filmstrip_visible = !self.prefs.filmstrip_visible;
                self.prefs_dirty = true;
            }
            Command::SetWorkspace(Workspace::Ingest) => self.open_ingest(ctx),
            Command::SetWorkspace(w) => {
                if w.available().is_ok() && !self.items.is_empty() {
                    if w == Workspace::Preview && self.workspace != Workspace::Preview {
                        self.enter_preview();
                    } else {
                        self.workspace = w;
                    }
                    self.scroll_to_cursor = true;
                }
            }
            Command::ZoomIn => self.set_cell(self.prefs.cell * 1.2),
            Command::ZoomOut => self.set_cell(self.prefs.cell / 1.2),
            Command::Click {
                idx,
                shift,
                command,
            } => {
                if self.pos.get(idx).copied().flatten().is_some() {
                    self.cursor = idx;
                    self.selection
                        .click(idx, Mods { shift, command }, &self.view);
                }
            }
            Command::MoveFocus {
                delta,
                shift,
                command,
            } => self.move_focus(delta, Mods { shift, command }),
            Command::Move(d) => self.move_focus(d, Mods::default()),
            Command::MovePage(d) => {
                let page = (self.last_visible_rows.len().max(1) * self.cols.max(1)) as isize;
                self.move_focus(d * page, Mods::default());
            }
            Command::Home => self.focus_to_pos(0, Mods::default()),
            Command::End => self.focus_to_pos(self.view.len().saturating_sub(1), Mods::default()),
            Command::SelectAll => self.selection.all(&self.view),
            Command::SelectNone => self.selection.clear(),
            Command::SelectBy(by) => self.select_by(by),
            Command::CopyPhotos => self.open_copy_dialog(),
            Command::CopyChooseDest => self.copy_choose_dest(),
            Command::CopyStart => self.copy_start(),
            Command::ExtractPreviews => self.open_extract_dialog(),
            Command::ExtractStart => self.extract_start(),
            Command::ExtractCancel => {
                if let Some(r) = &self.extract_running {
                    r.cancel();
                }
            }
            Command::ExtractReveal => {
                if let Some(first) = self
                    .extract_running
                    .as_ref()
                    .and_then(|r| r.done.as_ref())
                    .and_then(|o| o.extracted.first())
                {
                    crate::platform::Platform::reveal(&first.1);
                }
            }
            Command::CopyCancel => {
                if let Some(r) = &self.copy_running {
                    r.cancel();
                }
            }
            Command::CopyReveal => {
                if let Some(first) = self
                    .copy_running
                    .as_ref()
                    .and_then(|r| r.done.as_ref())
                    .and_then(|o| o.copied.first())
                {
                    crate::platform::Platform::reveal(&first.1);
                }
            }
            Command::Undo => {
                if let Some((label, apply)) = self.undo.undo() {
                    self.apply_marks(&apply);
                    self.status = format!("Undid {label}");
                }
            }
            Command::Redo => {
                if let Some((label, apply)) = self.undo.redo() {
                    self.apply_marks(&apply);
                    self.status = format!("Redid {label}");
                }
            }
            Command::SetFilter(f) => {
                self.filter = f;
                self.rebuild_view();
            }
            Command::ClearFilter(None) => {
                self.filter = Filter::default();
                self.rebuild_view();
            }
            Command::ClearFilter(Some(c)) => {
                self.filter.clear(c);
                self.rebuild_view();
            }
            Command::SetSort(s) => {
                self.prefs.sort = s;
                self.prefs_dirty = true;
                self.rebuild_view();
            }
            Command::ToggleSortReversed => {
                self.prefs.sort_reversed = !self.prefs.sort_reversed;
                self.prefs_dirty = true;
                self.rebuild_view();
            }
            Command::ToggleAdvanceAfterMark => {
                self.prefs.advance_after_mark = !self.prefs.advance_after_mark;
                self.prefs_dirty = true;
            }
            Command::LoadAssignment => {
                let mut dlg =
                    rfd::FileDialog::new().add_filter("Assignment", &["txt", "assignment"]);
                if let Some(dir) = self.prefs.assignment_file.as_ref().and_then(|p| p.parent()) {
                    dlg = dlg.set_directory(dir);
                }
                if let Some(p) = dlg.pick_file() {
                    self.load_assignment(&p);
                }
            }
            Command::LoadCodes => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Code replacements", &["txt"])
                    .pick_file()
                {
                    self.load_codes(&p);
                }
            }
            Command::ApplyStationery => self.apply_stationery(),
            Command::InsertCaptionTemplate => self.insert_caption_template(),
            Command::SaveCaption => {
                self.save_caption(true);
            }
            Command::SaveCaptionToSelection => self.save_caption_to_selection(),
            Command::RefreshFolder => self.refresh_folder(),
            Command::DeliverResendFailed => {
                let failed: Vec<PathBuf> = self
                    .deliver
                    .running
                    .as_ref()
                    .map(|r| {
                        r.files
                            .iter()
                            .zip(r.statuses.iter())
                            .filter(|(_, s)| **s == DStatus::Failed)
                            .map(|(f, _)| f.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                if failed.is_empty() {
                    self.status = "Nothing failed to resend".into();
                } else {
                    let name = self.deliver.running.as_ref().map(|r| r.target.clone());
                    self.deliver.last_outputs = failed;
                    if let Some(n) = name {
                        self.start_delivery(&n);
                    }
                }
            }
            Command::CropToggle => {
                if self.pv.crop.is_some() {
                    self.pv.crop = None;
                    self.status = "Crop cancelled".into();
                } else if self.workspace == Workspace::Preview && self.cursor < self.items.len() {
                    let idx = self.cursor;
                    let o = self.effective_orientation(idx);
                    let existing = edits::read(&self.items[idx].sidecar_path())
                        .ok()
                        .and_then(|e| e.crop)
                        .filter(|c| !c.is_full())
                        .map(|c| crop_stored_to_display([c.left, c.top, c.right, c.bottom], o));
                    self.pv.crop = Some(CropEdit {
                        rect: existing.unwrap_or([0.1, 0.1, 0.9, 0.9]),
                        ratio_i: 0,
                        overlay: CropOverlay::Thirds,
                        drag: None,
                    });
                    self.pv.mode = ZoomMode::Fit;
                    self.status = "Crop: drag the frame or its handles · R ratio · G guides · ⏎ apply · ⌫ remove · Esc cancel".into();
                }
            }
            Command::CropNextRatio => {
                if let Some(c) = self.pv.crop.as_mut() {
                    c.ratio_i = (c.ratio_i + 1) % CROP_RATIOS.len();
                    if let Some(r) = c.ratio() {
                        // Re-fit the current rectangle to the ratio around its centre.
                        let [l, t, rr, b] = c.rect;
                        let (cx, cy) = ((l + rr) / 2.0, (t + b) / 2.0);
                        let img = self
                            .pv
                            .last_rect
                            .map(|r| r.size())
                            .unwrap_or(egui::vec2(3.0, 2.0));
                        let mut w = rr - l;
                        let mut h = w * img.x / img.y / r;
                        if h > 1.0 {
                            h = 1.0;
                            w = h * img.y / img.x * r;
                        }
                        let mut rect = [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0];
                        let dx = (-rect[0]).max(0.0) - (rect[2] - 1.0).max(0.0);
                        let dy = (-rect[1]).max(0.0) - (rect[3] - 1.0).max(0.0);
                        rect[0] += dx;
                        rect[2] += dx;
                        rect[1] += dy;
                        rect[3] += dy;
                        c.rect = rect;
                    }
                    self.status = format!("Crop ratio: {}", CROP_RATIOS[c.ratio_i].1);
                }
            }
            Command::CropNextOverlay => {
                if let Some(c) = self.pv.crop.as_mut() {
                    c.overlay = match c.overlay {
                        CropOverlay::None => CropOverlay::Thirds,
                        CropOverlay::Thirds => CropOverlay::Golden,
                        CropOverlay::Golden => CropOverlay::None,
                    };
                }
            }
            Command::ToggleStack => {
                // The focused item is a head, or a member of an open stack.
                let head = if self.stacks.contains_key(&self.cursor) {
                    Some(self.cursor)
                } else {
                    self.stacks
                        .iter()
                        .find(|(_, m)| m.contains(&self.cursor))
                        .map(|(h, _)| *h)
                };
                if let Some(h) = head {
                    if !self.stacks_open.remove(&h) {
                        self.stacks_open.insert(h);
                    } else {
                        self.cursor = h;
                    }
                    let n = self.stacks.get(&h).map(|m| m.len() + 1).unwrap_or(1);
                    self.status = if self.stacks_open.contains(&h) {
                        format!("Burst of {n} expanded · S collapses")
                    } else {
                        format!("Burst of {n} collapsed · S expands")
                    };
                    self.rebuild_view();
                }
            }
            Command::ToggleLoupe => {
                self.pv.loupe = !self.pv.loupe;
                self.status = if self.pv.loupe {
                    format!(
                        "Loupe on · {:.0}% · ⌘-scroll changes magnification",
                        self.pv.zoom * 100.0
                    )
                } else {
                    "Loupe off".into()
                };
            }
            Command::CropCancel => {
                self.pv.crop = None;
                self.status = "Crop cancelled".into();
            }
            Command::CropClear => {
                if self.pv.crop.take().is_some() && self.cursor < self.items.len() {
                    self.write_crop(self.cursor, None);
                }
            }
            Command::CropApply => {
                if let Some(c) = self.pv.crop.take() {
                    if self.cursor < self.items.len() {
                        let o = self.effective_orientation(self.cursor);
                        let [l, t, r, b] = crop_display_to_stored(c.rect, o);
                        let crop = edits::Crop {
                            top: t,
                            left: l,
                            bottom: b,
                            right: r,
                        };
                        self.write_crop(self.cursor, Some(crop));
                    }
                }
            }
            Command::CyclePreviewBoost => {
                self.preview_boost = (self.preview_boost + 1) % 3;
                // Previews re-upload with the new gain; thumbnails are untouched.
                self.previews.clear();
                self.status = match self.preview_boost {
                    0 => "Preview at camera exposure".into(),
                    n => format!(
                        "Preview +{n} stop{} · view only, B cycles",
                        if n == 1 { "" } else { "s" }
                    ),
                };
            }
            Command::DeliverLast => {
                let name = self
                    .deliver
                    .pick
                    .clone()
                    .or_else(|| self.prefs.send_after_export.clone())
                    .or_else(|| self.prefs.delivery.first().map(|t| t.name.clone()));
                match name {
                    Some(n) => self.start_delivery(&n),
                    None => self.status = "No delivery target yet: Settings › Delivery".into(),
                }
            }
            Command::DeliverTest(name) => {
                let Some(t) = self.prefs.delivery.iter().find(|t| t.name == name).cloned() else {
                    return;
                };
                let secret = Platform::secret_get(&t.name).unwrap_or_default();
                self.deliver.test_result =
                    Some(match apature_deliver::test_connection(&t, &secret) {
                        Ok(()) => format!("{}: connected and logged in", t.name),
                        Err(e) => format!("{}: {e}", t.name),
                    });
            }
            Command::SaveCaptionAndNext => {
                self.save_caption(true);
                self.move_focus(1, Mods::default());
                self.sync_caption_draft();
            }
            Command::RevertCaption => {
                if let Some(d) = self.caption_draft.as_mut() {
                    d.fields = d.original.clone();
                }
            }
            Command::IngestChooseSource => {
                if let Some(p) = rfd::FileDialog::new()
                    .set_directory("/Volumes")
                    .pick_folder()
                {
                    self.ingest.opts.source = p;
                    self.ingest_options_changed();
                }
            }
            Command::IngestChoosePrimary => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.ingest.opts.primary = p;
                    self.ingest_options_changed();
                }
            }
            Command::IngestChooseSecondary => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.ingest.opts.secondary = Some(p);
                    self.ingest_options_changed();
                }
            }
            Command::ExportChooseDest => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.prefs.export.dest = Some(p);
                    self.prefs_dirty = true;
                }
            }
            Command::ExportStart => self.export_start(),
            Command::ExportCancel => {
                if let Some(r) = &self.export.running {
                    r.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Command::ExportOpenResult => {
                if let Some(d) = self.export.running.as_ref().map(|r| r.dest.clone()) {
                    self.export.running = None;
                    self.open_folder(&d);
                }
            }
            Command::ToggleZoom => self.toggle_zoom(None, self.pv.last_rect),
            Command::CopyMetadata => {
                if let Some(it) = self.items.get(self.cursor) {
                    let iptc = items::read_iptc(it);
                    let m = self.marks[self.cursor];
                    self.meta_clip = Some((m, iptc));
                    self.status = format!("Copied metadata of {}", it.display_name());
                }
            }
            Command::PasteMetadata => self.paste_metadata(),
            Command::IngestEject => {
                let src = self.ingest.opts.source.clone();
                match Platform::eject(&src) {
                    Some(root) => {
                        self.status = format!("Ejecting {}", root.display());
                        self.volumes_checked = Instant::now() - std::time::Duration::from_secs(2);
                    }
                    None => self.status = "Not a removable volume".into(),
                }
            }
            Command::RenamePhotos => self.open_rename_dialog(),
            Command::RenameApply => self.rename_apply(),
            Command::MoveToTrash => {
                if !self.items.is_empty() {
                    let mut targets = self.selection.targets(self.cursor);
                    targets.sort_by_key(|&i| self.pos[i].unwrap_or(usize::MAX));
                    // The system's own confirmation (a sheet on macOS); the
                    // in-window dialog stays for headless screenshot runs.
                    if self.launch.screenshot.is_some() {
                        self.trash_dialog = Some(targets);
                    } else {
                        let n = targets.len();
                        let files: usize = targets
                            .iter()
                            .map(|&i| apature_core::fileops::item_files(&self.items[i]).len())
                            .sum();
                        let ok = rfd::MessageDialog::new()
                            .set_level(rfd::MessageLevel::Warning)
                            .set_title(if n == 1 { "Move this photo to the Trash?" } else { "Move these photos to the Trash?" })
                            .set_description(format!(
                                "{n} photo{} ({files} file{} with sidecars and pairs) go to the Trash. You can put them back from the Finder.",
                                if n == 1 { "" } else { "s" },
                                if files == 1 { "" } else { "s" }
                            ))
                            .set_buttons(rfd::MessageButtons::OkCancelCustom("Move to Trash".into(), "Cancel".into()))
                            .show();
                        if ok == rfd::MessageDialogResult::Ok
                            || matches!(ok, rfd::MessageDialogResult::Custom(ref s) if s == "Move to Trash")
                        {
                            self.trash_dialog = Some(targets);
                            self.trash_confirm();
                        }
                    }
                }
            }
            Command::OpenAbout => self.about_open = true,
            Command::ReportBug => self.open_bug_dialog(),
            Command::CheckUpdates => self.start_update_check(ctx, true),
            Command::OpenUpdate => self.start_update_install(ctx),
            Command::OpenUpdatePage => {
                if let Some(u) = &self.update_available {
                    let _ = std::process::Command::new("open").arg(&u.page).spawn();
                }
            }
            Command::TrashConfirm => self.trash_confirm(),
            Command::CloseDialog => {
                self.bug_dialog = None;
                self.rename_dialog = None;
                self.trash_dialog = None;
                self.close_copy_dialog();
                self.extract_dialog = None;
                if self
                    .extract_running
                    .as_ref()
                    .is_some_and(|r| r.done.is_some())
                {
                    self.extract_running = None;
                }
            }
            Command::ToggleCompare => self.toggle_compare(),
            Command::IngestStart => self.ingest_start(),
            Command::CloseWindow => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Command::MinimizeWindow => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true))
            }
            Command::OpenShortcuts => {
                self.settings_open = true;
                self.settings_page = ui::settings::Page::Shortcuts;
            }
            Command::RevealLog => {
                if let Some(d) = apature_core::project_dirs() {
                    let _ = std::process::Command::new("open")
                        .arg("-R")
                        .arg(d.data_dir().join("apature.log"))
                        .spawn();
                }
            }
            Command::OpenRepo => {
                // This edition's repository; About links the original.
                let _ = std::process::Command::new("open")
                    .arg(crate::credits::FORK_URL)
                    .spawn();
            }
            Command::ShowMainWindow => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Command::LiveToggle => {
                let l = &mut self.ingest.live;
                l.on = !l.on;
                if l.on {
                    l.pending.clear();
                    l.handed.clear();
                    l.ingested = 0;
                    l.since = Some(Instant::now());
                    // Everything already there counts as seen: Live Ingest is
                    // for what arrives from now on.
                    let src = self.ingest.opts.source.clone();
                    let depth = if self.ingest.opts.include_subfolders {
                        8
                    } else {
                        1
                    };
                    if let Ok(items) = apature_core::scan(
                        &src,
                        &apature_core::ScanOptions {
                            recursive: depth > 1,
                        },
                    ) {
                        for it in &items {
                            for v in it.variants() {
                                self.ingest.live.handed.insert(v.path.clone());
                            }
                        }
                    }
                    self.status =
                        format!("Live Ingest watching {}", self.ingest.opts.source.display());
                    log::info!("live ingest on: {}", self.ingest.opts.source.display());
                } else {
                    self.status = format!(
                        "Live Ingest stopped · {} photos this session",
                        self.ingest.live.ingested
                    );
                    log::info!("live ingest off");
                }
            }
            Command::IngestCancel => {
                if let Some(r) = &self.ingest.running {
                    r.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Command::CloseTab => {
                let i = self.active;
                self.close_tab(i);
            }
            Command::NextTab => {
                if !self.tabs.is_empty() {
                    let i = (self.active + 1) % self.tabs.len();
                    self.switch_tab(i);
                }
            }
            Command::PrevTab => {
                if !self.tabs.is_empty() {
                    let i = (self.active + self.tabs.len() - 1) % self.tabs.len();
                    self.switch_tab(i);
                }
            }
            Command::SelectTab(i) => self.switch_tab(i),
            Command::OpenSettings => {
                self.settings_open = true;
                self.cache_size = None;
            }
            Command::ClearThumbCache => {
                let dir = self.cache_dir();
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::create_dir_all(&dir);
                self.cache_size = Some((0, 0));
                self.status = "Thumbnail cache cleared".into();
            }
            Command::IngestOpenResult => {
                let dest = self
                    .ingest
                    .plan
                    .as_ref()
                    .and_then(|p| p.entries.first())
                    .and_then(|e| e.primary.parent().map(|d| d.to_path_buf()));
                if let Some(d) = dest {
                    self.ingest.running = None;
                    self.ingest.plan = None;
                    self.ingest_window = false;
                    let already = self.tabs.iter().position(|t| t.folder == d);
                    self.open_folder(&d);
                    if already.is_some() {
                        // The tab was opened mid-ingest: make sure it shows every file.
                        self.refresh_folder();
                    }
                }
            }
            Command::SetRating(r) => self.mark(if r < 0 { "reject" } else { "rate" }, |m| {
                m.rating = Rating::clamp(r)
            }),
            Command::SetClass(c) => self.mark("class", |m| m.color_class = ColorClass::clamp(c)),
            Command::ToggleTag => {
                // A batch toggle sets every target to the opposite of the focused item.
                let to = !self
                    .marks
                    .get(self.cursor)
                    .map(|m| m.tagged)
                    .unwrap_or(false);
                self.mark(if to { "tag" } else { "untag" }, |m| m.tagged = to)
            }
            Command::Rotate(cw) => self.rotate(cw),
            Command::Escape => {
                if self.bug_dialog.is_some() {
                    self.bug_dialog = None;
                } else if self.pv.crop.is_some() {
                    self.dispatch_later = Some(Command::CropCancel);
                } else if self.copy_dialog.is_some() {
                    self.close_copy_dialog();
                } else if self.extract_dialog.is_some() {
                    self.dispatch_later = Some(Command::CloseDialog);
                } else if self.rename_dialog.is_some() || self.trash_dialog.is_some() {
                    self.rename_dialog = None;
                    self.trash_dialog = None;
                } else if self.filter_popup_open {
                    self.filter_popup_open = false;
                } else if self.workspace == Workspace::Preview && self.pv.compare.is_some() {
                    self.pv.compare = None;
                    self.status = String::new();
                } else if self.workspace == Workspace::Preview && self.pv.mode == ZoomMode::OneToOne
                {
                    self.toggle_zoom(None, self.pv.last_rect);
                } else if self.workspace == Workspace::Preview {
                    self.workspace = Workspace::Sheet;
                    self.scroll_to_cursor = true;
                } else if self.selection.len() > 1 {
                    self.selection.only(self.cursor);
                }
            }
            Command::ClosePreview => {
                if self.workspace == Workspace::Preview && self.pv.crop.is_none() {
                    self.pv.compare = None;
                    self.workspace = Workspace::Sheet;
                    self.scroll_to_cursor = true;
                }
            }
            Command::Activate => {
                if self.pv.crop.is_some() {
                    self.dispatch_later = Some(Command::CropApply);
                } else if self.workspace == Workspace::Sheet && !self.items.is_empty() {
                    self.enter_preview();
                } else if self.workspace == Workspace::Preview {
                    self.workspace = Workspace::Sheet;
                    self.scroll_to_cursor = true;
                }
            }
        }
        self.sync_menu();
        ctx.request_repaint();
    }

    /// Keyboard → commands. Command-modified keys are the native menu's job
    /// when a menu exists, except ⌘-arrows (move focus, keep selection).
    /// Bare keys are always this handler's job (see `keyonce`), and nothing
    /// fires while a text field has focus.
    fn keyboard_commands(&self, ctx: &egui::Context) -> Vec<Command> {
        let mut out = Vec::new();
        let has_menu = self.platform.has_menu();
        // The sidebar's folder tree holds focus without taking text: culling
        // keys still work, only the arrows and Return go to the tree.
        let tree = ui::sidebar::tree_has_focus(ctx);
        if !crate::keyonce::handler_takes(has_menu, ctx.wants_keyboard_input() && !tree, false) {
            return out;
        }
        if self.bug_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.trash_dialog.is_some()
            || self.copy_dialog.is_some()
            || self.extract_dialog.is_some()
        {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                out.push(Command::Escape);
            }
            return out;
        }
        ctx.input(|i| {
            // The modifiers held for this frame's key presses: a quick ⌥C can
            // release ⌥ within the same frame, leaving `i.modifiers` empty.
            let mods = i.events.iter().fold(i.modifiers, |m, e| match e {
                egui::Event::Key {
                    pressed: true,
                    modifiers,
                    ..
                } => m | *modifiers,
                _ => m,
            });
            let cmd = mods.command;
            let shift = mods.shift;
            let alt = mods.alt;
            let cols = self.cols.max(1) as isize;
            let in_preview = self.workspace == Workspace::Preview;
            let pm = self.prefs.shortcut_layout == ShortcutLayout::PhotoMechanic;
            let bare_rates = self.prefs.number_keys == crate::prefs::NumberKeys::Rating;
            let mv = |delta: isize| Command::MoveFocus {
                delta,
                shift,
                command: cmd,
            };
            let pressed = |k: Key| {
                i.key_pressed(k)
                    && !(tree
                        && matches!(
                            k,
                            Key::ArrowLeft
                                | Key::ArrowRight
                                | Key::ArrowUp
                                | Key::ArrowDown
                                | Key::Enter
                        ))
            };
            if pressed(Key::ArrowLeft) {
                out.push(mv(-1));
            }
            if pressed(Key::ArrowRight) {
                out.push(mv(1));
            }
            if pressed(Key::ArrowUp) {
                out.push(mv(if in_preview { -1 } else { -cols }));
            }
            if pressed(Key::ArrowDown) {
                out.push(mv(if in_preview { 1 } else { cols }));
            }
            // Photo Mechanic keys no menu item carries: [ and ] rotate, M
            // renames, I opens the caption (its Info window).
            if pm && !cmd && !alt && !shift && !mods.ctrl {
                if pressed(Key::OpenBracket) {
                    out.push(Command::Rotate(false));
                }
                if pressed(Key::CloseBracket) {
                    out.push(Command::Rotate(true));
                }
                if pressed(Key::M) {
                    out.push(Command::RenamePhotos);
                }
                if pressed(Key::I) {
                    out.push(Command::SetWorkspace(Workspace::Caption));
                }
            }
            if cmd {
                if !crate::keyonce::handler_takes(has_menu, false, true) {
                    return;
                }
                if !shift && pressed(Key::Y) {
                    out.push(Command::CopyPhotos);
                }
                if pm && !shift && pressed(Key::T) {
                    out.push(Command::SelectBy(crate::select_by::SelectBy::Tagged));
                }
                if pressed(Key::O) || pressed(if pm { Key::N } else { Key::T }) {
                    out.push(Command::PickFolder);
                }
                if pressed(Key::W) {
                    out.push(Command::CloseTab);
                }
                if alt && pressed(Key::C) {
                    out.push(Command::CopyMetadata);
                }
                if alt && pressed(Key::V) {
                    out.push(Command::PasteMetadata);
                }
                let rename = if pm {
                    pressed(Key::M)
                } else {
                    pressed(Key::R) && shift
                };
                if rename {
                    out.push(Command::RenamePhotos);
                }
                if pressed(Key::Backspace) {
                    out.push(Command::MoveToTrash);
                }
                if pressed(Key::Comma) {
                    out.push(Command::OpenSettings);
                }
                if pressed(Key::OpenBracket) {
                    out.push(Command::Rotate(false));
                }
                if pressed(Key::CloseBracket) {
                    out.push(Command::Rotate(true));
                }
                if pressed(Key::A) {
                    out.push(if shift && !pm {
                        Command::SelectNone
                    } else {
                        Command::SelectAll
                    });
                }
                if pm && pressed(Key::D) {
                    out.push(Command::SelectNone);
                }
                if pressed(Key::Z) {
                    out.push(if shift { Command::Redo } else { Command::Undo });
                }
                if pressed(Key::R) && !shift && !pm {
                    out.push(Command::RefreshFolder);
                }
                if pressed(Key::Equals) || pressed(Key::Plus) {
                    out.push(Command::ZoomIn);
                }
                if pressed(Key::Minus) {
                    out.push(Command::ZoomOut);
                }
                // ⌘ + number: the colour class in Photo Mechanic's layout,
                // otherwise the opposite of what the bare keys do.
                number_commands(i, !pm && !bare_rates, &mut out);
                return;
            }
            if pressed(Key::PageUp) {
                out.push(Command::MovePage(-1));
            }
            if pressed(Key::PageDown) {
                out.push(Command::MovePage(1));
            }
            if pressed(Key::Home) {
                out.push(Command::Home);
            }
            if pressed(Key::End) {
                out.push(Command::End);
            }
            if pressed(Key::Enter) {
                out.push(Command::Activate);
            }
            if pressed(Key::Space) {
                out.push(if in_preview {
                    Command::Move(1)
                } else {
                    Command::Activate
                });
            }
            if pressed(Key::Escape) {
                out.push(Command::Escape);
            }
            // ⌥C removes the crop (Photo Mechanic layout), in the crop tool.
            if pm && alt && !shift && !mods.ctrl && pressed(Key::C) {
                out.push(Command::CropClear);
            }
            // Bare keys. The menus show most of them, but macOS gives the
            // menu bar only ⌘/⌃ key equivalents, so this handler dispatches
            // them on every platform; `KeyOnce` drops any menu echo. With a
            // native menu, ⌃ and ⌥ combinations belong to the menu.
            if !has_menu || (!mods.ctrl && !alt) {
                number_commands(i, bare_rates, &mut out);
                if pressed(Key::T) {
                    out.push(Command::ToggleTag);
                }
                if pressed(Key::X) && !shift {
                    out.push(Command::SetRating(-1));
                }
                if pressed(Key::F) && shift {
                    out.push(Command::ToggleFilmstrip);
                }
                if pressed(Key::B) && !shift && in_preview {
                    out.push(Command::CyclePreviewBoost);
                }
                if pressed(Key::S) && !shift && self.prefs.stack_bursts {
                    out.push(Command::ToggleStack);
                }
                if in_preview && pressed(Key::L) && !shift {
                    out.push(Command::ToggleLoupe);
                }
                if pm && pressed(Key::F5) {
                    out.push(Command::RefreshFolder);
                }
                if in_preview && pressed(if pm { Key::C } else { Key::K }) && !shift {
                    out.push(Command::CropToggle);
                }
                if in_preview && self.pv.crop.is_some() {
                    if pressed(Key::R) && !shift {
                        out.push(Command::CropNextRatio);
                    }
                    if pressed(Key::G) && !shift {
                        out.push(Command::CropNextOverlay);
                    }
                    if pressed(Key::Backspace) || pressed(Key::Delete) {
                        out.push(Command::CropClear);
                    }
                }
                // G closes Preview (the crop tool keeps G for its guides).
                if pressed(Key::G)
                    && crate::platform::keymap::g_closes_preview(
                        in_preview,
                        self.pv.crop.is_some(),
                        shift,
                    )
                {
                    out.push(Command::ClosePreview);
                }
                if pressed(Key::Z) && !shift {
                    out.push(Command::ToggleZoom);
                }
                if pressed(if pm { Key::V } else { Key::C }) && !shift {
                    out.push(Command::ToggleCompare);
                }
            }
        });
        out
    }

    fn draw_drop_overlay(&self, ctx: &egui::Context, t: &Theme) {
        let a = ctx.animate_bool_with_time(
            egui::Id::new("drop_overlay_alpha"),
            self.drag_hover > 0,
            self.anim(0.12),
        );
        if a <= 0.0 {
            return;
        }
        let t = &Theme {
            accent: t.accent.gamma_multiply(a),
            text: t.text.gamma_multiply(a),
            ..*t
        };
        let r = ctx.content_rect();
        let p = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        p.rect_filled(
            r,
            egui::CornerRadius::ZERO,
            Color32::from_black_alpha((90.0 * a) as u8),
        );
        p.rect_stroke(
            r.shrink(S4),
            egui::CornerRadius::same(10),
            egui::Stroke::new(2.0, t.accent),
            egui::StrokeKind::Inside,
        );
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            "Drop a folder to open it as a contact sheet",
            egui::FontId::proportional(18.0),
            t.text,
        );
    }

    fn maybe_screenshot(&mut self, ctx: &egui::Context) {
        if self.quit_after_frame {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let Some(path) = self.launch.screenshot.clone() else {
            return;
        };
        if (self.launch.ingest_run || self.launch.export_run) && !self.run_done {
            // Hold the screenshot until the run has reported.
            ctx.request_repaint();
            return;
        }
        self.frames += 1;
        if self.frames == self.launch.screenshot_frame && !self.screenshot_requested {
            self.screenshot_requested = true;
            // Settings is drawn inline on the root window in screenshot mode
            // (ui::settings::show_inline), so the root capture covers it.
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        // Screenshot events for every viewport (the Settings window included)
        // arrive in the root input queue.
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, img.as_raw().to_vec())
            {
                match buf.save(&path) {
                    Ok(()) => log::info!("screenshot saved to {}", path.display()),
                    Err(e) => log::error!("screenshot failed: {e}"),
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    /// Bare Tab shows or hides the sidebar. It is taken out of the input
    /// before egui sees it, so egui doesn't also cycle focus on the main
    /// view; while a text field has focus or a dialog is open it is left
    /// alone, so Tab still moves between fields.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if !self.tab_is_free(ctx) {
            return;
        }
        let bare_tab = |e: &egui::Event| {
            matches!(e, egui::Event::Key { key: egui::Key::Tab, modifiers, .. } if modifiers.is_none())
        };
        if raw_input.events.iter().any(|e| {
            bare_tab(e) && matches!(e, egui::Event::Key { pressed: true, repeat: false, .. })
        }) {
            self.tab_toggle = true;
        }
        // Drop presses, repeats and releases alike so egui never sees them.
        raw_input.events.retain(|e| !bare_tab(e));
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt_ms = ctx.input(|i| i.stable_dt) * 1000.0;
        if dt_ms > 0.0 && dt_ms < 1000.0 {
            self.stats.frame_ms_ema = if self.stats.frame_ms_ema == 0.0 {
                dt_ms
            } else {
                self.stats.frame_ms_ema * 0.9 + dt_ms * 0.1
            };
            self.stats.frame_ms_max = self.stats.frame_ms_max.max(dt_ms);
        }
        self.poll_mark_writes();
        if self.mark_writer.pending() > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.applied_appearance != Some(self.prefs.appearance) {
            ctx.set_theme(match self.prefs.appearance {
                crate::prefs::Appearance::System => egui::ThemePreference::System,
                crate::prefs::Appearance::Dark => egui::ThemePreference::Dark,
                crate::prefs::Appearance::Light => egui::ThemePreference::Light,
            });
            self.applied_appearance = Some(self.prefs.appearance);
        }
        if self.applied_ui_scale != Some(self.prefs.ui_scale) {
            ctx.set_zoom_factor(self.prefs.ui_scale.clamp(0.8, 1.8));
            self.applied_ui_scale = Some(self.prefs.ui_scale);
        }
        let mut t = Theme::for_ctx(ctx);
        t = t.with_accent(self.prefs.accent);
        if self.prefs.high_contrast {
            t = t.high_contrast();
        }
        t.apply(ctx, self.reduce_motion());
        self.platform.on_first_frame();
        self.update_tick(ctx);
        self.update_install_tick();
        // Menu bar extra: what Apature is doing, in a few characters.
        let tray = if let Some(r) = self.ingest.running.as_ref().filter(|r| r.done.is_none()) {
            let total = self
                .ingest
                .plan
                .as_ref()
                .map(|p| p.entries.len())
                .unwrap_or(0)
                .max(1);
            format!("Ingest {}/{}", r.verified_count, total)
        } else if let Some(r) = self.deliver.running.as_ref().filter(|r| r.done.is_none()) {
            let sent = r.statuses.iter().filter(|s| **s == DStatus::Sent).count();
            format!("Sending {}/{}", sent, r.files.len())
        } else if let Some(r) = self.export.running.as_ref().filter(|r| !r.finished) {
            format!("Export {}/{}", r.ok + r.failed, r.statuses.len())
        } else if self.ingest.live.on {
            format!("Live · {}", self.ingest.live.ingested)
        } else {
            String::new()
        };
        self.platform.status_item(&tray, self.ingest.live.on);
        if self.status != self.last_status {
            self.last_status = self.status.clone();
            self.status_since = Some(Instant::now());
        }
        // Keep repainting while any timed animation is live.
        let live = |s: Option<Instant>, secs: f32| {
            s.is_some_and(|i| i.elapsed().as_secs_f32() < secs + 0.05)
        };
        let ingest_live =
            self.ingest.running.as_ref().is_some_and(|r| {
                r.started.elapsed().as_secs_f32() < 0.6 || live(r.done_since, 0.6)
            }) || self.export.running.as_ref().is_some_and(|r| {
                r.started.elapsed().as_secs_f32() < 0.6 || live(r.done_since, 0.6)
            });
        if !self.reduce_motion()
            && (live(self.sheet_fade_since, 0.16)
                || live(self.preview_since, 0.16)
                || live(self.focus_since, 0.16)
                || live(self.pv.zoom_since, 0.16)
                || live(self.status_since, 0.12)
                || live(self.ingest.plan_since, 0.16)
                || ingest_live)
        {
            ctx.request_repaint();
        }
        if self.volumes_checked.elapsed().as_secs_f32() >= 2.0 {
            self.volumes_checked = Instant::now();
            self.poll_tick(ctx);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
        if let Some(c) = self.dispatch_later.take() {
            self.dispatch(ctx, c);
        }

        self.drain_results(ctx);
        self.drain_ingest();
        self.drain_poll(ctx);
        self.drain_copy();
        self.drain_extract();
        self.drain_export();
        self.drain_deliver();
        self.handle_drops(ctx);
        if self.platform.set_shortcuts(self.prefs.shortcut_layout) {
            self.sync_menu();
            self.platform.set_class_names(&self.scheme.names);
        }
        let now = std::time::Instant::now();
        for c in self.platform.drain_menu() {
            // The bug report's fields take typing: a ⌘ key the menu bar
            // caught must not mark, select or move photos behind it. ⌘A
            // selects the field's text; only window items still work.
            if let Some(d) = self.bug_dialog.as_mut() {
                match c {
                    Command::SelectAll => d.select_all = true,
                    Command::CloseWindow
                    | Command::MinimizeWindow
                    | Command::ShowMainWindow => self.dispatch(ctx, c),
                    _ => {}
                }
                continue;
            }
            if self.key_once.admit(&c, crate::keyonce::Source::Menu, now) {
                self.dispatch(ctx, c);
            }
        }
        for c in self.keyboard_commands(ctx) {
            if self.key_once.admit(&c, crate::keyonce::Source::Keys, now) {
                self.dispatch(ctx, c);
            }
        }
        if std::mem::take(&mut self.tab_toggle) {
            self.toggle_side_panels();
        }

        if self.workspace == Workspace::Caption {
            self.sync_caption_draft();
        }
        ui::toolbar::show(self, ctx, &t);
        ui::tabs::show(self, ctx, &t);
        ui::infobar::show(self, ctx, &t);
        // Window title names the folder, like a document.
        let title = match &self.folder {
            Some(f) => format!(
                "{} — Apature",
                f.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None => "Apature".into(),
        };
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        if self.launch.settings && self.launch.screenshot.is_some() {
            ui::settings::show_inline(self, ctx, &t);
            self.maybe_screenshot(ctx);
            return;
        }
        ui::settings::show(self, ctx, &t);
        ui::dialogs::show(self, ctx, &t);
        ui::status::show(self, ctx, &t);
        ui::toasts::show(self, ctx, &t);
        ui::sidebar::show(self, ctx, &t);
        match self.workspace {
            Workspace::Caption => ui::caption::show_fields(self, ctx, &t),
            Workspace::Ingest | Workspace::Export => {}
            _ => ui::inspector::show(self, ctx, &t),
        }
        ui::filmstrip::show(self, ctx, &t);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.photo))
            .show(ctx, |ui| {
                if let Some(name) = self.folder_unplugged.clone() {
                    egui::Frame::NONE
                        .fill(t.raised)
                        .inner_margin(egui::Margin::symmetric(ui::tokens::S4 as i8, ui::tokens::S2 as i8))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{name} was unplugged. This folder lives on it: plug it back in to keep working, or close the tab. Marks you make now cannot be saved."
                                    ))
                                    .size(12.5)
                                    .color(t.reject),
                                );
                                if ui.small_button("Close tab").clicked() {
                                    self.dispatch_later = Some(Command::CloseTab);
                                }
                            });
                        });
                }
                if self.items.is_empty() {
                    ui.centered_and_justified(|ui| {
                        let msg = if self.folder.is_some() {
                            "No photos in this folder"
                        } else {
                            "Open a folder with ⌘O, or pick one in the sidebar"
                        };
                        ui.label(egui::RichText::new(msg).size(15.0).color(t.muted));
                    });
                    return;
                }
                if self.view.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("No photos match the filter · ⌥⌘F clears it")
                                .size(15.0)
                                .color(t.muted),
                        );
                    });
                    return;
                }
                match self.workspace {
                    Workspace::Sheet => ui::sheet::show(self, ui, &t),
                    Workspace::Preview => ui::preview::show(self, ui, &t),
                    Workspace::Caption => ui::caption::show_photo(self, ui, &t),
                    Workspace::Ingest => ui::ingest::show(self, ui, &t),
                    Workspace::Export => ui::export::show(self, ui, &t),
                }
            });
        self.show_ingest_window(ctx, &t);
        self.show_preview_window(ctx, &t);
        let r = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::background()).line_segment(
            [
                egui::pos2(r.left(), r.top() + ui::tokens::TOOLBAR_H),
                egui::pos2(r.right(), r.top() + ui::tokens::TOOLBAR_H),
            ],
            egui::Stroke::new(1.0, t.line),
        );
        ctx.layer_painter(egui::LayerId::background()).line_segment(
            [
                egui::pos2(r.left(), r.bottom() - ui::tokens::STATUS_H),
                egui::pos2(r.right(), r.bottom() - ui::tokens::STATUS_H),
            ],
            egui::Stroke::new(1.0, t.line),
        );

        self.draw_drop_overlay(ctx, &t);
        if self.prefs_dirty
            && !self.launch.sidebar_shot
            && self.prefs_saved_at.elapsed().as_secs_f32() > 1.0
        {
            self.prefs.save();
            self.prefs_dirty = false;
            self.prefs_saved_at = Instant::now();
        }
        self.maybe_screenshot(ctx);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // Also runs on quit: every queued mark reaches disk first.
        self.flush_marks();
        if self.launch.sidebar_shot {
            // Screenshot instance: never write prefs.
            return;
        }
        self.prefs.save();
    }
}

/// Two paths to the same folder: equal, or equal once symlinks and letter
/// case are resolved by the file system.
fn same_folder(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// One planned export filename for a photo: the template expansion
/// reduced to a single path component. Template fields come from metadata
/// (EXIF Model, …) and may contain separators, `..`, or be absolute, so
/// the expansion is sanitized before it is joined to the destination —
/// never the raw text. Falls back to the photo stem, then to
/// `export-<seq>`, so the name is never empty.
fn export_file_name(template: &str, facts: &apature_ingest::template::Facts, seq: u32) -> String {
    let expanded = if template.trim().is_empty() {
        format!("{}.jpg", facts.stem)
    } else {
        apature_ingest::template::expand(template, facts, seq)
    };
    let name = apature_ingest::template::sanitize_component(&expanded)
        .or_else(|| apature_ingest::template::sanitize_component(&facts.stem))
        .unwrap_or_else(|| format!("export-{seq}"));
    if name.to_lowercase().ends_with(".jpg") {
        name
    } else {
        format!("{name}.jpg")
    }
}

/// Suffix a planned name until it is unused in this run. The comparison
/// is case-insensitive: the destination may live on a case-insensitive
/// volume. Two photos expanding to the same name must not share one
/// output file — the export pool runs them in parallel.
fn dedup_export_name(name: &str, used: &mut HashSet<String>) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    };
    let mut candidate = name.to_string();
    let mut k = 1;
    while used.contains(&candidate.to_lowercase()) {
        candidate = format!("{stem}-{k}{ext}");
        k += 1;
    }
    used.insert(candidate.to_lowercase());
    candidate
}

/// Upload RGB; `gain_stops` brightens for viewing only (culling dark frames),
/// the file is untouched.
fn upload(ctx: &egui::Context, name: &str, rgb: &Rgb, gain_stops: u8) -> TextureHandle {
    let img = if gain_stops == 0 {
        ColorImage::from_rgb([rgb.width as usize, rgb.height as usize], &rgb.data)
    } else {
        let k = 1u16 << gain_stops;
        let boosted: Vec<u8> = rgb
            .data
            .iter()
            .map(|&v| (v as u16 * k).min(255) as u8)
            .collect();
        ColorImage::from_rgb([rgb.width as usize, rgb.height as usize], &boosted)
    };
    ctx.load_texture(name, img, TextureOptions::LINEAR)
}

#[cfg(test)]
mod prefetch_tests {
    use super::*;

    #[test]
    fn prefetch_goes_ahead_in_the_direction_of_travel() {
        assert_eq!(prefetch_plan(10, 100, 1), vec![11, 12, 9, 13]);
        assert_eq!(prefetch_plan(10, 100, -1), vec![9, 8, 11, 7]);
        // Clipped at the ends of the view.
        assert_eq!(prefetch_plan(0, 100, -1), vec![1]);
        assert_eq!(prefetch_plan(98, 100, 1), vec![99, 97]);
        assert!(prefetch_plan(0, 1, 1).is_empty());
    }

    #[test]
    fn eviction_drops_oldest_but_keeps_wanted() {
        let lru: VecDeque<usize> = (0..10).collect();
        // Count limit only: drop the two oldest not kept.
        let out = preview_evictions(&lru, |_| 1, &[0, 9], 8, usize::MAX);
        assert_eq!(out, vec![1, 2]);
        // Byte budget: 10 previews of 100 bytes, budget 350, keep 7..=9.
        let out = preview_evictions(&lru, |_| 100, &[7, 8, 9], 8, 350);
        assert_eq!(out, vec![0, 1, 2, 3, 4, 5, 6]);
        // Everything wanted: nothing goes, even over budget.
        let lru: VecDeque<usize> = (0..3).collect();
        assert!(preview_evictions(&lru, |_| 100, &[0, 1, 2], 1, 10).is_empty());
    }
}

#[cfg(test)]
mod crop_tests {
    use super::*;

    #[test]
    fn crop_mapping_round_trips_for_every_orientation() {
        let d = [0.1, 0.2, 0.7, 0.9];
        for o in [1, 3, 6, 8] {
            let s = crop_display_to_stored(d, o);
            let back = crop_stored_to_display(s, o);
            for k in 0..4 {
                assert!((back[k] - d[k]).abs() < 1e-6, "o={o} k={k}");
            }
            assert!(
                s[0] < s[2] && s[1] < s[3],
                "o={o} stored rect must stay ordered"
            );
        }
        // A frame turned right (6): the displayed top edge is the stored left edge.
        let s = crop_display_to_stored([0.0, 0.0, 1.0, 0.5], 6);
        assert!((s[0] - 0.0).abs() < 1e-6 && (s[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn same_folder_sees_through_symlinks() {
        let d = std::env::temp_dir().join(format!("apature-same-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Work Drive/2026")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(d.join("Work Drive"), d.join("alias")).unwrap();
        assert!(same_folder(&d.join("Work Drive"), &d.join("Work Drive/")));
        #[cfg(unix)]
        assert!(same_folder(&d.join("Work Drive"), &d.join("alias")));
        assert!(!same_folder(
            &d.join("Work Drive"),
            &d.join("Work Drive/2026")
        ));
        assert!(!same_folder(&d.join("gone"), &d.join("Work Drive")));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn latlon_parses_common_forms() {
        assert_eq!(parse_latlon("40.4461, -79.8955"), Some((40.4461, -79.8955)));
        assert_eq!(parse_latlon("40.4461 -79.8955"), Some((40.4461, -79.8955)));
        assert_eq!(parse_latlon("91, 0"), None);
        assert_eq!(parse_latlon("abc"), None);
    }
}

#[cfg(test)]
mod bulk_write_tests {
    use super::*;

    fn bulk(kind: BulkKind, total: usize) -> BulkWrite {
        BulkWrite::new((1..=total as u64).collect(), total, kind)
    }

    #[test]
    fn paste_finishes_with_file_counts() {
        let mut b = bulk(BulkKind::Paste, 3);
        assert_eq!(b.record(1, &Ok(1)), None);
        assert_eq!(b.record(2, &Ok(2)), None);
        assert_eq!(
            b.record(3, &Err("disk full".into())),
            Some("Pasted metadata onto 3 photos (3 files, 1 failed)".into())
        );
    }

    #[test]
    fn caption_counts_photos_not_files() {
        let mut b = bulk(BulkKind::Caption, 2);
        assert_eq!(b.record(1, &Ok(2)), None);
        assert_eq!(
            b.record(2, &Ok(1)),
            Some("Caption fields saved to 2 photos".into())
        );
        let mut b = bulk(BulkKind::Caption, 2);
        assert_eq!(b.record(1, &Ok(1)), None);
        assert_eq!(
            b.record(2, &Err("boom".into())),
            Some("Saved to 1 of 2 photos; last error: boom".into())
        );
    }

    #[test]
    fn rotate_keeps_its_direction() {
        let mut b = bulk(BulkKind::Rotate { dir: "right" }, 2);
        assert_eq!(b.record(1, &Ok(1)), None);
        assert_eq!(b.record(2, &Ok(1)), Some("Rotated 2 photos right".into()));
        let mut b = bulk(BulkKind::Rotate { dir: "left" }, 2);
        assert_eq!(b.record(1, &Ok(1)), None);
        assert_eq!(
            b.record(2, &Err("boom".into())),
            Some("Rotated 1 left; 1 could not be written (see log)".into())
        );
    }

    #[test]
    fn stationery_failure_reports_the_error() {
        let mut b = bulk(BulkKind::Stationery, 1);
        assert_eq!(
            b.record(1, &Err("boom".into())),
            Some("Pad failed: boom".into())
        );
    }

    #[test]
    fn unknown_seq_is_ignored() {
        let mut b = bulk(BulkKind::Paste, 1);
        assert_eq!(b.record(99, &Ok(1)), None);
        assert_eq!(b.ok, 0);
        assert_eq!(
            b.record(1, &Ok(1)),
            Some("Pasted metadata onto 1 photo (1 files)".into())
        );
    }

    #[test]
    fn read_failures_before_any_job_still_finish() {
        // Rotate where every sidecar read failed at submit time: no jobs,
        // the counts were set up front, and the text still reads right.
        let mut b = bulk(BulkKind::Rotate { dir: "right" }, 2);
        b.failed = 2;
        b.last_error = Some("no sidecar".into());
        assert_eq!(
            b.finish_text(),
            "Rotated 0 right; 2 could not be written (see log)"
        );
    }
}

#[cfg(test)]
mod export_names_tests {
    use super::*;
    use apature_core::item::Variant;
    use apature_core::{Kind, PhotoItem};
    use apature_ingest::template::Facts;

    fn facts_with(camera: Option<&str>, stem: &str) -> Facts {
        Facts {
            stem: stem.to_string(),
            ext: "jpg".into(),
            camera: camera.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn export_names_keep_metadata_inside_the_destination() {
        // Finding #5: a `{camera}` template with a hostile EXIF Model.
        let dest = Path::new("/tmp/chosen");
        let name = export_file_name("{camera}", &facts_with(Some("../outside"), "DSC_0001"), 1);
        assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "traversal must not survive sanitizing: {name}"
        );
        assert!(dest.join(&name).starts_with(dest));
        // An absolute model value must not discard the destination.
        let name = export_file_name("{camera}", &facts_with(Some("/etc/passwd"), "DSC_0001"), 1);
        assert!(dest.join(&name).starts_with(dest));
        // A model that sanitizes to nothing falls back to the stem.
        assert_eq!(
            export_file_name("{camera}", &facts_with(Some(".."), "DSC_0001"), 1),
            "DSC_0001.jpg"
        );
        // A missing model falls back too, and the name is never empty.
        let name = export_file_name("{camera}", &facts_with(None, "DSC_0001"), 1);
        assert!(!name.is_empty());
        assert!(dest.join(&name).starts_with(dest));
        // A real camera string with a slash becomes a flat filename.
        let name = export_file_name("{camera}", &facts_with(Some("Canon/EOS R5"), "DSC_0001"), 1);
        assert_eq!(name, "Canon-EOS R5.jpg");
    }

    #[test]
    fn duplicate_export_names_are_suffixed() {
        let mut used = HashSet::new();
        assert_eq!(dedup_export_name("same.jpg", &mut used), "same.jpg");
        assert_eq!(dedup_export_name("same.jpg", &mut used), "same-1.jpg");
        // Case-insensitive: SAME.jpg collides with same.jpg on APFS.
        assert_eq!(dedup_export_name("SAME.jpg", &mut used), "SAME-2.jpg");
        assert_eq!(dedup_export_name("other.jpg", &mut used), "other.jpg");
    }

    /// Synthetic JPEG carrying EXIF Model `../outside`, read through the
    /// real `facts_for` and named through the real helper: the planned
    /// output must stay inside the chosen destination.
    #[test]
    fn hostile_exif_model_stays_inside_via_facts_for() {
        let d = std::env::temp_dir().join(format!("apature-export-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let img = Rgb {
            width: 40,
            height: 20,
            data: vec![128; 40 * 20 * 3],
        };
        let mut bytes = apature_preview::decode::encode_jpeg(&img, 90).unwrap();
        let summary = apature_metadata::exif::ExifSummary {
            model: Some("../outside".into()),
            ..Default::default()
        };
        let block = apature_metadata::exif_write::minimal(&summary, 40, 20, None, None);
        let seg = apature_metadata::jpeg_xmp::segment(0xE1, "APP1", b"Exif\0\0", &block).unwrap();
        let mut with_exif = bytes[..2].to_vec();
        with_exif.extend_from_slice(&seg);
        with_exif.extend_from_slice(&bytes[2..]);
        bytes = with_exif;
        let src = d.join("DSC_0001.jpg");
        std::fs::write(&src, &bytes).unwrap();
        let item = PhotoItem {
            id: 0,
            stem: "DSC_0001".to_string(),
            primary: Variant {
                path: src.clone(),
                kind: Kind::Jpeg,
                size: 0,
                modified: None,
            },
            secondary: Vec::new(),
        };
        let facts = apature_ingest::plan::facts_for(&item, item.path(), None);
        assert_eq!(facts.camera.as_deref(), Some("../outside"));
        let dest = d.join("chosen");
        let name = export_file_name("{camera}", &facts, 1);
        assert!(
            dest.join(&name).starts_with(&dest),
            "planned {name} escapes {dest:?}"
        );
        assert!(!name.contains('/') && !name.contains(".."));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
