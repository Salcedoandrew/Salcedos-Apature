//! "Only new photos": a small JSON set of fingerprints of files that were
//! ingested and verified. The fingerprint is size + a hash of the first
//! 64 KiB + the file name, which is enough to recognise a card's file again
//! and cheap enough to compute for a thousand files on a card reader. It is
//! never used for verification; that is the full SHA-256 in `copy`.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default)]
pub struct Ledger {
    path: Option<PathBuf>,
    seen: BTreeSet<String>,
}

impl Ledger {
    pub fn load(path: &Path) -> Ledger {
        // A missing file is a first run and stays silent. A file that is
        // there but cannot be read or parsed is not silent: the ledger is
        // what "only new photos" trusts, and an empty ledger re-offers
        // photos already copied, so the loss is worth a warning in the log.
        let seen = match fs::read_to_string(path) {
            Ok(t) => match serde_json::from_str::<Vec<String>>(&t) {
                Ok(v) => v.into_iter().collect(),
                Err(e) => {
                    log::warn!(
                        "ingest ledger at {} is unreadable ({e}); treating it as empty — 'only new photos' will re-offer photos already copied",
                        path.display()
                    );
                    BTreeSet::new()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => {
                log::warn!(
                    "ingest ledger at {} cannot be read ({e}); treating it as empty — 'only new photos' will re-offer photos already copied",
                    path.display()
                );
                BTreeSet::new()
            }
        };
        Ledger {
            path: Some(path.to_path_buf()),
            seen,
        }
    }

    pub fn in_memory() -> Ledger {
        Ledger::default()
    }

    pub fn contains(&self, fingerprint: &str) -> bool {
        self.seen.contains(fingerprint)
    }

    pub fn insert(&mut self, fingerprint: String) {
        self.seen.insert(fingerprint);
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    pub fn save(&self) -> io::Result<()> {
        let Some(p) = &self.path else { return Ok(()) };
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir)?;
        }
        let v: Vec<&String> = self.seen.iter().collect();
        let tmp = p.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string(&v)?)?;
        fs::rename(tmp, p)
    }
}

/// Quick fingerprint: `size-hex(sha256(name + first 64 KiB))`.
pub fn fingerprint(path: &Path) -> io::Result<String> {
    let meta = fs::metadata(path)?;
    let mut f = fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut n = 0;
    while n < buf.len() {
        let r = f.read(&mut buf[n..])?;
        if r == 0 {
            break;
        }
        n += r;
    }
    let mut h = Sha256::new();
    h.update(
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
            .as_bytes(),
    );
    h.update(&buf[..n]);
    let hex: String = h
        .finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("{}-{hex}", meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("DSC_0001.NEF");
        fs::write(&a, b"abc").unwrap();
        let fp = fingerprint(&a).unwrap();
        assert!(fp.starts_with("3-"));
        let b = dir.path().join("DSC_0002.NEF");
        fs::write(&b, b"abc").unwrap();
        assert_ne!(
            fp,
            fingerprint(&b).unwrap(),
            "same bytes, different name → different card file"
        );
        let lp = dir.path().join("ledger/ingested.json");
        let mut l = Ledger::load(&lp);
        assert!(l.is_empty());
        l.insert(fp.clone());
        l.save().unwrap();
        let l2 = Ledger::load(&lp);
        assert!(l2.contains(&fp));
        assert_eq!(l2.len(), 1);
    }
}
