package app

import (
	"context"
	"fmt"
	"strings"
	"time"

	"charm.land/bubbles/v2/help"
	"charm.land/bubbles/v2/key"
	"charm.land/bubbles/v2/list"
	"charm.land/bubbles/v2/viewport"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

type screen int

const (
	screenList screen = iota
	screenDiff
)

type focusArea int

const (
	focusDiff focusArea = iota
	focusFiles
)

// reservedRows accounts for the header, padding, and footer that surround the
// list when computing its available size.
const reservedRows = 6

// sidebarWidth is the fixed column count used for the diff file sidebar. A
// small gap is rendered between the sidebar and the diff body, so the diff
// viewport receives `width - sidebarWidth - sidebarGap` columns.
const (
	sidebarWidth   = 30
	sidebarGap     = 1
	sidebarMinTerm = 30 // below this terminal width we hide the sidebar entirely
)

type prListMsg struct {
	prs []pullRequest
	err error
}

type diffMsg struct {
	pr   pullRequest
	diff string
	err  error
}

type approveMsg struct {
	pr  pullRequest
	err error
}

// prItem is the list.Item adapter for pullRequest. It also satisfies
// list.DefaultItem so the default delegate can render it.
type prItem struct {
	pr       pullRequest
	approved bool
}

// fileItem adapts a diffFile for use with the bubbles list. It implements
// list.DefaultItem; Description is intentionally empty to keep the sidebar
// compact (the default delegate renders the second line in a muted color but
// gracefully accepts an empty string).
type fileItem struct {
	file diffFile
}

func (f fileItem) Title() string       { return f.file.Path }
func (f fileItem) Description() string { return "" }
func (f fileItem) FilterValue() string { return f.file.Path }

func (p prItem) Title() string {
	title := fmt.Sprintf("%s #%d  %s", p.pr.Repository, p.pr.Number, p.pr.Title)
	if p.approved {
		title += "  " + okStyle.Render("✓ approved")
	}
	return title
}

func (p prItem) Description() string {
	return fmt.Sprintf("@%s • %s", p.pr.Author, p.pr.Request)
}

func (p prItem) FilterValue() string {
	return p.pr.Title + " " + p.pr.Repository + " " + p.pr.Author
}

type model struct {
	screen    screen
	loading   bool
	status    string
	err       string
	list      list.Model
	diffPR    *pullRequest
	diff      viewport.Model
	files     []diffFile
	fileList  list.Model
	focusArea focusArea
	keys      keyMap
	help      help.Model
	width     int
	height    int
	approved  map[string]bool
}

var (
	titleStyle = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("39"))
	mutedStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
	errorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("196"))
	okStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))

	// focusedPaneStyle outlines the currently focused pane (sidebar or diff)
	// with a subtle border so the user can tell which side will receive
	// scrolling input.
	focusedPaneStyle = lipgloss.NewStyle().
				Border(lipgloss.NormalBorder(), false, false, false, true).
				BorderForeground(lipgloss.Color("39"))
	unfocusedPaneStyle = lipgloss.NewStyle().
				Border(lipgloss.NormalBorder(), false, false, false, true).
				BorderForeground(lipgloss.Color("238"))
)

func newModel() model {
	vp := viewport.New()
	vp.SoftWrap = false

	l := list.New(nil, list.NewDefaultDelegate(), 0, 0)
	l.Title = "Review requests"
	l.SetShowStatusBar(true)
	l.SetFilteringEnabled(true)
	l.SetStatusBarItemName("review request", "review requests")
	// The app draws its own help footer, so suppress the list's built-in one.
	l.SetShowHelp(false)

	fl := list.New(nil, list.NewDefaultDelegate(), 0, 0)
	fl.Title = "Files"
	fl.SetShowStatusBar(false)
	fl.SetFilteringEnabled(false)
	fl.SetShowHelp(false)
	fl.SetShowPagination(false)

	return model{
		loading:   true,
		status:    "loading review requests...",
		list:      l,
		diff:      vp,
		fileList:  fl,
		focusArea: focusDiff,
		keys:      newKeyMap(),
		help:      help.New(),
		approved:  make(map[string]bool),
	}
}

func (m model) Init() tea.Cmd {
	return loadPRsCmd()
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.resizeChildren()
		return m, nil
	case tea.KeyPressMsg:
		return m.handleKey(msg)
	case prListMsg:
		m.loading = false
		if msg.err != nil {
			m.err = msg.err.Error()
			m.status = "failed to load review requests"
			return m, nil
		}
		m.err = ""
		items := make([]list.Item, 0, len(msg.prs))
		for _, pr := range msg.prs {
			items = append(items, prItem{pr: pr, approved: m.approved[pr.URL]})
		}
		cmd := m.list.SetItems(items)
		m.status = fmt.Sprintf("%d review request(s)", len(msg.prs))
		return m, cmd
	case diffMsg:
		m.loading = false
		if msg.err != nil {
			m.err = msg.err.Error()
			m.status = "failed to load diff"
			return m, nil
		}
		m.err = ""
		m.screen = screenDiff
		m.keys.setScreen(screenDiff)
		m.diffPR = &msg.pr
		// Parse file boundaries from the raw diff first — line numbers reported
		// by splitDiffFiles map 1:1 onto highlightDiff's output because the
		// highlighter never changes the newline layout.
		m.files = splitDiffFiles(msg.diff)
		items := make([]list.Item, 0, len(m.files))
		for _, f := range m.files {
			items = append(items, fileItem{file: f})
		}
		flCmd := m.fileList.SetItems(items)
		if len(m.files) > 0 {
			m.fileList.Select(0)
		}
		m.diff.SetContent(highlightDiff(msg.diff))
		m.diff.GotoTop()
		m.focusArea = focusDiff
		m.resizeChildren()
		m.status = "press a to approve, esc to go back"
		return m, flCmd
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
		m.screen = screenList
		m.keys.setScreen(screenList)
		return m, loadPRsCmd()
	}

	if m.screen == screenDiff {
		var cmd tea.Cmd
		m.diff, cmd = m.diff.Update(msg)
		return m, cmd
	}

	// Forward any other messages (timers, filter spinner, etc.) to the list.
	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	// While the list is filtering, let it consume keystrokes so the user can
	// type into the filter input without our shortcuts hijacking letters.
	if m.screen == screenList && m.list.FilterState() == list.Filtering {
		var cmd tea.Cmd
		m.list, cmd = m.list.Update(msg)
		return m, cmd
	}

	switch {
	case key.Matches(msg, m.keys.Quit):
		return m, tea.Quit
	case key.Matches(msg, m.keys.Help):
		m.help.ShowAll = !m.help.ShowAll
		return m, nil
	case key.Matches(msg, m.keys.Refresh):
		if m.loading {
			return m, nil
		}
		m.loading = true
		m.status = "refreshing..."
		m.err = ""
		return m, loadPRsCmd()
	}

	switch m.screen {
	case screenList:
		return m.handleListKey(msg)
	case screenDiff:
		return m.handleDiffKey(msg)
	default:
		return m, nil
	}
}

func (m model) handleListKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	if m.loading {
		return m, nil
	}
	if key.Matches(msg, m.keys.Open, m.keys.OpenAndApprove) {
		pr, ok := m.selectedPR()
		if !ok {
			return m, nil
		}
		m.loading = true
		m.status = "loading diff..."
		m.err = ""
		return m, loadDiffCmd(pr)
	}

	// Defer movement, filtering, and pagination to the list component.
	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m model) handleDiffKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Back):
		m.screen = screenList
		m.keys.setScreen(screenList)
		m.diffPR = nil
		m.files = nil
		m.focusArea = focusDiff
		m.status = fmt.Sprintf("%d review request(s)", len(m.list.Items()))
		return m, nil
	case key.Matches(msg, m.keys.Approve):
		if m.diffPR == nil || m.loading {
			return m, nil
		}
		pr := *m.diffPR
		m.loading = true
		m.status = "approving..."
		m.err = ""
		return m, approveCmd(pr)
	case key.Matches(msg, m.keys.ToggleFocus):
		if len(m.files) == 0 || !m.sidebarVisible() {
			// Without a sidebar there's nothing to toggle to.
			return m, nil
		}
		if m.focusArea == focusDiff {
			m.focusArea = focusFiles
		} else {
			m.focusArea = focusDiff
		}
		return m, nil
	case key.Matches(msg, m.keys.NextFile):
		m.jumpToFile(+1)
		return m, nil
	case key.Matches(msg, m.keys.PrevFile):
		m.jumpToFile(-1)
		return m, nil
	}

	if m.focusArea == focusFiles {
		prev := m.fileList.Index()
		var cmd tea.Cmd
		m.fileList, cmd = m.fileList.Update(msg)
		if idx := m.fileList.Index(); idx != prev && idx >= 0 && idx < len(m.files) {
			m.diff.SetYOffset(m.files[idx].StartLine)
		}
		return m, cmd
	}

	var cmd tea.Cmd
	m.diff, cmd = m.diff.Update(msg)
	return m, cmd
}

// jumpToFile scrolls the diff viewport to the next (delta=+1) or previous
// (delta=-1) file boundary, syncing the sidebar selection along the way.
func (m *model) jumpToFile(delta int) {
	if len(m.files) == 0 {
		return
	}
	// Find the file whose StartLine matches or contains the current YOffset.
	cur := m.diff.YOffset()
	idx := 0
	for i, f := range m.files {
		if f.StartLine <= cur {
			idx = i
		} else {
			break
		}
	}
	next := idx + delta
	if next < 0 {
		next = 0
	}
	if next >= len(m.files) {
		next = len(m.files) - 1
	}
	m.diff.SetYOffset(m.files[next].StartLine)
	m.fileList.Select(next)
}

// selectedPR returns the pullRequest currently highlighted in the list, if any.
func (m model) selectedPR() (pullRequest, bool) {
	item := m.list.SelectedItem()
	if item == nil {
		return pullRequest{}, false
	}
	p, ok := item.(prItem)
	if !ok {
		return pullRequest{}, false
	}
	return p.pr, true
}

func (m model) View() tea.View {
	content := m.renderHeader() + "\n\n"
	if m.screen == screenDiff {
		content += m.renderDiff()
	} else {
		content += m.renderList()
	}
	content += "\n" + m.renderFooter()

	v := tea.NewView(content)
	v.AltScreen = true
	v.MouseMode = tea.MouseModeCellMotion
	v.WindowTitle = "gh review"
	// Hide the cursor on list/diff screens by leaving Cursor nil; when set to
	// non-nil it would be shown at the given position.
	v.Cursor = nil
	return v
}

func (m model) renderHeader() string {
	parts := []string{titleStyle.Render("gh review")}
	if m.status != "" {
		parts = append(parts, mutedStyle.Render(m.status))
	}
	if m.loading {
		parts = append(parts, mutedStyle.Render("working"))
	}
	return strings.Join(parts, "  ")
}

func (m model) renderList() string {
	if m.err != "" {
		return errorStyle.Render(m.err)
	}
	return m.list.View()
}

func (m model) renderDiff() string {
	if m.err != "" {
		return errorStyle.Render(m.err)
	}
	if m.diffPR == nil {
		return mutedStyle.Render("No PR selected.")
	}
	header := fmt.Sprintf("%s  #%d  %s", m.diffPR.Repository, m.diffPR.Number, m.diffPR.Title)

	if !m.sidebarVisible() || len(m.files) == 0 {
		return titleStyle.Render(header) + "\n" + m.diff.View()
	}

	sidebarBox := unfocusedPaneStyle
	diffBox := focusedPaneStyle
	if m.focusArea == focusFiles {
		sidebarBox = focusedPaneStyle
		diffBox = unfocusedPaneStyle
	}

	sidebar := sidebarBox.Render(m.fileList.View())
	body := diffBox.Render(m.diff.View())
	joined := lipgloss.JoinHorizontal(lipgloss.Top, sidebar, body)
	return titleStyle.Render(header) + "\n" + joined
}

// sidebarVisible reports whether the terminal has enough room to render the
// file sidebar without crowding out the diff body.
func (m model) sidebarVisible() bool {
	return m.width > sidebarMinTerm
}

func (m model) renderFooter() string {
	return m.help.View(m.keys)
}

func (m *model) resizeChildren() {
	if m.width > 0 {
		m.help.SetWidth(m.width)
		// The sidebar consumes a fixed slice of horizontal space when visible;
		// account for its border (one column) and the inter-pane gap so the
		// diff body doesn't overflow into wrapped lines.
		diffWidth := m.width
		if m.sidebarVisible() {
			// sidebar = sidebarWidth content + 1 col left border
			// body left border = 1 col
			diffWidth = m.width - sidebarWidth - 2 - sidebarGap
			if diffWidth < 1 {
				diffWidth = 1
			}
		}
		m.diff.SetWidth(diffWidth)
	}
	if m.height > reservedRows {
		body := m.height - reservedRows
		m.diff.SetHeight(body)
		m.list.SetSize(m.width, body)
		if m.sidebarVisible() {
			m.fileList.SetSize(sidebarWidth, body)
		} else {
			m.fileList.SetSize(0, 0)
		}
	} else if m.width > 0 {
		m.list.SetSize(m.width, 0)
		m.fileList.SetSize(0, 0)
	}
}

func loadPRsCmd() tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		prs, err := loadReviewRequests(ctx)
		return prListMsg{prs: prs, err: err}
	}
}

func loadDiffCmd(pr pullRequest) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		diff, err := loadDiff(ctx, pr)
		return diffMsg{pr: pr, diff: diff, err: err}
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

func prLabel(pr pullRequest) string {
	return fmt.Sprintf("%s#%d", pr.Repository, pr.Number)
}
