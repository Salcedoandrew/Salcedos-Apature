//! Execute a plan sequentially with progress events and cancellation.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::copy::copy_verified;
use crate::ledger::Ledger;
use crate::plan::Plan;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Started {
        index: usize,
        source: PathBuf,
    },
    /// One destination verified.
    Verified {
        index: usize,
        destination: PathBuf,
    },
    Failed {
        index: usize,
        destination: PathBuf,
        error: String,
    },
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOutcome {
    pub source: PathBuf,
    pub verified: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

impl FileOutcome {
    pub fn fully_verified(&self, wanted: usize) -> bool {
        self.failed.is_empty() && self.verified.len() == wanted
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub outcomes: Vec<FileOutcome>,
    pub bytes_copied: u64,
    pub elapsed_secs: f64,
    pub cancelled: bool,
    /// Saving the ledger at the end failed; the copies themselves are fine,
    /// but "only new" will offer these files again next time.
    pub ledger_error: Option<String>,
}

impl Report {
    pub fn files_ok(&self) -> usize {
        self.outcomes.iter().filter(|o| o.failed.is_empty()).count()
    }
    pub fn files_failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| !o.failed.is_empty())
            .count()
    }
    /// True only when every file reached every destination.
    pub fn all_verified(&self, plan: &Plan) -> bool {
        !self.cancelled
            && self.outcomes.len() == plan.entries.len()
            && self.outcomes.iter().all(|o| o.failed.is_empty())
    }
}

/// Copy every entry: primary first (from the source), then the secondary
/// from the fresh primary copy (local disk, faster than the card). Each copy
/// is independently verified. The ledger is updated only for files whose
/// every destination verified, and saved at the end.
pub fn run(
    plan: &Plan,
    ledger: &mut Ledger,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(Event),
) -> Report {
    let t0 = Instant::now();
    let mut report = Report::default();
    let mut verified_per_item: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, e) in plan.entries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        on_event(Event::Started {
            index,
            source: e.source.clone(),
        });
        let mut outcome = FileOutcome {
            source: e.source.clone(),
            verified: Vec::new(),
            failed: Vec::new(),
        };
        let wanted = 1 + e.secondary.is_some() as usize;
        match copy_verified(&e.source, &e.primary) {
            Ok(r) => {
                report.bytes_copied += r.bytes;
                outcome.verified.push(e.primary.clone());
                on_event(Event::Verified {
                    index,
                    destination: e.primary.clone(),
                });
                if let Some(sec) = &e.secondary {
                    match copy_verified(&e.primary, sec) {
                        Ok(r2) => {
                            report.bytes_copied += r2.bytes;
                            outcome.verified.push(sec.clone());
                            on_event(Event::Verified {
                                index,
                                destination: sec.clone(),
                            });
                        }
                        Err(err) => {
                            outcome.failed.push((sec.clone(), err.to_string()));
                            on_event(Event::Failed {
                                index,
                                destination: sec.clone(),
                                error: err.to_string(),
                            });
                        }
                    }
                }
            }
            Err(err) => {
                outcome.failed.push((e.primary.clone(), err.to_string()));
                on_event(Event::Failed {
                    index,
                    destination: e.primary.clone(),
                    error: err.to_string(),
                });
            }
        }
        if outcome.fully_verified(wanted) {
            *verified_per_item.entry(e.item).or_default() += 1;
        }
        report.outcomes.push(outcome);
    }
    // A photo counts as ingested only when every file of it (variants and
    // companions) reached every destination; otherwise "only new" offers the
    // whole photo again next time, so a failed memo is retried.
    let mut planned_per_item: BTreeMap<usize, usize> = BTreeMap::new();
    for e in &plan.entries {
        *planned_per_item.entry(e.item).or_default() += 1;
    }
    for e in &plan.entries {
        if verified_per_item.get(&e.item) == planned_per_item.get(&e.item) {
            if let Some(fp) = &e.fingerprint {
                ledger.insert(fp.clone());
            }
        }
    }
    if let Err(e) = ledger.save() {
        report.ledger_error = Some(e.to_string());
    }
    report.elapsed_secs = t0.elapsed().as_secs_f64();
    on_event(Event::Finished);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan, Options};
    use std::fs;
    use std::path::Path;

    #[test]
    fn copies_to_both_destinations_verifies_and_records() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        for n in ["A.NEF", "A.JPG", "B.NEF"] {
            fs::write(src.join(n), n.repeat(1000)).unwrap();
        }
        let lp = t.path().join("ledger.json");
        let mut ledger = Ledger::load(&lp);
        let opts = Options {
            source: src.clone(),
            primary: t.path().join("P"),
            secondary: Some(t.path().join("S")),
            folder_template: "{filename}".into(),
            only_new: true,
            ..Default::default()
        };
        let plan = build_plan(&opts, &ledger).unwrap();
        assert_eq!(plan.entries.len(), 3);
        let mut events = Vec::new();
        let report = run(&plan, &mut ledger, &AtomicBool::new(false), |e| {
            events.push(e)
        });
        assert!(report.all_verified(&plan), "{report:?}");
        assert_eq!(report.files_ok(), 3);
        assert_eq!(report.bytes_copied, 2 * (5000 + 5000 + 5000));
        assert!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Verified { .. }))
                .count()
                == 6
        );
        assert!(matches!(events.last(), Some(Event::Finished)));
        assert!(Path::new(&t.path().join("S/A/A.JPG")).is_file());
        // Sources untouched, ledger persisted, second plan has nothing to do.
        assert_eq!(fs::read_dir(&src).unwrap().count(), 3);
        let ledger2 = Ledger::load(&lp);
        assert_eq!(ledger2.len(), 3);
        let again = build_plan(&opts, &ledger2).unwrap();
        assert!(again.entries.is_empty());
        assert_eq!(again.skipped.len(), 3);
    }

    #[test]
    fn a_ledger_that_cannot_be_saved_is_in_the_report() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("A.JPG"), "a").unwrap();
        // The ledger's folder is a file, so saving must fail.
        fs::write(t.path().join("blocked"), "").unwrap();
        let mut ledger = Ledger::load(&t.path().join("blocked/ledger.json"));
        let opts = Options {
            source: src,
            primary: t.path().join("P"),
            folder_template: "{filename}".into(),
            ..Default::default()
        };
        let plan = build_plan(&opts, &ledger).unwrap();
        let report = run(&plan, &mut ledger, &AtomicBool::new(false), |_| {});
        assert_eq!(report.files_ok(), 1, "the copy itself succeeded");
        assert!(report.ledger_error.is_some(), "{report:?}");
    }

    #[test]
    fn a_failed_ledger_save_is_reported_not_swallowed() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("A.JPG"), b"photo").unwrap();
        // The ledger path sits under a regular file, so saving it must fail.
        let blocker = t.path().join("blocker");
        fs::write(&blocker, b"not a folder").unwrap();
        let mut ledger = Ledger::load(&blocker.join("ledger.json"));
        let opts = Options {
            source: src.clone(),
            primary: t.path().join("P"),
            folder_template: String::new(),
            only_new: true,
            ..Default::default()
        };
        let plan = build_plan(&opts, &ledger).unwrap();
        let report = run(&plan, &mut ledger, &AtomicBool::new(false), |_| {});
        assert!(report.all_verified(&plan), "the copy itself succeeded");
        assert!(
            report.ledger_error.is_some(),
            "the save failure is in the report: {report:?}"
        );
    }

    #[test]
    fn a_failed_memo_keeps_its_photo_out_of_the_ledger_so_it_is_retried() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("DSC_0001.JPG"), b"photo").unwrap();
        fs::write(src.join("DSC_0001.WAV"), b"memo").unwrap();
        let opts = Options {
            source: src.clone(),
            primary: t.path().join("P"),
            folder_template: String::new(),
            only_new: true,
            ..Default::default()
        };
        let mut ledger = Ledger::in_memory();
        let plan = build_plan(&opts, &ledger).unwrap();
        assert_eq!(plan.entries.len(), 2);
        // The memo's destination is taken by a directory: its copy fails.
        let memo_dest = plan.entries[1].primary.clone();
        fs::create_dir_all(&memo_dest).unwrap();
        let report = run(&plan, &mut ledger, &AtomicBool::new(false), |_| {});
        assert_eq!(report.files_failed(), 1);
        assert!(
            ledger.is_empty(),
            "the photo is not done while its memo failed"
        );
        // Next "only new" plan offers the photo and its memo again.
        fs::remove_dir(&memo_dest).unwrap();
        fs::remove_file(&plan.entries[0].primary).unwrap();
        let again = build_plan(&opts, &ledger).unwrap();
        assert_eq!(again.entries.len(), 2);
        let report = run(&again, &mut ledger, &AtomicBool::new(false), |_| {});
        assert!(report.all_verified(&again));
        assert_eq!(ledger.len(), 1, "photo recorded once everything verified");
        assert!(build_plan(&opts, &ledger).unwrap().entries.is_empty());
    }

    #[test]
    fn a_failing_destination_is_reported_and_the_rest_continue() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("A.NEF"), b"aaaa").unwrap();
        fs::write(src.join("B.NEF"), b"bbbb").unwrap();
        // Secondary destination is a *file*, so every copy there fails.
        let bad = t.path().join("S");
        fs::write(&bad, b"not a dir").unwrap();
        let opts = Options {
            source: src.clone(),
            primary: t.path().join("P"),
            secondary: Some(bad.clone()),
            folder_template: String::new(),
            only_new: false,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        let mut ledger = Ledger::in_memory();
        let report = run(&plan, &mut ledger, &AtomicBool::new(false), |_| {});
        assert_eq!(report.files_failed(), 2);
        assert!(!report.all_verified(&plan));
        assert!(
            t.path().join("P/A.NEF").is_file(),
            "primary still succeeded"
        );
        assert!(
            ledger.is_empty(),
            "nothing recorded as ingested when a destination failed"
        );
        // Cancellation stops before the next file.
        let cancel = AtomicBool::new(true);
        let plan2 = build_plan(
            &Options {
                primary: t.path().join("P2"),
                secondary: None,
                ..opts.clone()
            },
            &Ledger::in_memory(),
        )
        .unwrap();
        let r2 = run(&plan2, &mut Ledger::in_memory(), &cancel, |_| {});
        assert!(r2.cancelled && r2.outcomes.is_empty());
    }
}
