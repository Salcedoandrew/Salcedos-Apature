//! Build the plan: which files, where they go, what collides.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use apature_core::{Kind, PhotoItem, ScanOptions, Variant};
use rayon::prelude::*;

use crate::ledger::{fingerprint, Ledger};
use crate::template::{expand, sanitize_component, Facts};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileFilter {
    #[default]
    All,
    RawOnly,
    JpegOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Options {
    pub source: PathBuf,
    pub include_subfolders: bool,
    pub primary: PathBuf,
    pub secondary: Option<PathBuf>,
    /// Folder template under each destination, e.g. `{date}-{headline}` or
    /// `{year}/{month}/{day}`. Empty = copy straight into the destination.
    pub folder_template: String,
    /// File name template without extension, e.g. `{headline}-{seqn}`.
    /// Empty = keep original names.
    pub rename_template: String,
    pub seq_start: u32,
    pub filter: FileFilter,
    /// Skip files whose fingerprint is in the ledger.
    pub only_new: bool,
    pub headline: Option<String>,
    /// What to do when a destination name is taken.
    pub on_collision: CollisionPolicy,
    /// Only photos captured on this (year, month, day); None = every day.
    pub day: Option<(i32, u32, u32)>,
    /// Only these source files (Live Ingest hands over the batch that has
    /// settled); None = everything the scan found.
    #[serde(default)]
    pub only_paths: Option<Vec<PathBuf>>,
    /// Keep the card's subfolders (e.g. `DCIM/100NIKON`) under the folder
    /// template, like Photo Mechanic's "Preserve folder structure".
    pub preserve_folders: bool,
    /// Copy a photo's companions along with it: the voice memo
    /// (`DSC_0001.WAV`) and an XMP sidecar already on the card. They take the
    /// photo's new name and keep their own extension.
    pub copy_companions: bool,
}

/// Photo Mechanic's `IngestAutoIncSuffix`: either refuse the plan (the
/// default, nothing is ever overwritten either way) or add `-1`, `-2`… to
/// the file name until it is free at every destination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CollisionPolicy {
    #[default]
    Refuse,
    Suffix,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            source: PathBuf::new(),
            include_subfolders: true,
            primary: PathBuf::new(),
            secondary: None,
            folder_template: "{date}-{headline}".into(),
            rename_template: String::new(),
            seq_start: 1,
            filter: FileFilter::All,
            only_new: true,
            headline: None,
            on_collision: CollisionPolicy::Refuse,
            day: None,
            only_paths: None,
            preserve_folders: false,
            copy_companions: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub source: PathBuf,
    pub size: u64,
    pub primary: PathBuf,
    pub secondary: Option<PathBuf>,
    /// Index of the photo item this file belongs to (pairs share one).
    pub item: usize,
    pub fingerprint: Option<String>,
    /// A voice memo or sidecar that travels with the photo, not a photo.
    pub companion: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collision {
    pub destination: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skip {
    pub source: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub entries: Vec<Entry>,
    pub skipped: Vec<Skip>,
    pub collisions: Vec<Collision>,
    /// Files that got a `-N` suffix because the name was taken.
    pub suffixed: usize,
    /// Voice memos and sidecars planned alongside the photos.
    pub companions: usize,
    pub total_bytes: u64,
    pub items: usize,
    /// Capture days present on the source with their photo counts (before
    /// the day filter), newest last.
    pub days: Vec<((i32, u32, u32), usize)>,
    /// Photos left out because they were taken on another day.
    pub other_days: usize,
    /// The sequence number the next ingest with the same counter should start
    /// at (seq_start + photos planned). Lets the app keep `{seqn}` running
    /// across runs, per headline.
    pub next_seq: u32,
}

impl Plan {
    pub fn can_run(&self) -> bool {
        self.collisions.is_empty() && !self.entries.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("source folder: {0}")]
    Source(#[from] apature_core::folder::ScanError),
    #[error("destination is inside the source, or the source inside the destination")]
    Nested,
    #[error("folder template produced an unusable name for {0}")]
    BadFolder(PathBuf),
    #[error("rename template produced an unusable name for {0}")]
    BadName(PathBuf),
}

/// Naming facts for one file of `item` (capture date/time from EXIF).
pub fn facts_for(item: &PhotoItem, variant_path: &Path, headline: Option<&str>) -> Facts {
    let exif = apature_metadata::exif::read(item.path()).ok();
    let (camera, make, lens, iso, f_number, exposure, focal_mm) = exif
        .as_ref()
        .map(|e| {
            (
                e.model.clone(),
                e.make.clone(),
                e.lens.clone(),
                e.iso,
                e.f_number,
                e.exposure_time,
                e.focal_length_mm,
            )
        })
        .unwrap_or_default();
    let (date, time) = exif
        .and_then(|e| e.date_time_original)
        .map(|d| {
            // "YYYY:MM:DD HH:MM:SS"
            let date = apature_metadata::codes::parse_date(&d);
            let time = (|| {
                let h: u32 = d.get(11..13)?.parse().ok()?;
                let m: u32 = d.get(14..16)?.parse().ok()?;
                let s: u32 = d.get(17..19)?.parse().ok()?;
                Some((h, m, s))
            })();
            (date, time)
        })
        .unwrap_or((None, None));
    Facts {
        stem: variant_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ext: variant_path
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        date,
        time,
        headline: headline.map(|h| h.to_string()),
        folder: variant_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned()),
        camera,
        make,
        lens,
        iso,
        f_number,
        exposure,
        focal_mm,
        today: Some(today()),
        dest: None,
    }
}

/// Build the plan. Fast enough to run on every option change for a card
/// (one EXIF read per item, one 64 KiB fingerprint per file when `only_new`).
/// What one photo would be called under `opts`: the folder (relative to the
/// destination, "" when the folder template is empty) and the file name.
/// Uses exactly the expansion and sanitising the planner uses, so the Ingest
/// sheet can show a live example while the templates are typed. `facts`
/// should carry a plausible stem/ext/date; the headline comes from `opts`.
pub fn example_names(opts: &Options, facts: &Facts) -> Result<(String, String), PlanError> {
    let facts = Facts {
        headline: opts.headline.clone(),
        ..facts.clone()
    };
    let seq = opts.seq_start;
    let folder = if opts.folder_template.trim().is_empty() {
        String::new()
    } else {
        let mut parts = Vec::new();
        for part in expand(&opts.folder_template, &facts, seq).split('/') {
            let Some(c) = sanitize_component(part) else {
                return Err(PlanError::BadFolder(PathBuf::from(&facts.stem)));
            };
            parts.push(c);
        }
        parts.join("/")
    };
    let name = if opts.rename_template.trim().is_empty() {
        if facts.ext.is_empty() {
            facts.stem.clone()
        } else {
            format!("{}.{}", facts.stem, facts.ext)
        }
    } else {
        let stem = sanitize_component(&expand(&opts.rename_template, &facts, seq))
            .ok_or_else(|| PlanError::BadName(PathBuf::from(&facts.stem)))?;
        if facts.ext.is_empty() {
            stem
        } else {
            format!("{stem}.{}", facts.ext)
        }
    };
    Ok((folder, name))
}

/// Files that belong to `item` but are not photos: a voice memo with the
/// same stem (Nikon and Canon write `DSC_0001.WAV`) and an XMP sidecar that
/// is already on the card. Either case of extension is found.
pub fn companions(item: &PhotoItem) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let base = &item.primary.path;
    for ext in ["WAV", "wav"] {
        let p = base.with_extension(ext);
        if p.is_file() && !out.contains(&p) {
            out.push(p);
            break;
        }
    }
    let sc = item.sidecar_path();
    if sc.is_file() {
        out.push(sc);
    }
    out
}

/// The card subfolder a file sits in, relative to the source, with every
/// component sanitised. Empty when the file is at the source root.
fn card_subfolder(source: &Path, file: &Path) -> PathBuf {
    let mut rel = PathBuf::new();
    if let Some(parent) = file.parent().and_then(|p| p.strip_prefix(source).ok()) {
        for c in parent.components() {
            if let Some(c) = sanitize_component(&c.as_os_str().to_string_lossy()) {
                rel.push(c);
            }
        }
    }
    rel
}

/// (year, month, day) for a count of days since 1970-01-01 — Howard
/// Hinnant's days-to-civil. The one implementation in the crate: `today`
/// and photocopy's file-time conversion both go through it.
pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Today's civil date (UTC) for the example line when no photo date is known.
pub fn today() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    civil_from_days(secs.div_euclid(86_400))
}

/// Add one file to the plan at `folder_rel/name` under each destination,
/// reporting a collision when the name is taken on disk or in this plan.
#[allow(clippy::too_many_arguments)]
fn place(
    plan: &mut Plan,
    seen_dest: &mut BTreeMap<PathBuf, PathBuf>,
    opts: &Options,
    folder_rel: &Path,
    name: &str,
    source: &Path,
    size: u64,
    item: usize,
    fingerprint: Option<String>,
    companion: bool,
) {
    let primary = opts.primary.join(folder_rel).join(name);
    let secondary = opts
        .secondary
        .as_ref()
        .map(|s| s.join(folder_rel).join(name));
    for dest in std::iter::once(&primary).chain(secondary.iter()) {
        if dest.exists() {
            plan.collisions.push(Collision {
                destination: dest.clone(),
                reason: "already exists at the destination".into(),
            });
        } else if let Some(other) = seen_dest.get(dest) {
            plan.collisions.push(Collision {
                destination: dest.clone(),
                reason: format!(
                    "two sources map to it: {} and {}",
                    other.display(),
                    source.display()
                ),
            });
        }
        seen_dest.insert(dest.clone(), source.to_path_buf());
    }
    plan.total_bytes += size;
    plan.entries.push(Entry {
        source: source.to_path_buf(),
        size,
        primary,
        secondary,
        item,
        // Always recorded for a photo so a later "only new" pass knows it,
        // even when this run copied everything. Companions follow their
        // photo and are never looked up on their own.
        fingerprint,
        companion,
    });
}

/// One variant that survived the ledger check, with the base name it
/// would take before any collision suffix.
struct PendingVariant {
    source: PathBuf,
    size: u64,
    fingerprint: Option<String>,
    stem: String,
    ext: String,
}

/// Smallest suffix `k` (0 = the plain names) whose names are free at every
/// destination and in this plan for every variant of the item and every
/// companion, so a pair keeps one common stem. Returns 0 when no suffix
/// up to 10000 is free; the entries are still placed and `place` reports
/// the collisions, so the plan is refused.
fn free_suffix(
    pending: &[PendingVariant],
    companion_exts: &[String],
    opts: &Options,
    folder_rel: &Path,
    seen_dest: &BTreeMap<PathBuf, PathBuf>,
) -> u32 {
    let is_taken = |n: &String| {
        let p = opts.primary.join(folder_rel).join(n);
        let s = opts.secondary.as_ref().map(|s| s.join(folder_rel).join(n));
        p.exists()
            || seen_dest.contains_key(&p)
            || s.as_ref()
                .is_some_and(|s| s.exists() || seen_dest.contains_key(s))
    };
    for k in 0..=10_000 {
        let mut names: Vec<String> = pending
            .iter()
            .map(|p| {
                if k == 0 {
                    format!("{}{}", p.stem, p.ext)
                } else {
                    format!("{}-{}{}", p.stem, k, p.ext)
                }
            })
            .collect();
        let cand_stem = if k == 0 {
            pending[0].stem.clone()
        } else {
            format!("{}-{k}", pending[0].stem)
        };
        names.extend(companion_exts.iter().map(|e| format!("{cand_stem}.{e}")));
        if names.iter().all(|n| !is_taken(n)) {
            return k;
        }
    }
    0
}

pub fn build_plan(opts: &Options, ledger: &Ledger) -> Result<Plan, PlanError> {
    if opts.primary.starts_with(&opts.source) || opts.source.starts_with(&opts.primary) {
        return Err(PlanError::Nested);
    }
    if let Some(s) = &opts.secondary {
        if s.starts_with(&opts.source) || opts.source.starts_with(s) {
            return Err(PlanError::Nested);
        }
    }
    let items = apature_core::scan(
        &opts.source,
        &ScanOptions {
            recursive: opts.include_subfolders,
        },
    )?;
    let mut plan = Plan::default();
    let mut seen_dest: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut seq = opts.seq_start;
    let mut item_count = 0usize;

    // `only_paths` membership is tested per variant per photo; a set makes
    // each test one lookup instead of a scan of the whole batch.
    let only_set: Option<HashSet<&PathBuf>> =
        opts.only_paths.as_ref().map(|paths| paths.iter().collect());
    let passes_filter = |v: &Variant| match opts.filter {
        FileFilter::All => true,
        FileFilter::RawOnly => v.kind == Kind::Raw,
        FileFilter::JpegOnly => v.kind == Kind::Jpeg,
    };
    // The EXIF read behind `facts_for` is the slow part of planning and is
    // independent per photo, so it runs in parallel. Everything after this
    // stays sequential and in card order: sequence numbers, collision
    // state and skip order all depend on it. `None` marks a photo the
    // `only_paths` check or the filter leaves out.
    let prepped: Vec<Option<Facts>> = items
        .par_iter()
        .map(|item| {
            if let Some(only) = &only_set {
                if !item.variants().any(|v| only.contains(&v.path)) {
                    return None;
                }
            }
            let first = item.variants().find(|v| passes_filter(*v))?;
            Some(facts_for(item, &first.path, opts.headline.as_deref()))
        })
        .collect();
    let mut day_counts: BTreeMap<(i32, u32, u32), usize> = BTreeMap::new();

    for (i, item) in items.iter().enumerate() {
        let Some(prepped_facts) = &prepped[i] else {
            continue;
        };
        let variants: Vec<_> = item.variants().filter(|v| passes_filter(*v)).collect();
        debug_assert!(!variants.is_empty());
        let mut facts0 = prepped_facts.clone();
        facts0.dest = opts
            .primary
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(d) = facts0.date {
            *day_counts.entry(d).or_default() += 1;
        }
        if let Some(want) = opts.day {
            if facts0.date != Some(want) {
                plan.other_days += 1;
                continue;
            }
        }
        // Folder: one per item so a pair never splits.
        let folder_rel: PathBuf = if opts.folder_template.trim().is_empty() {
            PathBuf::new()
        } else {
            let mut rel = PathBuf::new();
            for part in expand(&opts.folder_template, &facts0, seq).split('/') {
                let Some(c) = sanitize_component(part) else {
                    return Err(PlanError::BadFolder(variants[0].path.clone()));
                };
                rel.push(c);
            }
            rel
        };
        let folder_rel = if opts.preserve_folders {
            folder_rel.join(card_subfolder(&opts.source, &variants[0].path))
        } else {
            folder_rel
        };
        let mut added_any = false;
        // The stem the photo ended up with, so companions follow a suffix.
        let mut placed_stem: Option<String> = None;
        let item_companions = if opts.copy_companions {
            companions(item)
        } else {
            Vec::new()
        };
        let companion_exts: Vec<String> = item_companions
            .iter()
            .map(|c| {
                c.extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        // Decide which variants are actually placed (ledger check) and
        // their base names first: the suffix is chosen once for the whole
        // item below, so a RAW+JPEG pair never splits across stems.
        let mut pending: Vec<PendingVariant> = Vec::new();
        for v in &variants {
            // Read once: the ledger check and the entry share it.
            let fp = fingerprint(&v.path);
            if opts.only_new {
                match &fp {
                    Ok(fp) if ledger.contains(fp) => {
                        plan.skipped.push(Skip {
                            source: v.path.clone(),
                            reason: "already ingested".into(),
                        });
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        plan.skipped.push(Skip {
                            source: v.path.clone(),
                            reason: format!("unreadable: {e}"),
                        });
                        continue;
                    }
                }
            }
            let facts = Facts {
                stem: facts0.stem.clone(),
                ext: v
                    .path
                    .extension()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                ..facts0.clone()
            };
            let name = if opts.rename_template.trim().is_empty() {
                v.file_name()
            } else {
                let stem = sanitize_component(&expand(&opts.rename_template, &facts, seq))
                    .ok_or_else(|| PlanError::BadName(v.path.clone()))?;
                if facts.ext.is_empty() {
                    stem
                } else {
                    format!("{stem}.{}", facts.ext)
                }
            };
            let (stem, ext) = match name.rfind('.') {
                Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
                _ => (name.clone(), String::new()),
            };
            pending.push(PendingVariant {
                source: v.path.clone(),
                size: v.size,
                fingerprint: fp.ok(),
                stem,
                ext,
            });
        }
        // With the suffix policy, one suffix for the whole item: walk
        // `stem`, `stem-1`, `stem-2`… until every variant's name and every
        // companion's name is free at every destination and in this plan.
        // The old per-variant walk let the JPEG half keep the plain stem
        // while the RAW took `-1`, splitting the pair.
        let suffix_k = if opts.on_collision == CollisionPolicy::Suffix && !pending.is_empty() {
            free_suffix(&pending, &companion_exts, opts, &folder_rel, &seen_dest)
        } else {
            0
        };
        for p in &pending {
            let name = if suffix_k == 0 {
                format!("{}{}", p.stem, p.ext)
            } else {
                format!("{}-{}{}", p.stem, suffix_k, p.ext)
            };
            if suffix_k > 0 {
                plan.suffixed += 1;
            }
            if placed_stem.is_none() {
                placed_stem = Some(if suffix_k == 0 {
                    p.stem.clone()
                } else {
                    format!("{}-{suffix_k}", p.stem)
                });
            }
            place(
                &mut plan,
                &mut seen_dest,
                opts,
                &folder_rel,
                &name,
                &p.source,
                p.size,
                i,
                p.fingerprint.clone(),
                false,
            );
            added_any = true;
        }
        if added_any {
            let stem = placed_stem.unwrap_or_default();
            for c in item_companions {
                let ext = c
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = format!("{stem}.{ext}");
                let size = std::fs::metadata(&c).map(|m| m.len()).unwrap_or(0);
                place(
                    &mut plan,
                    &mut seen_dest,
                    opts,
                    &folder_rel,
                    &name,
                    &c,
                    size,
                    i,
                    None,
                    true,
                );
                plan.companions += 1;
            }
        }
        if added_any {
            item_count += 1;
            seq += 1;
        }
    }
    // The map iterates in day order, which is the sort `days` used to get.
    plan.days = day_counts.into_iter().collect();
    plan.next_seq = seq;
    // Deduplicate identical collision reports.
    let mut seen = BTreeSet::new();
    plan.collisions
        .retain(|c| seen.insert((c.destination.clone(), c.reason.clone())));
    plan.items = item_count;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_across_leap_rules() {
        for (secs, want) in [
            (0i64, (1970, 1, 1)),
            (-86_400, (1969, 12, 31)),
            (951_782_400, (2000, 2, 29)),  // 2000 is a leap year (÷400)
            (4_107_542_400, (2100, 3, 1)), // 2100 is not (÷100)
            (4_107_542_400 - 1, (2100, 2, 28)),
        ] {
            assert_eq!(civil_from_days(secs.div_euclid(86_400)), want, "{secs}");
        }
    }
    use std::fs;

    fn card(dir: &Path) {
        for n in ["DSC_0001.NEF", "DSC_0001.JPG", "DSC_0002.NEF", "notes.txt"] {
            fs::write(dir.join(n), format!("data-{n}")).unwrap();
        }
    }

    #[test]
    fn civil_from_days_covers_epoch_leap_and_century_years() {
        // Day counts checked against known Unix timestamps / 86_400.
        for (days, want) in [
            (0, (1970, 1, 1)),       // the epoch itself
            (11_016, (2000, 2, 29)), // 2000 is a leap year
            (47_541, (2100, 3, 1)),  // 2100 is not: no Feb 29 that year
        ] {
            assert_eq!(civil_from_days(days), want, "day {days}");
        }
    }

    #[test]
    fn plans_pairs_rename_and_two_destinations() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        card(&src);
        let opts = Options {
            source: src.clone(),
            primary: t.path().join("Work"),
            secondary: Some(t.path().join("Backup")),
            folder_template: "{headline}".into(),
            rename_template: "{headline}-{seqn}".into(),
            headline: Some("100726 SPO BGLF".into()),
            only_new: false,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        assert!(plan.can_run());
        assert_eq!(plan.items, 2);
        assert_eq!(plan.entries.len(), 3);
        let names: Vec<String> = plan
            .entries
            .iter()
            .map(|e| {
                e.primary
                    .strip_prefix(t.path())
                    .unwrap()
                    .display()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "Work/100726 SPO BGLF/100726 SPO BGLF-001.NEF",
                "Work/100726 SPO BGLF/100726 SPO BGLF-001.JPG",
                "Work/100726 SPO BGLF/100726 SPO BGLF-002.NEF"
            ]
        );
        assert!(plan.entries[0]
            .secondary
            .as_ref()
            .unwrap()
            .starts_with(t.path().join("Backup")));
        assert_eq!(plan.total_bytes, 3 * "data-DSC_0001.NEF".len() as u64);
    }

    #[test]
    fn companions_follow_the_photo_and_folders_can_be_preserved() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        let sub = src.join("DCIM").join("100NIKON");
        fs::create_dir_all(&sub).unwrap();
        for n in [
            "DSC_0001.NEF",
            "DSC_0001.WAV",
            "DSC_0001.XMP",
            "DSC_0002.NEF",
        ] {
            fs::write(sub.join(n), format!("data-{n}")).unwrap();
        }
        // A stray WAV with no photo stays on the card.
        fs::write(sub.join("MEMO_9.WAV"), b"x").unwrap();
        let opts = Options {
            source: src.clone(),
            primary: t.path().join("Work"),
            secondary: Some(t.path().join("Backup")),
            folder_template: "{headline}".into(),
            rename_template: "{headline}-{seqn}".into(),
            headline: Some("BDAY".into()),
            only_new: false,
            preserve_folders: true,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        assert!(plan.can_run(), "{:?}", plan.collisions);
        assert_eq!(plan.items, 2);
        assert_eq!(plan.companions, 2);
        let rel = |p: &Path| p.strip_prefix(t.path()).unwrap().display().to_string();
        let names: Vec<String> = plan.entries.iter().map(|e| rel(&e.primary)).collect();
        assert_eq!(
            names,
            vec![
                "Work/BDAY/DCIM/100NIKON/BDAY-001.NEF",
                "Work/BDAY/DCIM/100NIKON/BDAY-001.WAV",
                "Work/BDAY/DCIM/100NIKON/BDAY-001.XMP",
                "Work/BDAY/DCIM/100NIKON/BDAY-002.NEF",
            ]
        );
        assert!(plan.entries[1].companion && plan.entries[1].fingerprint.is_none());
        assert_eq!(
            rel(plan.entries[1].secondary.as_ref().unwrap()),
            "Backup/BDAY/DCIM/100NIKON/BDAY-001.WAV"
        );
        // Off: flat folders, photos only.
        let plain = build_plan(
            &Options {
                preserve_folders: false,
                copy_companions: false,
                ..opts.clone()
            },
            &Ledger::in_memory(),
        )
        .unwrap();
        assert_eq!(plain.companions, 0);
        assert_eq!(rel(&plain.entries[0].primary), "Work/BDAY/BDAY-001.NEF");
        assert_eq!(plain.entries.len(), 2);
    }

    #[test]
    fn a_suffixed_photo_takes_its_voice_memo_with_it() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        fs::write(src.path().join("DSC_0001.JPG"), b"one").unwrap();
        fs::write(src.path().join("DSC_0001.WAV"), b"memo").unwrap();
        fs::write(dst.path().join("DSC_0001.JPG"), b"old").unwrap();
        let opts = Options {
            source: src.path().to_path_buf(),
            primary: dst.path().to_path_buf(),
            folder_template: String::new(),
            only_new: false,
            on_collision: CollisionPolicy::Suffix,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        let names: Vec<String> = plan
            .entries
            .iter()
            .map(|e| {
                e.primary
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["DSC_0001-1.JPG", "DSC_0001-1.WAV"]);
    }

    #[test]
    fn a_taken_memo_name_moves_the_whole_photo_to_a_free_suffix() {
        // The photo's name is free but its memo's is not: with the suffix
        // policy both move to -1 instead of the plan being refused.
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        fs::write(src.path().join("DSC_0001.JPG"), b"one").unwrap();
        fs::write(src.path().join("DSC_0001.WAV"), b"memo").unwrap();
        fs::write(dst.path().join("DSC_0001.WAV"), b"old memo").unwrap();
        let opts = Options {
            source: src.path().to_path_buf(),
            primary: dst.path().to_path_buf(),
            folder_template: String::new(),
            only_new: false,
            on_collision: CollisionPolicy::Suffix,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        assert!(plan.can_run(), "{:?}", plan.collisions);
        let names: Vec<String> = plan
            .entries
            .iter()
            .map(|e| {
                e.primary
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["DSC_0001-1.JPG", "DSC_0001-1.WAV"]);
    }

    #[test]
    fn a_suffixed_pair_keeps_one_common_stem() {
        // Audit finding #4: with only SHOT.NEF at the destination, the
        // plan used to produce SHOT-1.NEF, SHOT.JPG, SHOT-1.XMP, splitting
        // the pair so the JPEG no longer scanned with its sidecar.
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for n in ["SHOT.NEF", "SHOT.JPG", "SHOT.XMP"] {
            fs::write(src.path().join(n), format!("data-{n}")).unwrap();
        }
        fs::write(dst.path().join("SHOT.NEF"), b"old raw").unwrap();
        let opts = Options {
            source: src.path().to_path_buf(),
            primary: dst.path().to_path_buf(),
            folder_template: String::new(),
            only_new: false,
            on_collision: CollisionPolicy::Suffix,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        assert!(plan.can_run(), "{:?}", plan.collisions);
        let names: Vec<String> = plan
            .entries
            .iter()
            .map(|e| {
                e.primary
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["SHOT-1.NEF", "SHOT-1.JPG", "SHOT-1.XMP"]);
        assert_eq!(plan.suffixed, 2);
    }

    #[test]
    fn options_saved_before_new_fields_still_load() {
        let old = r#"{"source":"/a","include_subfolders":true,"primary":"/b","secondary":null,
            "folder_template":"{headline}","rename_template":"","seq_start":1,"filter":"All",
            "only_new":true,"headline":null,"on_collision":"Refuse","day":null}"#;
        let o: Options = serde_json::from_str(old).unwrap();
        assert!(o.copy_companions && !o.preserve_folders);
        assert_eq!(o.folder_template, "{headline}");
    }

    #[test]
    fn example_names_match_the_planner_convention() {
        let opts = Options {
            folder_template: "{year4}/{month0} {monthname}/{mnn3} {day0} {headline}".into(),
            rename_template: "{headline}{seqn:3}-{month0}{day0}{yr2}".into(),
            headline: Some("RyansBday".into()),
            ..Default::default()
        };
        let facts = Facts {
            stem: "CAL_0735".into(),
            ext: "NEF".into(),
            date: Some((2026, 9, 26)),
            time: Some((19, 4, 0)),
            headline: None,
            ..Default::default()
        };
        let (folder, name) = example_names(&opts, &facts).unwrap();
        assert_eq!(folder, "2026/09 September/Sep 26 RyansBday");
        assert_eq!(name, "RyansBday001-092626.NEF");
        // Empty rename template keeps the original name; a template that
        // expands to nothing is an error, not a blank file name.
        let keep = Options {
            rename_template: String::new(),
            ..opts.clone()
        };
        assert_eq!(example_names(&keep, &facts).unwrap().1, "CAL_0735.NEF");
        let bad = Options {
            rename_template: "{headline}".into(),
            headline: None,
            ..opts
        };
        assert!(example_names(&bad, &facts).is_err());
    }

    #[test]
    fn day_filter_keeps_one_capture_day_and_reports_the_rest() {
        // Two plain JPEGs with no EXIF: date is None, so a day filter leaves both out
        // and reports them; without a filter both are planned and `days` is empty.
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("A.JPG"), b"a").unwrap();
        std::fs::write(src.path().join("B.JPG"), b"b").unwrap();
        let ledger = Ledger::default();
        let base = Options {
            source: src.path().to_path_buf(),
            primary: dst.path().to_path_buf(),
            folder_template: String::new(),
            only_new: false,
            ..Default::default()
        };
        let all = build_plan(&base, &ledger).unwrap();
        assert_eq!(all.entries.len(), 2);
        assert!(all.days.is_empty());
        let one = build_plan(
            &Options {
                day: Some((2026, 9, 26)),
                ..base.clone()
            },
            &ledger,
        )
        .unwrap();
        assert_eq!(one.entries.len(), 0);
        assert_eq!(one.other_days, 2);
    }

    #[test]
    fn suffix_policy_renames_instead_of_refusing() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("DSC_0001.JPG"), b"one").unwrap();
        std::fs::write(src.path().join("DSC_0002.JPG"), b"two").unwrap();
        // The first name is taken at the destination; the second is free.
        std::fs::write(dst.path().join("DSC_0001.JPG"), b"old").unwrap();
        let ledger = Ledger::default();
        let opts = Options {
            source: src.path().to_path_buf(),
            primary: dst.path().to_path_buf(),
            folder_template: String::new(),
            only_new: false,
            on_collision: CollisionPolicy::Suffix,
            ..Default::default()
        };
        let plan = build_plan(&opts, &ledger).unwrap();
        assert!(plan.collisions.is_empty(), "{:?}", plan.collisions);
        assert_eq!(plan.suffixed, 1);
        let names: Vec<String> = plan
            .entries
            .iter()
            .map(|e| {
                e.primary
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"DSC_0001-1.JPG".to_string()), "{names:?}");
        assert!(names.contains(&"DSC_0002.JPG".to_string()), "{names:?}");
        // Refuse is still the default and still refuses.
        let plan = build_plan(
            &Options {
                on_collision: CollisionPolicy::Refuse,
                ..opts
            },
            &ledger,
        )
        .unwrap();
        assert_eq!(plan.collisions.len(), 1);
    }

    #[test]
    fn refuses_collisions_nesting_and_honours_ledger() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("CARD");
        fs::create_dir_all(&src).unwrap();
        card(&src);
        // Existing file at the destination collides.
        let dest = t.path().join("Work");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("DSC_0001.NEF"), b"old").unwrap();
        let opts = Options {
            source: src.clone(),
            primary: dest.clone(),
            folder_template: String::new(),
            only_new: false,
            ..Default::default()
        };
        let plan = build_plan(&opts, &Ledger::in_memory()).unwrap();
        assert!(!plan.can_run());
        assert_eq!(plan.collisions.len(), 1);
        // Two sources to one name collide within the plan.
        let opts2 = Options {
            rename_template: "same".into(),
            primary: t.path().join("Other"),
            ..opts.clone()
        };
        let plan2 = build_plan(&opts2, &Ledger::in_memory()).unwrap();
        assert!(plan2
            .collisions
            .iter()
            .any(|c| c.reason.starts_with("two sources")));
        // Nested destination refused.
        let nested = Options {
            primary: src.join("sub"),
            ..opts.clone()
        };
        assert!(matches!(
            build_plan(&nested, &Ledger::in_memory()),
            Err(PlanError::Nested)
        ));
        // Ledger skips already-ingested files.
        let mut ledger = Ledger::in_memory();
        ledger.insert(fingerprint(&src.join("DSC_0002.NEF")).unwrap());
        let opts3 = Options {
            only_new: true,
            primary: t.path().join("Fresh"),
            ..opts.clone()
        };
        let plan3 = build_plan(&opts3, &ledger).unwrap();
        assert_eq!(plan3.entries.len(), 2);
        assert_eq!(plan3.skipped.len(), 1);
        assert_eq!(plan3.skipped[0].reason, "already ingested");
        // RAW-only filter.
        let opts4 = Options {
            filter: FileFilter::RawOnly,
            primary: t.path().join("RawOnly"),
            ..opts.clone()
        };
        assert_eq!(
            build_plan(&opts4, &Ledger::in_memory())
                .unwrap()
                .entries
                .len(),
            2
        );
    }
}
