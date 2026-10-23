//! Writes marks (rating, colour class, tag) to disk on a background thread,
//! so a slow drive or a large JPEG never stalls the window. Jobs run one at
//! a time in the order given; when several pile up for the same photo only
//! the newest is written. Anything else that reads or writes a sidecar
//! first calls [`MarkWriter::wait_idle`] so it never races a pending write.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use apature_core::{ColorClassScheme, Marks, PhotoItem};
use apature_metadata::items::{self, JpegMarks};

pub struct Job {
    pub seq: u64,
    pub item: PhotoItem,
    pub marks: Marks,
    pub scheme: ColorClassScheme,
    pub policy: JpegMarks,
}

pub struct Done {
    pub seq: u64,
    /// The photo's primary path, to find it again (the tab may have changed).
    pub path: PathBuf,
    /// Files written, or what went wrong.
    pub result: Result<usize, String>,
}

pub struct MarkWriter {
    tx: Option<Sender<Job>>,
    rx: Receiver<Done>,
    pending: Arc<(Mutex<usize>, Condvar)>,
    handle: Option<JoinHandle<()>>,
    next_seq: u64,
}

impl Default for MarkWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkWriter {
    pub fn new() -> Self {
        let (tx, jobs) = mpsc::channel::<Job>();
        let (done_tx, rx) = mpsc::channel::<Done>();
        let pending = Arc::new((Mutex::new(0usize), Condvar::new()));
        let p = pending.clone();
        let handle = std::thread::Builder::new()
            .name("apature-marks".into())
            .spawn(move || {
                while let Ok(first) = jobs.recv() {
                    let mut batch = vec![first];
                    batch.extend(jobs.try_iter());
                    let n = batch.len();
                    for job in latest_per_photo(batch) {
                        let result = items::write(&job.item, &job.marks, &job.scheme, job.policy)
                            .map(|w| w.len())
                            .map_err(|e| e.to_string());
                        let _ = done_tx.send(Done {
                            seq: job.seq,
                            path: job.item.path().to_path_buf(),
                            result,
                        });
                    }
                    let (m, cv) = &*p;
                    let mut left = m.lock().unwrap_or_else(|e| e.into_inner());
                    *left = left.saturating_sub(n);
                    cv.notify_all();
                }
            })
            .expect("spawn mark writer");
        MarkWriter {
            tx: Some(tx),
            rx,
            pending,
            handle: Some(handle),
            next_seq: 0,
        }
    }

    /// Queue a write; returns its sequence number. Never blocks.
    pub fn submit(
        &mut self,
        item: PhotoItem,
        marks: Marks,
        scheme: ColorClassScheme,
        policy: JpegMarks,
    ) -> u64 {
        self.next_seq += 1;
        let seq = self.next_seq;
        {
            let (m, _) = &*self.pending;
            *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
        let job = Job {
            seq,
            item,
            marks,
            scheme,
            policy,
        };
        if let Some(tx) = &self.tx {
            if let Err(mpsc::SendError(job)) = tx.send(job) {
                // The thread is gone (it panicked): write here rather than lose the mark.
                let (m, _) = &*self.pending;
                let mut left = m.lock().unwrap_or_else(|e| e.into_inner());
                *left = left.saturating_sub(1);
                log::error!(
                    "mark writer thread gone; writing {} inline",
                    job.item.display_name()
                );
                let _ = items::write(&job.item, &job.marks, &job.scheme, job.policy);
            }
        }
        seq
    }

    /// Results finished since the last call.
    pub fn poll(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }

    /// Writes queued or running.
    pub fn pending(&self) -> usize {
        *self.pending.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Block until every queued write is on disk (or `timeout` passes).
    /// Returns whether the queue drained.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let (m, cv) = &*self.pending;
        let left = m.lock().unwrap_or_else(|e| e.into_inner());
        let (left, _) = cv
            .wait_timeout_while(left, timeout, |n| *n > 0)
            .unwrap_or_else(|e| e.into_inner());
        *left == 0
    }
}

impl Drop for MarkWriter {
    /// Finish every queued write before the app goes away.
    fn drop(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Keep only the newest job for each photo, in the order those were queued.
fn latest_per_photo(batch: Vec<Job>) -> Vec<Job> {
    let mut seen = HashSet::new();
    let mut out: Vec<Job> = batch
        .into_iter()
        .rev()
        .filter(|j| seen.insert(j.item.path().to_path_buf()))
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::Rating;

    fn folder(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("apature-markw-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn items_in(d: &std::path::Path, names: &[&str]) -> Vec<PhotoItem> {
        for n in names {
            std::fs::write(d.join(n), b"not really a raw").unwrap();
        }
        apature_core::folder::scan(d, &Default::default()).unwrap()
    }

    fn stars(m: u8) -> Marks {
        Marks {
            rating: Rating(m as i8),
            ..Default::default()
        }
    }

    fn read_rating(it: &PhotoItem) -> i8 {
        items::read(it)
            .effective(it, &ColorClassScheme::default())
            .rating
            .0
    }

    #[test]
    fn writes_land_off_thread_and_wait_idle_flushes() {
        let d = folder("flush");
        let its = items_in(&d, &["A.RAF", "B.RAF"]);
        let mut w = MarkWriter::new();
        let s = ColorClassScheme::default();
        for it in &its {
            w.submit(it.clone(), stars(5), s.clone(), JpegMarks::Sidecar);
        }
        assert!(w.wait_idle(Duration::from_secs(10)));
        assert_eq!(w.pending(), 0);
        for it in &its {
            assert_eq!(read_rating(it), 5, "{}", it.display_name());
        }
        let done = w.poll();
        assert!(!done.is_empty());
        assert!(done.iter().all(|d| d.result == Ok(1)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_newest_mark_wins_when_writes_pile_up() {
        let d = folder("order");
        let it = items_in(&d, &["C.RAF"]).remove(0);
        let mut w = MarkWriter::new();
        let s = ColorClassScheme::default();
        for r in [1, 2, 3, 4, 5, 0, 5] {
            w.submit(it.clone(), stars(r), s.clone(), JpegMarks::Sidecar);
        }
        assert!(w.wait_idle(Duration::from_secs(10)));
        assert_eq!(read_rating(&it), 5);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dropping_the_writer_finishes_queued_writes() {
        let d = folder("drop");
        let it = items_in(&d, &["D.RAF"]).remove(0);
        {
            let mut w = MarkWriter::new();
            w.submit(
                it.clone(),
                stars(3),
                ColorClassScheme::default(),
                JpegMarks::Sidecar,
            );
        }
        assert_eq!(read_rating(&it), 3);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn coalescing_keeps_one_job_per_photo_in_queue_order() {
        let d = folder("coalesce");
        let its = items_in(&d, &["E.RAF", "F.RAF"]);
        let job = |seq, i: usize| Job {
            seq,
            item: its[i].clone(),
            marks: stars(1),
            scheme: ColorClassScheme::default(),
            policy: JpegMarks::Sidecar,
        };
        let kept: Vec<u64> = latest_per_photo(vec![job(1, 0), job(2, 1), job(3, 0)])
            .iter()
            .map(|j| j.seq)
            .collect();
        assert_eq!(kept, vec![2, 3]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
