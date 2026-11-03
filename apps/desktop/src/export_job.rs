//! Export off the UI thread: a small pool works through the jobs and
//! reports each file as it lands.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_core::PhotoItem;
use apature_export::{export, Outcome, Settings};
use apature_metadata::Iptc;

pub struct Job {
    pub item: PhotoItem,
    pub out: PathBuf,
    /// Assignment pad; the file's own fields win over it.
    pub pad: Option<Iptc>,
}

pub enum Msg {
    Started {
        index: usize,
    },
    Done {
        index: usize,
        result: Result<Outcome, String>,
    },
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Working,
    Done,
    Failed,
}

pub struct Running {
    pub cancel: Arc<AtomicBool>,
    pub statuses: Vec<Status>,
    pub errors: Vec<Option<String>>,
    pub outs: Vec<Option<PathBuf>>,
    pub started: Instant,
    pub done_since: Option<Instant>,
    pub finished: bool,
    pub ok: usize,
    pub failed: usize,
    pub bytes: u64,
    pub dest: PathBuf,
}

pub struct ExportJobs {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
}

impl ExportJobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = unbounded();
        ExportJobs { tx, rx, ctx }
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    pub fn start(&self, jobs: Vec<Job>, settings: Settings, dest: PathBuf) -> Running {
        let n = jobs.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let jobs = Arc::new(jobs);
        let next = Arc::new(AtomicUsize::new(0));
        let settings = Arc::new(settings);
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 4)
            .min(n.max(1));
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let c2 = cancel.clone();
        let started = std::thread::Builder::new()
            .name("export-run".into())
            .spawn(move || {
                let mut handles = Vec::new();
                for _ in 0..threads {
                    let jobs = jobs.clone();
                    let next = next.clone();
                    let settings = settings.clone();
                    let tx = tx.clone();
                    let ctx = ctx.clone();
                    let cancel = c2.clone();
                    let worker = std::thread::Builder::new()
                        .name("export-worker".into())
                        .spawn(move || loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= jobs.len() || cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            let job = &jobs[i];
                            let _ = tx.send(Msg::Started { index: i });
                            ctx.request_repaint();
                            let file = apature_metadata::items::read_iptc(&job.item);
                            let iptc = match &job.pad {
                                Some(p) => p.overlay(&file),
                                None => file,
                            };
                            let result = export(&job.item, &iptc, &settings, &job.out)
                                .map_err(|e| e.to_string());
                            let _ = tx.send(Msg::Done { index: i, result });
                            ctx.request_repaint();
                        });
                    match worker {
                        Ok(h) => handles.push(h),
                        Err(e) => log::error!("could not start an export worker: {e}"),
                    }
                }
                for h in handles {
                    let _ = h.join();
                }
                let _ = tx.send(Msg::Finished);
                ctx.request_repaint();
            });
        if let Err(e) = started {
            log::error!("could not start the export thread: {e}");
        }
        Running {
            cancel,
            statuses: vec![Status::Pending; n],
            errors: vec![None; n],
            outs: vec![None; n],
            started: Instant::now(),
            done_since: None,
            finished: false,
            ok: 0,
            failed: 0,
            bytes: 0,
            dest,
        }
    }
}
