//! `apature-cli`: the engine without the window. Useful for testing format support
//! and for benchmarking without GUI overhead.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;

use apature_core::{ColorClass, ColorClassScheme, Marks, Rating, ScanOptions};
use apature_metadata::items;
use apature_preview::{Prefer, Source, ThumbCache};

#[derive(Parser)]
#[command(name = "apature-cli", version, about = "Apature command line")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the photos in a folder with RAW+JPEG pairing and sidecar marks.
    Scan {
        dir: PathBuf,
        #[arg(long)]
        recursive: bool,
    },
    /// List the JPEG previews embedded in a file.
    Previews { file: PathBuf },
    /// Write the largest embedded preview out as a JPEG without re-encoding.
    Extract {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Pull the camera's own JPEG preview out of a RAW file.
    Preview {
        #[command(subcommand)]
        cmd: PreviewCmd,
    },
    /// Print the EXIF summary for a file.
    Exif { file: PathBuf },
    /// Generate thumbnails for a whole folder and report timings.
    Bench {
        dir: PathBuf,
        /// Long side of the generated thumbnail in pixels.
        #[arg(long, default_value_t = 320)]
        size: u32,
        /// Use (and populate) the on-disk thumbnail cache.
        #[arg(long)]
        cache: bool,
        /// Worker threads (default: all cores).
        #[arg(long)]
        threads: Option<usize>,
        /// Include subfolders.
        #[arg(long)]
        recursive: bool,
    },
    /// Set rating / colour class / tag on a file's XMP sidecar.
    Mark {
        file: PathBuf,
        #[arg(long)]
        rating: Option<i8>,
        #[arg(long)]
        class: Option<u8>,
        #[arg(long)]
        tag: Option<bool>,
    },
    /// Verified copy of one file (refuses to overwrite).
    Copy { src: PathBuf, dst: PathBuf },
    /// Apply an assignment file's IPTC pad (and nothing else) to photos.
    Pad {
        /// The key: value assignment file.
        #[arg(long)]
        assignment: PathBuf,
        files: Vec<PathBuf>,
    },
    /// Set the caption (and optionally headline/keywords) on photos, expanding
    /// \code\ replacements from the assignment.
    Caption {
        #[arg(long)]
        assignment: Option<PathBuf>,
        #[arg(long)]
        text: String,
        #[arg(long)]
        headline: Option<String>,
        /// Semicolon-separated.
        #[arg(long)]
        keywords: Option<String>,
        files: Vec<PathBuf>,
    },
    /// Print the IPTC fields of a photo.
    Iptc { file: PathBuf },
    /// Turn photos a quarter turn, losslessly (tiff:Orientation in the sidecar).
    Rotate {
        /// Counter-clockwise instead of clockwise.
        #[arg(long)]
        ccw: bool,
        /// Back to the camera's own orientation.
        #[arg(long, conflicts_with = "ccw")]
        reset: bool,
        files: Vec<PathBuf>,
    },
    /// Store a crop in the sidecar (crs:Crop*, fractions of the stored image),
    /// applied on export.
    Crop {
        /// top,left,bottom,right as fractions 0..1, e.g. 0.1,0.1,0.9,0.9
        #[arg(
            long,
            value_delimiter = ',',
            allow_hyphen_values = true,
            required_unless_present = "clear"
        )]
        rect: Option<Vec<f32>>,
        #[arg(long, conflicts_with = "rect")]
        clear: bool,
        files: Vec<PathBuf>,
    },
    /// Plan and run a verified ingest (sequential; never deletes the source).
    /// Export delivery JPEGs (resize, re-encode, EXIF + XMP + IPTC-IIM + ICC).
    Export {
        /// Files or folders.
        paths: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        /// Long side in pixels (0 = keep size).
        #[arg(long, default_value_t = 2048)]
        long: u32,
        #[arg(long, default_value_t = 90)]
        quality: u8,
        /// Naming template, e.g. "100726-SPO-BGLF-{seqn:3}" (default keeps names).
        #[arg(long)]
        rename: Option<String>,
        /// Assignment file whose fields fill in anything the photo lacks.
        #[arg(long)]
        assignment: Option<PathBuf>,
        #[arg(long)]
        no_iim: bool,
        #[arg(long)]
        no_exif: bool,
        /// Render RAW files with the macOS RAW engine instead of the embedded preview.
        #[arg(long)]
        decode_raw: bool,
        /// Write JPEG XL (.jxl) via cjxl: a lossless transcode of the JPEG.
        #[arg(long)]
        jxl: bool,
        #[arg(long)]
        overwrite: bool,
    },
    /// Rename photos in place with a template ({seqn} counts from --seq-start).
    Rename {
        paths: Vec<PathBuf>,
        #[arg(long)]
        template: String,
        #[arg(long, default_value_t = 1)]
        seq_start: u32,
        #[arg(long)]
        headline: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Move photos (all variants and sidecars) to the Trash.
    Trash { paths: Vec<PathBuf> },
    /// Send files to a desk over FTP, FTPS or SFTP. Password from --password
    /// or the APATURE_PASSWORD environment variable.
    Deliver {
        files: Vec<PathBuf>,
        /// Not needed for --protocol folder.
        #[arg(long, default_value = "")]
        host: String,
        #[arg(long, default_value = "")]
        user: String,
        /// ftp, ftps, sftp or folder (a mounted share or cloud sync folder).
        #[arg(long, default_value = "sftp")]
        protocol: String,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "")]
        dir: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
        #[arg(long)]
        password: Option<String>,
    },
    Ingest {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        primary: PathBuf,
        #[arg(long)]
        secondary: Option<PathBuf>,
        /// Folder template under each destination, e.g. "{headline}" or "{date}-{headline}".
        #[arg(long, default_value = "{headline}")]
        folder: String,
        /// File name template, e.g. "{headline}-{seqn}" (empty keeps names).
        #[arg(long, default_value = "")]
        rename: String,
        #[arg(long)]
        headline: Option<String>,
        /// Copy everything, ignoring the "already ingested" ledger.
        #[arg(long)]
        all: bool,
        /// Only print the plan.
        #[arg(long)]
        dry_run: bool,
        /// Add -1, -2… to a file name that is already taken instead of refusing.
        #[arg(long)]
        suffix: bool,
        /// Only photos captured on this day, e.g. 2026-09-26.
        #[arg(long)]
        day: Option<String>,
        /// Keep the card's subfolders (DCIM/100NIKON…) under the folder template.
        #[arg(long)]
        preserve_folders: bool,
        /// Leave voice memos (.WAV) and card sidecars (.XMP) behind.
        #[arg(long)]
        no_companions: bool,
    },
    /// Print where the thumbnail cache lives.
    CacheDir,
}

#[derive(Subcommand)]
enum PreviewCmd {
    /// Write one embedded preview out as a JPEG without re-encoding
    /// (or list them all with --list).
    Extract {
        file: PathBuf,
        /// largest (default), smallest, or a pixel count: the smallest
        /// preview at least that long on its long side.
        #[arg(long, default_value = "largest")]
        prefer: String,
        /// Output path; defaults to the input name with "-preview.jpg".
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Skip previews bigger than this many bytes.
        #[arg(long, default_value_t = apature_preview::DEFAULT_MAX_BYTES)]
        max_bytes: usize,
        /// List every embedded preview instead of writing one.
        #[arg(long)]
        list: bool,
    },
    /// List every embedded preview, largest first.
    List { file: PathBuf },
}

fn parse_prefer(s: &str) -> Result<Prefer> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "largest" | "large" => Prefer::Largest,
        "smallest" | "small" => Prefer::Smallest,
        n => Prefer::AtLeast(n.parse().with_context(|| {
            format!("--prefer: expected largest, smallest or a number, got {s:?}")
        })?),
    })
}

fn preview_cmd(cmd: PreviewCmd) -> Result<()> {
    match cmd {
        PreviewCmd::List { file }
        | PreviewCmd::Extract {
            file, list: true, ..
        } => previews(&file),
        PreviewCmd::Extract {
            file,
            prefer,
            out,
            max_bytes,
            list: false,
        } => {
            let prefer = parse_prefer(&prefer)?;
            let (c, bytes) = apature_preview::extract(&file, prefer, max_bytes)?;
            let out = out.unwrap_or_else(|| {
                let stem = file.file_stem().unwrap_or_default().to_string_lossy();
                file.with_file_name(format!("{stem}-preview.jpg"))
            });
            if out.exists() {
                bail!(
                    "{} already exists; pass --out to choose another path",
                    out.display()
                );
            }
            std::fs::write(&out, &bytes)?;
            println!(
                "wrote {}x{} ({} bytes) from {} to {}",
                c.width,
                c.height,
                c.length,
                c.source,
                out.display()
            );
            Ok(())
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Scan { dir, recursive } => scan(&dir, recursive),
        Cmd::Previews { file } => previews(&file),
        Cmd::Extract { file, out } => extract(&file, &out),
        Cmd::Preview { cmd } => preview_cmd(cmd),
        Cmd::Exif { file } => exif(&file),
        Cmd::Bench {
            dir,
            size,
            cache,
            threads,
            recursive,
        } => bench(&dir, size, cache, threads, recursive),
        Cmd::Mark {
            file,
            rating,
            class,
            tag,
        } => mark(&file, rating, class, tag),
        Cmd::Copy { src, dst } => {
            let r = apature_ingest::copy_verified(&src, &dst)?;
            println!(
                "copied {} bytes to {} (sha256 {})",
                r.bytes,
                r.destination.display(),
                r.sha256
            );
            Ok(())
        }
        Cmd::Pad { assignment, files } => pad(&assignment, &files),
        Cmd::Caption {
            assignment,
            text,
            headline,
            keywords,
            files,
        } => caption(assignment.as_deref(), &text, headline, keywords, &files),
        Cmd::Export {
            paths,
            out,
            long,
            quality,
            rename,
            assignment,
            no_iim,
            no_exif,
            decode_raw,
            jxl,
            overwrite,
        } => cmd_export(
            paths, out, long, quality, rename, assignment, no_iim, no_exif, decode_raw, jxl,
            overwrite,
        ),
        Cmd::Deliver {
            files,
            host,
            user,
            protocol,
            port,
            dir,
            key_file,
            password,
        } => {
            use apature_deliver::{send, Event, Kind, Target};
            let kind = match protocol.to_lowercase().as_str() {
                "ftp" => Kind::Ftp,
                "ftps" => Kind::Ftps,
                "sftp" => Kind::Sftp,
                "folder" => Kind::Folder,
                other => anyhow::bail!("--protocol must be ftp, ftps, sftp or folder, not {other}"),
            };
            if kind != Kind::Folder && (host.is_empty() || user.is_empty()) {
                anyhow::bail!("--host and --user are needed for {protocol}");
            }
            let target = Target {
                name: host.clone(),
                kind,
                host,
                port: port.unwrap_or(Target::default_port(kind)),
                user,
                dir,
                key_file,
            };
            let secret = password
                .or_else(|| std::env::var("APATURE_PASSWORD").ok())
                .unwrap_or_default();
            let rep = send(&target, &secret, &files, |e| match e {
                Event::Sent { index, bytes } => {
                    println!("  ✓ {} ({bytes} bytes)", files[index].display())
                }
                Event::Failed { index, error } => {
                    println!("  ✕ {} — {error}", files[index].display())
                }
                _ => {}
            })?;
            println!(
                "sent {} of {} files, {:.1} MB in {:.1} s",
                rep.sent,
                files.len(),
                rep.bytes as f64 / 1e6,
                rep.elapsed_secs
            );
            if !rep.failed.is_empty() {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Rename {
            paths,
            template,
            seq_start,
            headline,
            dry_run,
        } => cmd_rename(paths, template, seq_start, headline, dry_run),
        Cmd::Trash { paths } => cmd_trash(paths),
        Cmd::Rotate { ccw, reset, files } => cmd_rotate(ccw, reset, &files),
        Cmd::Crop { rect, clear, files } => cmd_crop(rect, clear, &files),
        Cmd::Ingest {
            source,
            primary,
            secondary,
            folder,
            rename,
            headline,
            all,
            dry_run,
            suffix,
            day,
            preserve_folders,
            no_companions,
        } => ingest(
            source,
            primary,
            secondary,
            folder,
            rename,
            headline,
            all,
            dry_run,
            suffix,
            day,
            preserve_folders,
            !no_companions,
        ),
        Cmd::Iptc { file } => {
            let item = item_for(&file)?;
            let i = items::read_iptc(&item);
            println!("{}", serde_json::to_string_pretty(&i)?);
            Ok(())
        }
        Cmd::CacheDir => {
            println!("{}", cache_dir()?.display());
            Ok(())
        }
    }
}

fn cache_dir() -> Result<PathBuf> {
    let dirs =
        apature_core::project_dirs().context("no home directory")?;
    Ok(dirs.cache_dir().join("thumbs"))
}

fn scan(dir: &Path, recursive: bool) -> Result<()> {
    let t0 = Instant::now();
    let items = apature_core::scan(dir, &ScanOptions { recursive })?;
    let scheme = ColorClassScheme::default();
    for it in &items {
        let src = items::read(it);
        let m = src.effective(it, &scheme);
        let pair = if it.is_pair() {
            format!(
                " +{}",
                it.secondary
                    .iter()
                    .map(|v| v.file_name())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        } else {
            String::new()
        };
        let has_any = |m: &apature_metadata::XmpMarks| {
            m.rating.is_some() || m.label.is_some() || m.color_class.is_some() || m.tagged.is_some()
        };
        let marked = src.sidecar.as_ref().is_some_and(has_any)
            || src.embedded.iter().any(|(_, m)| has_any(m));
        let mtext = if marked {
            format!(
                "  rating={} class={} tagged={}",
                m.rating.0, m.color_class.0, m.tagged
            )
        } else {
            String::new()
        };
        println!("{:>5}  {}{}{}", it.id, it.display_name(), pair, mtext);
        for (p, c) in src.conflicts(items::JpegMarks::default()) {
            println!(
                "       CONFLICT {}: sidecar={} embedded({})={}",
                c.field,
                c.sidecar,
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                c.embedded
            );
        }
    }
    eprintln!(
        "{} items in {:.1} ms",
        items.len(),
        t0.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

fn previews(file: &Path) -> Result<()> {
    let t0 = Instant::now();
    let src = Source::open(file)?;
    let dt = t0.elapsed();
    if src.candidates().is_empty() {
        bail!("no embedded JPEG found in {}", file.display());
    }
    println!(
        "{} ({} bytes), indexed in {:.2} ms",
        file.display(),
        src.len(),
        dt.as_secs_f64() * 1000.0
    );
    for c in src.candidates() {
        println!(
            "  {:>5}x{:<5} {:>9} bytes @ {:<10} {}",
            c.width, c.height, c.length, c.offset, c.source
        );
    }
    Ok(())
}

fn extract(file: &Path, out: &Path) -> Result<()> {
    let src = Source::open(file)?;
    let c = src.largest().context("no embedded JPEG")?;
    std::fs::write(out, src.bytes(c))?;
    println!(
        "wrote {}x{} ({} bytes) from {} to {}",
        c.width,
        c.height,
        c.length,
        c.source,
        out.display()
    );
    Ok(())
}

fn exif(file: &Path) -> Result<()> {
    let s = apature_metadata::exif::read(file)?;
    println!("camera:      {}", s.camera_line());
    println!("lens:        {}", s.lens.as_deref().unwrap_or("-"));
    println!(
        "captured:    {}",
        s.date_time_original.as_deref().unwrap_or("-")
    );
    println!("exposure:    {}", s.exposure_line());
    println!("orientation: {}", s.orientation);
    Ok(())
}

fn bench(
    dir: &Path,
    size: u32,
    use_cache: bool,
    threads: Option<usize>,
    recursive: bool,
) -> Result<()> {
    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok();
    }
    let t0 = Instant::now();
    let items = apature_core::scan(dir, &ScanOptions { recursive })?;
    let scan_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let cache = if use_cache {
        Some(ThumbCache::new(cache_dir()?)?)
    } else {
        None
    };
    let hits = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);

    let t1 = Instant::now();
    let mut times: Vec<f64> = items
        .par_iter()
        .map(|it| {
            let t = Instant::now();
            let key = ThumbCache::key(it.path(), it.primary.size, it.primary.modified, size);
            if let Some(c) = &cache {
                if c.get(&key).is_some() {
                    hits.fetch_add(1, Ordering::Relaxed);
                    return t.elapsed().as_secs_f64() * 1000.0;
                }
            }
            let result = (|| -> Result<()> {
                let src = Source::open(it.path())?;
                let orientation = apature_metadata::exif::from_bytes(src.data())
                    .map(|e| e.orientation)
                    .unwrap_or(1);
                let rgb = src.decode_for(size * 2)?;
                let rgb = apature_preview::rotate(rgb, orientation);
                let thumb = apature_preview::thumbnail(&rgb, size);
                if let Some(c) = &cache {
                    c.put(&key, &thumb)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("{}: {e}", it.display_name());
            }
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let total = t1.elapsed().as_secs_f64();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| {
        times
            .get(((times.len() as f64 * p) as usize).min(times.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0)
    };
    println!("folder:     {}", dir.display());
    println!("items:      {}  (scan {scan_ms:.1} ms)", items.len());
    println!("threads:    {}", rayon::current_num_threads());
    println!(
        "thumb size: {size}px  cache: {}",
        if use_cache { "on" } else { "off" }
    );
    println!(
        "wall:       {:.2} s  ({:.1} items/s)",
        total,
        items.len() as f64 / total.max(1e-9)
    );
    println!(
        "per item:   p50 {:.1} ms  p95 {:.1} ms  max {:.1} ms",
        pct(0.5),
        pct(0.95),
        times.last().copied().unwrap_or(0.0)
    );
    println!(
        "cache hits: {}  errors: {}",
        hits.load(Ordering::Relaxed),
        errors.load(Ordering::Relaxed)
    );
    Ok(())
}

/// Resolve a file to its item so a pair gets both the sidecar and the JPEG.
///
/// The scan behind this runs once per directory per run: the batch commands
/// (rotate, crop, pad, caption) resolve every file through here, and
/// re-scanning the folder for each file made them quadratic in folder size.
fn item_for(file: &Path) -> Result<apature_core::PhotoItem> {
    use std::collections::hash_map::Entry;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    let dir = file.parent().context("file has no parent directory")?;
    let stem = file
        .file_stem()
        .context("file has no stem")?
        .to_string_lossy()
        .to_lowercase();
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, HashMap<String, apature_core::PhotoItem>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap();
    let map = match cache.entry(dir.to_path_buf()) {
        Entry::Occupied(e) => e.into_mut(),
        Entry::Vacant(e) => {
            let items = apature_core::scan(dir, &ScanOptions::default())?;
            let mut map = HashMap::with_capacity(items.len());
            for it in items {
                // First item with a stem wins, as the old linear find did.
                map.entry(it.stem.to_lowercase()).or_insert(it);
            }
            e.insert(map)
        }
    };
    map.get(&stem)
        .cloned()
        .with_context(|| format!("{} is not a photo in {}", file.display(), dir.display()))
}

#[allow(clippy::too_many_arguments)]
fn ingest(
    source: PathBuf,
    primary: PathBuf,
    secondary: Option<PathBuf>,
    folder: String,
    rename: String,
    headline: Option<String>,
    all: bool,
    dry_run: bool,
    suffix: bool,
    day: Option<String>,
    preserve_folders: bool,
    copy_companions: bool,
) -> Result<()> {
    use apature_ingest::{build_plan, run, CollisionPolicy, Event, Ledger, Options};
    use std::time::Instant;
    let t_start = Instant::now();
    let day = match day {
        Some(s) => Some(
            apature_metadata::codes::parse_date(&s.replace('-', ":"))
                .ok_or_else(|| anyhow::anyhow!("--day wants YYYY-MM-DD, got {s}"))?,
        ),
        None => None,
    };
    let ledger_path = apature_core::project_dirs()
        .context("no home directory")?
        .data_dir()
        .join("ingested.json");
    let mut ledger = Ledger::load(&ledger_path);
    let opts = Options {
        source,
        primary,
        secondary,
        folder_template: folder,
        rename_template: rename,
        headline,
        only_new: !all,
        on_collision: if suffix {
            CollisionPolicy::Suffix
        } else {
            CollisionPolicy::Refuse
        },
        day,
        preserve_folders,
        copy_companions,
        ..Default::default()
    };
    let t_ledger = t_start.elapsed().as_secs_f64();
    let t0 = Instant::now();
    let plan = build_plan(&opts, &ledger)?;
    let t_plan = t0.elapsed().as_secs_f64();
    println!(
        "plan: {} files in {} photos ({} memos/sidecars), {:.1} MB, {} skipped, {} collisions",
        plan.entries.len(),
        plan.items,
        plan.companions,
        plan.total_bytes as f64 / 1e6,
        plan.skipped.len(),
        plan.collisions.len()
    );
    for c in &plan.collisions {
        println!("  COLLISION {} — {}", c.destination.display(), c.reason);
    }
    for e in plan.entries.iter().take(5) {
        println!("  {} → {}", e.source.display(), e.primary.display());
    }
    if dry_run {
        eprintln!(
            "timing: ledger load {t_ledger:.2} s, plan {t_plan:.2} s, total {:.2} s",
            t_start.elapsed().as_secs_f64()
        );
        return Ok(());
    }
    if !plan.can_run() {
        bail!("plan cannot run");
    }
    let t1 = Instant::now();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let report = run(&plan, &mut ledger, &cancel, |ev| match ev {
        Event::Verified { destination, .. } => println!("  ✓ {}", destination.display()),
        Event::Failed {
            destination, error, ..
        } => println!("  ✕ {} — {error}", destination.display()),
        _ => {}
    });
    println!(
        "{}: {} ok, {} failed, {:.1} MB in {:.1} s",
        if report.all_verified(&plan) {
            "every file verified at every destination"
        } else {
            "INCOMPLETE"
        },
        report.files_ok(),
        report.files_failed(),
        report.bytes_copied as f64 / 1e6,
        report.elapsed_secs
    );
    // Where the wall-clock time goes (the copy figure above includes the
    // ledger save; the rest is scanning, EXIF and fingerprints in the plan).
    eprintln!(
        "timing: ledger load {t_ledger:.2} s, plan {t_plan:.2} s, copy+verify+ledger save {:.2} s, total {:.2} s",
        t1.elapsed().as_secs_f64(),
        t_start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn cmd_rotate(ccw: bool, reset: bool, files: &[PathBuf]) -> Result<()> {
    use apature_metadata::edits;
    for f in files {
        let item = item_for(f)?;
        let sidecar = item.sidecar_path();
        let file_o = apature_metadata::exif::read(item.path())
            .map(|e| e.orientation)
            .unwrap_or(1);
        let now = edits::read(&sidecar)?.orientation_or(file_o);
        let next = if reset {
            None
        } else if ccw {
            Some(edits::rotate_ccw(now) as u8)
        } else {
            Some(edits::rotate_cw(now) as u8)
        };
        edits::write(
            &sidecar,
            &edits::Change {
                orientation: Some(next),
                ..Default::default()
            },
        )?;
        println!(
            "{}: orientation {now} → {}",
            item.display_name(),
            next.map(u32::from).unwrap_or(file_o)
        );
    }
    Ok(())
}

fn cmd_crop(rect: Option<Vec<f32>>, clear: bool, files: &[PathBuf]) -> Result<()> {
    use apature_metadata::edits::{self, Crop};
    let crop = match (clear, rect.as_deref()) {
        (true, _) => None,
        (false, Some(&[top, left, bottom, right])) => Some(
            Crop {
                top,
                left,
                bottom,
                right,
            }
            .normalized()
            .context("--rect wants top<bottom and left<right, each 0..1")?,
        ),
        _ => bail!("--rect wants four numbers: top,left,bottom,right"),
    };
    for f in files {
        let item = item_for(f)?;
        edits::write(
            &item.sidecar_path(),
            &edits::Change {
                crop: Some(crop),
                ..Default::default()
            },
        )?;
        match crop {
            Some(c) => println!(
                "{}: crop top {:.3} left {:.3} bottom {:.3} right {:.3}",
                item.display_name(),
                c.top,
                c.left,
                c.bottom,
                c.right
            ),
            None => println!("{}: crop cleared", item.display_name()),
        }
    }
    Ok(())
}

fn pad(assignment: &Path, files: &[PathBuf]) -> Result<()> {
    let a = apature_metadata::stationery::Assignment::parse(&std::fs::read_to_string(assignment)?)?;
    let fields = a.iptc();
    for f in files {
        let item = item_for(f)?;
        let w = items::write_iptc(&item, &fields, items::JpegMarks::default())?;
        println!("{}: pad applied → {} file(s)", item.display_name(), w.len());
    }
    Ok(())
}

fn caption(
    assignment: Option<&Path>,
    text: &str,
    headline: Option<String>,
    keywords: Option<String>,
    files: &[PathBuf],
) -> Result<()> {
    use apature_metadata::codes::{self, Vars};
    let a = match assignment {
        Some(p) => Some(apature_metadata::stationery::Assignment::parse(
            &std::fs::read_to_string(p)?,
        )?),
        None => None,
    };
    let codes_table = a.as_ref().map(|a| a.codes()).unwrap_or_default();
    for f in files {
        let item = item_for(f)?;
        let current = items::read_iptc(&item);
        let exif_date = apature_metadata::exif::read(item.path())
            .ok()
            .and_then(|e| e.date_time_original);
        let date = current
            .date_created
            .as_deref()
            .and_then(codes::parse_date)
            .or_else(|| exif_date.as_deref().and_then(codes::parse_date));
        let vars = Vars {
            date,
            credit: current
                .credit
                .clone()
                .or_else(|| a.as_ref().and_then(|a| a.credit_line())),
            headline: current.headline.clone(),
        };
        let expanded = codes::expand(text, &codes_table, &vars, '\\');
        let fields = apature_metadata::Iptc {
            description: Some(expanded.clone()),
            headline: headline.clone(),
            keywords: keywords.as_ref().map(|k| {
                k.split(';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }),
            ..Default::default()
        };
        let w = items::write_iptc(&item, &fields, items::JpegMarks::default())?;
        println!(
            "{}: {expanded}\n  → {} file(s)",
            item.display_name(),
            w.len()
        );
    }
    Ok(())
}

fn mark(file: &Path, rating: Option<i8>, class: Option<u8>, tag: Option<bool>) -> Result<()> {
    let item = item_for(file)?;
    let scheme = ColorClassScheme::default();
    let current = items::read_marks(&item, &scheme);
    let next = Marks {
        rating: rating.map(Rating::clamp).unwrap_or(current.rating),
        color_class: class.map(ColorClass::clamp).unwrap_or(current.color_class),
        tagged: tag.unwrap_or(current.tagged),
    };
    let written = items::write(&item, &next, &scheme, items::JpegMarks::default())?;
    for p in &written {
        println!("wrote {}", p.display());
    }
    println!(
        "{}: rating={} class={} ({}) tagged={}",
        item.display_name(),
        next.rating.0,
        next.color_class.0,
        scheme.name(next.color_class).unwrap_or("none"),
        next.tagged
    );
    Ok(())
}

/// Photos named by `paths`: a folder means every photo in it, a file means
/// the photo (pair) it belongs to.
fn items_for_paths(paths: Vec<PathBuf>) -> Result<Vec<apature_core::PhotoItem>> {
    let mut items: Vec<apature_core::PhotoItem> = Vec::new();
    let mut dirs: Vec<(PathBuf, Option<Vec<PathBuf>>)> = Vec::new();
    for p in paths {
        if p.is_dir() {
            dirs.push((p, None));
        } else if let Some(parent) = p.parent() {
            let parent = parent.to_path_buf();
            match dirs.iter_mut().find(|(d, f)| *d == parent && f.is_some()) {
                Some((_, Some(f))) => f.push(p.clone()),
                _ => dirs.push((parent, Some(vec![p.clone()]))),
            }
        }
    }
    for (dir, only) in dirs {
        let scanned = apature_core::scan(&dir, &apature_core::ScanOptions::default())?;
        for it in scanned {
            let wanted = match &only {
                None => true,
                Some(files) => it.variants().any(|v| files.iter().any(|f| f == &v.path)),
            };
            if wanted {
                items.push(it);
            }
        }
    }
    Ok(items)
}

/// Rename photos in place with a template; every variant and the sidecar move together.
fn cmd_rename(
    paths: Vec<PathBuf>,
    template: String,
    seq_start: u32,
    headline: Option<String>,
    dry_run: bool,
) -> Result<()> {
    use apature_ingest::plan::facts_for;
    use apature_ingest::template::expand;
    let items = items_for_paths(paths)?;
    if items.is_empty() {
        anyhow::bail!("nothing to rename");
    }
    let stems: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let facts = facts_for(it, it.path(), headline.as_deref());
            expand(&template, &facts, seq_start + i as u32)
        })
        .collect();
    let pairs: Vec<(&apature_core::PhotoItem, String)> = items.iter().zip(stems).collect();
    let plan = apature_core::fileops::plan_rename(&pairs)?;
    for (from, to) in &plan.moves {
        println!(
            "  {}  →  {}",
            from.display(),
            to.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    for (p, why) in &plan.collisions {
        println!("  ✕ {}  {why}", p.display());
    }
    if !plan.can_run() {
        anyhow::bail!("{} collision(s); nothing renamed", plan.collisions.len());
    }
    if dry_run {
        println!("dry run: {} files would be renamed", plan.moves.len());
        return Ok(());
    }
    let done = apature_core::fileops::rename(&plan)?;
    println!("renamed {} files", done.len());
    Ok(())
}

/// Move photos (every variant and the sidecar) to the Trash.
fn cmd_trash(paths: Vec<PathBuf>) -> Result<()> {
    let items = items_for_paths(paths)?;
    let files: Vec<PathBuf> = items
        .iter()
        .flat_map(apature_core::fileops::item_files)
        .collect();
    for f in &files {
        println!("  → Trash: {}", f.display());
    }
    apature_core::fileops::trash(&files)?;
    println!(
        "moved {} photos ({} files) to the Trash",
        items.len(),
        files.len()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_export(
    paths: Vec<PathBuf>,
    out: PathBuf,
    long: u32,
    quality: u8,
    rename: Option<String>,
    assignment: Option<PathBuf>,
    no_iim: bool,
    no_exif: bool,
    decode_raw: bool,
    jxl: bool,
    overwrite: bool,
) -> Result<()> {
    use apature_ingest::plan::facts_for;
    use apature_ingest::template::expand;
    let pad = match assignment {
        Some(p) => Some(apature_metadata::stationery::Assignment::parse(
            &std::fs::read_to_string(p)?,
        )?),
        None => None,
    };
    let items = items_for_paths(paths)?;
    if items.is_empty() {
        anyhow::bail!("nothing to export");
    }
    let settings = apature_export::Settings {
        long_side: long,
        quality,
        keep_exif: !no_exif,
        write_iim: !no_iim,
        overwrite,
        raw: if decode_raw {
            apature_export::RawPixels::SystemDecode
        } else {
            apature_export::RawPixels::EmbeddedPreview
        },
        format: if jxl {
            apature_export::Format::JpegXl
        } else {
            apature_export::Format::Jpeg
        },
        ..Default::default()
    };
    let headline = pad.as_ref().map(|a| a.slug.clone());
    // Names first (sequence numbers follow the listed order), then export in parallel.
    let jobs: Vec<(apature_core::PhotoItem, PathBuf, apature_metadata::Iptc)> = items
        .into_iter()
        .enumerate()
        .map(|(i, it)| {
            let mut facts = facts_for(&it, it.path(), headline.as_deref());
            facts.ext = "jpg".into();
            let name = match &rename {
                Some(t) => expand(t, &facts, (i + 1) as u32),
                None => format!("{}.jpg", facts.stem),
            };
            let name = if name.to_lowercase().ends_with(".jpg") {
                name
            } else {
                format!("{name}.jpg")
            };
            let file_iptc = apature_metadata::items::read_iptc(&it);
            let iptc = match &pad {
                Some(a) => a.iptc().overlay(&file_iptc),
                None => file_iptc,
            };
            let dest = out.join(name);
            (it, dest, iptc)
        })
        .collect();
    let t0 = std::time::Instant::now();
    let results: Vec<std::result::Result<apature_export::Outcome, apature_export::ExportError>> =
        jobs.par_iter()
            .map(|(it, dest, iptc)| apature_export::export(it, iptc, &settings, dest))
            .collect();
    let mut ok = 0;
    for r in &results {
        match r {
            Ok(o) => {
                ok += 1;
                println!(
                    "  ✓ {}  {}×{}  {:.1} MB  pixels:{:?} exif:{:?}  {} ms",
                    o.out.display(),
                    o.width,
                    o.height,
                    o.bytes as f64 / 1e6,
                    o.pixels,
                    o.exif,
                    o.millis
                );
            }
            Err(e) => println!("  ✕ {e}"),
        }
    }
    println!(
        "{ok} of {} exported in {:.1} s",
        results.len(),
        t0.elapsed().as_secs_f32()
    );
    if ok != results.len() {
        std::process::exit(1);
    }
    Ok(())
}
