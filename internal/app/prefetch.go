package app

import (
	"context"
	"sync"
	"time"

	tea "charm.land/bubbletea/v2"
	"golang.org/x/sync/errgroup"
)

const (
	prefetchTimeout     = 60 * time.Second
	prefetchConcurrency = 3
	prefetchTopN        = 3
)

// prefetcher fetches PR detail+diff in the background and writes the results
// into the shared detailCache. It is intentionally decoupled from the
// inflightLoader used by the foreground load: prefetches must run to
// completion regardless of cursor moves so the cache warms up.
type prefetcher struct {
	cache *detailCache
	sem   chan struct{}

	mu       sync.Mutex
	inflight map[string]struct{}
}

func newPrefetcher(cache *detailCache) *prefetcher {
	return &prefetcher{
		cache:    cache,
		sem:      make(chan struct{}, prefetchConcurrency),
		inflight: make(map[string]struct{}),
	}
}

// shouldStart reports whether a prefetch for pr should be launched. It returns
// false if the cache already contains the entry or another prefetch for the
// same URL is in flight. On true it marks the URL as inflight; the caller is
// responsible for calling finish when the goroutine exits.
func (p *prefetcher) shouldStart(pr pullRequest) bool {
	if p == nil || pr.URL == "" {
		return false
	}
	if p.cache != nil {
		if _, ok := p.cache.getMem(cacheKey(pr.URL, pr.UpdatedAt)); ok {
			return false
		}
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if _, busy := p.inflight[pr.URL]; busy {
		return false
	}
	p.inflight[pr.URL] = struct{}{}
	return true
}

func (p *prefetcher) finish(url string) {
	if p == nil {
		return
	}
	p.mu.Lock()
	delete(p.inflight, url)
	p.mu.Unlock()
}

// run blocks on the semaphore to bound concurrency, then fetches detail and
// diff in parallel and writes the result to the cache. It deliberately does
// not produce a tea.Msg: the cache is the only output.
func (p *prefetcher) run(pr pullRequest) {
	defer p.finish(pr.URL)

	p.sem <- struct{}{}
	defer func() { <-p.sem }()

	ctx, cancel := context.WithTimeout(context.Background(), prefetchTimeout)
	defer cancel()

	var (
		detail pullRequestDetail
		diff   string
	)
	g, gctx := errgroup.WithContext(ctx)
	g.Go(func() error {
		d, err := loadPRDetail(gctx, pr)
		if err != nil {
			return err
		}
		detail = d
		return nil
	})
	g.Go(func() error {
		d, err := loadDiff(gctx, pr)
		if isPRDiffTooLargeError(err) {
			diff = mutedStyle.Render("Diff omitted because GitHub reports this PR diff is too large to display.")
			return nil
		}
		if err != nil {
			return err
		}
		diff = d
		return nil
	})
	if err := g.Wait(); err != nil {
		return
	}
	if p.cache == nil {
		return
	}
	key := cacheKey(pr.URL, detail.UpdatedAt)
	p.cache.put(key, cacheEntry{Detail: detail, Diff: diff})
}

// prefetchCmd returns a tea.Cmd that, when executed, launches background
// prefetches for the supplied PRs. The command itself returns nil so the
// bubbletea message loop is unaffected; the cache is updated asynchronously.
func (p *prefetcher) prefetchCmd(prs []pullRequest) tea.Cmd {
	if p == nil || len(prs) == 0 {
		return nil
	}
	return func() tea.Msg {
		for _, pr := range prs {
			if !p.shouldStart(pr) {
				continue
			}
			go p.run(pr)
		}
		return nil
	}
}

// topN returns the first n PRs (or fewer if the slice is shorter).
func topN(prs []pullRequest, n int) []pullRequest {
	if n <= 0 || len(prs) == 0 {
		return nil
	}
	if len(prs) < n {
		n = len(prs)
	}
	return prs[:n]
}

// neighborPRs returns PRs at indices i-1, i+1, i+2 (skipping out-of-range).
func neighborPRs(prs []pullRequest, i int) []pullRequest {
	if len(prs) == 0 {
		return nil
	}
	offsets := []int{-1, 1, 2}
	out := make([]pullRequest, 0, len(offsets))
	for _, off := range offsets {
		j := i + off
		if j < 0 || j >= len(prs) {
			continue
		}
		out = append(out, prs[j])
	}
	return out
}
