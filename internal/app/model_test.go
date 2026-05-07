package app

import (
	"testing"

	"charm.land/lipgloss/v2"
)

func TestFrameSectionsUseSameRenderedWidth(t *testing.T) {
	m := newModel()
	m.width = 100
	m.height = 30
	m.loading = false
	m.prs = []pullRequest{
		{
			Repository: "jedipunkz/gcp-playground",
			Number:     2,
			Title:      "Add draw.io architecture diagram for Cloud Run",
			URL:        "https://example.test/pr/2",
			Author:     "Copilot",
			Request:    "@me",
		},
		{
			Repository: "jedipunkz/datadog-log-tail",
			Number:     17,
			Title:      "Improve Datadog API rate limit handling",
			URL:        "https://example.test/pr/17",
			Author:     "Copilot",
			Request:    "@me",
		},
	}
	detail := pullRequestDetail{pullRequest: m.prs[0], Body: "detail"}
	m.currentDetail = &detail
	m.resizeViewport()
	m.detailVP.SetContent("detail")

	want := m.frameWidth()
	if got := lipgloss.Width(m.renderGroupedList()); got != want {
		t.Fatalf("list width = %d, want %d", got, want)
	}
	if got := lipgloss.Width(m.renderDetailSection()); got != want {
		t.Fatalf("detail width = %d, want %d", got, want)
	}
}
