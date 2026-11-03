//! Installing an update in place, the way Alic does: no trip to the Finder.
//!
//! The payload is the release's disk image, which is what the releases carry.
//! It is downloaded to a temporary folder, its SHA-256 is checked against the
//! digest the release notes publish, the app is taken out of it with `ditto`,
//! and the copy is checked to be Apature before it counts. A running bundle
//! cannot be replaced while it runs, so the last step is a small shell
//! script: it waits for this process to quit, moves the old app aside, puts the
//! new build at the same path, deletes the old app, opens the new app and only
//! then lets go of the disk image. `Progress::Installed` is the signal for the
//! app to quit.
//!
//! The script writes every step to `apature.log` (Help › Reveal Log), so an
//! update that fails can be read afterwards. A failure before the swap leaves
//! the old app untouched and reopens it; a failed copy or a new app that will
//! not open puts the old one back, because nothing is deleted until the new
//! app has opened.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

/// The bundle identifier the download must carry, so another app served from
/// the same URL is never put in place of Apature.
const IDENTIFIER: &str = "dev.mccal.apature";

/// How long the swap waits for the old app to quit before replacing it anyway.
const QUIT_WAIT_TENTHS: u32 = 600;

#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// `total` is None when the server sends no length.
    Downloading {
        got: u64,
        total: Option<u64>,
    },
    /// Taking the app out of the disk image.
    Unpacking,
    /// The swap script is written and has been started.
    Swapping,
    /// The new app is staged and the script is running; quit so it can swap.
    Installed,
    Failed(String),
}

/// A running install: progress arrives from its own thread.
pub struct Installer {
    rx: Receiver<Progress>,
}

impl Installer {
    /// Download `update` and stage it for the swap. Fails at once, before
    /// anything is downloaded, when this build cannot be replaced (not a
    /// bundle, or a folder it cannot write to).
    pub fn spawn(update: &crate::updater::Update, ctx: egui::Context) -> Result<Installer, String> {
        let url = update.dmg.clone().ok_or_else(|| {
            "the release has no disk image to install from; use the release page".to_string()
        })?;
        // A download nobody can check is not installed: every release built
        // by `release.yml` publishes a SHA-256, so a missing one means this is
        // older or hand-made, and the release page is the honest answer.
        let expected = update.sha256.clone().ok_or_else(|| {
            format!(
                "Apature {} does not publish a SHA-256 for its disk image, so the download can't be checked; install it from the release page",
                update.version
            )
        })?;
        Self::from_url(url, Some(expected), ctx)
    }

    /// The same pipeline for a disk image at a URL (`--install-update URL`).
    /// `expected` is the digest the release published; `None` means nobody
    /// published one, which the release path never allows.
    pub fn from_url(
        url: String,
        expected: Option<String>,
        ctx: egui::Context,
    ) -> Result<Installer, String> {
        Self::start(
            target_bundle()?,
            move |work, report| {
                let dmg = work.join("Apature.dmg");
                download(&url, &dmg, report)?;
                if let Some(expected) = expected {
                    verify_digest(&dmg, &expected)?;
                }
                stage_from_image(&dmg, work, report)
            },
            ctx,
        )
    }

    /// The same pipeline for a disk image already on this Mac (`--install-update`).
    pub fn from_image(image: PathBuf, ctx: egui::Context) -> Result<Installer, String> {
        Self::start(
            target_bundle()?,
            move |work, report| stage_from_image(&image, work, report),
            ctx,
        )
    }

    fn start(
        target: PathBuf,
        prepare: impl FnOnce(&Path, &mut dyn FnMut(Progress)) -> Result<Staged, String> + Send + 'static,
        ctx: egui::Context,
    ) -> Result<Installer, String> {
        let work = std::env::temp_dir().join(format!("apature-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
        let (tx, rx) = channel();
        let report = move |p: Progress| {
            let _ = tx.send(p);
            ctx.request_repaint();
        };
        let thread = std::thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let mut report = report;
                match prepare(&work, &mut report) {
                    Ok(Staged { app, mount }) => {
                        let swap = Swap {
                            pid: std::process::id(),
                            target,
                            staged: app,
                            mount,
                            work: work.clone(),
                            log: log_path(&work),
                        };
                        report(Progress::Swapping);
                        match start_swap(&swap) {
                            Ok(()) => report(Progress::Installed),
                            Err(e) => {
                                let _ = std::fs::remove_dir_all(&work);
                                report(Progress::Failed(e));
                            }
                        }
                    }
                    Err(e) => {
                        // Nothing is going to be installed: the download goes
                        // with the failure.
                        let _ = std::fs::remove_dir_all(&work);
                        report(Progress::Failed(e));
                    }
                }
            });
        thread
            .map(|_| Installer { rx })
            .map_err(|e| format!("could not start the update: {e}"))
    }

    pub fn try_recv(&self) -> Option<Progress> {
        self.rx.try_recv().ok()
    }
}

/// The new app, out of its disk image, and the image's mount point.
struct Staged {
    app: PathBuf,
    mount: PathBuf,
}

/// What the swap script needs to know.
#[derive(Clone, Debug)]
struct Swap {
    /// The app process the script waits for (this one).
    pid: u32,
    /// The bundle to replace, in place.
    target: PathBuf,
    /// The new bundle, already out of the disk image.
    staged: PathBuf,
    /// Where that image is mounted; detached once the app is in place.
    mount: PathBuf,
    /// Temporary folder with the download and the copy; deleted at the end.
    work: PathBuf,
    /// The app's own log, where the script reports each step.
    log: PathBuf,
}

fn log_path(work: &Path) -> PathBuf {
    apature_core::project_dirs()
        .map(|d| d.data_dir().join("apature.log"))
        .unwrap_or_else(|| work.join("swap.log"))
}

/// The `.app` bundle this process runs from, from its executable path.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let ok = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    ok.then(|| bundle.to_path_buf())
}

/// The bundle to replace, or why this build cannot replace itself.
fn target_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("where am I? {e}"))?;
    target_for(&exe, writable)
}

/// `target_bundle` without the environment: `probe` stands in for the
/// writability test, so every refusal can be checked without a real bundle.
fn target_for(
    exe: &Path,
    probe: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Err(
            "Apature is running from a temporary copy macOS made of it. Move it to Applications, open it there, and try again"
                .into(),
        );
    }
    let bundle = bundle_of(exe).ok_or_else(|| {
        "this build is not inside an Apature.app bundle, so there is nothing to replace; use the release page"
            .to_string()
    })?;
    let parent = bundle
        .parent()
        .ok_or_else(|| format!("{} has no folder to write to", bundle.display()))?;
    probe(parent).map_err(|e| {
        format!(
            "{} can't be written to ({e}); move Apature to Applications and try again",
            parent.display()
        )
    })?;
    Ok(bundle)
}

/// Can this process replace files in `dir`? A real write, because permission
/// bits, ACLs and read-only volumes all have to agree.
fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".apature-update-{}", std::process::id()));
    std::fs::write(&probe, b"apature").map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

fn stage_from_image(
    image: &Path,
    work: &Path,
    report: &mut dyn FnMut(Progress),
) -> Result<Staged, String> {
    report(Progress::Unpacking);
    let mount = work.join("mnt");
    std::fs::create_dir_all(&mount).map_err(|e| format!("{}: {e}", mount.display()))?;
    run(
        "hdiutil",
        &[
            OsStr::new("attach"),
            OsStr::new("-readonly"),
            OsStr::new("-nobrowse"),
            OsStr::new("-mountpoint"),
            mount.as_os_str(),
            image.as_os_str(),
        ],
    )?;
    let staged = work.join("Apature.app");
    // The image stays mounted while the app quits and the swap runs: letting
    // go of it can take a while, and that must not hold the window up. The
    // swap script detaches it once the new app is in place.
    match stage_app(&mount, &staged).and_then(|()| check_apature(&staged)) {
        Ok(()) => Ok(Staged { app: staged, mount }),
        Err(e) => {
            detach(&mount);
            Err(e)
        }
    }
}

/// The download must be the bytes the release published. A digest that does
/// not match means something else was served, and nothing is installed.
fn verify_digest(image: &Path, expected: &str) -> Result<(), String> {
    let got =
        apature_ingest::sha256_file(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let expected = expected.trim();
    if !got.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "the download is not the file the release publishes (SHA-256 {got}, expected {expected}); nothing was changed"
        ));
    }
    Ok(())
}

fn stage_app(mount: &Path, staged: &Path) -> Result<(), String> {
    let src = app_in(mount)?;
    run("ditto", &[src.as_os_str(), staged.as_os_str()]).map(|_| ())
}

/// The copy must be Apature: a wrong file at the URL never goes in place.
fn check_apature(app: &Path) -> Result<(), String> {
    let plist = std::fs::read_to_string(app.join("Contents/Info.plist"))
        .map_err(|e| format!("the download has no Info.plist: {e}"))?;
    match bundle_identifier(&plist) {
        Some(id) if id == IDENTIFIER => Ok(()),
        Some(id) => Err(format!(
            "the download holds \"{id}\", not Apature; nothing was changed"
        )),
        None => Err("the download is not an Apature app; nothing was changed".into()),
    }
}

/// Best effort: give the disk image back.
fn detach(mount: &Path) {
    let _ = run(
        "hdiutil",
        &[
            OsStr::new("detach"),
            mount.as_os_str(),
            OsStr::new("-force"),
        ],
    );
}

/// The one `.app` in a mounted disk image.
fn app_in(dir: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() == Some(OsStr::new("app")) && path.is_dir() {
            return Ok(path);
        }
    }
    Err(format!("{} holds no app to install", dir.display()))
}

/// `CFBundleIdentifier` from an Info.plist, read as text: the file is XML and
/// the key appears once.
fn bundle_identifier(plist: &str) -> Option<String> {
    let after_key = &plist[plist.find("<key>CFBundleIdentifier</key>")?..];
    let start = after_key.find("<string>")? + "<string>".len();
    let len = after_key[start..].find("</string>")?;
    Some(after_key[start..start + len].trim().to_string())
}

fn run(program: &str, args: &[&OsStr]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(format!(
            "{program} failed{}",
            if err.is_empty() {
                String::new()
            } else {
                format!(": {err}")
            }
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn download(url: &str, dest: &Path, report: &mut dyn FnMut(Progress)) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", concat!("Apature/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut file = std::fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut body = response.body_mut().with_config().limit(u64::MAX).reader();
    let mut buf = [0u8; 64 * 1024];
    let mut got = 0u64;
    let mut reported = 0u64;
    report(Progress::Downloading { got, total });
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        got += n as u64;
        // A report per 128 KiB keeps the window responsive without flooding it.
        if got - reported >= 128 * 1024 || total == Some(got) {
            reported = got;
            report(Progress::Downloading { got, total });
        }
    }
    file.flush()
        .map_err(|e| format!("{}: {e}", dest.display()))?;
    if let Some(total) = total {
        if got != total {
            return Err(format!(
                "the download stopped early ({got} of {total} bytes); nothing was changed"
            ));
        }
    }
    Ok(())
}

fn start_swap(swap: &Swap) -> Result<(), String> {
    let script = swap.work.join("swap.sh");
    std::fs::write(&script, swap_script(swap)).map_err(|e| format!("{}: {e}", script.display()))?;
    let mut perms = std::fs::metadata(&script)
        .map_err(|e| format!("{}: {e}", script.display()))?
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    let _ = std::fs::set_permissions(&script, perms);
    if let Some(dir) = swap.log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&swap.log)
        .map_err(|e| format!("{}: {e}", swap.log.display()))?;
    let err = out
        .try_clone()
        .map_err(|e| format!("{}: {e}", swap.log.display()))?;
    std::process::Command::new("/bin/bash")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(out))
        .stderr(std::process::Stdio::from(err))
        .spawn()
        .map_err(|e| format!("could not start the swap: {e}"))?;
    Ok(())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The script that does the swap, once this process is gone.
fn swap_script(swap: &Swap) -> String {
    format!(
        r#"#!/bin/bash
# Apature update swap. Written by Apature when it installs an update: it waits
# for the old app to quit, puts the new build in its place, deletes the old app,
# opens the new app and lets go of the disk image. Every step is logged to the
# app's own log, so an update that failed can be read afterwards.

PID={pid}
TARGET={target}
NEW={new}
MOUNT={mount}
WORK={work}
LOG={log}
OPEN_CMD="${{APATURE_SWAP_OPEN:-/usr/bin/open}}"

# The app this script replaces is also the terminal that started it for a
# launch from a shell; a hang-up must not stop the swap.
trap '' HUP

log() {{ printf '%s %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*" >>"$LOG"; }}

# Best effort: the image can take a while to let go, and nothing waits on it.
unmount() {{
  if [ -n "$MOUNT" ]; then
    hdiutil detach "$MOUNT" -force >>"$LOG" 2>&1 || true
  fi
}}

log "waiting for Apature ($PID) to quit"
for _ in $(seq 1 {wait}); do
  kill -0 "$PID" 2>/dev/null || break
  sleep 0.1
done
log "Apature ($PID) has quit; replacing $TARGET"

OLD="$(dirname "$TARGET")/.$(basename "$TARGET").old.$$"
if ! mv "$TARGET" "$OLD" 2>>"$LOG"; then
  log "could not move the old app aside; it is untouched"
  unmount
  rm -rf "$WORK"
  "$OPEN_CMD" "$TARGET"
  exit 1
fi
log "old app moved aside to $OLD"

if ! ditto "$NEW" "$TARGET" 2>>"$LOG"; then
  log "could not put the new app in place; putting the old one back"
  rm -rf "$TARGET"
  mv "$OLD" "$TARGET"
  unmount
  rm -rf "$WORK"
  "$OPEN_CMD" "$TARGET"
  exit 1
fi
log "new app in place at $TARGET"

# The old app is kept until the new one opens: a copy that will not launch
# must leave something working behind. `open` answers once LaunchServices has
# taken the request, so this is when the new app counts as launched, not a
# promise that it will stay up.
if ! "$OPEN_CMD" "$TARGET" 2>>"$LOG"; then
  log "the new app did not open; putting the old one back"
  rm -rf "$TARGET"
  mv "$OLD" "$TARGET"
  unmount
  rm -rf "$WORK"
  "$OPEN_CMD" "$TARGET"
  exit 1
fi
log "new app opened at $TARGET"

rm -rf "$OLD"
log "old app deleted"

# Out of sight of the new app: let go of the image, then remove the download,
# the copy and this script.
( unmount; rm -rf "$WORK" ) >/dev/null 2>&1 &
exit 0
"#,
        pid = swap.pid,
        target = shell_quote(&swap.target.to_string_lossy()),
        new = shell_quote(&swap.staged.to_string_lossy()),
        mount = shell_quote(&swap.mount.to_string_lossy()),
        work = shell_quote(&swap.work.to_string_lossy()),
        log = shell_quote(&swap.log.to_string_lossy()),
        wait = QUIT_WAIT_TENTHS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_is_found_from_the_executable() {
        let exe = Path::new("/Applications/Apature.app/Contents/MacOS/Apature");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/Apature.app"))
        );
        // A plain binary (cargo run, the CLI) is not a bundle.
        assert_eq!(
            bundle_of(Path::new("/Users/me/apature/target/debug/apature")),
            None
        );
        assert_eq!(
            bundle_of(Path::new("/Applications/Apature.app/Contents/MacOS")),
            None
        );
        assert_eq!(bundle_of(Path::new("Apature")), None);
    }

    #[test]
    fn a_translocated_app_says_how_to_fix_it() {
        let exe = Path::new(
            "/private/var/folders/x/AppTranslocation/ABC-123/d/Apature.app/Contents/MacOS/Apature",
        );
        let err = target_for(exe, |_| Ok(())).expect_err("translocated");
        assert!(err.contains("Move it to Applications"), "{err}");
    }

    #[test]
    fn a_plain_binary_has_nothing_to_replace() {
        let err = target_for(Path::new("/tmp/apature"), |_| Ok(())).expect_err("no bundle");
        assert!(err.contains("release page"), "{err}");
    }

    #[test]
    fn a_read_only_folder_names_the_folder() {
        let exe = Path::new("/Volumes/Apature 0.1.0/Apature.app/Contents/MacOS/Apature");
        let err = target_for(exe, |_| Err("read-only file system".into())).expect_err("read only");
        assert!(err.contains("/Volumes/Apature 0.1.0"), "{err}");
        assert!(err.contains("read-only file system"), "{err}");
        assert_eq!(
            target_for(exe, |_| Ok(())),
            Ok(PathBuf::from("/Volumes/Apature 0.1.0/Apature.app"))
        );
    }

    #[test]
    fn the_identifier_is_read_from_the_plist() {
        let plist =
            "<dict>\n  <key>CFBundleIdentifier</key> <string>dev.mccal.apature</string>\n</dict>";
        assert_eq!(
            bundle_identifier(plist).as_deref(),
            Some("dev.mccal.apature")
        );
        assert_eq!(bundle_identifier("<dict></dict>"), None);
        assert_eq!(bundle_identifier("<key>CFBundleIdentifier</key>"), None);
    }

    #[test]
    fn the_script_quotes_what_it_is_given() {
        let swap = Swap {
            pid: 4242,
            target: PathBuf::from("/Applications/That's Apature.app"),
            staged: PathBuf::from("/tmp/apature-update-4242/Apature.app"),
            mount: PathBuf::from("/tmp/apature-update-4242/mnt"),
            work: PathBuf::from("/tmp/apature-update-4242"),
            log: PathBuf::from("/Users/me/Library/That's Apature.log"),
        };
        let script = swap_script(&swap);
        assert!(script.starts_with("#!/bin/bash"), "{script}");
        assert!(script.contains("PID=4242"), "{script}");
        assert!(
            script.contains(r"TARGET='/Applications/That'\''s Apature.app'"),
            "{script}"
        );
        assert!(
            script.contains("NEW='/tmp/apature-update-4242/Apature.app'"),
            "{script}"
        );
        assert!(
            script.contains("MOUNT='/tmp/apature-update-4242/mnt'"),
            "{script}"
        );
        assert!(
            script.contains("WORK='/tmp/apature-update-4242'"),
            "{script}"
        );
        assert!(
            script.contains(r"LOG='/Users/me/Library/That'\''s Apature.log'"),
            "{script}"
        );
        // The old app is replaced where it stands and then deleted.
        assert!(script.contains(r#"mv "$TARGET" "$OLD""#), "{script}");
        assert!(script.contains(r#"ditto "$NEW" "$TARGET""#), "{script}");
        assert!(script.contains(r#"rm -rf "$OLD""#), "{script}");
        assert!(script.contains(r#""$OPEN_CMD" "$TARGET""#), "{script}");
        // The old app outlives the new one opening: nothing is deleted first.
        let opens = script.find(r#""$OPEN_CMD" "$TARGET""#).expect("open");
        let deletes = script.find(r#"rm -rf "$OLD""#).expect("delete");
        assert!(
            opens < deletes,
            "the old app is deleted only after the new one opens"
        );
        // Nothing is left mounted or downloaded.
        assert!(
            script.contains(r#"hdiutil detach "$MOUNT" -force"#),
            "{script}"
        );
        assert!(script.contains(r#"rm -rf "$WORK""#), "{script}");
    }

    /// Runs the real script over fake bundles: the old app is replaced in
    /// place and deleted, and a failed copy puts it back.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_swap_replaces_the_old_app_and_deletes_it() {
        let root = std::env::temp_dir().join(format!("apature-swap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("Applications");
        let work = root.join("work");
        let target = apps.join("Apature.app");
        let staged = work.join("Apature.app");
        for (dir, mark) in [(&target, "old"), (&staged, "new")] {
            std::fs::create_dir_all(dir.join("Contents/MacOS")).expect("bundle");
            std::fs::write(dir.join("Contents/MacOS/mark.txt"), mark).expect("marker");
        }
        let log = root.join("apature.log");
        let swap = Swap {
            // A pid that is not running: the wait ends at once.
            pid: 4_000_000,
            target: target.clone(),
            staged,
            // No image behind this test: nothing to detach.
            mount: PathBuf::new(),
            work: work.clone(),
            log: log.clone(),
        };
        let script = work.join("swap.sh");
        std::fs::write(&script, swap_script(&swap)).expect("script");
        let status = std::process::Command::new("/bin/bash")
            .arg(&script)
            // No Finder in a test: `true` stands in for `open`.
            .env("APATURE_SWAP_OPEN", "/usr/bin/true")
            .status()
            .expect("run the swap");
        let written = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(status.success(), "swap failed: {written}");
        let marker =
            std::fs::read_to_string(target.join("Contents/MacOS/mark.txt")).expect("new app");
        assert_eq!(marker, "new", "the new app is in place");
        // The old app is gone from the folder it lived in.
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&apps)
            .expect("apps")
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(leftovers, vec![target.clone()], "only the new app is left");
        assert!(written.contains("old app deleted"), "{written}");
        assert!(written.contains("new app opened"), "{written}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A copy that cannot be made puts the old app back where it was.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_copy_restores_the_old_app() {
        let root =
            std::env::temp_dir().join(format!("apature-swap-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("Applications");
        let work = root.join("work");
        let target = apps.join("Apature.app");
        std::fs::create_dir_all(target.join("Contents/MacOS")).expect("bundle");
        std::fs::write(target.join("Contents/MacOS/mark.txt"), "old").expect("marker");
        std::fs::create_dir_all(&work).expect("work");
        let log = root.join("apature.log");
        let swap = Swap {
            pid: 4_000_000,
            target: target.clone(),
            // Nothing here: `ditto` fails and the script must undo its move.
            staged: work.join("missing.app"),
            mount: PathBuf::new(),
            work: work.clone(),
            log: log.clone(),
        };
        let script = work.join("swap.sh");
        std::fs::write(&script, swap_script(&swap)).expect("script");
        let status = std::process::Command::new("/bin/bash")
            .arg(&script)
            .env("APATURE_SWAP_OPEN", "/usr/bin/true")
            .status()
            .expect("run the swap");
        assert!(!status.success(), "the swap must report the failure");
        let marker =
            std::fs::read_to_string(target.join("Contents/MacOS/mark.txt")).expect("old app");
        assert_eq!(marker, "old", "the old app is back");
        let written = std::fs::read_to_string(&log).expect("log");
        assert!(written.contains("putting the old one back"), "{written}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A copy that is in place but will not open puts the old app back.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_new_app_that_will_not_open_restores_the_old_one() {
        let root = std::env::temp_dir().join(format!("apature-swap-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("Applications");
        let work = root.join("work");
        let target = apps.join("Apature.app");
        let staged = work.join("Apature.app");
        for (dir, mark) in [(&target, "old"), (&staged, "new")] {
            std::fs::create_dir_all(dir.join("Contents/MacOS")).expect("bundle");
            std::fs::write(dir.join("Contents/MacOS/mark.txt"), mark).expect("marker");
        }
        let log = root.join("apature.log");
        let swap = Swap {
            pid: 4_000_000,
            target: target.clone(),
            staged,
            mount: PathBuf::new(),
            work: work.clone(),
            log: log.clone(),
        };
        let script = work.join("swap.sh");
        std::fs::write(&script, swap_script(&swap)).expect("script");
        let status = std::process::Command::new("/bin/bash")
            .arg(&script)
            // `open` failing stands for a new app that will not launch.
            .env("APATURE_SWAP_OPEN", "/usr/bin/false")
            .status()
            .expect("run the swap");
        assert!(!status.success(), "the swap must report the failure");
        let marker =
            std::fs::read_to_string(target.join("Contents/MacOS/mark.txt")).expect("old app");
        assert_eq!(marker, "old", "the old app is back");
        let written = std::fs::read_to_string(&log).expect("log");
        assert!(written.contains("did not open"), "{written}");
        assert!(written.contains("putting the old one back"), "{written}");
        assert!(
            !written.contains("old app deleted"),
            "the old app was deleted before the new one opened: {written}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_download_that_does_not_match_the_published_digest_is_refused() {
        let root = std::env::temp_dir().join(format!("apature-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        let image = root.join("Apature.dmg");
        std::fs::write(&image, b"not the release").expect("image");
        let err = verify_digest(&image, &"00".repeat(32)).expect_err("mismatch");
        assert!(err.contains("not the file the release publishes"), "{err}");
        // The digest is compared case-insensitively and ignores padding.
        let real = apature_ingest::sha256_file(&image).expect("hash");
        assert_eq!(verify_digest(&image, &real.to_uppercase()), Ok(()));
        assert_eq!(verify_digest(&image, &format!(" {real}\n")), Ok(()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_release_without_a_published_digest_is_not_installed() {
        let update = crate::updater::Update {
            version: "9.9.9".into(),
            prerelease: false,
            page: "https://example.com/r".into(),
            dmg: Some("https://example.com/a.dmg".into()),
            sha256: None,
            notes: String::new(),
        };
        let err = match Installer::spawn(&update, egui::Context::default()) {
            Ok(_) => panic!("an uncheckable download must not be installed"),
            Err(e) => e,
        };
        assert!(err.contains("does not publish a SHA-256"), "{err}");
    }
}
