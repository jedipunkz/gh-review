package app

import "strings"

// diffFile records a single file boundary detected within a unified diff
// payload. StartLine refers to the 0-indexed line number of the leading
// `diff --git` header within the diff (raw and highlighted variants share the
// same line layout because highlightDiff preserves newline positions).
type diffFile struct {
	Path      string
	StartLine int
}

// splitDiffFiles parses a raw `git diff` payload and returns one diffFile per
// `diff --git a/<X> b/<Y>` header in the order they appear. The Path field
// resolves to the post-rename / new path (`b/Y`) by default, falling back to
// the old path (`a/X`) for deletions detected via a `deleted file mode` meta
// line that follows the header.
func splitDiffFiles(raw string) []diffFile {
	if raw == "" {
		return nil
	}
	lines := strings.Split(raw, "\n")
	var files []diffFile
	for i, line := range lines {
		if !strings.HasPrefix(line, "diff --git ") {
			continue
		}
		path, deleted := parseDiffGitPaths(line)
		if path == "" {
			continue
		}
		if deleted {
			// already chose the a/ side in parseDiffGitPaths
		} else if hasDeletedMarker(lines, i+1) {
			if a := pickASide(line); a != "" {
				path = a
			}
		}
		files = append(files, diffFile{Path: path, StartLine: i})
	}
	return files
}

// parseDiffGitPaths extracts the `b/` side path from a `diff --git` header.
// When the header lists identical paths (typical) the post-image path is
// returned. When parsing fails it returns an empty string. The second return
// value is reserved for future use; deletions are currently inferred from
// surrounding meta lines via hasDeletedMarker.
func parseDiffGitPaths(line string) (string, bool) {
	const prefix = "diff --git "
	rest := strings.TrimPrefix(line, prefix)
	// Expected forms: `a/foo b/foo` or `"a/foo bar" "b/foo bar"`. We avoid the
	// quoted form for simplicity — git only quotes when filenames contain
	// special characters, which is rare in PRs we render.
	parts := strings.Fields(rest)
	if len(parts) < 2 {
		return "", false
	}
	b := parts[len(parts)-1]
	if strings.HasPrefix(b, "b/") {
		return strings.TrimPrefix(b, "b/"), false
	}
	// Fallback: try the a/ side.
	a := parts[0]
	if strings.HasPrefix(a, "a/") {
		return strings.TrimPrefix(a, "a/"), true
	}
	return b, false
}

// pickASide returns the a/<X> path (without the `a/` prefix) from a diff --git
// header, used as fallback for deleted files.
func pickASide(line string) string {
	const prefix = "diff --git "
	rest := strings.TrimPrefix(line, prefix)
	parts := strings.Fields(rest)
	if len(parts) < 2 {
		return ""
	}
	a := parts[0]
	if strings.HasPrefix(a, "a/") {
		return strings.TrimPrefix(a, "a/")
	}
	return ""
}

// hasDeletedMarker scans the meta block that follows a `diff --git` header to
// see if the file is being deleted. The meta block ends once the next
// `diff --git` header or hunk (`@@`) is encountered.
func hasDeletedMarker(lines []string, start int) bool {
	for i := start; i < len(lines); i++ {
		l := lines[i]
		if strings.HasPrefix(l, "diff --git ") || strings.HasPrefix(l, "@@ ") {
			return false
		}
		if strings.HasPrefix(l, "deleted file mode ") {
			return true
		}
	}
	return false
}
