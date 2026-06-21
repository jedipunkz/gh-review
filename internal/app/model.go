package app

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"

	"charm.land/bubbles/v2/spinner"
	"charm.land/bubbles/v2/textinput"
	"charm.land/bubbles/v2/viewport"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/glamour"
	"github.com/charmbracelet/glamour/styles"
	"github.com/charmbracelet/x/ansi"
	"github.com/mattn/go-runewidth"
	"golang.org/x/sync/errgroup"
)

type prListMsg struct {
	prs []pullRequest
	err error
}

type diffMsg struct {
	pr     pullRequest
	detail pullRequestDetail
	diff   string
	err    error
}

type approveMsg struct {
	pr  pullRequest
	err error
}

type copyURLMsg struct {
	pr  pullRequest
	err error
}

type updateCheckTickMsg time.Time

type updateCheckMsg struct {
	previousSignature string
	currentSignature  string
	previousCount     int
	count             int
	prs               []pullRequest
	err               error
}

type updateNotice struct {
	count int
	id    int
}

type popupDismissMsg struct {
	id int
}

type debounceFireMsg struct {
	seq int
	url string
}

const popupDismissDelay = 6 * time.Second
const detailLoadDebounce = 80 * time.Millisecond

type model struct {
	loading        bool
	status         string
	err            string
	prs            []pullRequest
	prSignature    string
	prListLoaded   bool
	cursor         int
	listOffset     int
	currentDetail  *pullRequestDetail
	currentDiff    string
	detailLoading  bool
	detailErr      string
	loadingForURL  string
	detailVP       viewport.Model
	width          int
	height         int
	approved       map[string]bool
	pendingApprove *pullRequest
	updateNotice   *updateNotice
	markedPRs      map[string]bool
	popupSeq       int
	cache          *detailCache
	inflight       *inflightLoader
	prefetcher     *prefetcher
	debounceSeq    int
	searchInput    textinput.Model
	searchActive   bool
	spinner        spinner.Model
	activeTab      int
}

const maxListItems = 10

const (
	tabAwaiting = iota
	tabReviewed
	tabCount
)

var (
	tokyoNightFG         = lipgloss.Color("#c0caf5")
	tokyoNightMuted      = lipgloss.Color("#565f89")
	tokyoNightBlue       = lipgloss.Color("#7aa2f7")
	tokyoNightCyan       = lipgloss.Color("#7dcfff")
	tokyoNightGreen      = lipgloss.Color("#9ece6a")
	tokyoNightMagenta    = lipgloss.Color("#bb9af7")
	tokyoNightOrange     = lipgloss.Color("#ff9e64")
	tokyoNightRed        = lipgloss.Color("#f7768e")
	tokyoNightYellow     = lipgloss.Color("#e0af68")
	tokyoNightSelected   = lipgloss.Color("#3d59a1")
	tokyoNightSelectedFg = lipgloss.Color("#ffffff")
	tokyoNightBarBG      = lipgloss.Color("#1f2335")
	tokyoNightInk        = lipgloss.Color("#1a1b26")

	// Top / bottom bars share a single background so the UI reads as one
	// framed app rather than loose lines of text.
	barStyle       = lipgloss.NewStyle().Background(tokyoNightBarBG)
	badgeStyle     = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightInk).Background(tokyoNightBlue)
	barStatusStyle = lipgloss.NewStyle().Foreground(tokyoNightFG).Background(tokyoNightBarBG)
	barMutedStyle  = lipgloss.NewStyle().Foreground(tokyoNightMuted).Background(tokyoNightBarBG)
	barErrorStyle  = lipgloss.NewStyle().Foreground(tokyoNightRed).Background(tokyoNightBarBG)
	helpKeyStyle   = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightYellow).Background(tokyoNightBarBG)
	helpDescStyle  = lipgloss.NewStyle().Foreground(tokyoNightMuted).Background(tokyoNightBarBG)
	helpSepStyle   = lipgloss.NewStyle().Foreground(frameGray).Background(tokyoNightBarBG)
	cursorBarStyle = lipgloss.NewStyle().Foreground(tokyoNightCyan).Background(tokyoNightSelected)

	titleStyle          = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightBlue)
	selectedStyle       = lipgloss.NewStyle().Foreground(tokyoNightFG).Background(tokyoNightSelected)
	selectedTextStyle   = lipgloss.NewStyle().Foreground(tokyoNightSelectedFg).Background(tokyoNightSelected)
	selectedMutedStyle  = lipgloss.NewStyle().Foreground(tokyoNightYellow).Background(tokyoNightSelected)
	selectedOKStyle     = lipgloss.NewStyle().Foreground(tokyoNightGreen).Background(tokyoNightSelected)
	selectedErrorStyle  = lipgloss.NewStyle().Foreground(tokyoNightRed).Background(tokyoNightSelected)
	mutedStyle          = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	errorStyle          = lipgloss.NewStyle().Foreground(tokyoNightRed)
	okStyle             = lipgloss.NewStyle().Foreground(tokyoNightGreen)
	listHeaderStyle     = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	listRepoStyle       = lipgloss.NewStyle().Foreground(tokyoNightCyan)
	listNumStyle        = lipgloss.NewStyle().Foreground(tokyoNightBlue)
	listTitleStyle      = lipgloss.NewStyle().Foreground(tokyoNightFG)
	listAuthorStyle     = lipgloss.NewStyle().Foreground(tokyoNightMagenta)
	detailTitleStyle    = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightMagenta)
	detailMetaKeyStyle  = lipgloss.NewStyle().Foreground(tokyoNightYellow)
	detailMetaTextStyle = lipgloss.NewStyle().Foreground(tokyoNightFG)
	detailRuleStyle     = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	frameGray           = lipgloss.Color("#414868")
	listFrameStyle      = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(frameGray)
	detailFrameStyle    = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(frameGray).PaddingLeft(1)
	updateNoticeStyle   = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(tokyoNightYellow).Padding(0, 1)
	approvePopupStyle   = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(tokyoNightGreen).Padding(1, 2)
	approveButtonStyle  = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightGreen)
	cancelButtonStyle   = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightRed)
	markStyle           = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightYellow)
	activeTabStyle      = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightInk).Background(tokyoNightBlue).Padding(0, 1)
	inactiveTabStyle    = lipgloss.NewStyle().Foreground(tokyoNightMuted).Background(tokyoNightBarBG).Padding(0, 1)
	typeMeStyle         = lipgloss.NewStyle().Foreground(tokyoNightCyan)
	typeTeamStyle       = lipgloss.NewStyle().Foreground(tokyoNightOrange)
	diffFileHeaderStyle = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightCyan)
	diffMetaStyle       = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	diffHunkStyle       = lipgloss.NewStyle().Foreground(tokyoNightMagenta)
	diffAddStyle        = lipgloss.NewStyle().Foreground(tokyoNightGreen)
	diffDelStyle        = lipgloss.NewStyle().Foreground(tokyoNightRed)
)

const updateCheckInterval = time.Minute

func newModel() model {
	vp := viewport.New()
	vp.SoftWrap = true
	cache := newDetailCache()
	ti := textinput.New()
	ti.Placeholder = "filter repo / title / author"
	ti.Prompt = ""
	sp := spinner.New(spinner.WithSpinner(spinner.MiniDot))
	sp.Style = lipgloss.NewStyle().Foreground(tokyoNightCyan)
	return model{
		loading:     true,
		status:      "loading review requests...",
		detailVP:    vp,
		approved:    make(map[string]bool),
		markedPRs:   make(map[string]bool),
		cache:       cache,
		inflight:    newInflightLoader(),
		prefetcher:  newPrefetcher(cache),
		searchInput: ti,
		spinner:     sp,
	}
}

func (m model) Init() tea.Cmd {
	return tea.Batch(loadPRsCmd(), updateCheckTickCmd(), m.spinner.Tick)
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.resizeViewport()
		// Re-render so the glamour-formatted description re-wraps to the new
		// viewport width instead of being soft-wrapped by the viewport.
		if m.currentDetail != nil {
			m.detailVP.SetContent(renderDiffContent(*m.currentDetail, m.currentDiff, m.detailVP.Width()))
		}
		return m, nil
	case tea.KeyPressMsg:
		if m.searchActive {
			return m.handleSearchKey(msg)
		}
		return m.handleKey(msg)
	case spinner.TickMsg:
		var cmd tea.Cmd
		m.spinner, cmd = m.spinner.Update(msg)
		return m, cmd
	case updateCheckTickMsg:
		cmds := []tea.Cmd{updateCheckTickCmd()}
		if !m.loading && m.updateNotice == nil && m.prListLoaded {
			cmds = append(cmds, checkForUpdatesCmd(m.prSignature, len(m.prs)))
		}
		return m, tea.Batch(cmds...)
	case updateCheckMsg:
		if msg.err != nil || msg.previousSignature != m.prSignature || msg.currentSignature == m.prSignature {
			return m, nil
		}
		if msg.count < msg.previousCount {
			prs := msg.prs
			return m, func() tea.Msg {
				return prListMsg{prs: prs}
			}
		}
		newURLs := newPRURLs(m.prs, msg.prs)
		for _, url := range newURLs {
			m.markedPRs[url] = true
		}
		prevPRs, prevCursor := m.prs, m.cursor
		m.prs = msg.prs
		m.prSignature = msg.currentSignature
		m.reconcileCursor(prevPRs, prevCursor)
		m.ensureCursorVisible()
		m.resizeViewport()
		m.popupSeq++
		m.updateNotice = &updateNotice{count: msg.count, id: m.popupSeq}
		m.status = fmt.Sprintf("%d review request(s)", len(m.prs))
		var cmds []tea.Cmd
		if len(newURLs) > 0 {
			cmds = append(cmds, playNotifySoundCmd())
		}
		cmds = append(cmds, dismissPopupCmd(m.popupSeq))
		if detailCmd := m.refreshDetailIfNeeded(); detailCmd != nil {
			cmds = append(cmds, detailCmd)
		}
		return m, tea.Batch(cmds...)
	case popupDismissMsg:
		if m.updateNotice != nil && m.updateNotice.id == msg.id {
			m.updateNotice = nil
			m.status = fmt.Sprintf("%d review request(s)", len(m.prs))
		}
		return m, nil
	case prListMsg:
		m.loading = false
		if msg.err != nil {
			m.err = msg.err.Error()
			m.status = "failed to load review requests"
			return m, nil
		}
		m.err = ""
		prevPRs, prevCursor := m.prs, m.cursor
		m.prs = msg.prs
		m.prSignature = prListSignature(msg.prs)
		m.prListLoaded = true
		m.updateNotice = nil
		m.pruneMarkedPRs()
		m.reconcileCursor(prevPRs, prevCursor)
		m.ensureCursorVisible()
		m.status = fmt.Sprintf("%d review request(s)", len(m.prs))
		m.resizeViewport()
		if len(m.prs) > 0 {
			cmds := []tea.Cmd{m.triggerDetailLoad()}
			if pre := m.prefetchTopCmd(); pre != nil {
				cmds = append(cmds, pre)
			}
			return m, tea.Batch(cmds...)
		}
		return m, nil
	case diffMsg:
		if msg.pr.URL != m.loadingForURL {
			return m, nil
		}
		// A canceled load belongs to a previous cursor position; drop it
		// silently so the user does not see a spurious error.
		if errors.Is(msg.err, context.Canceled) {
			return m, nil
		}
		m.detailLoading = false
		if msg.err != nil {
			m.detailErr = msg.err.Error()
			return m, nil
		}
		m.detailErr = ""
		d := msg.detail
		m.currentDetail = &d
		m.currentDiff = msg.diff
		m.detailVP.SetContent(renderDiffContent(msg.detail, msg.diff, m.detailVP.Width()))
		m.detailVP.GotoTop()
		return m, nil
	case debounceFireMsg:
		if msg.seq != m.debounceSeq {
			return m, nil
		}
		if len(m.prs) == 0 || m.cursor >= len(m.prs) {
			return m, nil
		}
		pr := m.prs[m.cursor]
		if pr.URL != msg.url || pr.URL != m.loadingForURL {
			return m, nil
		}
		return m, loadDiffCmd(pr, m.cache, m.inflight)
	case approveMsg:
		m.loading = false
		if msg.err != nil {
			m.err = msg.err.Error()
			m.status = "failed to approve"
			return m, nil
		}
		m.err = ""
		m.approved[msg.pr.URL] = true
		m.status = "approved " + prLabel(msg.pr)
		// The PR now moves to the Reviewed tab; keep the cursor on a row that
		// still belongs to the active tab so the highlight stays valid until the
		// reload lands.
		m.ensureCursorVisible()
		return m, loadPRsCmd()
	case copyURLMsg:
		if msg.err != nil {
			m.err = msg.err.Error()
			m.status = "failed to copy URL"
			return m, nil
		}
		m.err = ""
		m.status = "copied " + prLabel(msg.pr) + " URL"
		return m, nil
	}
	return m, nil
}

func (m model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	key := msg.String()
	if m.updateNotice != nil {
		m.updateNotice = nil
	}
	if m.pendingApprove != nil {
		return m.handleApproveConfirmation(key)
	}
	m.clearMarkOnSelected()

	switch key {
	case "ctrl+c", "q":
		return m, tea.Quit
	case "r":
		if m.loading {
			return m, nil
		}
		m.loading = true
		m.status = "refreshing..."
		m.err = ""
		return m, loadPRsCmd()
	case "ctrl+n":
		if !m.loading && m.advanceCursor(+1) {
			m.ensureCursorVisible()
			return m, m.detailAndPrefetchCmds()
		}
	case "ctrl+p":
		if !m.loading && m.advanceCursor(-1) {
			m.ensureCursorVisible()
			return m, m.detailAndPrefetchCmds()
		}
	case "h":
		return m.switchTab(-1)
	case "l":
		return m.switchTab(+1)
	case "/":
		if !m.loading {
			m.searchActive = true
			cmd := m.searchInput.Focus()
			m.resizeViewport()
			return m, cmd
		}
	case "j":
		m.detailVP.ScrollDown(1)
	case "k":
		m.detailVP.ScrollUp(1)
	case "pgdown":
		m.detailVP.PageDown()
	case "pgup":
		m.detailVP.PageUp()
	case "y":
		if !m.loading && len(m.prs) > 0 {
			pr := m.prs[m.cursor]
			m.status = "copying URL..."
			m.err = ""
			return m, copyURLCmd(pr)
		}
	case "a":
		if m.currentDetail != nil && !m.loading && !m.detailLoading {
			pr := m.currentDetail.pullRequest
			m.err = ""
			m.pendingApprove = &pr
			m.status = "approval confirmation open"
			return m, nil
		}
	}
	return m, nil
}

func (m model) handleSearchKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	key := msg.String()
	switch key {
	case "ctrl+c":
		return m, tea.Quit
	case "esc":
		m.searchActive = false
		m.searchInput.Reset()
		m.searchInput.Blur()
		m.ensureCursorVisible()
		m.resizeViewport()
		return m, nil
	case "enter":
		m.searchActive = false
		m.searchInput.Blur()
		m.resizeViewport()
		return m, nil
	case "ctrl+n":
		if !m.loading && m.advanceCursor(+1) {
			m.ensureCursorVisible()
			return m, m.detailAndPrefetchCmds()
		}
		return m, nil
	case "ctrl+p":
		if !m.loading && m.advanceCursor(-1) {
			m.ensureCursorVisible()
			return m, m.detailAndPrefetchCmds()
		}
		return m, nil
	}
	prev := m.cursor
	var cmd tea.Cmd
	m.searchInput, cmd = m.searchInput.Update(msg)
	m.ensureCursorVisible()
	var detailCmd tea.Cmd
	if m.cursor != prev && !m.loading && len(m.prs) > 0 {
		detailCmd = m.detailAndPrefetchCmds()
	}
	return m, tea.Batch(cmd, detailCmd)
}

func (m model) handleApproveConfirmation(key string) (tea.Model, tea.Cmd) {
	pr := *m.pendingApprove
	switch strings.ToLower(key) {
	case "ctrl+c":
		return m, tea.Quit
	case "y", "yes":
		m.pendingApprove = nil
		m.loading = true
		m.status = "approving..."
		m.err = ""
		return m, approveCmd(pr)
	case "c", "cancel", "esc":
		m.pendingApprove = nil
		m.status = "approval canceled"
		m.err = ""
		return m, nil
	default:
		m.status = "press y to approve or c to cancel"
		return m, nil
	}
}

// switchTab moves the active tab by delta and repositions the cursor onto the
// first PR of the newly selected tab so the detail panel follows the change.
func (m model) switchTab(delta int) (tea.Model, tea.Cmd) {
	if m.loading {
		return m, nil
	}
	next := m.activeTab + delta
	if next < 0 || next >= tabCount {
		return m, nil
	}
	m.activeTab = next
	m.listOffset = 0
	matched := m.matchingIndices()
	if len(matched) == 0 {
		m.currentDetail = nil
		m.detailLoading = false
		m.loadingForURL = ""
		m.detailErr = ""
		m.detailVP.SetContent("")
		m.resizeViewport()
		return m, nil
	}
	m.cursor = matched[0]
	m.ensureCursorVisible()
	m.resizeViewport()
	return m, m.detailAndPrefetchCmds()
}

// isReviewed reports whether a PR should appear under the Reviewed tab: either
// it was approved in this session or GitHub already reports an APPROVED
// decision.
func (m model) isReviewed(pr pullRequest) bool {
	return m.approved[pr.URL] || pr.ReviewDecision == "APPROVED"
}

func (m model) prMatchesTab(pr pullRequest) bool {
	if m.activeTab == tabReviewed {
		return m.isReviewed(pr)
	}
	return !m.isReviewed(pr)
}

func prMatchesQuery(pr pullRequest, q string) bool {
	if q == "" {
		return true
	}
	return strings.Contains(strings.ToLower(pr.Repository), q) ||
		strings.Contains(strings.ToLower(pr.Title), q) ||
		strings.Contains(strings.ToLower(pr.Author), q)
}

// matchingIndices returns indices into m.prs that pass both the active tab and
// the search filter, preserving list order.
func (m model) matchingIndices() []int {
	q := strings.ToLower(strings.TrimSpace(m.searchInput.Value()))
	out := make([]int, 0, len(m.prs))
	for i, pr := range m.prs {
		if !m.prMatchesTab(pr) {
			continue
		}
		if !prMatchesQuery(pr, q) {
			continue
		}
		out = append(out, i)
	}
	return out
}

// tabCounts returns how many search-matching PRs fall into each tab,
// independent of which tab is active, so the tab bar shows real totals.
func (m model) tabCounts() (awaiting, reviewed int) {
	q := strings.ToLower(strings.TrimSpace(m.searchInput.Value()))
	for _, pr := range m.prs {
		if !prMatchesQuery(pr, q) {
			continue
		}
		if m.isReviewed(pr) {
			reviewed++
		} else {
			awaiting++
		}
	}
	return
}

// visiblePRIndices returns the indices into m.prs that should be shown in
// the list panel after filtering and pagination.
func (m model) visiblePRIndices() []int {
	matched := m.matchingIndices()
	n := len(matched)
	if n <= maxListItems {
		return matched
	}
	start := m.listOffset
	if start < 0 {
		start = 0
	}
	maxOffset := n - maxListItems
	if start > maxOffset {
		start = maxOffset
	}
	return matched[start : start+maxListItems]
}

// advanceCursor moves the cursor by delta positions within the currently
// matching PR set. Returns true if the cursor moved.
func (m *model) advanceCursor(delta int) bool {
	matched := m.matchingIndices()
	if len(matched) == 0 {
		return false
	}
	pos := cursorPosition(matched, m.cursor)
	if pos == -1 {
		m.cursor = matched[0]
		return true
	}
	newPos := pos + delta
	if newPos < 0 || newPos >= len(matched) {
		return false
	}
	m.cursor = matched[newPos]
	return true
}

func (m *model) ensureCursorVisible() {
	matched := m.matchingIndices()
	n := len(matched)
	if n == 0 {
		m.listOffset = 0
		return
	}
	pos := cursorPosition(matched, m.cursor)
	if pos == -1 {
		for i, idx := range matched {
			if idx >= m.cursor {
				m.cursor = idx
				pos = i
				break
			}
		}
		if pos == -1 {
			pos = n - 1
			m.cursor = matched[pos]
		}
	}
	if n <= maxListItems {
		m.listOffset = 0
		return
	}
	if pos < m.listOffset {
		m.listOffset = pos
	} else if pos >= m.listOffset+maxListItems {
		m.listOffset = pos - maxListItems + 1
	}
	maxOffset := n - maxListItems
	if m.listOffset > maxOffset {
		m.listOffset = maxOffset
	}
	if m.listOffset < 0 {
		m.listOffset = 0
	}
}

// reconcileCursor repositions the cursor after the PR list changes so it keeps
// pointing at the same PR (matched by URL) instead of a fixed numeric index.
// When the previously selected PR is gone (e.g. just approved), it falls back
// to the nearest surviving PR after it, then before it, so the user's place in
// the list is preserved even when an item above the cursor is removed.
func (m *model) reconcileCursor(prevPRs []pullRequest, prevCursor int) {
	if len(m.prs) == 0 {
		m.cursor = 0
		return
	}
	if prevCursor >= 0 && prevCursor < len(prevPRs) {
		prevURL := prevPRs[prevCursor].URL
		if idx := indexOfPRURL(m.prs, prevURL); idx >= 0 {
			m.cursor = idx
			return
		}
		for i := prevCursor + 1; i < len(prevPRs); i++ {
			if idx := indexOfPRURL(m.prs, prevPRs[i].URL); idx >= 0 {
				m.cursor = idx
				return
			}
		}
		for i := prevCursor - 1; i >= 0; i-- {
			if idx := indexOfPRURL(m.prs, prevPRs[i].URL); idx >= 0 {
				m.cursor = idx
				return
			}
		}
	}
	if m.cursor >= len(m.prs) {
		m.cursor = len(m.prs) - 1
	}
	if m.cursor < 0 {
		m.cursor = 0
	}
}

func indexOfPRURL(prs []pullRequest, url string) int {
	if url == "" {
		return -1
	}
	for i, pr := range prs {
		if pr.URL == url {
			return i
		}
	}
	return -1
}

func cursorPosition(indices []int, cursor int) int {
	for i, idx := range indices {
		if idx == cursor {
			return i
		}
	}
	return -1
}

func (m *model) refreshDetailIfNeeded() tea.Cmd {
	if len(m.prs) == 0 {
		m.currentDetail = nil
		m.detailLoading = false
		m.loadingForURL = ""
		m.detailVP.SetContent("")
		return nil
	}
	if m.cursor >= len(m.prs) {
		m.cursor = len(m.prs) - 1
	}
	if m.prs[m.cursor].URL == m.loadingForURL {
		return nil
	}
	return m.triggerDetailLoad()
}

func (m *model) clearMarkOnSelected() {
	if len(m.markedPRs) == 0 || len(m.prs) == 0 {
		return
	}
	if m.cursor < 0 || m.cursor >= len(m.prs) {
		return
	}
	url := m.prs[m.cursor].URL
	if m.markedPRs[url] {
		delete(m.markedPRs, url)
	}
}

func (m *model) pruneMarkedPRs() {
	if len(m.markedPRs) == 0 {
		return
	}
	alive := make(map[string]bool, len(m.prs))
	for _, pr := range m.prs {
		alive[pr.URL] = true
	}
	for url := range m.markedPRs {
		if !alive[url] {
			delete(m.markedPRs, url)
		}
	}
}

func newPRURLs(prev, curr []pullRequest) []string {
	existing := make(map[string]bool, len(prev))
	for _, pr := range prev {
		existing[pr.URL] = true
	}
	var added []string
	for _, pr := range curr {
		if !existing[pr.URL] {
			added = append(added, pr.URL)
		}
	}
	return added
}

func dismissPopupCmd(id int) tea.Cmd {
	return tea.Tick(popupDismissDelay, func(time.Time) tea.Msg {
		return popupDismissMsg{id: id}
	})
}

// prefetchTopCmd warms the cache with the first prefetchTopN PRs, skipping
// the one currently under the cursor (it is already being loaded in the
// foreground).
func (m *model) prefetchTopCmd() tea.Cmd {
	if m.prefetcher == nil || len(m.prs) == 0 {
		return nil
	}
	candidates := topN(m.prs, prefetchTopN)
	filtered := make([]pullRequest, 0, len(candidates))
	for i, pr := range candidates {
		if i == m.cursor {
			continue
		}
		filtered = append(filtered, pr)
	}
	return m.prefetcher.prefetchCmd(filtered)
}

// detailAndPrefetchCmds combines the foreground detail load with a background
// prefetch of the cursor's neighbors.
func (m *model) detailAndPrefetchCmds() tea.Cmd {
	cmds := []tea.Cmd{m.triggerDetailLoad()}
	if m.prefetcher != nil {
		if pre := m.prefetcher.prefetchCmd(neighborPRs(m.prs, m.cursor)); pre != nil {
			cmds = append(cmds, pre)
		}
	}
	return tea.Batch(cmds...)
}

func (m *model) triggerDetailLoad() tea.Cmd {
	if len(m.prs) == 0 {
		return nil
	}
	pr := m.prs[m.cursor]
	m.loadingForURL = pr.URL
	m.currentDetail = nil
	m.detailLoading = true
	m.detailErr = ""
	m.detailVP.GotoTop()

	cache := m.cache
	key := cacheKey(pr.URL, pr.UpdatedAt)
	if entry, ok := cache.getMem(key); ok {
		// Cache hit: cancel any in-flight network load, bump debounce seq so
		// any pending tick is ignored, and render immediately.
		m.inflight.cancel()
		m.debounceSeq++
		return func() tea.Msg {
			return diffMsg{pr: pr, detail: entry.Detail, diff: entry.Diff}
		}
	}
	if entry, ok := cache.getDisk(key); ok {
		m.inflight.cancel()
		m.debounceSeq++
		immediate := func() tea.Msg {
			return diffMsg{pr: pr, detail: entry.Detail, diff: entry.Diff}
		}
		// Schedule a background refresh so any new commits/comments land in
		// cache. Run it through the inflight loader so a later cursor move
		// cancels the lingering gh subprocess.
		return tea.Batch(immediate, loadDiffCmd(pr, cache, m.inflight))
	}
	// Cache miss: debounce so rapid cursor movement does not spawn a flurry
	// of gh subprocesses.
	m.debounceSeq++
	seq := m.debounceSeq
	url := pr.URL
	return tea.Tick(detailLoadDebounce, func(time.Time) tea.Msg {
		return debounceFireMsg{seq: seq, url: url}
	})
}

func (m model) View() tea.View {
	parts := []string{m.renderHeader()}
	if m.searchVisible() {
		parts = append(parts, m.renderSearchLine())
	}
	parts = append(parts,
		m.renderGroupedList(),
		m.renderDetailSection(),
		m.renderFooter(),
	)
	view := strings.Join(parts, "\n")
	if m.updateNotice != nil {
		view = m.overlayUpdateNotice(view)
	}
	if m.pendingApprove != nil {
		view = m.overlayApproveConfirmation(view)
	}
	return tea.NewView(view)
}

func (m model) searchVisible() bool {
	return m.searchActive || m.searchInput.Value() != ""
}

func (m model) renderSearchLine() string {
	if m.searchActive {
		return mutedStyle.Render("/ ") + m.searchInput.View()
	}
	return mutedStyle.Render(fmt.Sprintf("filter: %s  (/ to edit, esc to clear)", m.searchInput.Value()))
}

func (m model) renderHeader() string {
	w := m.frameWidth()
	badge := badgeStyle.Render(" gh review ")
	right := m.renderHeaderStatus(max(1, w-lipgloss.Width(badge)-1))
	gap := max(1, w-lipgloss.Width(badge)-lipgloss.Width(right))
	return badge + barStyle.Render(strings.Repeat(" ", gap)) + right
}

// renderHeaderStatus builds the right-aligned status segment of the top bar,
// truncating the status text so the whole bar stays on a single line.
func (m model) renderHeaderStatus(maxW int) string {
	var lead string
	if m.loading || m.detailLoading {
		lead = m.spinner.View() + " "
	}
	style := barMutedStyle
	if m.err != "" {
		style = barErrorStyle
	}
	avail := max(1, maxW-lipgloss.Width(lead))
	txt := runewidth.Truncate(m.status, avail, "…")
	return barStyle.Render(lead) + style.Render(txt)
}

func (m model) renderGroupedList() string {
	if m.err != "" && len(m.prs) == 0 {
		return errorStyle.Render(m.err)
	}
	if len(m.prs) == 0 {
		return mutedStyle.Render("No open PRs are requesting your review.")
	}

	boxW := m.frameWidth()
	contentW := frameContentWidth(listFrameStyle, boxW)
	items := m.visiblePRIndices()

	lines := []string{m.renderListHeader(contentW)}
	if len(items) == 0 {
		lines = append(lines, " "+mutedStyle.Render(m.emptyListMessage()))
	} else {
		for _, idx := range items {
			lines = append(lines, m.renderPRLine(idx, m.prs[idx], contentW))
		}
	}
	inner := len(lines)
	frameH := inner + 2
	frame := listFrameStyle.Width(boxW).Height(inner).MaxHeight(frameH).Render(strings.Join(lines, "\n"))
	return m.renderTabBar() + "\n" + frame
}

// emptyListMessage explains why the current tab has no rows: an active search
// filter that matched nothing, or simply an empty tab.
func (m model) emptyListMessage() string {
	if strings.TrimSpace(m.searchInput.Value()) != "" {
		return "No PRs match the current filter."
	}
	if m.activeTab == tabReviewed {
		return "No reviewed PRs."
	}
	return "No PRs awaiting your review."
}

// renderTabBar draws the Awaiting Review / Reviewed tabs with per-tab counts,
// highlighting the active one.
func (m model) renderTabBar() string {
	awaiting, reviewed := m.tabCounts()
	labels := [tabCount]string{
		fmt.Sprintf("Awaiting Review %d", awaiting),
		fmt.Sprintf("Reviewed %d", reviewed),
	}
	parts := make([]string, tabCount)
	for i, label := range labels {
		if i == m.activeTab {
			parts[i] = activeTabStyle.Render(label)
		} else {
			parts[i] = inactiveTabStyle.Render(label)
		}
	}
	return " " + strings.Join(parts, " ")
}

const (
	colMarkW    = 3
	colTypeW    = 6
	colRepoW    = 28
	colNumW     = 6
	colAuthorW  = 15
	colApproveW = 11
)

func listTitleWidth(boxW int) int {
	// budget: leading/trailing space (2) + mark + type + repo + num + title + author + approve columns and separators.
	fixed := 2 + colMarkW + 1 + colTypeW + 2 + colRepoW + 2 + colNumW + 2 + 2 + 1 + colAuthorW + 2 + colApproveW
	return max(10, boxW-fixed)
}

func (m model) renderListHeader(boxW int) string {
	titleW := listTitleWidth(boxW)
	line := fmt.Sprintf("%s %s  %s  %s  %s   %s  %s",
		padRight("", colMarkW),
		padRight("Type", colTypeW),
		padRight("Repository", colRepoW),
		padRight("#", colNumW),
		padRight("Title", titleW),
		padRight("Author", colAuthorW),
		padRight("Approve", colApproveW),
	)
	return " " + listHeaderStyle.Render(line)
}

func (m model) renderPRLine(idx int, pr pullRequest, boxW int) string {
	titleW := listTitleWidth(boxW)
	approve := m.approveLabel(pr)
	selected := idx == m.cursor

	sp1, sp2 := " ", "  "
	if selected {
		sp1 = selectedStyle.Render(" ")
		sp2 = selectedStyle.Render("  ")
	}

	line := m.markCell(pr, selected) + sp1 +
		m.typeCell(pr, selected) + sp2 +
		m.listRepoStyle(selected).Render(padRight(pr.Repository, colRepoW)) + sp2 +
		m.listNumStyle(selected).Render(padRight(fmt.Sprintf("#%d", pr.Number), colNumW)) + sp2 +
		m.listTitleStyle(selected).Render(padRight(pr.Title, titleW)) + sp2 +
		m.listAuthorStyle(selected).Render("@"+padRight(pr.Author, colAuthorW)) + sp2 +
		m.approveStyle(approve, selected).Render(padRight(approve, colApproveW))

	if selected {
		return cursorBarStyle.Render("▌") + line + sp1
	}
	return " " + line
}

func (m model) markCell(pr pullRequest, selected bool) string {
	if m.markedPRs[pr.URL] {
		style := markStyle
		if selected {
			style = style.Background(tokyoNightSelected)
		}
		return style.Render(padRight("[!]", colMarkW))
	}
	cell := strings.Repeat(" ", colMarkW)
	if selected {
		return selectedStyle.Render(cell)
	}
	return cell
}

// typeCell renders a small [me]/[team] badge showing whether the review was
// requested directly (review-requested:@me) or via a team.
func (m model) typeCell(pr pullRequest, selected bool) string {
	style := typeTeamStyle
	label := "[team]"
	if strings.Contains(pr.Request, "@me") {
		style = typeMeStyle
		label = "[me]"
	}
	if selected {
		style = style.Background(tokyoNightSelected)
	}
	return style.Render(padRight(label, colTypeW))
}

func (m model) approveLabel(pr pullRequest) string {
	if m.approved[pr.URL] || pr.ReviewDecision == "APPROVED" {
		return "approved"
	}
	switch pr.ReviewDecision {
	case "CHANGES_REQUESTED":
		return "changes"
	case "REVIEW_REQUIRED":
		return "required"
	default:
		return "-"
	}
}

func (m model) listRepoStyle(selected bool) lipgloss.Style {
	if selected {
		return selectedTextStyle
	}
	return listRepoStyle
}

func (m model) listNumStyle(selected bool) lipgloss.Style {
	if selected {
		return selectedMutedStyle
	}
	return listNumStyle
}

func (m model) listTitleStyle(selected bool) lipgloss.Style {
	if selected {
		return selectedTextStyle.Bold(true)
	}
	return listTitleStyle
}

func (m model) listAuthorStyle(selected bool) lipgloss.Style {
	if selected {
		return selectedTextStyle
	}
	return listAuthorStyle
}

func (m model) approveStyle(label string, selected bool) lipgloss.Style {
	if selected {
		switch label {
		case "approved":
			return selectedOKStyle
		case "changes":
			return selectedErrorStyle
		default:
			return selectedMutedStyle
		}
	}
	switch label {
	case "approved":
		return okStyle
	case "changes":
		return errorStyle
	default:
		return mutedStyle
	}
}

func (m model) renderDetailSection() string {
	boxW := m.frameWidth()
	vpH := m.detailVP.Height()
	var content string
	switch {
	case m.detailLoading:
		content = m.spinner.View() + " " + mutedStyle.Render("loading detail...")
	case m.detailErr != "":
		content = errorStyle.Render(m.detailErr)
	case m.currentDetail == nil:
		content = mutedStyle.Render("No detail loaded.")
	default:
		content = m.detailVP.View()
	}
	return detailFrameStyle.Width(boxW).Height(vpH).MaxHeight(vpH + 2).Render(content)
}

func (m model) frameWidth() int {
	if m.width > 0 {
		return m.width
	}
	return 40
}

func frameContentWidth(style lipgloss.Style, width int) int {
	return max(1, width-style.GetHorizontalFrameSize())
}

func (m model) renderFooter() string {
	bindings := [][2]string{
		{"h/l", "tabs"},
		{"ctrl+n/p", "list"},
		{"j/k", "scroll"},
		{"pgup/dn", "page"},
		{"/", "filter"},
		{"y", "copy"},
		{"a", "approve"},
		{"r", "refresh"},
		{"q", "quit"},
	}
	sep := helpSepStyle.Render(" · ")
	parts := make([]string, len(bindings))
	for i, b := range bindings {
		parts[i] = helpKeyStyle.Render(b[0]) + " " + helpDescStyle.Render(b[1])
	}
	return renderBar(" "+strings.Join(parts, sep), m.frameWidth())
}

// renderBar pads or truncates content to exactly w columns on a single line,
// filling any slack with the shared bar background.
func renderBar(content string, w int) string {
	cw := lipgloss.Width(content)
	if cw > w {
		return ansi.Truncate(content, w, "…")
	}
	if cw < w {
		return content + barStyle.Render(strings.Repeat(" ", w-cw))
	}
	return content
}

func (m *model) resizeViewport() {
	if m.width == 0 || m.height == 0 {
		return
	}
	listH := m.computeListSectionHeight()
	searchH := 0
	if m.searchVisible() {
		searchH = 1
	}
	// layout: header(1) + search?(searchH) + list(listH) + detailBorder(2) + vpH + footer(1)
	vpH := max(3, m.height-4-listH-searchH)
	vpW := max(20, frameContentWidth(detailFrameStyle, m.frameWidth()))
	m.detailVP.SetHeight(vpH)
	m.detailVP.SetWidth(vpW)
	m.searchInput.SetWidth(max(20, m.frameWidth()-4))
}

func (m model) computeListSectionHeight() int {
	items := len(m.visiblePRIndices())
	if items == 0 {
		items = 1 // empty-state message line
	}
	inner := 1 + items // header line + item rows
	// tab bar (1) + frame top/bottom borders (2) + inner content
	return 1 + 2 + inner
}

func loadPRsCmd() tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		prs, err := loadReviewRequests(ctx)
		return prListMsg{prs: prs, err: err}
	}
}

func updateCheckTickCmd() tea.Cmd {
	return tea.Tick(updateCheckInterval, func(t time.Time) tea.Msg {
		return updateCheckTickMsg(t)
	})
}

func checkForUpdatesCmd(previousSignature string, previousCount int) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		prs, err := loadReviewRequests(ctx)
		return updateCheckMsg{
			previousSignature: previousSignature,
			currentSignature:  prListSignature(prs),
			previousCount:     previousCount,
			count:             len(prs),
			prs:               prs,
			err:               err,
		}
	}
}

// inflightLoader tracks the currently running detail load so that a newer
// cursor move can cancel its gh subprocesses before issuing a new one.
// Each begin() bumps gen; done() only clears the stored cancel when its
// generation still matches, so a newer load's cancel is never wiped by an
// older completion.
type inflightLoader struct {
	mu       sync.Mutex
	cancelFn context.CancelFunc
	gen      uint64
}

func newInflightLoader() *inflightLoader { return &inflightLoader{} }

// begin cancels any previous load, then registers a new context. The returned
// done callback releases resources and clears the stored cancel func when
// this generation is still the most recent one.
func (l *inflightLoader) begin(timeout time.Duration) (context.Context, func()) {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	if l == nil {
		return ctx, cancel
	}
	l.mu.Lock()
	if l.cancelFn != nil {
		l.cancelFn()
	}
	l.gen++
	myGen := l.gen
	l.cancelFn = cancel
	l.mu.Unlock()
	done := func() {
		l.mu.Lock()
		if l.gen == myGen {
			l.cancelFn = nil
		}
		l.mu.Unlock()
		cancel()
	}
	return ctx, done
}

// cancel any in-flight load. Safe to call when nothing is running.
func (l *inflightLoader) cancel() {
	if l == nil {
		return
	}
	l.mu.Lock()
	if l.cancelFn != nil {
		l.cancelFn()
		l.cancelFn = nil
	}
	l.mu.Unlock()
}

func loadDiffCmd(pr pullRequest, cache *detailCache, inflight *inflightLoader) tea.Cmd {
	return func() tea.Msg {
		ctx, done := inflight.begin(60 * time.Second)
		defer done()

		var (
			detail  pullRequestDetail
			diff    string
			diffErr error
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
			// Treat too_large as a non-fatal condition so the detail goroutine
			// still produces a usable result.
			if isPRDiffTooLargeError(err) {
				diff = mutedStyle.Render("Diff omitted because GitHub reports this PR diff is too large to display.")
				diffErr = nil
				return nil
			}
			diff = d
			diffErr = err
			return err
		})
		if err := g.Wait(); err != nil {
			// If only the diff failed (non-too_large), surface that error; detail
			// errors are surfaced via g.Wait()'s first-error semantics too.
			if ctx.Err() != nil {
				return diffMsg{pr: pr, err: context.Canceled}
			}
			return diffMsg{pr: pr, err: err}
		}
		if diffErr != nil {
			if ctx.Err() != nil {
				return diffMsg{pr: pr, err: context.Canceled}
			}
			return diffMsg{pr: pr, err: diffErr}
		}
		// Use the fetched UpdatedAt so the cache key matches what the next
		// list refresh will produce for this PR.
		key := cacheKey(pr.URL, detail.UpdatedAt)
		cache.put(key, cacheEntry{Detail: detail, Diff: diff})
		return diffMsg{pr: pr, detail: detail, diff: diff}
	}
}

func approveCmd(pr pullRequest) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		err := approvePR(ctx, pr)
		return approveMsg{pr: pr, err: err}
	}
}

var notifySoundFile = "/System/Library/Sounds/Pop.aiff"

var playNotifySoundCmd = func() tea.Cmd {
	if runtime.GOOS != "darwin" {
		return nil
	}
	return func() tea.Msg {
		cmd := exec.Command("afplay", notifySoundFile)
		if err := cmd.Start(); err != nil {
			return nil
		}
		go func() { _ = cmd.Wait() }()
		return nil
	}
}

func copyURLCmd(pr pullRequest) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()

		cmd := exec.CommandContext(ctx, "pbcopy")
		cmd.Stdin = strings.NewReader(pr.URL)
		err := cmd.Run()
		if ctx.Err() != nil {
			err = ctx.Err()
		}
		return copyURLMsg{pr: pr, err: err}
	}
}

func prListSignature(prs []pullRequest) string {
	parts := make([]string, 0, len(prs))
	for _, pr := range prs {
		parts = append(parts, strings.Join([]string{
			pr.URL,
			pr.UpdatedAt.UTC().Format(time.RFC3339Nano),
			pr.Request,
			pr.ReviewDecision,
		}, "\x00"))
	}
	sort.Strings(parts)
	return strings.Join(parts, "\x01")
}

func prLabel(pr pullRequest) string {
	return fmt.Sprintf("%s#%d", pr.Repository, pr.Number)
}

func (m model) overlayUpdateNotice(base string) string {
	popup := m.renderUpdateNoticePopup()
	if popup == "" {
		return base
	}
	width := lipgloss.Width(base)
	height := lipgloss.Height(base)
	if width <= 0 || height <= 0 {
		return base
	}
	pw := lipgloss.Width(popup)
	ph := lipgloss.Height(popup)
	x := max(0, width-pw-1)
	y := max(0, height-ph-1)
	return lipgloss.NewCompositor(
		lipgloss.NewLayer(base),
		lipgloss.NewLayer(popup).X(x).Y(y).Z(1),
	).Render()
}

func (m model) overlayApproveConfirmation(base string) string {
	popup := m.renderApproveConfirmationPopup()
	if popup == "" {
		return base
	}
	width := lipgloss.Width(base)
	height := lipgloss.Height(base)
	if width <= 0 || height <= 0 {
		return base
	}
	pw := lipgloss.Width(popup)
	ph := lipgloss.Height(popup)
	x := max(0, (width-pw)/2)
	y := max(0, (height-ph)/2)
	return lipgloss.NewCompositor(
		lipgloss.NewLayer(base),
		lipgloss.NewLayer(popup).X(x).Y(y).Z(2),
	).Render()
}

func (m model) renderUpdateNoticePopup() string {
	count := "review requests changed"
	if m.updateNotice != nil && m.updateNotice.count >= 0 {
		count = fmt.Sprintf("%d review request(s)", m.updateNotice.count)
	}
	body := strings.Join([]string{
		titleStyle.Render("Review updated"),
		mutedStyle.Render(count),
	}, "\n")
	return updateNoticeStyle.Render(body)
}

func (m model) renderApproveConfirmationPopup() string {
	if m.pendingApprove == nil {
		return ""
	}
	pr := *m.pendingApprove
	body := strings.Join([]string{
		titleStyle.Render("Approve pull request?"),
		mutedStyle.Render(prLabel(pr)),
		"",
		approveButtonStyle.Render("y Yes") + mutedStyle.Render("   ") + cancelButtonStyle.Render("c Cancel"),
	}, "\n")
	return approvePopupStyle.Render(body)
}

func renderDiffContent(detail pullRequestDetail, diff string, width int) string {
	var b strings.Builder
	b.WriteString(detailTitleStyle.Render(fmt.Sprintf("%s  #%d  %s", detail.Repository, detail.Number, detail.Title)))
	b.WriteByte('\n')
	b.WriteString(formatMeta("Author", "@"+detail.Author))
	b.WriteString("  ")
	b.WriteString(formatMeta("Review request", detail.Request))
	if branch := branchLabel(detail); branch != "" {
		b.WriteString("  ")
		b.WriteString(formatMeta("Branch", branch))
	}
	b.WriteByte('\n')
	b.WriteString(formatMeta("State", nonEmpty(detail.MergeStateStatus, "unknown")))
	b.WriteString("  ")
	b.WriteString(formatReviewMeta(detail.ReviewDecision))
	b.WriteString("  ")
	b.WriteString(formatMeta("Files", fmt.Sprintf("%d", detail.ChangedFiles)))
	b.WriteString("  ")
	b.WriteString(okStyle.Render(fmt.Sprintf("+%d", detail.Additions)))
	b.WriteString(" ")
	b.WriteString(errorStyle.Render(fmt.Sprintf("-%d", detail.Deletions)))
	if len(detail.Reviewers) > 0 {
		b.WriteByte('\n')
		b.WriteString(formatReviewersMeta(detail.Reviewers))
	}
	if !detail.CreatedAt.IsZero() || !detail.UpdatedAt.IsZero() {
		b.WriteByte('\n')
		if !detail.CreatedAt.IsZero() {
			b.WriteString(formatMeta("Created", detail.CreatedAt.Format("2006-01-02 15:04")))
		}
		if !detail.UpdatedAt.IsZero() {
			if !detail.CreatedAt.IsZero() {
				b.WriteString("  ")
			}
			b.WriteString(formatMeta("Updated", detail.UpdatedAt.Format("2006-01-02 15:04")))
		}
	}
	if len(detail.Labels) > 0 {
		b.WriteByte('\n')
		b.WriteString(formatMeta("Labels", strings.Join(detail.Labels, ", ")))
	}
	b.WriteString("\n\n")
	body := strings.TrimSpace(detail.Body)
	if body == "" {
		b.WriteString(mutedStyle.Render("No description."))
	} else {
		b.WriteString(renderMarkdown(body, width))
	}
	b.WriteString("\n\n")
	b.WriteString(detailRuleStyle.Render(strings.Repeat("─", 80)))
	b.WriteString("\n\n")
	b.WriteString(highlightDiff(diff))
	return b.String()
}

// renderMarkdown renders a PR description as tokyonight-themed markdown using
// glamour. The width drives word-wrapping so the output matches the detail
// viewport. On any renderer error it falls back to the raw body so the
// description is never lost.
func renderMarkdown(body string, width int) string {
	if width < 20 {
		width = 80
	}
	r, err := glamour.NewTermRenderer(
		glamour.WithStyles(styles.TokyoNightStyleConfig),
		glamour.WithWordWrap(width),
		glamour.WithEmoji(),
	)
	if err != nil {
		return body
	}
	out, err := r.Render(body)
	if err != nil {
		return body
	}
	// glamour pads the output with surrounding blank lines; trim them so the
	// description sits flush against the surrounding metadata and rule.
	return strings.Trim(out, "\n")
}

// highlightDiff applies tokyonight colors to a unified diff. The input is
// assumed to come from `gh pr diff` (plain text); pre-styled placeholders
// like the too-large notice start with an ANSI escape and fall through
// unchanged.
func highlightDiff(diff string) string {
	if diff == "" {
		return diff
	}
	lines := strings.Split(diff, "\n")
	var b strings.Builder
	b.Grow(len(diff) + len(lines)*8)
	for i, line := range lines {
		switch {
		case strings.HasPrefix(line, "diff --git"):
			b.WriteString(diffFileHeaderStyle.Render(line))
		case strings.HasPrefix(line, "index "),
			strings.HasPrefix(line, "new file mode"),
			strings.HasPrefix(line, "deleted file mode"),
			strings.HasPrefix(line, "old mode"),
			strings.HasPrefix(line, "new mode"),
			strings.HasPrefix(line, "rename from "),
			strings.HasPrefix(line, "rename to "),
			strings.HasPrefix(line, "similarity index"),
			strings.HasPrefix(line, "Binary files"),
			strings.HasPrefix(line, "--- "),
			strings.HasPrefix(line, "+++ "):
			b.WriteString(diffMetaStyle.Render(line))
		case strings.HasPrefix(line, "@@"):
			b.WriteString(diffHunkStyle.Render(line))
		case strings.HasPrefix(line, "+"):
			b.WriteString(diffAddStyle.Render(line))
		case strings.HasPrefix(line, "-"):
			b.WriteString(diffDelStyle.Render(line))
		default:
			b.WriteString(line)
		}
		if i < len(lines)-1 {
			b.WriteByte('\n')
		}
	}
	return b.String()
}

func formatMeta(label, value string) string {
	return detailMetaKeyStyle.Render(label+":") + " " + detailMetaTextStyle.Render(value)
}

func formatReviewMeta(decision string) string {
	value := nonEmpty(decision, "none")
	return detailMetaKeyStyle.Render("Review:") + " " + reviewDecisionStyle(decision).Render(value)
}

func formatReviewersMeta(reviewers []reviewSummary) string {
	parts := make([]string, 0, len(reviewers))
	for _, reviewer := range reviewers {
		name := "@" + reviewer.Author
		state := reviewDecisionStyle(reviewer.State).Render(reviewer.State)
		parts = append(parts, detailMetaTextStyle.Render(name)+" "+state)
	}
	return detailMetaKeyStyle.Render("Reviewed by:") + " " + strings.Join(parts, detailMetaTextStyle.Render(", "))
}

func reviewDecisionStyle(decision string) lipgloss.Style {
	switch decision {
	case "APPROVED":
		return okStyle
	case "CHANGES_REQUESTED":
		return errorStyle
	case "REVIEW_REQUIRED":
		return lipgloss.NewStyle().Foreground(tokyoNightYellow)
	default:
		return detailMetaTextStyle
	}
}

func nonEmpty(s, fallback string) string {
	if s == "" {
		return fallback
	}
	return s
}

func branchLabel(detail pullRequestDetail) string {
	if detail.HeadRefName == "" && detail.BaseRefName == "" {
		return ""
	}
	if detail.HeadRefName == "" {
		return detail.BaseRefName
	}
	if detail.BaseRefName == "" {
		return detail.HeadRefName
	}
	return detail.HeadRefName + " -> " + detail.BaseRefName
}

func padRight(s string, width int) string {
	if width <= 0 {
		return ""
	}
	truncated := runewidth.Truncate(s, width, "...")
	w := runewidth.StringWidth(truncated)
	if w < width {
		return truncated + strings.Repeat(" ", width-w)
	}
	return truncated
}
