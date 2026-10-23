//! The verified copy primitive: never overwrite, write to a temporary name,
//! rename only after the copy is flushed and its SHA-256 matches the
//! source, never delete the source.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

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
    let tmp = temp_name(dst);
    let result = (|| {
        let mut input = File::open(src)?;
        let mut output = File::create(&tmp)?;
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
        fs::rename(&tmp, dst)?;
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

fn temp_name(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    dst.with_file_name(format!(".{name}.part{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
