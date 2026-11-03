//! Delivery runs on a worker thread; the UI polls `Msg`s each frame.

use std::path::PathBuf;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_deliver::{send, Event, Report, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    Sending,
    Sent,
    Failed,
}

pub enum Msg {
    Event(Event),
    Done(Result<Report, String>),
}

pub struct Running {
    pub target: String,
    pub files: Vec<PathBuf>,
    pub statuses: Vec<Status>,
    pub errors: Vec<Option<String>>,
    pub bytes: u64,
    pub started: Instant,
    pub done: Option<Result<Report, String>>,
}

pub struct DeliverJobs {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
}

impl DeliverJobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = unbounded();
        DeliverJobs { tx, rx, ctx }
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    pub fn start(&self, target: Target, secret: String, files: Vec<PathBuf>) -> Running {
        let n = files.len();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let t2 = target.clone();
        let f2 = files.clone();
        let started = std::thread::Builder::new()
            .name("deliver".into())
            .spawn(move || {
                let result = send(&t2, &secret, &f2, |e| {
                    let _ = tx.send(Msg::Event(e));
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string());
                let _ = tx.send(Msg::Done(result));
                ctx.request_repaint();
            });
        if let Err(e) = started {
            log::error!("could not start the deliver thread: {e}");
        }
        Running {
            target: target.name,
            files,
            statuses: vec![Status::Waiting; n],
            errors: vec![None; n],
            bytes: 0,
            started: Instant::now(),
            done: None,
        }
    }
}
