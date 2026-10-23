//! Copy Photos… off the UI thread: the copy runs on a worker thread and
//! reports progress over a channel; the dialog (or the status line, once
//! the dialog is hidden) shows it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_ingest::{run_copy, CopyOutcome, CopyPlan, CopyProgress};
use apature_metadata::Iptc;

pub enum CopyMsg {
    Progress(CopyProgress),
    Done(CopyOutcome),
}

pub struct CopyRunning {
    pub cancel: Arc<AtomicBool>,
    pub dest: PathBuf,
    pub photos: usize,
    pub progress: Option<CopyProgress>,
    pub done: Option<CopyOutcome>,
}

impl CopyRunning {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    /// 0.0–1.0 by bytes (files when the sizes are unknown).
    pub fn fraction(&self) -> f32 {
        match &self.progress {
            Some(p) if p.bytes_total > 0 => p.bytes_done as f32 / p.bytes_total as f32,
            Some(p) if p.files_total > 0 => p.files_done as f32 / p.files_total as f32,
            _ => 0.0,
        }
    }
}

pub struct CopyJobs {
    tx: Sender<CopyMsg>,
    rx: Receiver<CopyMsg>,
    ctx: egui::Context,
}

impl CopyJobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = unbounded();
        CopyJobs { tx, rx, ctx }
    }

    pub fn try_recv(&self) -> Option<CopyMsg> {
        self.rx.try_recv().ok()
    }

    pub fn start(&self, plan: CopyPlan, pad: Option<Iptc>, dest: PathBuf) -> CopyRunning {
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let photos = plan.photos;
        std::thread::Builder::new()
            .name("copy-photos".into())
            .spawn(move || {
                let out = run_copy(&plan, &c2, pad.as_ref(), |p| {
                    let _ = tx.send(CopyMsg::Progress(p.clone()));
                    ctx.request_repaint();
                });
                let _ = tx.send(CopyMsg::Done(out));
                ctx.request_repaint();
            })
            .expect("spawn copy thread");
        CopyRunning {
            cancel,
            dest,
            photos,
            progress: None,
            done: None,
        }
    }
}
