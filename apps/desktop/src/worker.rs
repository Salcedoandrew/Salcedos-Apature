//! Background decode pool with viewport-driven priorities.
//!
//! The UI re-requests every visible item each frame with a priority equal to
//! its distance from the viewport; a request for an item already queued just
//! updates its priority, so scrolling re-orders the queue instead of growing
//! it. Workers always pop the lowest priority value first.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_metadata::ExifSummary;
use apature_preview::{Rgb, Source, ThumbCache};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobKind {
    Thumb,
    Preview,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub idx: usize,
    pub kind: JobKind,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub long_side: u32,
    /// Generation of the folder this job belongs to; stale results are dropped.
    pub generation: u64,
}

pub struct Done {
    pub idx: usize,
    pub kind: JobKind,
    pub generation: u64,
    /// The file that was decoded. A result belongs to the photo at `idx`
    /// only while that slot still holds this path (see [`Done::belongs_to`]).
    pub path: PathBuf,
    pub result: Result<Rgb, String>,
    pub exif: Option<ExifSummary>,
    pub millis: f32,
    pub cache_hit: bool,
    /// Preview only: the decode is the file's full resolution (nothing larger exists).
    pub full: bool,
    /// Width × height of the largest image this file can show (its biggest
    /// embedded preview, or the JPEG itself), turned upright like `result`.
    /// Known from a thumbnail too, so the preview can draw a small image at
    /// the size the full one will have.
    pub native: Option<[u32; 2]>,
}

impl Done {
    /// Is this result for the photo now at `idx` in the folder of
    /// `generation`? A late result (another folder or tab, or a slot that
    /// now holds a different file) is dropped: it must never replace the
    /// image on screen, and never move the focus.
    pub fn belongs_to(&self, generation: u64, item_path: Option<&std::path::Path>) -> bool {
        self.generation == generation && item_path == Some(self.path.as_path())
    }
}

type Key = (usize, JobKind);

/// Priority at which warm-up work starts. Anything below is a visible row or a
/// preview and always outranks it.
pub const WARM_PRIO_BASE: u32 = 10_000;

#[derive(Default)]
struct Queue {
    /// (priority, kind, index): at equal priority a preview goes first.
    order: BTreeSet<(u32, u8, usize)>,
    prio: HashMap<Key, u32>,
    jobs: HashMap<Key, Job>,
    inflight: HashSet<Key>,
    /// Warm jobs being decoded right now, so they can be capped.
    warm_running: usize,
}

fn kind_u8(k: JobKind) -> u8 {
    match k {
        JobKind::Preview => 0,
        JobKind::Thumb => 1,
    }
}

struct Shared {
    q: Mutex<Queue>,
    /// How many threads warm-up may occupy at once.
    warm_cap: usize,
    cv: Condvar,
    cache: Option<ThumbCache>,
    ctx: egui::Context,
    tx: Sender<Done>,
}

pub struct Pool {
    shared: Arc<Shared>,
    rx: Receiver<Done>,
}

impl Pool {
    pub fn new(ctx: egui::Context, cache: Option<ThumbCache>, threads: usize) -> Self {
        let (tx, rx) = unbounded();
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue::default()),
            // One thread always stays free for a visible row or a preview.
            warm_cap: threads.saturating_sub(1).max(1),
            cv: Condvar::new(),
            cache,
            ctx,
            tx,
        });
        for i in 0..threads.max(1) {
            let s = shared.clone();
            let started = std::thread::Builder::new()
                .name(format!("decode-{i}"))
                .spawn(move || worker_loop(s));
            if let Err(e) = started {
                log::error!("could not start the worker: {e}");
            }
        }
        Pool { shared, rx }
    }

    pub fn request(&self, job: Job, prio: u32) {
        let key = (job.idx, job.kind);
        let mut q = lock_queue(&self.shared);
        if q.inflight.contains(&key) {
            return;
        }
        if let Some(old) = q.prio.get(&key).copied() {
            if old == prio {
                return;
            }
            q.order.remove(&(old, kind_u8(job.kind), job.idx));
        }
        q.order.insert((prio, kind_u8(job.kind), job.idx));
        q.prio.insert(key, prio);
        q.jobs.insert(key, job);
        drop(q);
        self.shared.cv.notify_one();
    }

    /// Drop queued preview decodes for photos not in `keep` (the user has
    /// flipped past them). Decodes already running finish and are cached.
    pub fn retain_previews(&self, keep: &[usize]) {
        let mut q = lock_queue(&self.shared);
        let stale: Vec<Key> = q
            .jobs
            .keys()
            .filter(|(i, k)| *k == JobKind::Preview && !keep.contains(i))
            .copied()
            .collect();
        for key in stale {
            if let Some(p) = q.prio.remove(&key) {
                q.order.remove(&(p, kind_u8(key.1), key.0));
            }
            q.jobs.remove(&key);
        }
    }

    pub fn clear_pending(&self) {
        let mut q = lock_queue(&self.shared);
        q.order.clear();
        q.prio.clear();
        q.jobs.clear();
    }

    pub fn pending(&self) -> usize {
        let q = lock_queue(&self.shared);
        q.order.len() + q.inflight.len()
    }

    /// Is a job for this slot already waiting or in flight? Warm-up asks so it
    /// does not count an item it queued on an earlier frame as new work, and so
    /// one whose queued job was dropped is picked up again.
    pub fn queued(&self, idx: usize, kind: JobKind) -> bool {
        let q = lock_queue(&self.shared);
        q.prio.contains_key(&(idx, kind)) || q.inflight.contains(&(idx, kind))
    }

    pub fn try_recv(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }
}

/// The queue lock, recovered if a worker panicked while holding it: one bad
/// frame must not poison every later frame into a panic of its own.
fn lock_queue(s: &Shared) -> std::sync::MutexGuard<'_, Queue> {
    s.q.lock().unwrap_or_else(|e| e.into_inner())
}

fn worker_loop(s: Arc<Shared>) {
    loop {
        let (job, warm) = {
            let mut q = lock_queue(&s);
            loop {
                if let Some(&(prio, k, idx)) = q.order.iter().next() {
                    // Warm-up work may not take the last thread. A thumbnail
                    // or preview that scrolls into view while a folder warms
                    // would otherwise wait for a whole decode to finish.
                    if prio >= WARM_PRIO_BASE && q.warm_running >= s.warm_cap {
                        let (guard, _) =
                            s.cv.wait_timeout(q, Duration::from_millis(100))
                                .unwrap_or_else(|e| e.into_inner());
                        q = guard;
                        continue;
                    }
                    q.order.remove(&(prio, k, idx));
                    let kind = if k == 0 {
                        JobKind::Preview
                    } else {
                        JobKind::Thumb
                    };
                    let key = (idx, kind);
                    q.prio.remove(&key);
                    if let Some(job) = q.jobs.remove(&key) {
                        q.inflight.insert(key);
                        if prio >= WARM_PRIO_BASE {
                            q.warm_running += 1;
                        }
                        break (job, prio >= WARM_PRIO_BASE);
                    }
                } else {
                    q = s.cv.wait(q).unwrap_or_else(|e| e.into_inner());
                }
            }
        };
        let done = run(&s, &job);
        {
            let mut q = lock_queue(&s);
            q.inflight.remove(&(job.idx, job.kind));
            if warm {
                q.warm_running = q.warm_running.saturating_sub(1);
            }
        }
        s.cv.notify_all();
        if s.tx.send(done).is_err() {
            return;
        }
        s.ctx.request_repaint();
    }
}

/// Pixels arrive turned by the file's own orientation (that is what the
/// thumbnail cache holds). A rotation in the sidecar (`tiff:Orientation`)
/// replaces it: undo the file's turn, apply the sidecar's.
fn sidecar_orientation(path: &std::path::Path, file_o: u32, rgb: Rgb) -> Rgb {
    use apature_metadata::edits;
    let sidecar = ["xmp", "XMP"]
        .iter()
        .map(|e| path.with_extension(e))
        .find(|p| p.is_file());
    let Some(o) = sidecar
        .and_then(|p| edits::read(&p).ok())
        .and_then(|e| e.orientation)
        .map(u32::from)
    else {
        return rgb;
    };
    if o == file_o {
        return rgb;
    }
    apature_preview::rotate(apature_preview::rotate(rgb, edits::inverse(file_o)), o)
}

fn run(s: &Shared, job: &Job) -> Done {
    let t0 = Instant::now();
    let mut cache_hit = false;
    let key = ThumbCache::key(&job.path, job.size, job.modified, job.long_side);

    let mut native: Option<[u32; 2]> = None;
    let result: Result<(Rgb, Option<ExifSummary>, bool), String> = (|| {
        let src = Source::open(&job.path).map_err(|e| e.to_string())?;
        // The thumbnail cache remembers the original's size, and scanning a
        // RAW for its embedded JPEGs is the expensive part of a warm hit.
        native = if job.kind == JobKind::Thumb {
            s.cache.as_ref().and_then(|c| c.get_native(&key))
        } else {
            None
        };
        native = native.or_else(|| src.largest().map(|c| [c.width, c.height]));
        let exif = apature_metadata::exif::from_bytes(src.data());
        let orientation = exif.as_ref().map(|e| e.orientation).unwrap_or(1);

        if job.kind == JobKind::Thumb {
            if let Some(c) = &s.cache {
                if let Some(rgb) = c.get(&key) {
                    cache_hit = true;
                    return Ok((rgb, exif, false));
                }
            }
        }

        // Decode at twice the thumb size then box-filter down: sharper than
        // taking the nearest DCT scale directly, still far cheaper than a
        // full decode.
        let target = match job.kind {
            JobKind::Thumb => job.long_side * 2,
            JobKind::Preview => job.long_side,
        };
        let rgb = src.decode_for(target).map_err(|e| e.to_string())?;
        // The decoder stops at the nearest DCT scale at or above the target;
        // when it could not reach the target, this is everything the file has.
        let full = job.kind == JobKind::Preview && rgb.long_side() <= job.long_side;
        let rgb = apature_preview::rotate(rgb, orientation);
        let rgb = match job.kind {
            JobKind::Thumb => apature_preview::thumbnail(&rgb, job.long_side),
            // Never hand the GPU more than was asked for (texture limits).
            JobKind::Preview if rgb.long_side() > job.long_side => {
                apature_preview::thumbnail(&rgb, job.long_side)
            }
            JobKind::Preview => rgb,
        };
        if job.kind == JobKind::Thumb {
            if let Some(c) = &s.cache {
                if let Err(e) = c.put(&key, &rgb) {
                    log::warn!("thumb cache write failed: {e}");
                }
                if let Some(n) = native {
                    c.put_native(&key, n);
                }
            }
        }
        Ok((rgb, exif, full))
    })();

    let (result, exif, full) = match result {
        Ok((rgb, exif, full)) => {
            let file_o = exif.as_ref().map(|e| e.orientation).unwrap_or(1);
            let rgb = sidecar_orientation(&job.path, file_o, rgb);
            native = native.map(|n| upright(n, [rgb.width, rgb.height]));
            (Ok(rgb), exif, full)
        }
        Err(e) => (Err(e), None, false),
    };
    Done {
        idx: job.idx,
        kind: job.kind,
        generation: job.generation,
        path: job.path.clone(),
        result,
        exif,
        millis: t0.elapsed().as_secs_f32() * 1000.0,
        cache_hit,
        full,
        native,
    }
}

/// Turn the stored size `native` to match the decoded (already upright)
/// image `shown`: swap width and height when one is landscape and the
/// other portrait.
fn upright(native: [u32; 2], shown: [u32; 2]) -> [u32; 2] {
    let [w, h] = native;
    let [sw, sh] = shown;
    if w != h && sw != sh && (w > h) != (sw > sh) {
        [h, w]
    } else {
        native
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(idx: usize, generation: u64, path: &str) -> Done {
        Done {
            idx,
            kind: JobKind::Preview,
            generation,
            path: PathBuf::from(path),
            result: Err("x".into()),
            exif: None,
            millis: 0.0,
            cache_hit: false,
            full: true,
            native: None,
        }
    }

    #[test]
    fn native_size_follows_the_turned_image() {
        // A portrait frame (EXIF 6/8): the stored preview is 6000×4000, the
        // decoded thumbnail is upright 213×320.
        assert_eq!(upright([6000, 4000], [213, 320]), [4000, 6000]);
        assert_eq!(upright([6000, 4000], [320, 213]), [6000, 4000]);
        assert_eq!(upright([4000, 4000], [320, 320]), [4000, 4000]);
    }

    fn job(idx: usize, kind: JobKind) -> Job {
        Job {
            idx,
            kind,
            path: PathBuf::from(format!("/x/{idx}.jpg")),
            size: 0,
            modified: None,
            long_side: 100,
            generation: 0,
        }
    }

    #[test]
    fn current_preview_beats_thumbs_and_stale_previews_drop() {
        // No workers: inspect the queue only.
        let (tx, rx) = unbounded();
        let pool = Pool {
            shared: Arc::new(Shared {
                q: Mutex::new(Queue::default()),
                warm_cap: 1,
                cv: Condvar::new(),
                cache: None,
                ctx: egui::Context::default(),
                tx,
            }),
            rx,
        };
        pool.request(job(1, JobKind::Thumb), 0);
        pool.request(job(40, JobKind::Preview), 0); // flipped past
        pool.request(job(41, JobKind::Preview), 0); // current
        pool.request(job(42, JobKind::Preview), 1); // next
        pool.retain_previews(&[41, 42]);
        let q = pool.shared.q.lock().unwrap();
        let order: Vec<(u32, u8, usize)> = q.order.iter().copied().collect();
        assert_eq!(order, vec![(0, 0, 41), (0, 1, 1), (1, 0, 42)]);
        assert!(!q.jobs.contains_key(&(40, JobKind::Preview)));
    }

    #[test]
    fn stale_preview_result_is_ignored() {
        let d = done(69, 7, "/shoot/DSCF7641.RAF");
        // Same folder, same slot, same file: applied.
        assert!(d.belongs_to(7, Some(std::path::Path::new("/shoot/DSCF7641.RAF"))));
        // The slot now holds another file (rescan, reorder): dropped.
        assert!(!d.belongs_to(7, Some(std::path::Path::new("/shoot/DSCF7640.RAF"))));
        // Another folder or tab (new generation): dropped.
        assert!(!d.belongs_to(8, Some(std::path::Path::new("/shoot/DSCF7641.RAF"))));
        // The slot no longer exists: dropped.
        assert!(!d.belongs_to(7, None));
    }
}
