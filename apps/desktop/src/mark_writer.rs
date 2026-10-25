//! Writes marks (rating, colour class, tag), IPTC fields (captions, pads,
//! pastes) and sidecar edits (crops, turns) to disk on a background thread,
//! so a slow drive or a large JPEG never stalls the window. Jobs run one at
//! a time in the order given, so two writes to the same sidecar can never
//! interleave and lose each other's fields. Anything that reads a sidecar
//! from disk first calls [`MarkWriter::wait_idle`] so it never reads stale
//! data. When several jobs pile up for the same photo only the newest is
//! written, except edits, where a crop and a turn touch different fields
//! and both must land.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use apature_core::{ColorClassScheme, Marks, PhotoItem};
use apature_metadata::edits;
use apature_metadata::items::{self, JpegMarks};
use apature_metadata::Iptc;

pub struct Job {
    pub seq: u64,
    pub item: PhotoItem,
    pub work: Work,
}

/// One sidecar write. All three kinds are read-modify-write on the sidecar
/// (or the JPEG), so running them one at a time in queue order is what
/// keeps a rating from clobbering a caption.
pub enum Work {
    /// Rating, colour class, tag.
    Marks {
        marks: Marks,
        /// Boxed: the scheme is 216 bytes (eight names), and the other
        /// variants are a few bytes; an unboxed scheme trips
        /// clippy::large_enum_variant and bloats every queued job.
        scheme: Box<ColorClassScheme>,
        policy: JpegMarks,
    },
    /// Caption fields, stationery pad, pasted metadata.
    /// Boxed: an Iptc is ~500 bytes of optional fields, and the other
    /// variants are a few bytes; unboxed it trips
    /// clippy::large_enum_variant and bloats every queued job.
    Iptc { iptc: Box<Iptc>, policy: JpegMarks },
    /// Crop or turn; read-modify-write on the photo's sidecar.
    Edits { change: edits::Change },
}

/// The kind of write, for coalescing and for the failure label in [`Done`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Marks,
    Iptc,
    Edits,
}

impl Work {
    fn kind(&self) -> Kind {
        match self {
            Work::Marks { .. } => Kind::Marks,
            Work::Iptc { .. } => Kind::Iptc,
            Work::Edits { .. } => Kind::Edits,
        }
    }

    /// Short label for the failure message when this write fails.
    fn label(&self) -> &'static str {
        match self {
            Work::Marks { .. } => "marks",
            Work::Iptc { .. } => "metadata",
            Work::Edits { .. } => "edits",
        }
    }
}

pub struct Done {
    pub seq: u64,
    /// The photo's primary path, to find it again (the tab may have changed).
    pub path: PathBuf,
    /// What kind of write this was, for the failure message.
    pub label: &'static str,
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

/// Do one queued write. Runs on the writer thread (and inline, as a
/// fallback, if that thread is gone).
fn run(job: &Job) -> Result<usize, String> {
    match &job.work {
        Work::Marks {
            marks,
            scheme,
            policy,
        } => items::write(&job.item, marks, scheme, *policy)
            .map(|w| w.len())
            .map_err(|e| e.to_string()),
        Work::Iptc { iptc, policy } => items::write_iptc(&job.item, iptc, *policy)
            .map(|w| w.len())
            .map_err(|e| e.to_string()),
        Work::Edits { change } => edits::write(&job.item.sidecar_path(), change)
            .map(|_| 1)
            .map_err(|e| e.to_string()),
    }
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
                        let result = run(&job);
                        let _ = done_tx.send(Done {
                            seq: job.seq,
                            path: job.item.path().to_path_buf(),
                            label: job.work.label(),
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

    /// Queue a marks write; returns its sequence number. Never blocks.
    pub fn submit(
        &mut self,
        item: PhotoItem,
        marks: Marks,
        scheme: ColorClassScheme,
        policy: JpegMarks,
    ) -> u64 {
        let seq = self.next_seq();
        self.enqueue(Job {
            seq,
            item,
            work: Work::Marks {
                marks,
                scheme: Box::new(scheme),
                policy,
            },
        })
    }

    /// Queue an IPTC write (caption, pad, paste); returns its sequence
    /// number. Never blocks.
    pub fn submit_iptc(&mut self, item: PhotoItem, iptc: Iptc, policy: JpegMarks) -> u64 {
        let seq = self.next_seq();
        self.enqueue(Job {
            seq,
            item,
            work: Work::Iptc {
                iptc: Box::new(iptc),
                policy,
            },
        })
    }

    /// Queue a sidecar edit (crop, turn); returns its sequence number.
    /// Never blocks.
    pub fn submit_edits(&mut self, item: PhotoItem, change: edits::Change) -> u64 {
        let seq = self.next_seq();
        self.enqueue(Job {
            seq,
            item,
            work: Work::Edits { change },
        })
    }

    fn next_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn enqueue(&mut self, job: Job) -> u64 {
        let seq = job.seq;
        {
            let (m, _) = &*self.pending;
            *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
        if let Some(tx) = &self.tx {
            if let Err(mpsc::SendError(job)) = tx.send(job) {
                // The thread is gone (it panicked): write here rather than lose the work.
                let (m, _) = &*self.pending;
                let mut left = m.lock().unwrap_or_else(|e| e.into_inner());
                *left = left.saturating_sub(1);
                log::error!(
                    "mark writer thread gone; writing {} inline",
                    job.item.display_name()
                );
                let _ = run(&job);
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

/// Keep only the newest job for each photo and kind, in the order those
/// were queued. Marks and IPTC for the same photo are different kinds and
/// both land; edits are never dropped, because a crop and a turn touch
/// different fields and the earlier one must not vanish.
fn latest_per_photo(batch: Vec<Job>) -> Vec<Job> {
    let mut seen = HashSet::new();
    let mut out: Vec<Job> = batch
        .into_iter()
        .rev()
        .filter(|j| match j.work.kind() {
            Kind::Edits => true,
            kind => seen.insert((j.item.path().to_path_buf(), kind)),
        })
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
    fn coalescing_keeps_one_job_per_photo_and_kind_in_queue_order() {
        let d = folder("coalesce");
        let its = items_in(&d, &["E.RAF", "F.RAF"]);
        let job = |seq, i: usize| Job {
            seq,
            item: its[i].clone(),
            work: Work::Marks {
                marks: stars(1),
                scheme: Box::new(ColorClassScheme::default()),
                policy: JpegMarks::Sidecar,
            },
        };
        let kept: Vec<u64> = latest_per_photo(vec![job(1, 0), job(2, 1), job(3, 0)])
            .iter()
            .map(|j| j.seq)
            .collect();
        assert_eq!(kept, vec![2, 3]);
        // A marks job and an IPTC job for the same photo are different
        // kinds: both land, in queue order.
        let iptc = |seq, i: usize| Job {
            seq,
            item: its[i].clone(),
            work: Work::Iptc {
                iptc: Box::new(Iptc {
                    headline: Some("hi".into()),
                    ..Default::default()
                }),
                policy: JpegMarks::Sidecar,
            },
        };
        let kept: Vec<u64> = latest_per_photo(vec![job(1, 0), iptc(2, 0), job(3, 0)])
            .iter()
            .map(|j| j.seq)
            .collect();
        assert_eq!(kept, vec![2, 3]);
        // Edits are never coalesced: a crop and a turn touch different
        // fields, and the earlier one must not vanish.
        let edit = |seq, i: usize, change: edits::Change| Job {
            seq,
            item: its[i].clone(),
            work: Work::Edits { change },
        };
        let kept: Vec<u64> = latest_per_photo(vec![
            edit(
                1,
                0,
                edits::Change {
                    crop: Some(Some(edits::Crop {
                        top: 0.1,
                        left: 0.2,
                        bottom: 0.9,
                        right: 0.8,
                    })),
                    ..Default::default()
                },
            ),
            edit(
                2,
                0,
                edits::Change {
                    orientation: Some(Some(6)),
                    ..Default::default()
                },
            ),
        ])
        .iter()
        .map(|j| j.seq)
        .collect();
        assert_eq!(kept, vec![1, 2]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn iptc_job_lands_without_clobbering_marks() {
        let d = folder("iptc-marks");
        let it = items_in(&d, &["G.RAF"]).remove(0);
        let mut w = MarkWriter::new();
        let s = ColorClassScheme::default();
        w.submit(it.clone(), stars(4), s, JpegMarks::Sidecar);
        w.submit_iptc(
            it.clone(),
            Iptc {
                headline: Some("Hello".into()),
                ..Default::default()
            },
            JpegMarks::Sidecar,
        );
        assert!(w.wait_idle(Duration::from_secs(10)));
        assert_eq!(read_rating(&it), 4);
        assert_eq!(items::read_iptc(&it).headline.as_deref(), Some("Hello"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn iptc_coalescing_keeps_the_newest() {
        let d = folder("iptc-order");
        let it = items_in(&d, &["H.RAF"]).remove(0);
        let mut w = MarkWriter::new();
        for h in ["First", "Second"] {
            w.submit_iptc(
                it.clone(),
                Iptc {
                    headline: Some(h.into()),
                    ..Default::default()
                },
                JpegMarks::Sidecar,
            );
        }
        assert!(w.wait_idle(Duration::from_secs(10)));
        assert_eq!(items::read_iptc(&it).headline.as_deref(), Some("Second"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn edits_jobs_all_land_in_order() {
        let d = folder("edits-order");
        let it = items_in(&d, &["I.RAF"]).remove(0);
        let mut w = MarkWriter::new();
        w.submit_edits(
            it.clone(),
            edits::Change {
                crop: Some(Some(edits::Crop {
                    top: 0.1,
                    left: 0.2,
                    bottom: 0.9,
                    right: 0.8,
                })),
                ..Default::default()
            },
        );
        w.submit_edits(
            it.clone(),
            edits::Change {
                orientation: Some(Some(6)),
                ..Default::default()
            },
        );
        assert!(w.wait_idle(Duration::from_secs(10)));
        let e = edits::read(&it.sidecar_path()).unwrap();
        assert_eq!(e.orientation, Some(6));
        let c = e.crop.expect("crop landed");
        assert!((c.top - 0.1).abs() < 0.001 && (c.left - 0.2).abs() < 0.001);
        let _ = std::fs::remove_dir_all(&d);
    }
}
