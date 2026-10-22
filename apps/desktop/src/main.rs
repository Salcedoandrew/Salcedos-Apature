//! Apature desktop. `apature [folder] [--theme dark|light] [--screenshot out.png [--frames N]]`
//! `--ingest-run --ingest-source <card> --ingest-dest <dir>` ingests headlessly and reports;
//! `--export-run --export-dest <dir>` does the same for export.
//!
//! The screenshot flags exist so the shell can be checked visually in both
//! appearances without a bundled app or screen-recording permission.

mod app;
mod commands;
mod copy_job;
mod deliver_job;
mod export_job;
mod filter;
mod ingest_job;
mod keyonce;
mod platform;
mod prefs;
mod select_by;
mod selection;
mod ui;
mod undo;
mod updater;
mod worker;

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub folder: Option<PathBuf>,
    pub theme: Option<egui::Theme>,
    pub screenshot: Option<PathBuf>,
    /// With --preview: open the crop tool (layout checks).
    pub crop: bool,
    /// With --ingest: start Live Ingest on the source at launch (tests).
    pub live: bool,
    /// Open the About window at launch (screenshots).
    pub about: bool,
    /// Run the update check now and log the result (tests).
    pub check_updates: bool,
    /// Window size at launch (for layout checks at small sizes).
    pub size: Option<(f32, f32)>,
    pub screenshot_frame: u64,
    /// Hidden: select everything and open the filter popup, for screenshots.
    pub demo: bool,
    /// Start in the Caption workspace (screenshots).
    pub caption: bool,
    /// Load this assignment file at start.
    pub assignment: Option<PathBuf>,
    /// Start in the Ingest workspace (screenshots).
    pub ingest: bool,
    pub ingest_source: Option<PathBuf>,
    pub ingest_dest: Option<PathBuf>,
    /// Start the ingest as soon as the plan is ready, print the report, then
    /// quit (or screenshot first when `--screenshot` is given). End-to-end
    /// check of the real app path without a mouse.
    pub ingest_run: bool,
    /// Start in the Export workspace, all photos in view, optional destination.
    pub export: bool,
    pub export_dest: Option<PathBuf>,
    /// Export at once, print the report, then quit (or screenshot first).
    pub export_run: bool,
    /// Start in Preview (screenshots); `--zoom11` at 1:1, `--compare` the first three.
    pub preview: bool,
    pub zoom11: bool,
    pub compare: bool,
    /// Open the Rename / Trash dialog at start (screenshots).
    pub rename_dialog: bool,
    pub trash_dialog: bool,
    /// Open the Settings window at start (screenshots).
    pub settings: bool,
    /// Extra folders to open as tabs.
    pub more_folders: Vec<PathBuf>,
}

fn parse_args() -> Launch {
    let mut l = Launch {
        screenshot_frame: 120,
        ..Default::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--theme" => {
                l.theme = match args.next().as_deref() {
                    Some("light") => Some(egui::Theme::Light),
                    Some("dark") => Some(egui::Theme::Dark),
                    _ => None,
                }
            }
            "--screenshot" => l.screenshot = args.next().map(PathBuf::from),
            "--size" => {
                l.size = args.next().and_then(|v| {
                    let (w, h) = v.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
            }
            "--demo" => l.demo = true,
            "--caption" => l.caption = true,
            "--ingest" => l.ingest = true,
            "--crop" => l.crop = true,
            "--live" => l.live = true,
            "--about" => l.about = true,
            "--check-updates" => l.check_updates = true,
            "--ingest-source" => l.ingest_source = args.next().map(PathBuf::from),
            "--ingest-dest" => l.ingest_dest = args.next().map(PathBuf::from),
            "--export" => l.export = true,
            "--export-dest" => l.export_dest = args.next().map(PathBuf::from),
            "--export-run" => {
                l.export = true;
                l.export_run = true;
            }
            "--ingest-run" => {
                l.ingest = true;
                l.ingest_run = true;
            }
            "--settings" => l.settings = true,
            "--rename-dialog" => l.rename_dialog = true,
            "--trash-dialog" => l.trash_dialog = true,
            "--preview" => l.preview = true,
            "--zoom11" => {
                l.preview = true;
                l.zoom11 = true;
            }
            "--compare" => {
                l.preview = true;
                l.compare = true;
            }
            "--assignment" => l.assignment = args.next().map(PathBuf::from),
            "--frames" => {
                l.screenshot_frame = args.next().and_then(|v| v.parse().ok()).unwrap_or(120)
            }
            _ => {
                if l.folder.is_none() {
                    l.folder = Some(PathBuf::from(a));
                } else {
                    l.more_folders.push(PathBuf::from(a));
                }
            }
        }
    }
    l
}

/// The app icon, decoded from the bundled PNG (see assets/icon.svg for the source).
fn icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon-256.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn main() -> eframe::Result {
    init_logging();
    let launch = parse_args();
    let (w, h) = launch.size.unwrap_or((1400.0, 900.0));
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
        .with_min_inner_size([760.0, 480.0])
        .with_title("Apature")
        .with_app_id("dev.mccal.apature");
    if let Some(i) = icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(i));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "dev.mccal.apature",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, launch)))),
    )
}

/// Log to stderr and to `apature.log` in Application Support (info and up;
/// `RUST_LOG` raises it). The file rotates once past 5 MB to `apature.log.1`,
/// so a crash mid-ingest leaves a record of what verified.
fn init_logging() {
    use std::io::Write;
    let file = directories::ProjectDirs::from("dev", "mccal", "Apature").map(|d| {
        let dir = d.data_dir().to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("apature.log");
        if std::fs::metadata(&path)
            .map(|m| m.len() > 5_000_000)
            .unwrap_or(false)
        {
            let _ = std::fs::rename(&path, dir.join("apature.log.1"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    });
    struct Tee(Option<std::fs::File>);
    impl Write for Tee {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            let _ = std::io::stderr().write_all(b);
            if let Some(f) = self.0.as_mut() {
                let _ = f.write_all(b);
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if let Some(f) = self.0.as_mut() {
                f.flush()?;
            }
            Ok(())
        }
    }
    let tee = Tee(file.and_then(|r| r.ok()));
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .target(env_logger::Target::Pipe(Box::new(tee)))
        .init();
    log::info!(
        "Apature {} build {} starting",
        env!("CARGO_PKG_VERSION"),
        option_env!("APATURE_BUILD").unwrap_or("dev")
    );
}
