//! Extract JPEG Previews from RAW Photos… off the UI thread, with progress
//! and Cancel, like Copy Photos….

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_core::{ColorClassScheme, PhotoItem};
use apature_export::extract::{run_extract, ExtractOptions, ExtractOutcome, ExtractProgress};

pub enum ExtractMsg {
    Progress(ExtractProgress),
    Done(ExtractOutcome),
}

pub struct ExtractRunning {
    pub cancel: Arc<AtomicBool>,
    pub photos: usize,
    pub progress: Option<ExtractProgress>,
    pub done: Option<ExtractOutcome>,
}

impl ExtractRunning {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn fraction(&self) -> f32 {
        match &self.progress {
            Some(p) if p.total > 0 => p.done as f32 / p.total as f32,
            _ => 0.0,
        }
    }
}

pub struct ExtractJobs {
    tx: Sender<ExtractMsg>,
    rx: Receiver<ExtractMsg>,
    ctx: egui::Context,
}

impl ExtractJobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = unbounded();
        ExtractJobs { tx, rx, ctx }
    }

    pub fn try_recv(&self) -> Option<ExtractMsg> {
        self.rx.try_recv().ok()
    }

    pub fn start(
        &self,
        photos: Vec<PhotoItem>,
        dest: Option<PathBuf>,
        opts: ExtractOptions,
        scheme: ColorClassScheme,
    ) -> ExtractRunning {
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let n = photos.len();
        std::thread::Builder::new()
            .name("extract-previews".into())
            .spawn(move || {
                let out = run_extract(&photos, dest.as_deref(), &opts, &scheme, &c2, |p| {
                    let _ = tx.send(ExtractMsg::Progress(p.clone()));
                    ctx.request_repaint();
                });
                let _ = tx.send(ExtractMsg::Done(out));
                ctx.request_repaint();
            })
            .expect("spawn extract thread");
        ExtractRunning {
            cancel,
            photos: n,
            progress: None,
            done: None,
        }
    }
}
