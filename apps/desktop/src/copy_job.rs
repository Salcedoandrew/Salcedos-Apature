//! Copy Photos… off the UI thread: the copy runs on a worker thread and
//! reports progress over a channel; the dialog (or the status line, once
//! the dialog is hidden) shows it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// When the copy started, and when its outcome arrived.
    pub started: Instant,
    pub finished: Option<Instant>,
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
    /// Time from start to finish (or to now while still copying).
    pub fn elapsed(&self) -> Duration {
        self.finished.unwrap_or_else(Instant::now).saturating_duration_since(self.started)
    }
    /// Bytes copied so far, or in total once done.
    pub fn bytes_copied(&self) -> u64 {
        match (&self.done, &self.progress) {
            (Some(o), _) => o.bytes,
            (None, Some(p)) => p.bytes_done,
            _ => 0,
        }
    }
}

/// apature.log line for a copy's start.
pub fn start_log_line(dest: &std::path::Path, files: usize, bytes: u64) -> String {
    format!("copy photos: start → {}: {files} files, {bytes} bytes", dest.display())
}

/// "done", "cancelled" or "failed" (any file failed) for a finished copy.
pub fn outcome_status(o: &CopyOutcome) -> &'static str {
    if o.cancelled {
        "cancelled"
    } else if !o.failed.is_empty() {
        "failed"
    } else {
        "done"
    }
}

/// apature.log line for a copy's end.
pub fn end_log_line(status: &str, files: usize, failed: usize, bytes: u64, elapsed: Duration) -> String {
    let failed = if failed > 0 { format!(", {failed} failed") } else { String::new() };
    format!(
        "copy photos: {status}: {files} files copied{failed}, {bytes} bytes in {}, {}",
        format_elapsed(elapsed),
        format_speed(mb_per_sec(bytes, elapsed))
    )
}

/// Log a finished copy: one end line, plus one line per file that failed.
pub fn log_outcome(o: &CopyOutcome, elapsed: Duration) {
    for (path, why) in &o.failed {
        log::error!("copy photos: failed {}: {why}", path.display());
    }
    let line = end_log_line(outcome_status(o), o.copied.len(), o.failed.len(), o.bytes, elapsed);
    if o.failed.is_empty() {
        log::info!("{line}");
    } else {
        log::warn!("{line}");
    }
}

/// "5 sec", "1 min 12 sec", "1 hr 3 min".
pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{} hr {} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min {} sec", s / 60, s % 60)
    } else {
        format!("{s} sec")
    }
}

/// Average speed in MB/sec (1 MB = 1,000,000 bytes); None when no time has passed.
pub fn mb_per_sec(bytes: u64, d: Duration) -> Option<f64> {
    let secs = d.as_secs_f64();
    (secs > 0.0).then(|| bytes as f64 / 1e6 / secs)
}

/// "268 MB/sec"; "—" when the speed is unknown.
pub fn format_speed(mbps: Option<f64>) -> String {
    match mbps {
        Some(v) if v.is_finite() && v >= 10.0 => format!("{v:.0} MB/sec"),
        Some(v) if v.is_finite() => format!("{v:.1} MB/sec"),
        _ => "—".into(),
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
        log::info!("{}", start_log_line(&dest, plan.entries.len(), plan.total_bytes));
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
            started: Instant::now(),
            finished: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_formats() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "0 sec");
        assert_eq!(format_elapsed(Duration::from_millis(5_400)), "5 sec");
        assert_eq!(format_elapsed(Duration::from_secs(72)), "1 min 12 sec");
        assert_eq!(format_elapsed(Duration::from_secs(3_780)), "1 hr 3 min");
    }

    #[test]
    fn speed_guards_zero_and_formats() {
        assert_eq!(mb_per_sec(1_000, Duration::ZERO), None);
        assert_eq!(format_speed(mb_per_sec(1_000, Duration::ZERO)), "—");
        let v = mb_per_sec(10_300_000_000, Duration::from_millis(38_433)).unwrap();
        assert_eq!(format_speed(Some(v)), "268 MB/sec");
        assert_eq!(format_speed(mb_per_sec(2_500_000, Duration::from_secs(1))), "2.5 MB/sec");
        assert_eq!(format_speed(mb_per_sec(0, Duration::from_secs(3))), "0.0 MB/sec");
    }

    #[test]
    fn log_lines() {
        assert_eq!(
            start_log_line(std::path::Path::new("/Volumes/CARD/Job"), 546, 10_300_000_000),
            "copy photos: start → /Volumes/CARD/Job: 546 files, 10300000000 bytes"
        );
        assert_eq!(
            end_log_line("done", 546, 0, 10_300_000_000, Duration::from_millis(38_433)),
            "copy photos: done: 546 files copied, 10300000000 bytes in 38 sec, 268 MB/sec"
        );
        assert_eq!(
            end_log_line("failed", 543, 3, 0, Duration::ZERO),
            "copy photos: failed: 543 files copied, 3 failed, 0 bytes in 0 sec, —"
        );
        let mut o = CopyOutcome::default();
        assert_eq!(outcome_status(&o), "done");
        o.failed.push(("a.cr3".into(), "disk full".into()));
        assert_eq!(outcome_status(&o), "failed");
        o.cancelled = true;
        assert_eq!(outcome_status(&o), "cancelled");
    }
}
