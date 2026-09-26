use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::gh::{format_rfc3339_nanos, PullRequestDetail};

pub const DISK_CACHE_DIR_NAME: &str = "gh-review";
pub const DISK_CACHE_MAX_FILE: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheEntry {
    #[serde(default)]
    pub detail: PullRequestDetail,
    #[serde(default)]
    pub diff: String,
}

pub struct DetailCache {
    mem: Mutex<HashMap<String, CacheEntry>>,
    dir: Option<PathBuf>,
}

pub fn cache_key(url: &str, updated_at: SystemTime) -> String {
    let raw = format!("{}@{}", url, format_rfc3339_nanos(updated_at));
    let sum = Sha256::digest(raw.as_bytes());
    let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    hex[..16].to_string()
}

impl Default for DetailCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailCache {
    pub fn new() -> Self {
        let dir = dirs::cache_dir().map(|p| p.join(DISK_CACHE_DIR_NAME));
        DetailCache {
            mem: Mutex::new(HashMap::new()),
            dir,
        }
    }

    pub fn with_dir(dir: Option<PathBuf>) -> Self {
        DetailCache {
            mem: Mutex::new(HashMap::new()),
            dir,
        }
    }

    pub fn get_mem(&self, key: &str) -> Option<CacheEntry> {
        self.mem.lock().unwrap().get(key).cloned()
    }

    pub fn get_disk(&self, key: &str) -> Option<CacheEntry> {
        let dir = self.dir.as_ref()?;
        let path = dir.join(format!("{key}.json"));
        let data = fs::read(&path).ok()?;
        let e: CacheEntry = serde_json::from_slice(&data).ok()?;
        // Touch mtime so the LRU pruning treats a hit as recent.
        let now = filetime::FileTime::from_system_time(SystemTime::now());
        let _ = filetime::set_file_times(&path, now, now);
        self.mem.lock().unwrap().insert(key.to_string(), e.clone());
        Some(e)
    }

    pub fn put(&self, key: &str, e: CacheEntry) {
        self.mem.lock().unwrap().insert(key.to_string(), e.clone());
        self.write_disk(key, e);
    }

    fn write_disk(&self, key: &str, e: CacheEntry) {
        let dir = match self.dir.as_ref() {
            Some(d) => d,
            None => return,
        };
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let data = match serde_json::to_vec(&e) {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.join(format!("{key}.json"));
        let tmp = dir.join(format!("{key}.{}.tmp", std::process::id()));
        if fs::write(&tmp, &data).is_err() {
            let _ = fs::remove_file(&tmp);
            return;
        }
        if fs::rename(&tmp, &path).is_err() {
            let _ = fs::remove_file(&tmp);
            return;
        }
        self.prune_disk(dir);
    }

    fn prune_disk(&self, dir: &Path) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let meta = match ent.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((path, mtime));
        }
        if files.len() <= DISK_CACHE_MAX_FILE {
            return;
        }
        files.sort_by_key(|(_, m)| *m);
        let excess = files.len() - DISK_CACHE_MAX_FILE;
        for (path, _) in files.into_iter().take(excess) {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::PullRequest;

    fn new_test_cache(dir: &Path) -> DetailCache {
        DetailCache::with_dir(Some(dir.to_path_buf()))
    }

    fn sample_pr(url: &str) -> PullRequest {
        PullRequest {
            url: url.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_cache_key_changes_with_updated_at() {
        let url = "https://example.test/pr/1";
        let t1 = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1747200000);
        let t2 = t1 + std::time::Duration::from_secs(1);
        assert_ne!(cache_key(url, t1), cache_key(url, t2));
    }

    #[test]
    fn test_cache_put_then_get_mem() {
        let dir = tempfile::tempdir().unwrap();
        let c = new_test_cache(dir.path());
        let key = "abc1234567890def";
        let entry = CacheEntry {
            detail: PullRequestDetail {
                base: sample_pr("u"),
                ..Default::default()
            },
            diff: "diff body".into(),
        };
        c.put(key, entry);

        let got = c.get_mem(key).expect("memory cache miss after put");
        assert_eq!(got.diff, "diff body");
        assert_eq!(got.detail.base.number, 0);
    }

    #[test]
    fn test_cache_get_disk_after_fresh_load() {
        let dir = tempfile::tempdir().unwrap();
        let c = new_test_cache(dir.path());
        let key = "abc1234567890def";
        let entry = CacheEntry {
            detail: PullRequestDetail {
                base: sample_pr("u"),
                ..Default::default()
            },
            diff: "diff body".into(),
        };
        c.put(key, entry);

        // Simulate a fresh process by dropping memory only.
        let c2 = new_test_cache(dir.path());
        let got = c2.get_disk(key).expect("disk cache miss after put");
        assert_eq!(got.diff, "diff body");
        assert!(
            c2.get_mem(key).is_some(),
            "getDisk should populate memory cache"
        );
    }

    #[test]
    fn test_cache_prune_removes_oldest_files() {
        let dir = tempfile::tempdir().unwrap();
        let c = new_test_cache(dir.path());
        // Backdate the first 5 files so prune deletes them first.
        let old = filetime::FileTime::from_unix_time(1_000_000, 0);
        for i in 0..5 {
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(i as u64);
            let key = cache_key("https://example.test/pr/x", t);
            c.put(
                &key,
                CacheEntry {
                    detail: Default::default(),
                    diff: "x".into(),
                },
            );
            let path = dir.path().join(format!("{key}.json"));
            filetime::set_file_times(&path, old, old).unwrap();
        }
        for i in 0..DISK_CACHE_MAX_FILE {
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_100_000 + i as u64);
            let key = cache_key("https://example.test/pr/y", t);
            c.put(
                &key,
                CacheEntry {
                    detail: Default::default(),
                    diff: "y".into(),
                },
            );
        }

        let count = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count();
        assert!(count <= DISK_CACHE_MAX_FILE, "prune left {count} files");
    }
}
