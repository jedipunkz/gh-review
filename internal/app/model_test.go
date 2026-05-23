package app

import (
	"context"
	"errors"
	"reflect"
	"strconv"
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

func TestRenderDiffContentShowsReviewedBy(t *testing.T) {
	detail := pullRequestDetail{
		pullRequest: pullRequest{
			Repository: "owner/repo",
			Number:     42,
			Title:      "Add reviewer header",
			Author:     "octocat",
			Request:    "@me",
		},
		Reviewers: []reviewSummary{
			{Author: "alice", State: "APPROVED"},
			{Author: "bob", State: "CHANGES_REQUESTED"},
		},
	}

	out := renderDiffContent(detail, "")
	for _, want := range []string{"Reviewed by:", "@alice", "APPROVED", "@bob", "CHANGES_REQUESTED"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered detail missing %q: %q", want, out)
		}
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

func TestUpdateCheckPlaysSoundOnlyForNewReviewRequests(t *testing.T) {
	orig := playNotifySoundCmd
	var playCount int
	playNotifySoundCmd = func() tea.Cmd {
		playCount++
		return nil
	}
	t.Cleanup(func() { playNotifySoundCmd = orig })

	prs := []pullRequest{
		{
			URL:       "https://example.test/pr/1",
			UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
			Request:   "@me",
		},
	}

	m := newModel()
	m.prs = prs
	m.prSignature = prListSignature(prs)
	updatedExisting := []pullRequest{
		{
			URL:       "https://example.test/pr/1",
			UpdatedAt: time.Date(2026, 5, 14, 10, 5, 0, 0, time.UTC),
			Request:   "@me",
		},
	}
	updated, _ := m.Update(updateCheckMsg{
		previousSignature: m.prSignature,
		currentSignature:  prListSignature(updatedExisting),
		previousCount:     1,
		count:             1,
		prs:               updatedExisting,
	})
	if playCount != 0 {
		t.Fatalf("signature-only change should not play sound, playCount=%d", playCount)
	}

	m = updated.(model)
	added := []pullRequest{
		updatedExisting[0],
		{
			URL:       "https://example.test/pr/2",
			UpdatedAt: time.Date(2026, 5, 14, 10, 6, 0, 0, time.UTC),
			Request:   "@me",
		},
	}
	m.Update(updateCheckMsg{
		previousSignature: m.prSignature,
		currentSignature:  prListSignature(added),
		previousCount:     1,
		count:             2,
		prs:               added,
	})
	if playCount != 1 {
		t.Fatalf("new review request should play sound once, playCount=%d", playCount)
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

func TestListShowsAtMostMaxListItems(t *testing.T) {
	m := newModel()
	m.width = 120
	m.height = 40
	m.loading = false
	for i := range maxListItems + 5 {
		m.prs = append(m.prs, pullRequest{
			Repository: "owner/repo",
			Number:     i + 1,
			Title:      "title",
			URL:        prURL(i),
			Author:     "alice",
			Request:    "@me",
		})
	}
	m.resizeViewport()

	out := m.renderGroupedList()
	for i := range maxListItems {
		want := "#" + itoa(i+1)
		if !strings.Contains(out, want) {
			t.Fatalf("expected list to contain %q", want)
		}
	}
	for i := maxListItems; i < len(m.prs); i++ {
		notWant := "#" + itoa(i+1)
		if strings.Contains(out, notWant) {
			t.Fatalf("expected list to hide %q when scrolled to top", notWant)
		}
	}
}

func TestCtrlNScrollsBeyondMaxListItems(t *testing.T) {
	m := newModel()
	m.width = 120
	m.height = 40
	m.loading = false
	total := maxListItems + 3
	for i := range total {
		m.prs = append(m.prs, pullRequest{
			Repository: "owner/repo",
			Number:     i + 1,
			Title:      "title",
			URL:        prURL(i),
			Author:     "alice",
			Request:    "@me",
		})
	}
	m.resizeViewport()

	got := tea.Model(m)
	for range maxListItems {
		next, _ := got.(model).handleKey(keyMsg("ctrl+n"))
		got = next
	}
	final := got.(model)
	if final.cursor != maxListItems {
		t.Fatalf("cursor = %d, want %d", final.cursor, maxListItems)
	}
	if final.listOffset != 1 {
		t.Fatalf("listOffset = %d, want 1", final.listOffset)
	}

	out := final.renderGroupedList()
	if strings.Contains(out, "#1 ") || strings.Contains(out, "#1 ") {
		t.Fatalf("first PR should be scrolled out of view: %q", out)
	}
	want := "#" + itoa(maxListItems+1)
	if !strings.Contains(out, want) {
		t.Fatalf("expected list to reveal %q after scrolling", want)
	}
}

func prURL(i int) string {
	return "https://example.test/pr/" + strconv.Itoa(i+1)
}

func itoa(i int) string {
	return strconv.Itoa(i)
}

func TestInflightLoaderBeginCancelsPrevious(t *testing.T) {
	l := newInflightLoader()
	firstCtx, firstDone := l.begin(time.Minute)
	defer firstDone()
	if firstCtx.Err() != nil {
		t.Fatalf("first ctx canceled prematurely: %v", firstCtx.Err())
	}

	secondCtx, secondDone := l.begin(time.Minute)
	defer secondDone()

	if !errors.Is(firstCtx.Err(), context.Canceled) {
		t.Fatalf("first ctx should be canceled after second begin, got %v", firstCtx.Err())
	}
	if secondCtx.Err() != nil {
		t.Fatalf("second ctx should be live, got %v", secondCtx.Err())
	}
}

func TestInflightLoaderCancelStopsCurrent(t *testing.T) {
	l := newInflightLoader()
	ctx, done := l.begin(time.Minute)
	defer done()

	l.cancel()
	if !errors.Is(ctx.Err(), context.Canceled) {
		t.Fatalf("ctx should be canceled, got %v", ctx.Err())
	}
}

func TestInflightLoaderDoneFromOlderGenerationDoesNotWipeNewer(t *testing.T) {
	l := newInflightLoader()
	_, firstDone := l.begin(time.Minute)
	_, secondDone := l.begin(time.Minute)
	defer secondDone()

	// Older generation finishes after the newer one took over. It must not
	// clear the newer generation's stored cancel func.
	firstDone()

	l.mu.Lock()
	if l.cancelFn == nil {
		l.mu.Unlock()
		t.Fatal("newer generation cancel was wiped by older done()")
	}
	l.mu.Unlock()
}

func TestDiffMsgWithContextCanceledIsIgnored(t *testing.T) {
	m := newModel()
	m.loading = false
	m.detailLoading = true
	m.detailErr = ""
	m.loadingForURL = "https://example.test/pr/1"
	m.prs = []pullRequest{{URL: "https://example.test/pr/1"}}

	updated, _ := m.Update(diffMsg{pr: m.prs[0], err: context.Canceled})
	got := updated.(model)
	if got.detailErr != "" {
		t.Fatalf("canceled diff should not set detailErr, got %q", got.detailErr)
	}
	if !got.detailLoading {
		t.Fatal("canceled diff should leave detailLoading=true for the newer load to finish")
	}
}

func TestDebounceFireMsgWithStaleSeqIsIgnored(t *testing.T) {
	m := newModel()
	m.loading = false
	m.prs = []pullRequest{{URL: "https://example.test/pr/1"}}
	m.loadingForURL = "https://example.test/pr/1"
	m.debounceSeq = 5

	_, cmd := m.Update(debounceFireMsg{seq: 4, url: m.prs[0].URL})
	if cmd != nil {
		t.Fatal("stale debounce seq should not fire load command")
	}
}

func TestTriggerDetailLoadCacheHitIsImmediate(t *testing.T) {
	m := newModel()
	pr := pullRequest{
		URL:       "https://example.test/pr/1",
		UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
	}
	m.prs = []pullRequest{pr}
	key := cacheKey(pr.URL, pr.UpdatedAt)
	m.cache.mu.Lock()
	m.cache.mem[key] = cacheEntry{Detail: pullRequestDetail{pullRequest: pr}, Diff: "diff body"}
	m.cache.mu.Unlock()

	cmd := m.triggerDetailLoad()
	if cmd == nil {
		t.Fatal("cache hit should return a command")
	}
	msg := cmd()
	dm, ok := msg.(diffMsg)
	if !ok {
		t.Fatalf("cache hit should produce diffMsg, got %T", msg)
	}
	if dm.diff != "diff body" {
		t.Fatalf("diff = %q, want %q", dm.diff, "diff body")
	}
}

func TestTriggerDetailLoadCacheMissReturnsDebounceTick(t *testing.T) {
	m := newModel()
	pr := pullRequest{
		URL:       "https://example.test/pr/miss",
		UpdatedAt: time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC),
	}
	m.prs = []pullRequest{pr}

	startSeq := m.debounceSeq
	cmd := m.triggerDetailLoad()
	if cmd == nil {
		t.Fatal("cache miss should return a debounce command")
	}
	if m.debounceSeq != startSeq+1 {
		t.Fatalf("debounceSeq = %d, want %d", m.debounceSeq, startSeq+1)
	}
	// We cannot easily exercise the tick without sleeping; just confirm the
	// returned closure executes without panicking when invoked.
	got := cmd()
	if _, ok := got.(debounceFireMsg); !ok {
		t.Fatalf("expected debounceFireMsg, got %T", got)
	}
}

func TestSearchFilterMatchesRepositoryTitleAndAuthor(t *testing.T) {
	m := modelWithPRsForSearch()

	tests := []struct {
		name  string
		query string
		want  []int
	}{
		{name: "repository", query: "api", want: []int{0}},
		{name: "title", query: "billing", want: []int{1}},
		{name: "author", query: "CAROL", want: []int{2}},
		{name: "empty returns all", query: " ", want: []int{0, 1, 2}},
		{name: "no match", query: "missing", want: []int{}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			m.searchInput.SetValue(tt.query)
			got := m.matchingIndices()
			if !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("matchingIndices() = %#v, want %#v", got, tt.want)
			}
		})
	}
}

func TestSearchCursorMovesWithinFilteredMatches(t *testing.T) {
	m := modelWithPRsForSearch()
	m.searchInput.SetValue("fix")
	m.cursor = 0

	if !m.advanceCursor(+1) {
		t.Fatal("advanceCursor should move to the next filtered match")
	}
	if m.cursor != 2 {
		t.Fatalf("cursor = %d, want 2", m.cursor)
	}
	if m.advanceCursor(+1) {
		t.Fatal("advanceCursor should stop at the last filtered match")
	}
	if !m.advanceCursor(-1) {
		t.Fatal("advanceCursor should move to the previous filtered match")
	}
	if m.cursor != 0 {
		t.Fatalf("cursor = %d, want 0", m.cursor)
	}
}

func TestEnsureCursorVisibleSnapsToNearestFilteredMatch(t *testing.T) {
	m := modelWithPRsForSearch()
	m.searchInput.SetValue("fix")
	m.cursor = 1

	m.ensureCursorVisible()

	if m.cursor != 2 {
		t.Fatalf("cursor = %d, want nearest filtered index 2", m.cursor)
	}
	if m.listOffset != 0 {
		t.Fatalf("listOffset = %d, want 0", m.listOffset)
	}
}

func modelWithPRsForSearch() model {
	m := newModel()
	m.prs = []pullRequest{
		{Repository: "owner/api", Number: 1, Title: "Fix API auth", URL: "https://example.test/pr/1", Author: "alice", Request: "@me"},
		{Repository: "owner/web", Number: 2, Title: "Add billing view", URL: "https://example.test/pr/2", Author: "bob", Request: "@me"},
		{Repository: "owner/cli", Number: 3, Title: "Fix command output", URL: "https://example.test/pr/3", Author: "carol", Request: "org/team"},
	}
	return m
}
