//! Extract JPEG Previews from RAW Photos… (as in Photo Mechanic): the
//! largest JPEG inside each RAW becomes `name.jpg`, next to the RAW or in a
//! chosen folder. The picture's bytes are written exactly as extracted;
//! only metadata segments (Exif, XMP, IPTC-IIM, ICC) are spliced in after
//! the start marker. A free name is always found (`name-1.jpg`…); no file
//! is ever replaced, and the RAWs are only read.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rayon::prelude::*;

use apature_core::{ColorClassScheme, Kind, PhotoItem};
use apature_metadata::jpeg_xmp::{header_segments, is_exif_segment};
use apature_metadata::{edits, exif, exif_write, iim, iptc, items, xmp, XmpMarks};
use apature_preview::embedded::Prefer;

use crate::app1;

/// The dialog's checkboxes; all on by default except Extract to source folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExtractOptions {
    /// Copy IPTC/XMP Info: caption, keywords… from the sidecar or the RAW.
    pub copy_iptc: bool,
    /// Preserve Exif data where possible: camera, lens, exposure, date.
    pub keep_exif: bool,
    /// Preserve image preferences (Tag and Color Class, and the rating).
    pub keep_marks: bool,
    /// Embed ICC Profile: keep the preview's, or sRGB when it has none.
    pub embed_icc: bool,
    /// Write next to the RAW instead of asking for a folder.
    pub to_source: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        ExtractOptions {
            copy_iptc: true,
            keep_exif: true,
            keep_marks: true,
            embed_icc: true,
            to_source: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractProgress {
    pub done: usize,
    pub total: usize,
    pub current: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractOutcome {
    /// (RAW, JPEG written)
    pub extracted: Vec<(PathBuf, PathBuf)>,
    /// (photo, why it was skipped)
    pub skipped: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

/// The RAW file of a photo (its RAW half for a RAW+JPEG pair).
pub fn raw_of(item: &PhotoItem) -> Option<&Path> {
    item.variants()
        .find(|v| v.kind == Kind::Raw)
        .map(|v| v.path.as_path())
}

/// Extract every photo's preview; non-RAW photos are skipped. `dest` is
/// the folder for all of them, or `None` for each RAW's own folder.
///
/// Photos extract in parallel: the per-photo work (preview extraction,
/// metadata splice, write) is independent, and only the free-name
/// handout ([`FreeNames`]) and the progress callback are serialised.
/// Results are collected in input order, so the outcome lists read as
/// they did sequentially. Progress is reported as each photo finishes
/// (`done` = photos completed) instead of as each one starts.
pub fn run_extract(
    photos: &[PhotoItem],
    dest: Option<&Path>,
    opts: &ExtractOptions,
    scheme: &ColorClassScheme,
    cancel: &AtomicBool,
    on_progress: impl FnMut(&ExtractProgress) + Send,
) -> ExtractOutcome {
    let total = photos.len();
    let names = FreeNames::new();
    let done = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let on_progress = Mutex::new(on_progress);

    enum One {
        Done(PathBuf, PathBuf),
        Skipped(PathBuf, String),
        /// Never started: Cancel was already set when its turn came.
        NotStarted,
    }

    let results: Vec<One> = photos
        .par_iter()
        .map(|item| {
            let item_path = item.path().to_path_buf();
            if cancel.load(Ordering::Relaxed) {
                cancelled.store(true, Ordering::Relaxed);
                return One::NotStarted;
            }
            let one = match raw_of(item) {
                None => One::Skipped(item_path.clone(), "not a RAW photo".into()),
                Some(raw) => {
                    let dir = match dest {
                        Some(d) => d.to_path_buf(),
                        None => raw.parent().map(Path::to_path_buf).unwrap_or_default(),
                    };
                    match extract_one(item, raw, &dir, opts, scheme, &names) {
                        Ok(jpg) => One::Done(raw.to_path_buf(), jpg),
                        Err(why) => One::Skipped(raw.to_path_buf(), why),
                    }
                }
            };
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut cb) = on_progress.lock() {
                cb(&ExtractProgress {
                    done: n,
                    total,
                    current: item_path,
                });
            }
            one
        })
        .collect();

    let mut out = ExtractOutcome::default();
    for one in results {
        match one {
            One::Done(raw, jpg) => out.extracted.push((raw, jpg)),
            One::Skipped(p, why) => out.skipped.push((p, why)),
            One::NotStarted => {}
        }
    }
    out.cancelled = cancelled.load(Ordering::Relaxed);
    if !out.cancelled {
        if let Ok(mut cb) = on_progress.lock() {
            cb(&ExtractProgress {
                done: total,
                total,
                current: PathBuf::new(),
            });
        }
    }
    out
}

/// One RAW's preview into `dir`; returns the JPEG's path.
pub fn extract_one(
    item: &PhotoItem,
    raw: &Path,
    dir: &Path,
    opts: &ExtractOptions,
    scheme: &ColorClassScheme,
    names: &FreeNames,
) -> Result<PathBuf, String> {
    let (_, jpeg) = apature_preview::extract(raw, Prefer::Largest, usize::MAX)
        .map_err(|_| "no JPEG preview inside".to_string())?;
    if !apature_preview::jpeg_info::is_jpeg(&jpeg) {
        return Err("no JPEG preview inside".into());
    }
    let file = with_metadata(item, raw, &jpeg, opts, scheme)?;
    fs::create_dir_all(dir).map_err(|e| format!("can't create the folder: {e}"))?;
    let stem = raw
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "preview".into());
    let (path, mut f) = create_free(dir, &stem, names)?;
    let written = f.write_all(&file).and_then(|_| f.sync_all());
    if let Err(e) = written {
        drop(f);
        let _ = fs::remove_file(&path); // our own, just created
        return Err(format!("couldn't write: {e}"));
    }
    let when = capture_time(raw).or_else(|| fs::metadata(raw).ok().and_then(|m| m.modified().ok()));
    if let Some(t) = when {
        let mut times = FileTimes::new().set_modified(t).set_accessed(t);
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::FileTimesExt;
            let created = if capture_time(raw).is_some() {
                Some(t)
            } else {
                fs::metadata(raw).ok().and_then(|m| m.created().ok())
            };
            if let Some(c) = created {
                times = times.set_created(c);
            }
        }
        let _ = f.set_times(times);
    }
    Ok(path)
}

/// Free output names for one extraction run, shared across the parallel
/// workers. Each folder's listing is read ONCE, on first use, into a set
/// of lowercased names; every name handed out joins its folder's set, so
/// picking a name never rescans the folder (the old per-file `read_dir`
/// made bulk extraction quadratic in folder size).
pub struct FreeNames {
    state: Mutex<HashMap<PathBuf, HashSet<String>>>,
}

impl FreeNames {
    pub fn new() -> Self {
        FreeNames {
            state: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for FreeNames {
    fn default() -> Self {
        Self::new()
    }
}

/// `stem.jpg`, else `stem-1.jpg`…: created with create_new, so an existing
/// file (any case, on a case-insensitive disk) is never opened for writing.
/// The lock is held only for this check-and-create, not for the per-photo
/// decode and metadata work around it.
fn create_free(dir: &Path, stem: &str, names: &FreeNames) -> Result<(PathBuf, File), String> {
    let mut state = names.state.lock().unwrap();
    // Names unavailable in this folder, ignoring case even on a
    // case-sensitive disk, so name.jpg never lands beside a camera's
    // NAME.JPG: the on-disk snapshot plus this run's handouts.
    let used = state.entry(dir.to_path_buf()).or_insert_with(|| {
        fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().to_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    });
    for k in 0..10_000u32 {
        let name = if k == 0 {
            format!("{stem}.jpg")
        } else {
            format!("{stem}-{k}.jpg")
        };
        let lower = name.to_lowercase();
        if used.contains(&lower) {
            continue;
        }
        let path = dir.join(&name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => {
                used.insert(lower);
                return Ok((path, f));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Appeared since the snapshot (or a case twin of it);
                // remember it and try the next suffix.
                used.insert(lower);
                continue;
            }
            Err(e) => return Err(format!("can't create {name}: {e}")),
        }
    }
    Err("no free name".into())
}

/// The preview with our metadata segments after SOI (and its JFIF APP0).
/// The preview's own Exif is replaced (kept and re-oriented when Preserve
/// Exif is on), its XMP/IIM replaced only when we write those.
fn with_metadata(
    item: &PhotoItem,
    raw: &Path,
    jpeg: &[u8],
    opts: &ExtractOptions,
    scheme: &ColorClassScheme,
) -> Result<Vec<u8>, String> {
    let segs = header_segments(jpeg).map_err(|e| format!("unreadable preview: {e}"))?;
    let summary = exif::read(raw).unwrap_or_default();
    let file_o = summary.orientation.clamp(1, 8);
    // A rotation made in Apature (sidecar) wins, as for exports.
    let orientation = edits::read(&item.sidecar_path())
        .map(|e| e.orientation_or(file_o))
        .unwrap_or(file_o) as u16;
    let (w, h) = apature_preview::jpeg_info::dimensions(jpeg).unwrap_or((0, 0));
    let fields = if opts.copy_iptc {
        items::read_iptc(item)
    } else {
        Default::default()
    };

    let mut add: Vec<Vec<u8>> = Vec::new();
    let own_exif = segs.iter().find(|s| is_exif_segment(jpeg, s));
    let exif_block = if opts.keep_exif {
        let copied = own_exif
            .map(|s| s.payload(jpeg)[6..].to_vec())
            .and_then(|mut b| exif_write::set_orientation(&mut b, orientation).then_some(b));
        match copied {
            Some(b) => b,
            None => exif_write::minimal_oriented(
                &summary,
                w,
                h,
                fields
                    .creator
                    .as_ref()
                    .and_then(|c| c.first())
                    .map(String::as_str),
                fields.copyright.as_deref(),
                orientation,
            ),
        }
    } else {
        // No camera data, but the picture must still stand the right way up.
        exif_write::orientation_only(orientation)
    };
    add.push(app1(b"Exif\0\0", &exif_block).map_err(|e| e.to_string())?);

    let mut packet = String::new();
    if opts.copy_iptc && !fields.is_empty() {
        packet = iptc::apply("", &fields).map_err(|e| e.to_string())?;
    }
    if opts.keep_marks {
        let m = items::read_marks(item, scheme);
        packet =
            xmp::apply(&packet, &XmpMarks::from_marks(&m, scheme)).map_err(|e| e.to_string())?;
    }
    let write_xmp = !packet.is_empty();
    if write_xmp {
        add.push(app1(b"http://ns.adobe.com/xap/1.0/\0", packet.as_bytes()).map_err(|e| e.to_string())?);
    }
    let has_icc = segs
        .iter()
        .any(|s| s.marker == 0xE2 && s.payload(jpeg).starts_with(b"ICC_PROFILE\0"));
    if opts.embed_icc && !has_icc {
        add.push(icc_segment(&srgb_profile()));
    }
    let write_iim = opts.copy_iptc && !fields.is_empty();
    if write_iim {
        add.push(iim::app13_segment(&fields).map_err(|e| e.to_string())?);
    }

    let drop = |s: &apature_metadata::jpeg_xmp::Segment| -> bool {
        let p = s.payload(jpeg);
        is_exif_segment(jpeg, s)
            || (write_xmp && s.marker == 0xE1 && p.starts_with(b"http://ns.adobe.com/xap/1.0/\0"))
            || (write_iim && s.marker == 0xED)
    };
    // SOI, then the JFIF APP0 when it comes first, then ours, then the rest.
    let mut out = Vec::with_capacity(jpeg.len() + add.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&jpeg[..2]);
    let mut at = 2;
    if let Some(first) = segs.first().filter(|s| s.marker == 0xE0 && s.start == 2) {
        out.extend_from_slice(&jpeg[first.start..first.end]);
        at = first.end;
    }
    for a in &add {
        out.extend_from_slice(a);
    }
    let from = at;
    for s in segs.iter().filter(|s| s.start >= from) {
        if s.start > at {
            out.extend_from_slice(&jpeg[at..s.start]);
        }
        if !drop(s) {
            out.extend_from_slice(&jpeg[s.start..s.end]);
        }
        at = s.end;
    }
    out.extend_from_slice(&jpeg[at..]);
    Ok(out)
}

fn icc_segment(profile: &[u8]) -> Vec<u8> {
    let len = 2 + 12 + 2 + profile.len();
    let mut v = vec![0xFF, 0xE2];
    v.extend_from_slice(&(len as u16).to_be_bytes());
    v.extend_from_slice(b"ICC_PROFILE\0");
    v.extend_from_slice(&[1, 1]);
    v.extend_from_slice(profile);
    v
}

/// A compact ICC v2 sRGB display profile (IEC 61966-2-1 primaries adapted
/// to D50, 1024-point tone curve).
pub fn srgb_profile() -> Vec<u8> {
    fn s15(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }
    fn xyz(x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut d = b"XYZ \0\0\0\0".to_vec();
        for v in [x, y, z] {
            d.extend_from_slice(&s15(v));
        }
        d
    }
    let mut curv = b"curv\0\0\0\0".to_vec();
    curv.extend_from_slice(&1024u32.to_be_bytes());
    for i in 0..1024 {
        let v = i as f64 / 1023.0;
        let lin = if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        };
        curv.extend_from_slice(&((lin * 65535.0).round() as u16).to_be_bytes());
    }
    let name = b"sRGB IEC61966-2.1\0";
    let mut desc = b"desc\0\0\0\0".to_vec();
    desc.extend_from_slice(&(name.len() as u32).to_be_bytes());
    desc.extend_from_slice(name);
    desc.extend_from_slice(&[0; 8]); // Unicode language and count
    desc.extend_from_slice(&[0; 3]); // ScriptCode code and count
    desc.extend_from_slice(&[0; 67]);
    let mut cprt = b"text\0\0\0\0".to_vec();
    cprt.extend_from_slice(b"No copyright, use freely\0");
    let datas: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", desc),
        (b"cprt", cprt),
        (b"wtpt", xyz(0.9505, 1.0, 1.0891)),
        (b"rXYZ", xyz(0.4360747, 0.2225045, 0.0139322)),
        (b"gXYZ", xyz(0.3850649, 0.7168786, 0.0971045)),
        (b"bXYZ", xyz(0.1430804, 0.0606169, 0.7141733)),
        (b"rTRC", curv),
    ];
    let tags = datas.len() + 2; // gTRC and bTRC share rTRC's data
    let mut table = (tags as u32).to_be_bytes().to_vec();
    let mut body = Vec::new();
    let mut at = 128 + 4 + 12 * tags;
    let mut trc = (0, 0);
    for (sig, d) in &datas {
        table.extend_from_slice(*sig);
        table.extend_from_slice(&(at as u32).to_be_bytes());
        table.extend_from_slice(&(d.len() as u32).to_be_bytes());
        if *sig == b"rTRC" {
            trc = (at, d.len());
        }
        body.extend_from_slice(d);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        at = 128 + 4 + 12 * tags + body.len();
    }
    for sig in [b"gTRC", b"bTRC"] {
        table.extend_from_slice(sig);
        table.extend_from_slice(&(trc.0 as u32).to_be_bytes());
        table.extend_from_slice(&(trc.1 as u32).to_be_bytes());
    }
    let size = 128 + table.len() + body.len();
    let mut p = Vec::with_capacity(size);
    p.extend_from_slice(&(size as u32).to_be_bytes());
    p.extend_from_slice(&[0; 4]); // CMM
    p.extend_from_slice(&[2, 0x10, 0, 0]); // version 2.1
    p.extend_from_slice(b"mntrRGB XYZ ");
    for v in [2026u16, 1, 1, 0, 0, 0] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    p.extend_from_slice(b"acsp");
    p.extend_from_slice(&[0; 24]); // platform, flags, manufacturer, model, attributes
    p.extend_from_slice(&[0; 4]); // perceptual
    for v in [0.9642, 1.0, 0.8249] {
        p.extend_from_slice(&s15(v));
    }
    p.resize(128, 0);
    p.extend_from_slice(&table);
    p.extend_from_slice(&body);
    p
}

/// The RAW's DateTimeOriginal as a local time.
fn capture_time(raw: &Path) -> Option<SystemTime> {
    let s = exif::read(raw).ok()?.date_time_original?;
    local_time(&s)
}

/// "YYYY:MM:DD HH:MM:SS" (camera local time) → SystemTime.
fn local_time(s: &str) -> Option<SystemTime> {
    let n: Vec<i32> = s
        .split([':', ' ', '-', 'T'])
        .filter(|p| !p.is_empty())
        .take(6)
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [y, mo, d, h, mi, se] = n[..] else {
        return None;
    };
    if !(1971..=9999).contains(&y) || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    #[cfg(unix)]
    let secs = {
        // SAFETY: mktime only reads/normalises the struct we own.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = y - 1900;
        tm.tm_mon = mo - 1;
        tm.tm_mday = d;
        tm.tm_hour = h;
        tm.tm_min = mi;
        tm.tm_sec = se;
        tm.tm_isdst = -1;
        let t = unsafe { libc::mktime(&mut tm) };
        if t < 0 {
            return None;
        }
        t as u64
    };
    #[cfg(not(unix))]
    let secs = {
        // Days from civil (UTC; no local zone lookup off Unix).
        let (y2, m2) = if mo <= 2 {
            (y - 1, mo + 9)
        } else {
            (y, mo - 3)
        };
        let era = y2.div_euclid(400);
        let yoe = y2 - era * 400;
        let doy = (153 * m2 + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = (era * 146097 + doe - 719468) as i64;
        (days * 86400 + (h * 3600 + mi * 60 + se) as i64) as u64
    };
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apature_core::tiff::{tag, Tiff};
    use apature_core::{ColorClass, Marks, Rating};

    fn preview_jpeg() -> Vec<u8> {
        let mut data = Vec::with_capacity(400 * 200 * 3);
        for _y in 0..200 {
            for x in 0..400 {
                let v = if x < 200 { 0 } else { 255 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let img = apature_preview::Rgb {
            width: 400,
            height: 200,
            data,
        };
        apature_preview::decode::encode_jpeg(&img, 90).unwrap()
    }

    /// A NEF-shaped TIFF: Make, Model, Orientation 6, a capture date in the
    /// Exif IFD and the preview as JPEGInterchangeFormat.
    fn fake_nef(path: &Path, jpeg: &[u8]) {
        let le32 = |v: u32| v.to_le_bytes();
        let entries: [(u16, u16, u32, [u8; 4]); 6] = [
            (tag::MAKE, 2, 6, le32(100)),
            (tag::MODEL, 2, 6, le32(120)),
            (tag::ORIENTATION, 3, 1, [6, 0, 0, 0]),
            (tag::JPEG_INTERCHANGE_FORMAT, 4, 1, le32(256)),
            (
                tag::JPEG_INTERCHANGE_FORMAT_LENGTH,
                4,
                1,
                le32(jpeg.len() as u32),
            ),
            (tag::EXIF_IFD, 4, 1, le32(150)),
        ];
        let mut v = apature_core::tiff::testutil::build_le(&entries, b"NIKON\0", 100);
        v.resize(120, 0);
        v.extend_from_slice(b"Z 6_2\0");
        v.resize(150, 0);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&tag::DATE_TIME_ORIGINAL.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&20u32.to_le_bytes());
        v.extend_from_slice(&200u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.resize(200, 0);
        v.extend_from_slice(b"2026:10:06 14:03:11\0");
        v.resize(256, 0);
        v.extend_from_slice(jpeg);
        fs::write(path, v).unwrap();
    }

    fn shoot(dir: &Path) -> Vec<PhotoItem> {
        apature_core::scan(dir, &apature_core::ScanOptions::default()).unwrap()
    }

    fn run(items: &[PhotoItem], dest: Option<&Path>, opts: &ExtractOptions) -> ExtractOutcome {
        run_extract(
            items,
            dest,
            opts,
            &ColorClassScheme::default(),
            &AtomicBool::new(false),
            |_| {},
        )
    }

    #[test]
    fn preview_bytes_kept_and_metadata_carried_over() {
        let d = tempfile::tempdir().unwrap();
        let jpeg = preview_jpeg();
        let raw = d.path().join("DSC_0001.NEF");
        fake_nef(&raw, &jpeg);
        // Caption and marks in the RAW's sidecar.
        let scheme = ColorClassScheme::default();
        let ip = apature_metadata::Iptc {
            headline: Some("Pirates win".into()),
            keywords: Some(vec!["baseball".into(), "PNC Park".into()]),
            ..Default::default()
        };
        let marks = Marks {
            rating: Rating::clamp(4),
            color_class: ColorClass::clamp(2),
            tagged: true,
        };
        let text = xmp::apply(
            &iptc::apply("", &ip).unwrap(),
            &XmpMarks::from_marks(&marks, &scheme),
        )
        .unwrap();
        fs::write(d.path().join("DSC_0001.xmp"), text).unwrap();
        let raw_before = fs::read(&raw).unwrap();
        let items = shoot(d.path());

        let out = run(&items, None, &ExtractOptions::default());
        assert!(out.skipped.is_empty(), "{:?}", out.skipped);
        let jpg = &out.extracted[0].1;
        assert_eq!(jpg, &d.path().join("DSC_0001.jpg"));
        let got = fs::read(jpg).unwrap();

        // The image data is the preview's, byte for byte (from its first
        // non-APPn segment on).
        let tail = |b: &[u8]| {
            let s = header_segments(b).unwrap();
            let end = s
                .iter()
                .filter(|s| (0xE0..=0xEF).contains(&s.marker))
                .map(|s| s.end)
                .max()
                .unwrap_or(2);
            b[end..].to_vec()
        };
        assert_eq!(tail(&got), tail(&jpeg));
        // Exif: camera, date, orientation 6 (the preview is stored sideways).
        let ex = exif::from_bytes(&got).expect("exif");
        assert_eq!(ex.model.as_deref(), Some("Z 6_2"));
        assert_eq!(
            ex.date_time_original.as_deref(),
            Some("2026:10:06 14:03:11")
        );
        assert_eq!(ex.orientation, 6);
        // XMP: caption, keywords, rating, colour class, tag.
        let packet = xmp::embedded_packet_in_jpeg(&got).expect("xmp");
        let back = iptc::parse(packet);
        assert_eq!(back.headline.as_deref(), Some("Pirates win"));
        assert_eq!(
            back.keywords.as_deref(),
            Some(&["baseball".to_string(), "PNC Park".to_string()][..])
        );
        assert_eq!(xmp::parse(packet).to_marks(&scheme), marks);
        // IIM and an sRGB profile.
        let segs = header_segments(&got).unwrap();
        assert!(segs.iter().any(|s| s.marker == 0xED));
        let icc = segs
            .iter()
            .find(|s| s.marker == 0xE2)
            .expect("icc")
            .payload(&got);
        assert!(icc.starts_with(b"ICC_PROFILE\0\x01\x01"));
        let prof = &icc[14..];
        assert_eq!(
            u32::from_be_bytes(prof[..4].try_into().unwrap()) as usize,
            prof.len()
        );
        assert_eq!(&prof[36..40], b"acsp");
        // Dated like the capture.
        let m = fs::metadata(jpg).unwrap().modified().unwrap();
        assert_eq!(Some(m), local_time("2026:10:06 14:03:11"));
        // The RAW is untouched.
        assert_eq!(fs::read(&raw).unwrap(), raw_before);
    }

    #[test]
    fn unchecked_boxes_leave_things_out_but_keep_orientation() {
        let d = tempfile::tempdir().unwrap();
        let raw = d.path().join("A.NEF");
        fake_nef(&raw, &preview_jpeg());
        fs::write(
            d.path().join("A.xmp"),
            iptc::apply(
                "",
                &apature_metadata::Iptc {
                    headline: Some("x".into()),
                    ..Default::default()
                },
            )
            .unwrap(),
        )
        .unwrap();
        let opts = ExtractOptions {
            copy_iptc: false,
            keep_exif: false,
            keep_marks: false,
            embed_icc: false,
            to_source: true,
        };
        let out = run(&shoot(d.path()), None, &opts);
        let got = fs::read(&out.extracted[0].1).unwrap();
        let ex = exif::from_bytes(&got).expect("orientation block");
        assert_eq!(ex.orientation, 6);
        assert!(ex.model.is_none() && ex.date_time_original.is_none());
        assert!(xmp::embedded_packet_in_jpeg(&got).is_none());
        let segs = header_segments(&got).unwrap();
        assert!(!segs.iter().any(|s| s.marker == 0xED || s.marker == 0xE2));
        // The parse check: Exif block is a valid TIFF.
        let (t, first) = Tiff::parse(
            &segs
                .iter()
                .find(|s| is_exif_segment(&got, s))
                .unwrap()
                .payload(&got)[6..],
        )
        .unwrap();
        assert!(t.read_ifd(first).is_ok());
    }

    #[test]
    fn never_overwrites_and_skips_non_raw() {
        let d = tempfile::tempdir().unwrap();
        let jpeg = preview_jpeg();
        let raw = d.path().join("B.NEF");
        fake_nef(&raw, &jpeg);
        // A RAW+JPEG pair: B.JPG already exists (any case counts).
        fs::write(d.path().join("B.JPG"), b"camera jpeg").unwrap();
        fs::write(d.path().join("B-1.jpg"), b"someone else's").unwrap();
        fs::write(d.path().join("solo.jpg"), &jpeg).unwrap();
        let raw_before = fs::read(&raw).unwrap();
        let items = shoot(d.path());
        let out = run(&items, None, &ExtractOptions::default());
        assert_eq!(out.extracted.len(), 1);
        assert_eq!(out.extracted[0].1.file_name().unwrap(), "B-2.jpg");
        assert_eq!(fs::read(d.path().join("B.JPG")).unwrap(), b"camera jpeg");
        assert_eq!(
            fs::read(d.path().join("B-1.jpg")).unwrap(),
            b"someone else's"
        );
        // solo.jpg and B-1.jpg are JPEG-only photos.
        assert_eq!(out.skipped.len(), 2);
        assert!(out.skipped.iter().all(|(_, why)| why == "not a RAW photo"));
        assert_eq!(fs::read(&raw).unwrap(), raw_before);
        // Again into a chosen folder: first B.jpg, then B-1.jpg.
        let to = d.path().join("previews");
        let raw_items: Vec<PhotoItem> = items.into_iter().filter(|i| raw_of(i).is_some()).collect();
        assert_eq!(
            run(&raw_items, Some(&to), &ExtractOptions::default()).extracted[0].1,
            to.join("B.jpg")
        );
        assert_eq!(
            run(&raw_items, Some(&to), &ExtractOptions::default()).extracted[0].1,
            to.join("B-1.jpg")
        );
        assert_eq!(fs::read(&raw).unwrap(), raw_before);
    }

    #[test]
    fn a_raw_without_a_preview_is_skipped_with_a_reason() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("C.NEF"), b"II*\0\x08\0\0\0\0\0\0\0\0\0").unwrap();
        let out = run(&shoot(d.path()), None, &ExtractOptions::default());
        assert!(out.extracted.is_empty());
        assert_eq!(out.skipped[0].1, "no JPEG preview inside");
        assert!(!d.path().join("C.jpg").exists());
    }
}
