use std::sync::Arc;
use std::time::Duration;

use crate::cache::{CacheEntry, DetailCache};
use crate::gh::{self, Gh, PullRequest};

pub const PREFETCH_TIMEOUT: Duration = Duration::from_secs(60);
pub const PREFETCH_CONCURRENCY: usize = 3;
pub const PREFETCH_TOP_N: usize = 3;

pub struct Prefetcher {
    pub cache: Arc<DetailCache>,
    sem: Arc<tokio::sync::Semaphore>,
    inflight: tokio::sync::Mutex<std::collections::HashSet<String>>,
}

impl Prefetcher {
    pub fn new(cache: Arc<DetailCache>) -> Self {
        Prefetcher {
            cache,
            sem: Arc::new(tokio::sync::Semaphore::new(PREFETCH_CONCURRENCY)),
            inflight: tokio::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub async fn should_start(&self, pr: &PullRequest) -> bool {
        if pr.url.is_empty() {
            return false;
        }
        let key = gh::cache_key_of(pr);
        if self.cache.get_mem(&key).is_some() {
            return false;
        }
        let mut inflight = self.inflight.lock().await;
        if inflight.contains(&pr.url) {
            return false;
        }
        inflight.insert(pr.url.clone());
        true
    }

    pub async fn finish(&self, url: &str) {
        self.inflight.lock().await.remove(url);
    }

    pub fn spawn(self: &Arc<Self>, g: &Gh, prs: Vec<PullRequest>) {
        let g = g.clone();
        let this = self.clone();
        tokio::spawn(async move {
            for pr in prs {
                if !this.should_start(&pr).await {
                    continue;
                }
                let this2 = this.clone();
                let g2 = g.clone();
                tokio::spawn(async move {
                    this2.run(g2, pr).await;
                });
            }
        });
    }

    async fn run(&self, g: Gh, pr: PullRequest) {
        let url = pr.url.clone();
        let _permit = match self.sem.acquire().await {
            Ok(p) => p,
            Err(_) => {
                self.finish(&url).await;
                return;
            }
        };

        let result =
            tokio::time::timeout(PREFETCH_TIMEOUT, gh::load_detail_and_diff(&g, &pr)).await;

        if let Ok(Ok((detail, diff))) = result {
            let key = gh::cache_key_of_updated(&url, detail.base.updated_at);
            self.cache.put(&key, CacheEntry { detail, diff });
        }
        self.finish(&url).await;
    }
}

pub fn top_n(prs: &[PullRequest], n: usize) -> Vec<PullRequest> {
    if n == 0 || prs.is_empty() {
        return Vec::new();
    }
    let n = n.min(prs.len());
    prs[..n].to_vec()
}

pub fn neighbor_prs(prs: &[PullRequest], i: usize) -> Vec<PullRequest> {
    if prs.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for off in [-1i64, 1, 2] {
        let j = i as i64 + off;
        if j < 0 || j >= prs.len() as i64 {
            continue;
        }
        out.push(prs[j as usize].clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_should_start_rejects_empty_url() {
        let cache = Arc::new(DetailCache::with_dir(None));
        let p = Prefetcher::new(cache);
        assert!(
            !p.should_start(&PullRequest::default()).await,
            "an empty url is unfetchable"
        );
    }

    // A gh runner answering `pr view` with a fixed JSON and `pr diff` with
    // fixed text, so Prefetcher::run runs the real fetch path.
    fn canned_runner() -> Gh {
        let view = serde_json::json!({
            "number": 1,
            "title": "t",
            "url": "https://github.com/o/r/pull/1",
            "body": "b",
            "createdAt": "2026-05-01T00:00:00Z",
            "updatedAt": "2026-05-02T03:04:05Z",
            "baseRefName": "main",
            "headRefName": "feat/x",
            "reviewDecision": "",
            "mergeStateStatus": "CLEAN",
            "additions": 1,
            "deletions": 0,
            "changedFiles": 1,
            "author": {"login": "octocat"},
            "labels": [],
            "latestReviews": [],
        })
        .to_string()
        .into_bytes();
        Gh::with_runner(std::sync::Arc::new(move |args| {
            if args.len() > 1 && args[1] == "diff" {
                Ok(b"diff body".to_vec())
            } else {
                Ok(view.clone())
            }
        }))
    }

    fn pr_with_fixed_updated_at() -> PullRequest {
        PullRequest {
            url: "https://github.com/o/r/pull/1".into(),
            updated_at: gh::parse_rfc3339("2026-05-02T03:04:05Z"),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_run_fetches_and_caches() {
        let cache = Arc::new(DetailCache::with_dir(None));
        let p = Prefetcher::new(cache.clone());
        let pr = pr_with_fixed_updated_at();

        p.run(canned_runner(), pr.clone()).await;

        let key = gh::cache_key_of(&pr);
        let got = cache
            .get_mem(&key)
            .unwrap_or_else(|| panic!("the prefetched entry must land in the cache under {key}"));
        assert_eq!(got.diff, "diff body");
        assert_eq!(got.detail.body, "b");
    }

    #[tokio::test]
    async fn test_spawn_warms_the_cache_in_the_background() {
        let cache = Arc::new(DetailCache::with_dir(None));
        let p = Arc::new(Prefetcher::new(cache.clone()));
        let pr = pr_with_fixed_updated_at();
        let key = gh::cache_key_of(&pr);

        p.spawn(&canned_runner(), vec![pr]);

        for _ in 0..200 {
            if cache.get_mem(&key).is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("spawn must warm the cache for {key}");
    }

    fn pr(url: &str) -> PullRequest {
        PullRequest {
            url: url.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_top_n_returns_at_most_n() {
        let prs = vec![
            pr("https://example.test/pr/1"),
            pr("https://example.test/pr/2"),
        ];
        let got = top_n(&prs, 5);
        assert_eq!(got.len(), 2);
        let got = top_n(&prs, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].url, "https://example.test/pr/1");
        assert!(top_n(&[], 3).is_empty());
        assert!(top_n(&prs, 0).is_empty());
    }

    #[test]
    fn test_neighbor_prs_skips_out_of_range() {
        let prs: Vec<PullRequest> = (0..5).map(|i| pr(&i.to_string())).collect();
        let got = neighbor_prs(&prs, 0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].url, "1");
        assert_eq!(got[1].url, "2");
        let got = neighbor_prs(&prs, 2);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].url, "1");
        assert_eq!(got[1].url, "3");
        assert_eq!(got[2].url, "4");
        let got = neighbor_prs(&prs, 4);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].url, "3");
        assert!(neighbor_prs(&[], 0).is_empty());
    }

    #[tokio::test]
    async fn test_prefetcher_should_start_skips_cache_hit() {
        let cache = Arc::new(DetailCache::with_dir(None));
        let p = Prefetcher::new(cache.clone());
        let mut pr = pr("https://example.test/pr/cached");
        pr.updated_at = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100);
        cache.put(
            &gh::cache_key_of(&pr),
            CacheEntry {
                detail: Default::default(),
                diff: "x".into(),
            },
        );
        assert!(
            !p.should_start(&pr).await,
            "shouldStart must return false when entry is in cache"
        );
    }

    #[tokio::test]
    async fn test_prefetcher_should_start_dedups_inflight() {
        let cache = Arc::new(DetailCache::with_dir(None));
        let p = Prefetcher::new(cache);
        let pr = pr("https://example.test/pr/new");
        assert!(
            p.should_start(&pr).await,
            "first shouldStart should return true"
        );
        assert!(
            !p.should_start(&pr).await,
            "second shouldStart for same URL should be dedup'd"
        );
        p.finish(&pr.url).await;
        assert!(
            p.should_start(&pr).await,
            "shouldStart should be allowed again after finish"
        );
    }
}
