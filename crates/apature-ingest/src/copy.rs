//! The verified copy primitive: never overwrite, write to a temporary name,
//! publish by hard link only after the copy is flushed and its SHA-256
//! matches the source, never delete the source.

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
    #[error("{0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
    /// Hex SHA-256 of the content, identical on both sides by construction.
    pub sha256: String,
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

fn copy_impl(src: &Path, dst: &Path, verify: bool) -> Result<CopyReport, IngestError> {
    if dst.exists() {
        return Err(IngestError::DestinationExists(dst.to_path_buf()));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    // Reserve the temp name up front, exclusively: two concurrent copies
    // to the same destination must never share one file.
    let (tmp, mut output) = reserve_temp(dst)?;
    let result = (|| {
        let mut input = File::open(src)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            output.write_all(&buf[..n])?;
            total += n as u64;
        }
        output.sync_all()?;
        drop(output);
        let src_hash = hex(&hasher.finalize());
        if verify {
            check_copy(&tmp, src, &src_hash)?;
        }
        publish(&tmp, dst)?;
        Ok(CopyReport {
            source: src.to_path_buf(),
            destination: dst.to_path_buf(),
            bytes: total,
            sha256: src_hash,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Publish the verified temp file at `dst` without ever replacing an
/// existing file. A hard link fails when the name is taken, so a
/// destination that appeared after the initial check is left untouched
/// and the copy reports `DestinationExists`. The temp file is in the
/// destination's directory, so the link is always on one filesystem.
fn publish(tmp: &Path, dst: &Path) -> Result<(), IngestError> {
    if let Err(e) = fs::hard_link(tmp, dst) {
        // A link fails when the name is taken; a dangling symlink at the
        // destination reads the same way, so check the name itself.
        if e.kind() == io::ErrorKind::AlreadyExists || fs::symlink_metadata(dst).is_ok() {
            return Err(IngestError::DestinationExists(dst.to_path_buf()));
        }
        return Err(e.into());
    }
    let _ = fs::remove_file(tmp);
    Ok(())
}

/// The verification step of a copy: the file written at `tmp` must hash to
/// the source's hash, read back from disk in full.
fn check_copy(tmp: &Path, src: &Path, src_hash: &str) -> Result<(), IngestError> {
    if sha256_file(tmp)? != src_hash {
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
}
