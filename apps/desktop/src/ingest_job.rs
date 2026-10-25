//! Ingest off the UI thread: plan building and the copy run both happen on
//! worker threads and report back over a channel.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};

use apature_core::item::kind_for_extension;
use apature_core::{PhotoItem, Variant};
use apature_ingest::{build_plan, run, Entry, Event, Ledger, Options, Plan, Report};
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

    /// Run the plan; afterwards optionally apply `pad` to the verified
    /// primary copies of the photos this ingest made (so the ingested
    /// folder opens already stationeried). Photos that were already in
    /// the destination folder are never touched; see `apply_pad`.
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
                // The pad borrows the report, so it is computed before the
                // report is sent — but the message still goes out after
                // it, as before.
                let pad_result = if let (Some(pad), true) = (&pad, all_ok) {
                    Some(apply_pad(&plan, &report, pad, jpeg, &c2))
                } else {
                    None
                };
                let _ = tx.send(Msg::Report(report));
                if let Some((files, failed)) = pad_result {
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

/// Rebuild one ingested photo from the plan entries that copied it. The
/// first entry is the primary variant (plan order is variant order: RAW
/// first, then JPEG/HEIC); the rest are the secondary variants.
fn photo_item_from_entries(entries: &[&Entry]) -> Option<PhotoItem> {
    let first = entries.first()?;
    let stem = first
        .primary
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let mut variants = entries.iter().filter_map(|e| {
        let ext = e
            .primary
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or_default();
        let kind = kind_for_extension(ext)?;
        // The copy verified, so the destination file is the source's
        // size; its mtime comes from the file itself.
        let size = std::fs::metadata(&e.primary)
            .map(|m| m.len())
            .unwrap_or(e.size);
        let modified = std::fs::metadata(&e.primary)
            .and_then(|m| m.modified())
            .ok();
        Some(Variant {
            path: e.primary.clone(),
            kind,
            size,
            modified,
        })
    });
    let primary = variants.next()?;
    Some(PhotoItem {
        id: first.item,
        stem,
        primary,
        secondary: variants.collect(),
    })
}

/// Apply `pad` to the primary copies of the photos in `plan` — and only
/// those. Each photo is rebuilt from the plan entries that copied it, so
/// a photo that was already in the destination folder (an earlier shoot)
/// is never touched, and neither is any sidecar this ingest did not
/// create. Only entries whose primary copy verified get the pad.
/// Returns (files written, items failed).
fn apply_pad(
    plan: &Plan,
    report: &Report,
    pad: &Iptc,
    jpeg: apature_metadata::items::JpegMarks,
    cancel: &AtomicBool,
) -> (usize, usize) {
    // Group the non-companion entries by photo, in plan order.
    // Companions (voice memos, sidecars already on the card) are not
    // photos; `write_iptc` derives the sidecar from the photo itself.
    let mut groups: BTreeMap<usize, Vec<&Entry>> = BTreeMap::new();
    for (i, e) in plan.entries.iter().enumerate() {
        if e.companion {
            continue;
        }
        let verified = report
            .outcomes
            .get(i)
            .is_some_and(|o| o.failed.is_empty() && o.verified.contains(&e.primary));
        if verified {
            groups.entry(e.item).or_default().push(e);
        }
    }
    let mut files = 0;
    let mut failed = 0;
    for entries in groups.values() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(item) = photo_item_from_entries(entries) else {
            continue;
        };
        match apature_metadata::items::write_iptc(&item, pad, jpeg) {
            Ok(w) => files += w.len(),
            Err(_) => failed += 1,
        }
    }
    (files, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::Kind;
    use apature_ingest::FileOutcome;
    use apature_metadata::items::{read_iptc, write_iptc, JpegMarks};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn jpeg_skeleton() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(b"JFIF\0\x01\x01\x01\0\x48\0\x48\0\0");
        v.extend_from_slice(&[
            0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0, 0x12, 0xFF, 0xD9,
        ]);
        v
    }

    /// A unique scratch dir under the system temp dir (std-only: the
    /// desktop crate has no tempfile dev-dependency). Callers remove it.
    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "apature-pad-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn jpeg_item(id: usize, path: &Path) -> PhotoItem {
        PhotoItem {
            id,
            stem: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            primary: Variant {
                path: path.to_path_buf(),
                kind: Kind::Jpeg,
                size: fs::metadata(path).unwrap().len(),
                modified: None,
            },
            secondary: vec![],
        }
    }

    fn caption(text: &str) -> Iptc {
        Iptc {
            description: Some(text.into()),
            ..Default::default()
        }
    }

    fn verified_plan_entry(source: &Path, primary: &Path, item: usize) -> (Entry, FileOutcome) {
        (
            Entry {
                source: source.to_path_buf(),
                size: fs::metadata(source).unwrap().len(),
                primary: primary.to_path_buf(),
                secondary: None,
                item,
                fingerprint: None,
                companion: false,
            },
            FileOutcome {
                source: source.to_path_buf(),
                verified: vec![primary.to_path_buf()],
                failed: vec![],
            },
        )
    }

    #[test]
    fn pad_applies_only_to_the_photos_this_ingest_copied() {
        let root = tempdir();
        let src = root.join("CARD");
        let dest = root.join("Dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();

        // OLD.JPG already lives in the destination with its own caption.
        let old_path = dest.join("OLD.JPG");
        fs::write(&old_path, jpeg_skeleton()).unwrap();
        let old_item = jpeg_item(0, &old_path);
        write_iptc(&old_item, &caption("Old caption"), JpegMarks::Embed).unwrap();
        let old_bytes = fs::read(&old_path).unwrap();
        assert_eq!(
            read_iptc(&old_item).description.as_deref(),
            Some("Old caption")
        );

        // NEW.JPG is the photo this ingest copied into the same folder.
        let new_src = src.join("NEW.JPG");
        fs::write(&new_src, jpeg_skeleton()).unwrap();
        let new_dest = dest.join("NEW.JPG");
        fs::copy(&new_src, &new_dest).unwrap();

        let (entry, outcome) = verified_plan_entry(&new_src, &new_dest, 0);
        let plan = Plan {
            entries: vec![entry],
            ..Default::default()
        };
        let report = Report {
            outcomes: vec![outcome],
            ..Default::default()
        };
        let (files, failed) = apply_pad(
            &plan,
            &report,
            &caption("New assignment"),
            JpegMarks::Embed,
            &AtomicBool::new(false),
        );
        assert_eq!((files, failed), (1, 0));

        // The new photo got the pad; the old one is byte-identical.
        assert_eq!(
            read_iptc(&jpeg_item(0, &new_dest)).description.as_deref(),
            Some("New assignment")
        );
        assert_eq!(
            fs::read(&old_path).unwrap(),
            old_bytes,
            "OLD.JPG must be untouched by the pad"
        );
        assert_eq!(
            read_iptc(&old_item).description.as_deref(),
            Some("Old caption")
        );
        assert!(!dest.join("OLD.xmp").exists());
        assert!(!dest.join("OLD.XMP").exists());
        assert!(
            !dest.join("NEW.xmp").exists(),
            "JPEG-only + Embed: no sidecar"
        );
        assert!(
            !dest.join("NEW.XMP").exists(),
            "JPEG-only + Embed: no sidecar"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pad_applies_to_a_copied_pair_and_leaves_an_older_pair_alone() {
        let root = tempdir();
        let src = root.join("CARD");
        let dest = root.join("Dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();

        // An earlier shoot's pair already lives in the destination.
        let old_nef = dest.join("OLD.NEF");
        let old_jpg = dest.join("OLD.JPG");
        fs::write(&old_nef, b"not a real nef").unwrap();
        fs::write(&old_jpg, jpeg_skeleton()).unwrap();
        let old_item = PhotoItem {
            id: 0,
            stem: "OLD".into(),
            primary: Variant {
                path: old_nef.clone(),
                kind: Kind::Raw,
                size: fs::metadata(&old_nef).unwrap().len(),
                modified: None,
            },
            secondary: vec![Variant {
                path: old_jpg.clone(),
                kind: Kind::Jpeg,
                size: fs::metadata(&old_jpg).unwrap().len(),
                modified: None,
            }],
        };
        write_iptc(&old_item, &caption("Old caption"), JpegMarks::Embed).unwrap();
        let old_snapshot: Vec<(PathBuf, Vec<u8>)> = ["OLD.NEF", "OLD.JPG", "OLD.XMP"]
            .iter()
            .map(|n| (dest.join(n), fs::read(dest.join(n)).unwrap()))
            .collect();

        // This ingest copied a RAW+JPEG pair into the same folder.
        let new_nef_src = src.join("NEW.NEF");
        let new_jpg_src = src.join("NEW.JPG");
        fs::write(&new_nef_src, b"not a real nef").unwrap();
        fs::write(&new_jpg_src, jpeg_skeleton()).unwrap();
        let new_nef = dest.join("NEW.NEF");
        let new_jpg = dest.join("NEW.JPG");
        fs::copy(&new_nef_src, &new_nef).unwrap();
        fs::copy(&new_jpg_src, &new_jpg).unwrap();
        let (e0, o0) = verified_plan_entry(&new_nef_src, &new_nef, 0);
        let (e1, o1) = verified_plan_entry(&new_jpg_src, &new_jpg, 0);
        let plan = Plan {
            entries: vec![e0, e1],
            ..Default::default()
        };
        let report = Report {
            outcomes: vec![o0, o1],
            ..Default::default()
        };
        let (files, failed) = apply_pad(
            &plan,
            &report,
            &caption("New assignment"),
            JpegMarks::Embed,
            &AtomicBool::new(false),
        );
        // Sidecar + embedded JPEG.
        assert_eq!((files, failed), (2, 0));
        assert_eq!(
            read_iptc(&jpeg_item(0, &new_jpg)).description.as_deref(),
            Some("New assignment")
        );
        assert!(dest.join("NEW.XMP").exists());
        for (path, bytes) in old_snapshot {
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "{} must be untouched by the pad",
                path.display()
            );
        }
        assert_eq!(
            read_iptc(&old_item).description.as_deref(),
            Some("Old caption")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pad_skips_entries_whose_copy_did_not_verify() {
        let root = tempdir();
        let src = root.join("CARD");
        let dest = root.join("Dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        let new_src = src.join("NEW.JPG");
        fs::write(&new_src, jpeg_skeleton()).unwrap();
        let new_dest = dest.join("NEW.JPG");
        fs::copy(&new_src, &new_dest).unwrap();
        let (entry, _) = verified_plan_entry(&new_src, &new_dest, 0);
        let plan = Plan {
            entries: vec![entry],
            ..Default::default()
        };
        let report = Report {
            outcomes: vec![FileOutcome {
                source: new_src.clone(),
                verified: vec![],
                failed: vec![(new_dest.clone(), "hash mismatch".into())],
            }],
            ..Default::default()
        };
        let (files, failed) = apply_pad(
            &plan,
            &report,
            &caption("New assignment"),
            JpegMarks::Embed,
            &AtomicBool::new(false),
        );
        assert_eq!((files, failed), (0, 0));
        assert!(read_iptc(&jpeg_item(0, &new_dest)).description.is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
