package app

import (
	"strings"
	"testing"
	"time"

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

	want := m.width
	if got := m.frameWidth(); got != want {
		t.Fatalf("frame width = %d, want %d", got, want)
	}
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

func TestApproveLabelUsesReviewDecision(t *testing.T) {
	m := newModel()
	tests := []struct {
		name string
		pr   pullRequest
		want string
	}{
		{
			name: "approved",
			pr:   pullRequest{ReviewDecision: "APPROVED"},
			want: "approved",
		},
		{
			name: "changes requested",
			pr:   pullRequest{ReviewDecision: "CHANGES_REQUESTED"},
			want: "changes",
		},
		{
			name: "review required",
			pr:   pullRequest{ReviewDecision: "REVIEW_REQUIRED"},
			want: "required",
		},
		{
			name: "unknown",
			pr:   pullRequest{},
			want: "-",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := m.approveLabel(tt.pr); got != tt.want {
				t.Fatalf("approveLabel() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestApproveLabelUsesLocalApprovedState(t *testing.T) {
	m := newModel()
	pr := pullRequest{URL: "https://example.test/pr/1"}
	m.approved[pr.URL] = true

	if got := m.approveLabel(pr); got != "approved" {
		t.Fatalf("approveLabel() = %q, want approved", got)
	}
}

func TestUpdateCheckShowsNoticeWhenSignatureChanges(t *testing.T) {
	prs := []pullRequest{
		{
			URL:       "https://example.test/pr/1",
			UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
		},
	}
	m := newModel()
	m.prs = prs
	m.prSignature = prListSignature(prs)

	newPRs := []pullRequest{
		{
			URL:       "https://example.test/pr/1",
			UpdatedAt: time.Date(2026, 5, 14, 10, 1, 0, 0, time.UTC),
		},
	}
	updated, _ := m.Update(updateCheckMsg{
		previousSignature: m.prSignature,
		currentSignature:  prListSignature(newPRs),
		previousCount:     1,
		count:             1,
		prs:               newPRs,
	})

	got := updated.(model)
	if got.updateNotice == nil {
		t.Fatal("update check did not show notice")
	}
	if got.status != "1 review request(s)" {
		t.Fatalf("status = %q, want 1 review request(s)", got.status)
	}
	if !got.prs[0].UpdatedAt.Equal(newPRs[0].UpdatedAt) {
		t.Fatal("update check did not refresh prs in place")
	}
}

func TestUpdateCheckShowsNoticeWhenPRsIncrease(t *testing.T) {
	prs := []pullRequest{
		{
			URL:       "https://example.test/pr/1",
			UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
		},
	}
	m := newModel()
	m.prs = prs
	m.prSignature = prListSignature(prs)

	newPRs := []pullRequest{
		prs[0],
		{
			URL:       "https://example.test/pr/2",
			UpdatedAt: time.Date(2026, 5, 14, 10, 5, 0, 0, time.UTC),
		},
	}
	updated, cmd := m.Update(updateCheckMsg{
		previousSignature: m.prSignature,
		currentSignature:  prListSignature(newPRs),
		previousCount:     1,
		count:             2,
		prs:               newPRs,
	})

	got := updated.(model)
	if got.updateNotice == nil {
		t.Fatal("increased PR count should show notice")
	}
	if cmd == nil {
		t.Fatal("increased PR count should return sound command")
	}
}

func TestUpdateCheckSilentReloadWhenPRsDecrease(t *testing.T) {
	prs := []pullRequest{
		{
			URL:       "https://example.test/pr/1",
			UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
		},
		{
			URL:       "https://example.test/pr/2",
			UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
		},
	}
	m := newModel()
	m.loading = false
	m.prListLoaded = true
	m.prs = prs
	m.prSignature = prListSignature(prs)

	newPRs := []pullRequest{prs[0]}
	updated, cmd := m.Update(updateCheckMsg{
		previousSignature: m.prSignature,
		currentSignature:  prListSignature(newPRs),
		previousCount:     2,
		count:             1,
		prs:               newPRs,
	})

	got := updated.(model)
	if got.updateNotice != nil {
		t.Fatal("decreased PR count should not show notice")
	}
	if cmd == nil {
		t.Fatal("decreased PR count should return reload command")
	}

	reloadMsg, ok := cmd().(prListMsg)
	if !ok {
		t.Fatalf("decreased PR count should dispatch prListMsg, got %T", cmd())
	}
	if len(reloadMsg.prs) != 1 {
		t.Fatalf("reload msg should contain 1 pr, got %d", len(reloadMsg.prs))
	}
}

func TestUpdateCheckMarksNewPRs(t *testing.T) {
	prs := []pullRequest{
		{URL: "https://example.test/pr/1"},
	}
	m := newModel()
	m.prs = prs
	m.prSignature = prListSignature(prs)

	newPRs := []pullRequest{
		prs[0],
		{URL: "https://example.test/pr/2"},
	}
	updated, _ := m.Update(updateCheckMsg{
		previousSignature: m.prSignature,
		currentSignature:  prListSignature(newPRs),
		previousCount:     1,
		count:             2,
		prs:               newPRs,
	})

	got := updated.(model)
	if !got.markedPRs["https://example.test/pr/2"] {
		t.Fatal("new PR was not marked")
	}
	if got.markedPRs["https://example.test/pr/1"] {
		t.Fatal("previously known PR should not be marked")
	}
}

func TestKeyPressClearsMarkOnSelectedPR(t *testing.T) {
	m := newModel()
	m.loading = false
	m.prs = []pullRequest{
		{URL: "https://example.test/pr/1"},
		{URL: "https://example.test/pr/2"},
	}
	m.markedPRs["https://example.test/pr/1"] = true
	m.markedPRs["https://example.test/pr/2"] = true
	m.cursor = 1

	updated, _ := m.handleKey(keyMsg("j"))
	got := updated.(model)
	if got.markedPRs["https://example.test/pr/2"] {
		t.Fatal("mark on selected PR should be cleared on key press")
	}
	if !got.markedPRs["https://example.test/pr/1"] {
		t.Fatal("mark on non-selected PR should remain")
	}
}

func TestPopupDismissMsgClearsCurrentNoticeOnly(t *testing.T) {
	m := newModel()
	m.updateNotice = &updateNotice{count: 1, id: 5}
	m.popupSeq = 5
	m.prs = []pullRequest{{URL: "u"}}

	updated, _ := m.Update(popupDismissMsg{id: 4})
	if updated.(model).updateNotice == nil {
		t.Fatal("stale dismiss should not clear notice")
	}

	updated, _ = updated.(model).Update(popupDismissMsg{id: 5})
	if updated.(model).updateNotice != nil {
		t.Fatal("matching dismiss should clear notice")
	}
}

func TestKeyPressDismissesUpdateNotice(t *testing.T) {
	m := newModel()
	m.loading = false
	m.prs = []pullRequest{{URL: "https://example.test/pr/1"}}
	m.popupSeq = 7
	m.updateNotice = &updateNotice{count: 1, id: 7}

	updated, _ := m.handleKey(keyMsg("j"))
	if updated.(model).updateNotice != nil {
		t.Fatal("key press should dismiss update notice")
	}
}

func TestOverlayUpdateNoticePreservesBaseAndPlacesPopupBottomRight(t *testing.T) {
	m := newModel()
	m.width = 60
	m.height = 12
	m.updateNotice = &updateNotice{count: 3, id: 1}

	base := strings.Repeat("ABCDEFGHIJ", 6)
	lines := make([]string, m.height)
	for i := range lines {
		lines[i] = base
	}
	joined := strings.Join(lines, "\n")

	rendered := m.overlayUpdateNotice(joined)
	renderedLines := strings.Split(rendered, "\n")
	if len(renderedLines) != m.height {
		t.Fatalf("rendered height = %d, want %d", len(renderedLines), m.height)
	}

	if !strings.HasPrefix(renderedLines[0], "ABCDEFGHIJ") {
		t.Fatalf("top-left line should keep base content, got %q", renderedLines[0])
	}
	if !strings.HasPrefix(renderedLines[m.height/2], "ABCDEFGHIJ") {
		t.Fatalf("middle line should keep base content, got %q", renderedLines[m.height/2])
	}

	popupRow := renderedLines[m.height-3]
	if !strings.Contains(popupRow, "Review updated") && !strings.Contains(popupRow, "review request") {
		// the popup spans multiple rows; ensure at least one of the popup rows
		// near the bottom contains popup content.
		joinedTail := strings.Join(renderedLines[m.height-5:], "\n")
		if !strings.Contains(joinedTail, "Review updated") {
			t.Fatalf("popup not present in bottom rows: %q", joinedTail)
		}
	}
}

func TestRenderListShowsMarkForNewPR(t *testing.T) {
	m := newModel()
	m.width = 120
	m.height = 30
	m.loading = false
	m.prs = []pullRequest{
		{
			Repository: "owner/repo",
			Number:     1,
			Title:      "first",
			URL:        "https://example.test/pr/1",
			Author:     "alice",
			Request:    "@me",
		},
		{
			Repository: "owner/repo",
			Number:     2,
			Title:      "second",
			URL:        "https://example.test/pr/2",
			Author:     "bob",
			Request:    "@me",
		},
	}
	m.markedPRs["https://example.test/pr/2"] = true
	m.resizeViewport()

	out := m.renderGroupedList()
	if !strings.Contains(out, "[!]") {
		t.Fatal("expected [!] mark in rendered list")
	}
}

func TestPrListMsgPrunesStaleMarks(t *testing.T) {
	m := newModel()
	m.markedPRs["https://example.test/pr/1"] = true
	m.markedPRs["https://example.test/pr/2"] = true

	updated, _ := m.Update(prListMsg{prs: []pullRequest{{URL: "https://example.test/pr/1"}}})
	got := updated.(model)
	if got.markedPRs["https://example.test/pr/2"] {
		t.Fatal("mark for removed PR should be pruned")
	}
	if !got.markedPRs["https://example.test/pr/1"] {
		t.Fatal("mark for surviving PR should remain")
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
