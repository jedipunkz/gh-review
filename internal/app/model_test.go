package app

import (
	"testing"

	tea "charm.land/bubbletea/v2"
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

func TestApproveKeyRequiresConfirmation(t *testing.T) {
	m := modelWithLoadedDetail()

	updated, cmd := m.handleKey(keyMsg("a"))
	if cmd != nil {
		t.Fatal("approve key returned command before confirmation")
	}

	got := updated.(model)
	if got.pendingApprove == nil {
		t.Fatal("approve key did not set pending confirmation")
	}
	if got.loading {
		t.Fatal("approve key set loading before confirmation")
	}
}

func TestApproveConfirmationYesStartsApprove(t *testing.T) {
	m := modelWithLoadedDetail()
	updated, _ := m.handleKey(keyMsg("a"))
	m = updated.(model)

	updated, cmd := m.handleKey(keyMsg("y"))
	if cmd == nil {
		t.Fatal("yes confirmation did not return approve command")
	}

	got := updated.(model)
	if got.pendingApprove != nil {
		t.Fatal("yes confirmation did not clear pending approval")
	}
	if !got.loading {
		t.Fatal("yes confirmation did not set loading")
	}
}

func TestApproveConfirmationNoCancelsApprove(t *testing.T) {
	m := modelWithLoadedDetail()
	updated, _ := m.handleKey(keyMsg("a"))
	m = updated.(model)

	updated, cmd := m.handleKey(keyMsg("n"))
	if cmd != nil {
		t.Fatal("no confirmation returned command")
	}

	got := updated.(model)
	if got.pendingApprove != nil {
		t.Fatal("no confirmation did not clear pending approval")
	}
	if got.loading {
		t.Fatal("no confirmation set loading")
	}
}

func modelWithLoadedDetail() model {
	m := newModel()
	m.loading = false
	m.prs = []pullRequest{
		{
			Repository: "owner/repo",
			Number:     123,
			Title:      "Test PR",
			URL:        "https://example.test/pr/123",
			Author:     "octocat",
			Request:    "@me",
		},
	}
	detail := pullRequestDetail{pullRequest: m.prs[0]}
	m.currentDetail = &detail
	return m
}

func keyMsg(key string) tea.KeyPressMsg {
	return tea.KeyPressMsg(tea.Key{Text: key, Code: []rune(key)[0]})
}
