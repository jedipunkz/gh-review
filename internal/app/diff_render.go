package app

import (
	"strings"

	"charm.land/lipgloss/v2"
)

var (
	diffFileHeaderStyle = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("11"))
	diffMetaStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
	diffOldFileStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("174"))
	diffNewFileStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("108"))
	diffHunkStyle       = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("51"))
	diffAddStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	diffDelStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color("196"))
)

func highlightDiff(raw string) string {
	if raw == "" {
		return raw
	}
	var b strings.Builder
	b.Grow(len(raw) + len(raw)/4)
	lines := strings.Split(raw, "\n")
	for i, line := range lines {
		b.WriteString(styleDiffLine(line))
		if i < len(lines)-1 {
			b.WriteByte('\n')
		}
	}
	return b.String()
}

func styleDiffLine(line string) string {
	switch {
	case strings.HasPrefix(line, "diff --git "):
		return diffFileHeaderStyle.Render(line)
	case strings.HasPrefix(line, "index "),
		strings.HasPrefix(line, "new file mode "),
		strings.HasPrefix(line, "deleted file mode "),
		strings.HasPrefix(line, "similarity index "),
		strings.HasPrefix(line, "rename "),
		strings.HasPrefix(line, "copy "):
		return diffMetaStyle.Render(line)
	case strings.HasPrefix(line, "--- "):
		return diffOldFileStyle.Render(line)
	case strings.HasPrefix(line, "+++ "):
		return diffNewFileStyle.Render(line)
	case strings.HasPrefix(line, "@@ "):
		return diffHunkStyle.Render(line)
	case strings.HasPrefix(line, "+"):
		return diffAddStyle.Render(line)
	case strings.HasPrefix(line, "-"):
		return diffDelStyle.Render(line)
	default:
		return line
	}
}
