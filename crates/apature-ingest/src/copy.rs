//! The verified copy primitive: never overwrite, write to a temporary name,
//! publish by hard link only after the copy is flushed and its SHA-256
//! matches the source, never delete the source.
//!
//! Two filesystem quirks are handled here rather than at the call sites,
//! because either one turns a perfectly good copy into a failed ingest:
//!
//! * **Hard links.** exFAT on macOS (FSKit) refuses `link(2)` with
//!   `ENOTSUP`. The copy then writes its verified bytes into a destination
//!   opened exclusively, so an existing file is never replaced, instead of
//!   failing the whole card with "Operation not supported (os error 45)".
//! * **Durable flushes.** That filesystem can also refuse `F_FULLFSYNC`,
//!   which is exactly what std's `sync_all` issues on Apple targets and
//!   then gives up on. The copy degrades to `fsync(2)` and reports the
//!   weaker [`SyncQuality`]; every other flush error still fails.
//!
//! Both fallbacks are scoped to the errnos that mean "this filesystem does
//! not do that" and never to a blanket retry, and both are reported in
//! [`CopyReport`] so a caller can tell a full-fidelity copy from a degraded
//! one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("verification failed for {0}: source and copy differ")]
    Mismatch(PathBuf),
    /// An I/O step failed. `op` names the step in the photographer's terms
    /// and `path` the file it was working on, so a failure on an unusual
    /// filesystem says which step was refused instead of only the errno.
    #[error("{op} failed for {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl IngestError {
    pub(crate) fn io(op: &'static str, path: &Path, source: io::Error) -> Self {
        IngestError::Io {
            op,
            path: path.to_path_buf(),
            source,
        }
    }
}

/// How a finished copy reached its final name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publish {
    /// Linked from the verified temp file: the usual path, no second write.
    HardLink,
    /// The filesystem refused hard links, so the verified bytes were written
    /// into a destination created exclusively.
    ExclusiveCopy,
}

/// How the copy was forced to disk before it was published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncQuality {
    /// `F_FULLFSYNC` on Apple targets: the data reached the media.
    Full,
    /// The filesystem refused a full flush and the copy fell back to
    /// `fsync(2)`, which the kernel may satisfy without flushing the media.
    Fallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
    /// Hex SHA-256 of the content, identical on both sides by construction.
    pub sha256: String,
    /// How the copy took its final name.
    pub published: Publish,
    /// How durably the copy was flushed first.
    pub sync: SyncQuality,
}

/// Copy `src` to `dst` with verification. Creates parent directories.
pub fn copy_verified(src: &Path, dst: &Path) -> Result<CopyReport, IngestError> {
    copy_impl(src, dst, true)
}

/// Copy `src` to `dst` without the verification re-read: the copy is
/// written, flushed and synced exactly as in [`copy_verified`], but never
/// read back, so corruption in flight would go undetected. Verification is
/// the default everywhere in the app; this entry point is for callers who
/// check the copy some other way (or whose destination makes a second full
/// read the bottleneck). The report's hash is the source's.
pub fn copy_unverified(src: &Path, dst: &Path) -> Result<CopyReport, IngestError> {
    copy_impl(src, dst, false)
}

/// The one system call a copy depends on, taken as a parameter so tests can
/// stand in for a filesystem that refuses it.
type Linker = fn(&Path, &Path) -> io::Result<()>;

fn copy_impl(src: &Path, dst: &Path, verify: bool) -> Result<CopyReport, IngestError> {
    // `fs::hard_link` is generic over `AsRef<Path>` and does not coerce to a
    // plain function pointer, so the production linker is a concrete wrapper.
    fn link(tmp: &Path, dst: &Path) -> io::Result<()> {
        fs::hard_link(tmp, dst)
    }
    copy_impl_with(src, dst, verify, link)
}

fn copy_impl_with(
    src: &Path,
    dst: &Path,
    verify: bool,
    link: Linker,
) -> Result<CopyReport, IngestError> {
    if dst.exists() {
        return Err(IngestError::DestinationExists(dst.to_path_buf()));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| IngestError::io("create the destination folder", parent, e))?;
    }
    // Reserve the temp name up front, exclusively: two concurrent copies
    // to the same destination must never share one file.
    let (tmp, mut output) =
        reserve_temp(dst).map_err(|e| IngestError::io("reserve a temporary copy name", dst, e))?;
    let result = (|| {
        let mut input = File::open(src).map_err(|e| IngestError::io("open the source", src, e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let n = input
                .read(&mut buf)
                .map_err(|e| IngestError::io("read the source", src, e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            output
                .write_all(&buf[..n])
                .map_err(|e| IngestError::io("write the copy", &tmp, e))?;
            total += n as u64;
        }
        let sync = sync_durable(&output, &tmp)?;
        drop(output);
        let src_hash = hex(&hasher.finalize());
        if verify {
            check_copy(&tmp, src, &src_hash)?;
        }
        let published = publish(&tmp, dst, link)?;
        Ok(CopyReport {
            source: src.to_path_buf(),
            destination: dst.to_path_buf(),
            bytes: total,
            sha256: src_hash,
            published,
            sync,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Publish the verified temp file at `dst` without ever replacing an
/// existing file. A hard link fails when the name is taken, so a
/// destination that appeared after the initial check is left untouched and
/// the copy reports `DestinationExists`. The temp file is in the
/// destination's directory, so the link is always on one filesystem.
///
/// A filesystem that cannot link at all (exFAT) falls back to an exclusive
/// copy of the same verified bytes. Nothing else is retried: a permission
/// error or a full disk is reported as the failure it is.
fn publish(tmp: &Path, dst: &Path, link: Linker) -> Result<Publish, IngestError> {
    match link(tmp, dst) {
        Ok(()) => {
            let _ = fs::remove_file(tmp);
            Ok(Publish::HardLink)
        }
        Err(e) => {
            // A link fails when the name is taken; a dangling symlink at the
            // destination reads the same way, so check the name itself.
            if e.kind() == io::ErrorKind::AlreadyExists || fs::symlink_metadata(dst).is_ok() {
                return Err(IngestError::DestinationExists(dst.to_path_buf()));
            }
            if !is_unsupported(&e) {
                return Err(IngestError::io("link the copy into place", dst, e));
            }
            publish_exclusive(tmp, dst)?;
            Ok(Publish::ExclusiveCopy)
        }
    }
}

/// Publish by copying the verified temp file into a destination opened with
/// `create_new`, which is the only exclusive "this name is still free"
/// primitive that exFAT supports. `rename` is deliberately not used: it
/// would silently replace a file that appeared during the copy, which is
/// the one thing the primitive promises never to do.
///
/// The destination is re-hashed after it is flushed, because a filesystem
/// odd enough to refuse `link(2)` is not one to trust with a single pass,
/// and a copy that does not read back identical is removed rather than
/// published.
fn publish_exclusive(tmp: &Path, dst: &Path) -> Result<(), IngestError> {
    let mut output = match OpenOptions::new().write(true).create_new(true).open(dst) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(IngestError::DestinationExists(dst.to_path_buf()));
        }
        Err(e) => {
            return Err(IngestError::io(
                "create the destination without replacing it",
                dst,
                e,
            ));
        }
    };
    let result = (|| {
        let mut input =
            File::open(tmp).map_err(|e| IngestError::io("open the copy to publish it", tmp, e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = input
                .read(&mut buf)
                .map_err(|e| IngestError::io("read the copy to publish it", tmp, e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            output
                .write_all(&buf[..n])
                .map_err(|e| IngestError::io("write the destination", dst, e))?;
        }
        sync_durable(&output, dst)?;
        drop(output);
        let written = hex(&hasher.finalize());
        let landed = sha256_file_ctx(dst, "read the destination back to verify it")?;
        if landed != written {
            return Err(IngestError::Mismatch(dst.to_path_buf()));
        }
        let _ = fs::remove_file(tmp);
        Ok(())
    })();
    if result.is_err() {
        // The name was created by us and is not a valid copy, so it must
        // not survive as one.
        let _ = fs::remove_file(dst);
    }
    result
}

/// The verification step of a copy: the file written at `tmp` must hash to
/// the source's hash, read back from disk in full.
fn check_copy(tmp: &Path, src: &Path, src_hash: &str) -> Result<(), IngestError> {
    if sha256_file_ctx(tmp, "read the copy back to verify it")? != src_hash {
        return Err(IngestError::Mismatch(src.to_path_buf()));
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// [`sha256_file`] with the failing step attached, for use inside the copy.
fn sha256_file_ctx(path: &Path, op: &'static str) -> Result<String, IngestError> {
    sha256_file(path).map_err(|e| IngestError::io(op, path, e))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Counter that makes every reserved temp name in this process unique, so
/// two concurrent copies to the same destination never share a file. The
/// process id keeps names distinct across processes; `create_new` retries
/// past stale names left behind by a crashed run.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Reserve a temp file next to `dst` with exclusive creation and return it
/// (path plus open handle) so the caller writes through the reserved file.
fn reserve_temp(dst: &Path) -> io::Result<(PathBuf, File)> {
    let name = dst
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for _ in 0..100 {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = dst.with_file_name(format!(".{name}.part{}-{n}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(f) => return Ok((candidate, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a temporary copy name",
    ))
}

/// Does this error say "this filesystem does not do that", as opposed to a
/// real failure? Only such errors are worth a fallback: a permission error
/// or a full disk must be reported, never retried another way.
#[cfg(unix)]
fn is_unsupported(err: &io::Error) -> bool {
    match err.raw_os_error() {
        Some(code) => code == libc::ENOTSUP || code == libc::EOPNOTSUPP,
        None => false,
    }
}

#[cfg(not(unix))]
fn is_unsupported(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Unsupported
}

/// A full flush may be refused with a wider set of errnos than a link: some
/// filesystems answer `EINVAL` or `ENOTTY` to `F_FULLFSYNC`, and all that is
/// lost there is the media-level guarantee, not the copy.
#[cfg(unix)]
fn is_sync_unsupported(err: &io::Error) -> bool {
    match err.raw_os_error() {
        Some(code) => is_unsupported(err) || code == libc::EINVAL || code == libc::ENOTTY,
        None => false,
    }
}

#[cfg(not(unix))]
fn is_sync_unsupported(err: &io::Error) -> bool {
    is_unsupported(err) || err.kind() == io::ErrorKind::InvalidInput
}

/// Force the finished copy to disk before it is published. On Apple targets
/// this asks for `F_FULLFSYNC` explicitly, because that is what std's
/// `sync_all` does there and it fails hard when a filesystem refuses; the
/// refusal is downgraded to `fsync(2)` and reported as [`SyncQuality::Fallback`].
fn sync_durable(file: &File, path: &Path) -> Result<SyncQuality, IngestError> {
    if let Err(e) = full_flush(file) {
        if !is_sync_unsupported(&e) {
            return Err(IngestError::io("flush the copy to disk", path, e));
        }
        plain_flush(file).map_err(|e| IngestError::io("flush the copy to disk", path, e))?;
        return Ok(SyncQuality::Fallback);
    }
    Ok(SyncQuality::Full)
}

/// `F_FULLFSYNC` on Apple targets: the flush that actually reaches the media.
#[cfg(target_vendor = "apple")]
fn full_flush(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_vendor = "apple"))]
fn full_flush(file: &File) -> io::Result<()> {
    file.sync_all()
}

/// Plain `fsync(2)`: everything the kernel offers when the filesystem
/// refuses a full flush.
#[cfg(unix)]
fn plain_flush(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::fsync(file.as_raw_fd()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn plain_flush(file: &File) -> io::Result<()> {
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_megabyte_copy_crosses_buffer_edges_intact() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("BIG.NEF");
        // 3 MiB + a bit, so reads land on and straddle the 1 MiB buffer.
        let data: Vec<u8> = (0..3 * (1 << 20) + 12_345)
            .map(|i: usize| (i * 31 % 251) as u8)
            .collect();
        fs::write(&src, &data).unwrap();
        let dst = dir.path().join("out/BIG.NEF");
        let r = copy_verified(&src, &dst).unwrap();
        assert_eq!(r.bytes, data.len() as u64);
        assert!(fs::read(&dst).unwrap() == data, "copy differs");
        assert_eq!(r.sha256, hex(&Sha256::digest(&data)));
    }

    #[test]
    fn a_copy_that_reads_back_different_is_not_published() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("A.NEF");
        fs::write(&src, b"original").unwrap();
        let tmp = dir.path().join(".A.NEF.part");
        fs::write(&tmp, b"corrupted").unwrap();
        let dst = dir.path().join("A-copy.NEF");
        let hash = sha256_file(&src).unwrap();
        assert!(matches!(
            check_copy(&tmp, &src, &hash),
            Err(IngestError::Mismatch(_))
        ));
        assert!(!dst.exists(), "a bad copy never takes the real name");
        assert!(src.exists());
    }

    #[test]
    fn copies_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0001.NEF");
        fs::write(&src, b"raw bytes here").unwrap();
        let dst = dir.path().join("out/2026/DSC_0001.NEF");
        let r = copy_verified(&src, &dst).unwrap();
        assert_eq!(r.bytes, 14);
        assert_eq!(fs::read(&dst).unwrap(), b"raw bytes here");
        assert_eq!(r.sha256, sha256_file(&src).unwrap());
        assert!(src.exists(), "source must never be removed");
        assert!(matches!(
            copy_verified(&src, &dst),
            Err(IngestError::DestinationExists(_))
        ));
        assert!(!dir.path().join("out/2026").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")));
    }

    #[test]
    fn copies_and_verifies_a_large_patterned_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0002.NEF");
        // ~3 MiB of a deterministic pattern, so any dropped, duplicated or
        // reordered chunk changes the bytes and the hash.
        let data: Vec<u8> = (0..3 * 1024 * 1024 + 13)
            .map(|i| ((i * 31 + 7) % 251) as u8)
            .collect();
        fs::write(&src, &data).unwrap();
        let dst = dir.path().join("out/DSC_0002.NEF");
        let r = copy_verified(&src, &dst).unwrap();
        assert_eq!(r.bytes, data.len() as u64);
        assert_eq!(fs::read(&dst).unwrap(), data);
        let mut hasher = Sha256::new();
        hasher.update(&data);
        assert_eq!(r.sha256, hex(&hasher.finalize()));
        assert_eq!(r.sha256, sha256_file(&dst).unwrap());
        assert!(src.exists(), "source must never be removed");
    }

    #[test]
    fn a_copy_that_differs_from_the_source_is_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0003.NEF");
        fs::write(&src, b"the real photo bytes").unwrap();
        // What landed on disk is not what the source hashed to.
        let tmp = dir.path().join(".DSC_0003.NEF.part");
        fs::write(&tmp, b"corrupted in flight!").unwrap();
        let src_hash = sha256_file(&src).unwrap();
        assert!(matches!(
            check_copy(&tmp, &src, &src_hash),
            Err(IngestError::Mismatch(p)) if p == src
        ));
        // The same check passes when the copy is faithful.
        fs::write(&tmp, b"the real photo bytes").unwrap();
        assert!(check_copy(&tmp, &src, &src_hash).is_ok());
    }

    #[test]
    fn unverified_copy_copies_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0004.NEF");
        let data: Vec<u8> = (0..100_000).map(|i| ((i * 17 + 3) % 241) as u8).collect();
        fs::write(&src, &data).unwrap();
        let dst = dir.path().join("out/DSC_0004.NEF");
        let r = copy_unverified(&src, &dst).unwrap();
        assert_eq!(r.bytes, data.len() as u64);
        assert_eq!(fs::read(&dst).unwrap(), data);
        assert_eq!(r.sha256, sha256_file(&src).unwrap());
        assert!(matches!(
            copy_unverified(&src, &dst),
            Err(IngestError::DestinationExists(_))
        ));
    }

    #[test]
    fn reserved_temp_names_are_unique_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("DSC_0001.NEF");
        let (p1, f1) = reserve_temp(&dst).unwrap();
        let (p2, f2) = reserve_temp(&dst).unwrap();
        assert_ne!(p1, p2, "two reservations must not share a name");
        assert!(p1.exists() && p2.exists());
        drop(f1);
        drop(f2);
        fs::remove_file(&p1).unwrap();
        fs::remove_file(&p2).unwrap();
    }

    #[test]
    fn a_destination_created_during_the_copy_is_not_replaced() {
        // Audit finding #3, reproduced the way the audit did: a FIFO
        // source blocks the copy after its first chunk, so the test can
        // create the destination mid-copy. The copy must refuse and the
        // preexisting file must survive intact.
        use std::io::Write as _;
        use std::sync::mpsc::channel;

        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("DSC_0001.NEF");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let dst = dir.path().join("out/DSC_0001.NEF");
        let (first_tx, first_rx) = channel::<()>();
        let (go_tx, go_rx) = channel::<()>();
        let chunk = vec![0xABu8; 1 << 20];
        let fifo_w = fifo.clone();
        let chunk_w = chunk.clone();
        let writer = std::thread::spawn(move || {
            let mut w = File::create(&fifo_w).unwrap();
            w.write_all(&chunk_w).unwrap();
            w.flush().unwrap();
            first_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            w.write_all(&chunk_w).unwrap();
            w.flush().unwrap();
            // Dropping `w` closes the FIFO: the reader sees EOF.
        });
        let fifo_r = fifo.clone();
        let dst_r = dst.clone();
        let copier = std::thread::spawn(move || copy_verified(&fifo_r, &dst_r));
        first_rx.recv().unwrap();
        // The destination appears while the copy is still in flight.
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        fs::write(&dst, b"someone else's photo").unwrap();
        go_tx.send(()).unwrap();
        writer.join().unwrap();
        let result = copier.join().unwrap();
        assert!(matches!(result, Err(IngestError::DestinationExists(p)) if p == dst));
        assert_eq!(fs::read(&dst).unwrap(), b"someone else's photo");
        assert!(!dir.path().join("out").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")));
    }

    #[test]
    fn concurrent_copies_to_the_same_destination_do_not_share_a_temp() {
        // Two copies racing to one destination: exactly one wins, the
        // other refuses, and the winner's bytes are intact. With the old
        // PID-only temp name both shared one file and both failed with a
        // hash mismatch.
        let dir = tempfile::tempdir().unwrap();
        let data_a: Vec<u8> = (0..4 * 1024 * 1024)
            .map(|i| ((i * 31 + 7) % 251) as u8)
            .collect();
        let data_b: Vec<u8> = (0..4 * 1024 * 1024)
            .map(|i| ((i * 17 + 3) % 241) as u8)
            .collect();
        let src_a = dir.path().join("A.NEF");
        let src_b = dir.path().join("B.NEF");
        fs::write(&src_a, &data_a).unwrap();
        fs::write(&src_b, &data_b).unwrap();
        let dst = dir.path().join("out/DSC.NEF");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let spawn = |src: PathBuf, dst: PathBuf, b: std::sync::Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                b.wait();
                copy_verified(&src, &dst)
            })
        };
        let h1 = spawn(src_a, dst.clone(), barrier.clone());
        let h2 = spawn(src_b, dst.clone(), barrier);
        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();
        assert!(r1.is_ok() ^ r2.is_ok(), "r1={r1:?} r2={r2:?}");
        let winner = if r1.is_ok() { &data_a } else { &data_b };
        assert_eq!(fs::read(&dst).unwrap(), *winner);
        let loser = if r1.is_ok() { &r2 } else { &r1 };
        assert!(matches!(loser, Err(IngestError::DestinationExists(_))));
        assert!(!dir.path().join("out").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")));
    }

    /// A linker that reports the filesystem cannot make hard links, the way
    /// exFAT on macOS does (`ENOTSUP`, errno 45 on Apple targets).
    #[cfg(unix)]
    fn no_links(_tmp: &Path, _dst: &Path) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::ENOTSUP))
    }

    #[cfg(unix)]
    #[test]
    fn a_filesystem_without_links_publishes_by_exclusive_copy() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0005.CR3");
        let data: Vec<u8> = (0..250_000).map(|i| ((i * 29 + 11) % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        let dst = dir.path().join("out/2026/DSC_0005.CR3");
        let r = copy_impl_with(&src, &dst, true, no_links).unwrap();
        assert_eq!(r.published, Publish::ExclusiveCopy);
        assert_eq!(fs::read(&dst).unwrap(), data, "the published bytes differ");
        assert_eq!(r.sha256, sha256_file(&dst).unwrap());
        assert!(src.exists(), "source must never be removed");
        assert!(!dir.path().join("out/2026").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")));
    }

    #[cfg(unix)]
    #[test]
    fn the_exclusive_publish_leaves_an_existing_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join(".DSC.part");
        fs::write(&tmp, b"the copy we were going to publish").unwrap();
        let dst = dir.path().join("DSC.CR3");
        fs::write(&dst, b"someone else's photo").unwrap();
        assert!(matches!(
            publish_exclusive(&tmp, &dst),
            Err(IngestError::DestinationExists(p)) if p == dst
        ));
        assert_eq!(fs::read(&dst).unwrap(), b"someone else's photo");
        // The refusal happens before anything is created, so the caller's
        // temp is untouched and its own cleanup still decides.
        assert!(tmp.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_error_that_is_not_unsupported_is_not_retried() {
        fn denied(_tmp: &Path, _dst: &Path) -> io::Result<()> {
            Err(io::Error::from_raw_os_error(libc::EACCES))
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("DSC_0006.CR3");
        fs::write(&src, b"photo bytes").unwrap();
        let dst = dir.path().join("out/DSC_0006.CR3");
        let err = copy_impl_with(&src, &dst, true, denied).unwrap_err();
        assert!(
            matches!(err, IngestError::Io { op, .. } if op == "link the copy into place"),
            "{err}"
        );
        assert!(err.to_string().contains("Permission denied"), "{err}");
        assert!(!dst.exists(), "a refused publish leaves no destination");
        assert!(!dir.path().join("out").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")));
    }

    #[cfg(unix)]
    #[test]
    fn only_unsupported_errnos_are_treated_as_unsupported() {
        let errno = io::Error::from_raw_os_error;
        assert!(is_unsupported(&errno(libc::ENOTSUP)));
        assert!(is_unsupported(&errno(libc::EOPNOTSUPP)));
        for code in [libc::EACCES, libc::EEXIST, libc::ENOSPC, libc::EIO] {
            assert!(
                !is_unsupported(&errno(code)),
                "errno {code} is a real failure"
            );
        }
        // A full flush is refused with a wider set than a link: some
        // filesystems answer EINVAL or ENOTTY, and only the media-level
        // guarantee is lost, not the copy.
        assert!(is_sync_unsupported(&errno(libc::EINVAL)));
        assert!(is_sync_unsupported(&errno(libc::ENOTTY)));
        assert!(!is_sync_unsupported(&errno(libc::ENOSPC)));
    }

    #[test]
    fn errors_name_the_step_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("GONE.CR3");
        let dst = dir.path().join("out/GONE.CR3");
        let err = copy_verified(&missing, &dst).unwrap_err();
        assert!(
            matches!(err, IngestError::Io { op, .. } if op == "open the source"),
            "{err}"
        );
        let text = err.to_string();
        assert!(text.starts_with("open the source failed for "), "{text}");
        assert!(text.contains("GONE.CR3"), "{text}");
    }

    #[test]
    fn a_durable_flush_succeeds_on_a_writable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flush.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"bytes to flush").unwrap();
        assert_eq!(sync_durable(&f, &path).unwrap(), SyncQuality::Full);
    }
}
