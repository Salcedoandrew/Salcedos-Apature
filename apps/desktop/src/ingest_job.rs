//! Ingest off the UI thread: plan building and the copy run both happen on
//! worker threads and report back over a channel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_ingest::{build_plan, run, Event, Ledger, Options, Plan, Report};
use apature_metadata::Iptc;

pub enum Msg {
    PlanReady {
        generation: u64,
        result: Result<Plan, String>,
    },
    Event(Event),
    Report(Report),
    PadApplied {
        files: usize,
        failed: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Pending,
    Copying,
    Verified,
    Failed,
}

pub struct Running {
    pub cancel: Arc<AtomicBool>,
    pub statuses: Vec<FileStatus>,
    pub errors: Vec<Option<String>>,
    pub verified_count: usize,
    /// Bytes verified so far (one count per destination), drives the bar.
    pub bytes_done: u64,
    pub started: Instant,
    pub done_since: Option<Instant>,
    pub done: Option<Report>,
    pub pad: Option<(usize, usize)>,
}

pub struct IngestJobs {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    ledger_path: PathBuf,
}

impl IngestJobs {
    pub fn new(ctx: egui::Context, ledger_path: PathBuf) -> Self {
        let (tx, rx) = unbounded();
        IngestJobs {
            tx,
            rx,
            ctx,
            ledger_path,
        }
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    pub fn plan(&self, opts: Options, generation: u64) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let ledger_path = self.ledger_path.clone();
        std::thread::Builder::new()
            .name("ingest-plan".into())
            .spawn(move || {
                let ledger = Ledger::load(&ledger_path);
                let result = build_plan(&opts, &ledger).map_err(|e| e.to_string());
                let _ = tx.send(Msg::PlanReady { generation, result });
                ctx.request_repaint();
            })
            .expect("spawn plan thread");
    }

    /// Run the plan; afterwards optionally apply `pad` to every verified
    /// primary copy (so the ingested folder opens already stationeried).
    pub fn start(
        &self,
        plan: Plan,
        pad: Option<Iptc>,
        jpeg: apature_metadata::items::JpegMarks,
    ) -> Running {
        let cancel = Arc::new(AtomicBool::new(false));
        let n = plan.entries.len();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let ledger_path = self.ledger_path.clone();
        let c2 = cancel.clone();
        std::thread::Builder::new()
            .name("ingest-run".into())
            .spawn(move || {
                let mut ledger = Ledger::load(&ledger_path);
                let report = run(&plan, &mut ledger, &c2, |e| {
                    let _ = tx.send(Msg::Event(e));
                    ctx.request_repaint();
                });
                let all_ok = report.all_verified(&plan);
                let _ = tx.send(Msg::Report(report));
                if let (Some(pad), true) = (pad, all_ok) {
                    let mut files = 0;
                    let mut failed = 0;
                    // One item per primary folder; scan each folder once.
                    let mut dirs: Vec<PathBuf> = plan
                        .entries
                        .iter()
                        .filter_map(|e| e.primary.parent().map(|p| p.to_path_buf()))
                        .collect();
                    dirs.sort();
                    dirs.dedup();
                    for d in dirs {
                        if let Ok(items) =
                            apature_core::scan(&d, &apature_core::ScanOptions::default())
                        {
                            for it in &items {
                                if !c2.load(Ordering::Relaxed) {
                                    match apature_metadata::items::write_iptc(it, &pad, jpeg) {
                                        Ok(w) => files += w.len(),
                                        Err(_) => failed += 1,
                                    }
                                }
                            }
                        }
                    }
                    let _ = tx.send(Msg::PadApplied { files, failed });
                }
                ctx.request_repaint();
            })
            .expect("spawn run thread");
        Running {
            cancel,
            statuses: vec![FileStatus::Pending; n],
            errors: vec![None; n],
            verified_count: 0,
            bytes_done: 0,
            started: Instant::now(),
            done_since: None,
            done: None,
            pad: None,
        }
    }
}
