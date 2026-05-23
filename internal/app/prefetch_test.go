package app

import (
	"testing"
	"time"
)

func TestTopNReturnsAtMostN(t *testing.T) {
	prs := []pullRequest{
		{URL: "https://example.test/pr/1"},
		{URL: "https://example.test/pr/2"},
	}
	got := topN(prs, 5)
	if len(got) != 2 {
		t.Fatalf("topN with n>len should return full slice, got %d", len(got))
	}
	got = topN(prs, 1)
	if len(got) != 1 || got[0].URL != "https://example.test/pr/1" {
		t.Fatalf("topN(1) = %+v", got)
	}
	if topN(nil, 3) != nil {
		t.Fatal("topN(nil) should be nil")
	}
	if topN(prs, 0) != nil {
		t.Fatal("topN(_, 0) should be nil")
	}
}

func TestNeighborPRsSkipsOutOfRange(t *testing.T) {
	prs := []pullRequest{
		{URL: "0"}, {URL: "1"}, {URL: "2"}, {URL: "3"}, {URL: "4"},
	}
	// i=0: skip -1, include 1, 2
	got := neighborPRs(prs, 0)
	if len(got) != 2 || got[0].URL != "1" || got[1].URL != "2" {
		t.Fatalf("neighborPRs(0) = %+v", got)
	}
	// i=2: include 1, 3, 4
	got = neighborPRs(prs, 2)
	if len(got) != 3 || got[0].URL != "1" || got[1].URL != "3" || got[2].URL != "4" {
		t.Fatalf("neighborPRs(2) = %+v", got)
	}
	// i=4 (last): include 3, skip 5, skip 6
	got = neighborPRs(prs, 4)
	if len(got) != 1 || got[0].URL != "3" {
		t.Fatalf("neighborPRs(last) = %+v", got)
	}
	if neighborPRs(nil, 0) != nil {
		t.Fatal("neighborPRs(nil) should be nil")
	}
}

func TestPrefetcherShouldStartSkipsCacheHit(t *testing.T) {
	cache := newTestCache(t)
	p := newPrefetcher(cache)
	pr := pullRequest{
		URL:       "https://example.test/pr/cached",
		UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
	}
	cache.put(cacheKey(pr.URL, pr.UpdatedAt), cacheEntry{Diff: "x"})
	if p.shouldStart(pr) {
		t.Fatal("shouldStart must return false when entry is in cache")
	}
}

func TestPrefetcherShouldStartDedupsInflight(t *testing.T) {
	p := newPrefetcher(newTestCache(t))
	pr := pullRequest{
		URL:       "https://example.test/pr/new",
		UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
	}
	if !p.shouldStart(pr) {
		t.Fatal("first shouldStart should return true")
	}
	if p.shouldStart(pr) {
		t.Fatal("second shouldStart for same URL should be dedup'd")
	}
	p.finish(pr.URL)
	if !p.shouldStart(pr) {
		t.Fatal("shouldStart should be allowed again after finish")
	}
}

func TestPrefetchCmdReturnsNilMsg(t *testing.T) {
	p := newPrefetcher(newTestCache(t))
	// Empty input should produce no command at all.
	if cmd := p.prefetchCmd(nil); cmd != nil {
		t.Fatal("prefetchCmd(nil) should return nil tea.Cmd")
	}
	// Non-empty input returns a cmd whose msg is nil. We don't invoke run()
	// here because it would call out to gh; instead pre-populate inflight so
	// shouldStart returns false and the goroutine is never spawned.
	pr := pullRequest{
		URL:       "https://example.test/pr/skip",
		UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
	}
	p.inflight[pr.URL] = struct{}{}
	cmd := p.prefetchCmd([]pullRequest{pr})
	if cmd == nil {
		t.Fatal("prefetchCmd with prs should return a tea.Cmd")
	}
	if msg := cmd(); msg != nil {
		t.Fatalf("prefetchCmd msg should be nil, got %T", msg)
	}
}
