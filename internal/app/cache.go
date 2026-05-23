package app

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"
)

const (
	diskCacheDirName = "gh-review"
	diskCacheMaxFile = 200
)

type cacheEntry struct {
	Detail pullRequestDetail `json:"detail"`
	Diff   string            `json:"diff"`
}

// detailCache holds per-PR detail+diff in memory and on disk.
// Keyed by URL plus UpdatedAt so a PR update naturally invalidates entries
// without requiring an explicit flush.
type detailCache struct {
	mu  sync.Mutex
	mem map[string]cacheEntry
	dir string
}

func newDetailCache() *detailCache {
	c := &detailCache{mem: make(map[string]cacheEntry)}
	if base, err := os.UserCacheDir(); err == nil {
		c.dir = filepath.Join(base, diskCacheDirName)
	}
	return c
}

func cacheKey(url string, updatedAt time.Time) string {
	raw := url + "@" + updatedAt.UTC().Format(time.RFC3339Nano)
	sum := sha256.Sum256([]byte(raw))
	return hex.EncodeToString(sum[:])[:16]
}

func (c *detailCache) getMem(key string) (cacheEntry, bool) {
	if c == nil {
		return cacheEntry{}, false
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	e, ok := c.mem[key]
	return e, ok
}

func (c *detailCache) getDisk(key string) (cacheEntry, bool) {
	if c == nil || c.dir == "" {
		return cacheEntry{}, false
	}
	path := filepath.Join(c.dir, key+".json")
	data, err := os.ReadFile(path)
	if err != nil {
		return cacheEntry{}, false
	}
	var e cacheEntry
	if err := json.Unmarshal(data, &e); err != nil {
		return cacheEntry{}, false
	}
	// Touch mtime so the LRU pruning treats a hit as recent.
	now := time.Now()
	_ = os.Chtimes(path, now, now)
	c.mu.Lock()
	c.mem[key] = e
	c.mu.Unlock()
	return e, true
}

func (c *detailCache) put(key string, e cacheEntry) {
	if c == nil {
		return
	}
	c.mu.Lock()
	c.mem[key] = e
	c.mu.Unlock()
	c.writeDisk(key, e)
}

func (c *detailCache) writeDisk(key string, e cacheEntry) {
	if c.dir == "" {
		return
	}
	if err := os.MkdirAll(c.dir, 0o755); err != nil {
		return
	}
	data, err := json.Marshal(e)
	if err != nil {
		return
	}
	path := filepath.Join(c.dir, key+".json")
	tmp, err := os.CreateTemp(c.dir, key+".*.tmp")
	if err != nil {
		return
	}
	tmpName := tmp.Name()
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		os.Remove(tmpName)
		return
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmpName)
		return
	}
	if err := os.Rename(tmpName, path); err != nil {
		os.Remove(tmpName)
		return
	}
	c.pruneDisk()
}

func (c *detailCache) pruneDisk() {
	if c.dir == "" {
		return
	}
	entries, err := os.ReadDir(c.dir)
	if err != nil {
		return
	}
	type fileInfo struct {
		path  string
		mtime time.Time
	}
	files := make([]fileInfo, 0, len(entries))
	for _, ent := range entries {
		if ent.IsDir() {
			continue
		}
		name := ent.Name()
		if filepath.Ext(name) != ".json" {
			continue
		}
		info, err := ent.Info()
		if err != nil {
			continue
		}
		files = append(files, fileInfo{
			path:  filepath.Join(c.dir, name),
			mtime: info.ModTime(),
		})
	}
	if len(files) <= diskCacheMaxFile {
		return
	}
	sort.Slice(files, func(i, j int) bool {
		return files[i].mtime.Before(files[j].mtime)
	})
	for i := 0; i < len(files)-diskCacheMaxFile; i++ {
		_ = os.Remove(files[i].path)
	}
}
