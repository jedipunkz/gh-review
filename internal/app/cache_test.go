package app

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func newTestCache(t *testing.T) *detailCache {
	t.Helper()
	c := &detailCache{
		mem: make(map[string]cacheEntry),
		dir: t.TempDir(),
	}
	return c
}

func TestCacheKeyChangesWithUpdatedAt(t *testing.T) {
	url := "https://example.test/pr/1"
	t1 := time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC)
	t2 := t1.Add(time.Second)
	if cacheKey(url, t1) == cacheKey(url, t2) {
		t.Fatal("cache key must change when UpdatedAt changes")
	}
}

func TestCachePutThenGetMem(t *testing.T) {
	c := newTestCache(t)
	key := "abc1234567890def"
	entry := cacheEntry{
		Detail: pullRequestDetail{pullRequest: pullRequest{URL: "u", Number: 1}},
		Diff:   "diff body",
	}
	c.put(key, entry)

	got, ok := c.getMem(key)
	if !ok {
		t.Fatal("memory cache miss after put")
	}
	if got.Diff != "diff body" || got.Detail.Number != 1 {
		t.Fatalf("memory cache returned wrong entry: %+v", got)
	}
}

func TestCacheGetDiskAfterFreshLoad(t *testing.T) {
	c := newTestCache(t)
	key := "abc1234567890def"
	entry := cacheEntry{
		Detail: pullRequestDetail{pullRequest: pullRequest{URL: "u", Number: 1}},
		Diff:   "diff body",
	}
	c.put(key, entry)

	// Simulate a fresh process by clearing memory only.
	c.mem = make(map[string]cacheEntry)

	got, ok := c.getDisk(key)
	if !ok {
		t.Fatal("disk cache miss after put")
	}
	if got.Diff != "diff body" || got.Detail.Number != 1 {
		t.Fatalf("disk cache returned wrong entry: %+v", got)
	}
	if _, ok := c.getMem(key); !ok {
		t.Fatal("getDisk should populate memory cache")
	}
}

func TestCachePruneRemovesOldestFiles(t *testing.T) {
	c := newTestCache(t)
	total := diskCacheMaxFile + 5
	old := time.Now().Add(-1 * time.Hour)
	for i := 0; i < total; i++ {
		key := cacheKey("https://example.test/pr/x", time.Unix(int64(i), 0))
		c.put(key, cacheEntry{Diff: "x"})
		// Backdate older files so prune deletes them first.
		if i < 5 {
			path := filepath.Join(c.dir, key+".json")
			if err := os.Chtimes(path, old, old); err != nil {
				t.Fatal(err)
			}
		}
	}
	c.pruneDisk()

	entries, err := os.ReadDir(c.dir)
	if err != nil {
		t.Fatal(err)
	}
	jsonCount := 0
	for _, e := range entries {
		if filepath.Ext(e.Name()) == ".json" {
			jsonCount++
		}
	}
	if jsonCount > diskCacheMaxFile {
		t.Fatalf("prune left %d files, want <= %d", jsonCount, diskCacheMaxFile)
	}
}
