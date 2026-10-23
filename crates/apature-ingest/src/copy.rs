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
        verify_and_publish(src, &tmp, dst, &src_hash)?;
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

/// Re-read the flushed temporary copy; only when it hashes to `src_hash`
/// does it take the destination name.
fn verify_and_publish(src: &Path, tmp: &Path, dst: &Path, src_hash: &str) -> Result<(), IngestError> {
    if sha256_file(tmp)? != src_hash {
        return Err(IngestError::Mismatch(src.to_path_buf()));
    }
    fs::rename(tmp, dst)?;
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
    fn a_multi_megabyte_copy_crosses_buffer_edges_intact() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("BIG.NEF");
        // 3 MiB + a bit, so reads land on and straddle the 1 MiB buffer.
        let data: Vec<u8> = (0..3 * (1 << 20) + 12_345).map(|i: usize| (i * 31 % 251) as u8).collect();
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
            verify_and_publish(&src, &tmp, &dst, &hash),
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
}
