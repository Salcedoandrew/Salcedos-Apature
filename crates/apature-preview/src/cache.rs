//! On-disk thumbnail cache. Keys are a hash of (path, size, mtime, long side),
//! so edits or re-ingests invalidate automatically. Values are JPEGs.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::decode::{decode_jpeg, encode_jpeg, Rgb};

#[derive(Clone, Debug)]
pub struct ThumbCache {
    dir: PathBuf,
}

impl ThumbCache {
    pub fn new(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(ThumbCache { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Cache key for a file. `modified`/`size` come from the directory scan so
    /// no extra stat is needed.
    pub fn key(path: &Path, size: u64, modified: Option<SystemTime>, long_side: u32) -> String {
        let mut h = DefaultHasher::new();
        path.hash(&mut h);
        size.hash(&mut h);
        modified
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            .hash(&mut h);
        long_side.hash(&mut h);
        format!("{:016x}-{long_side}", h.finish())
    }

    fn path_for(&self, key: &str) -> PathBuf {
        // Two-level fan-out keeps directories small with tens of thousands of entries.
        self.dir.join(&key[..2]).join(format!("{key}.jpg"))
    }

    pub fn get(&self, key: &str) -> Option<Rgb> {
        let bytes = fs::read(self.path_for(key)).ok()?;
        decode_jpeg(&bytes, None).ok()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.path_for(key).is_file()
    }

    /// Delete the oldest entries until the cache holds at most `budget_bytes`.
    ///
    /// Whole-folder warm-up fills this directory and nothing ever removed an
    /// entry, so without a sweep a big library grows without limit. Keys carry
    /// the file's mtime and size, so an edit or a re-import leaves the old JPEG
    /// behind too: the sweep is the only thing that reclaims those.
    pub fn sweep(&self, budget_bytes: u64) -> io::Result<u64> {
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        for shard in fs::read_dir(&self.dir)?.flatten() {
            if !shard.path().is_dir() {
                continue;
            }
            for entry in fs::read_dir(shard.path())?.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jpg") {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    let seen = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    files.push((seen, meta.len(), path));
                }
            }
        }
        let mut total: u64 = files.iter().map(|(_, len, _)| *len).sum();
        if total <= budget_bytes {
            return Ok(0);
        }
        files.sort_by_key(|(seen, _, _)| *seen);
        let mut removed = 0;
        for (_, len, path) in &files {
            if total <= budget_bytes {
                break;
            }
            if fs::remove_file(path).is_ok() {
                total = total.saturating_sub(*len);
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn put(&self, key: &str, img: &Rgb) -> io::Result<()> {
        let bytes = encode_jpeg(img, 88).map_err(io::Error::other)?;
        let final_path = self.path_for(key);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = final_path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, bytes)?;
        fs::rename(tmp, final_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_drops_the_oldest_until_under_budget() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbCache::new(dir.path().join("thumbs")).unwrap();
        let img = Rgb {
            width: 32,
            height: 32,
            data: vec![128; 32 * 32 * 3],
        };
        let key = |i: u64| ThumbCache::key(Path::new("/a/b.NEF"), i, None, 320);
        for i in 0..4u64 {
            cache.put(&key(i), &img).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let one = cache.path_for(&key(0)).metadata().unwrap().len();
        let removed = cache.sweep(one * 2).unwrap();
        assert!(removed >= 2, "removed {removed}");
        let total: u64 = (0..4u64)
            .map(|i| cache.path_for(&key(i)))
            .filter_map(|p| p.metadata().ok())
            .map(|m| m.len())
            .sum();
        assert!(total <= one * 2, "cache still {total} bytes");
        // The newest entries are the ones that survived.
        assert!(cache.contains(&key(3)));
    }

    #[test]
    fn put_then_get() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbCache::new(dir.path().join("thumbs")).unwrap();
        let img = Rgb {
            width: 4,
            height: 2,
            data: vec![200; 24],
        };
        let key = ThumbCache::key(Path::new("/a/b.NEF"), 10, None, 320);
        assert!(!cache.contains(&key));
        cache.put(&key, &img).unwrap();
        assert!(cache.contains(&key));
        let back = cache.get(&key).unwrap();
        assert_eq!((back.width, back.height), (4, 2));
        let other = ThumbCache::key(Path::new("/a/b.NEF"), 11, None, 320);
        assert_ne!(key, other);
    }
}
