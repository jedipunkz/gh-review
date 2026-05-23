package app

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"charm.land/bubbles/v2/help"
	"charm.land/bubbles/v2/key"
	"charm.land/bubbles/v2/list"
	"charm.land/bubbles/v2/progress"
	"charm.land/bubbles/v2/spinner"
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

// Layout constants for the bordered panes. Each pane uses a rounded border
// (1 col per side) and horizontal padding (1 col per side), so the inner
// content area is 4 columns narrower than the pane and 2 rows shorter than
// the pane itself. The constants encode these overheads in a single place so
// resizeChildren and View stay in sync.
const (
	borderH  = 2 // 1 left + 1 right
	borderV  = 2 // 1 top + 1 bottom
	paddingH = 2 // Padding(0, 1) -> 1 left + 1 right
	// paneOverheadH is the total horizontal space (border + padding) consumed
	// by a single bordered pane.
	paneOverheadH = borderH + paddingH

	// minBorderedWidth / minBorderedHeight are the thresholds below which we
	// drop the borders entirely and fall back to plain text — borders eat too
	// much space on tiny terminals and would otherwise hide the content.
	minBorderedWidth  = 40
	minBorderedHeight = 10
)

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
// list.DefaultItem so the default delegate can render it. The `selected`
// flag drives the multi-select checkbox marker rendered in Title.
type prItem struct {
	pr       pullRequest
	approved bool
	selected bool
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
	marker := mutedStyle.Render("[ ] ")
	if p.selected {
		marker = accentStyle.Render("[x] ")
	}
	title := fmt.Sprintf("%s%s #%d  %s", marker, p.pr.Repository, p.pr.Number, p.pr.Title)
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
	// selected tracks which PRs (by URL) are currently checked for bulk
	// approve. Items are toggled with space on the list screen.
	selected map[string]bool
	// progress drives the in-app gradient progress bar shown while a bulk
	// approve is running. It animates between SetPercent calls.
	progress progress.Model
	// bulkActive guards the bulk-progress UI so only one bulk approve is in
	// flight at a time and most key bindings can be suppressed during the run.
	bulkActive bool
	bulkTotal  int
	bulkDone   int
	bulkErrs   []error
	// spinner animates next to loadingLabel while a non-bulk async command is
	// in flight (PR list / diff / single approve). Bulk approve uses the
	// progress bar instead, so the spinner is suppressed during bulk runs to
	// avoid two competing indicators in the header.
	spinner      spinner.Model
	loadingLabel string
}

// lightDark resolves a pair of (light, dark) colors based on the terminal
// background. Built once at package init so we don't probe the terminal on
// every render — the result is stable for the life of the process.
var lightDark = lipgloss.LightDark(lipgloss.HasDarkBackground(os.Stdin, os.Stdout))

var (
	// titleStyle is adaptive: a deeper blue on light terminals, a brighter
	// cyan on dark ones, so the header reads cleanly in both themes.
	titleStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lightDark(lipgloss.Color("27"), lipgloss.Color("39")))
	mutedStyle = lipgloss.NewStyle().
			Foreground(lightDark(lipgloss.Color("240"), lipgloss.Color("245")))
	errorStyle = lipgloss.NewStyle().
			Foreground(lightDark(lipgloss.Color("160"), lipgloss.Color("196")))
	okStyle = lipgloss.NewStyle().
		Foreground(lightDark(lipgloss.Color("28"), lipgloss.Color("42")))
	// accentStyle highlights the selection marker on selected PR rows so the
	// "[x]" reads as a strong visual cue even on busy backgrounds.
	accentStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lightDark(lipgloss.Color("27"), lipgloss.Color("39")))

	// borderColor is the resting border color used for non-focused panes and
	// the outer header/content/footer frames. accentColor highlights the
	// currently focused pane on the diff screen.
	borderColor = lightDark(lipgloss.Color("250"), lipgloss.Color("240"))
	accentColor = lightDark(lipgloss.Color("27"), lipgloss.Color("39"))

	// paneStyle is the shared frame used by the header, content, and footer
	// rows. The width is set per-render so the frame spans the full terminal.
	paneStyle = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(borderColor).
			Padding(0, 1)

	// focusedPaneStyle outlines the currently focused pane (sidebar or diff)
	// with a rounded accent border; unfocusedPaneStyle keeps the same shape
	// but uses the resting border color so the focused side stands out.
	focusedPaneStyle = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(accentColor)
	unfocusedPaneStyle = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(borderColor)
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

	sp := spinner.New(spinner.WithSpinner(spinner.Dot))
	sp.Style = lipgloss.NewStyle().Foreground(accentColor)

	return model{
		loading:      true,
		status:       "loading review requests...",
		loadingLabel: "fetching review requests...",
		list:         l,
		diff:         vp,
		fileList:     fl,
		focusArea:    focusDiff,
		keys:         newKeyMap(),
		help:         help.New(),
		approved:     make(map[string]bool),
		selected:     make(map[string]bool),
		progress:     progress.New(progress.WithDefaultBlend()),
		spinner:      sp,
	}
}

func (m model) Init() tea.Cmd {
	// Kick off the initial PR fetch and start ticking the spinner so the
	// header animates immediately while the request is in flight.
	return tea.Batch(loadPRsCmd(), m.spinner.Tick)
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
		m.loadingLabel = ""
		if msg.err != nil {
			m.err = msg.err.Error()
			m.status = "failed to load review requests"
			return m, nil
		}
		m.err = ""
		// Drop selections for PRs that no longer appear in the refreshed list
		// (e.g. they were approved & dismissed) so stale URLs don't quietly
		// hang around in m.selected.
		alive := make(map[string]bool, len(msg.prs))
		for _, pr := range msg.prs {
			alive[pr.URL] = true
		}
		for url := range m.selected {
			if !alive[url] {
				delete(m.selected, url)
			}
		}
		items := make([]list.Item, 0, len(msg.prs))
		for _, pr := range msg.prs {
			items = append(items, prItem{
				pr:       pr,
				approved: m.approved[pr.URL],
				selected: m.selected[pr.URL],
			})
		}
		cmd := m.list.SetItems(items)
		m.status = fmt.Sprintf("%d review request(s)", len(msg.prs))
		return m, cmd
	case diffMsg:
		m.loading = false
		m.loadingLabel = ""
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
		m.loadingLabel = ""
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
		// Re-enter the loading state for the implicit list refresh that follows
		// a successful approve, and restart the spinner tick.
		m.loading = true
		m.loadingLabel = "fetching review requests..."
		return m, tea.Batch(loadPRsCmd(), m.spinner.Tick)
	case bulkApproveStartMsg:
		m.bulkActive = true
		m.bulkTotal = msg.total
		m.bulkDone = 0
		m.bulkErrs = nil
		m.err = ""
		m.status = fmt.Sprintf("approving 0/%d...", msg.total)
		cmd := m.progress.SetPercent(0)
		return m, cmd
	case bulkApproveProgressMsg:
		m.bulkDone = msg.done
		if msg.err != nil {
			m.bulkErrs = append(m.bulkErrs, msg.err)
		} else {
			m.approved[msg.pr.URL] = true
			delete(m.selected, msg.pr.URL)
		}
		m.status = fmt.Sprintf("approving %d/%d...", msg.done, m.bulkTotal)
		var pct float64
		if m.bulkTotal > 0 {
			pct = float64(msg.done) / float64(m.bulkTotal)
		}
		cmd := m.progress.SetPercent(pct)
		return m, cmd
	case bulkApproveDoneMsg:
		m.bulkActive = false
		failed := len(m.bulkErrs)
		ok := m.bulkTotal - failed
		if failed > 0 {
			m.status = fmt.Sprintf("approved %d (%d failed)", ok, failed)
			// Surface the first error so the user has something to act on; the
			// rest are silently dropped because they'd typically be redundant.
			m.err = m.bulkErrs[0].Error()
		} else {
			m.status = fmt.Sprintf("approved %d", ok)
		}
		m.selected = make(map[string]bool)
		// The implicit list refresh after a bulk approve is a plain async load,
		// so flip back into the spinner-driven loading state for the user.
		m.loading = true
		m.loadingLabel = "fetching review requests..."
		return m, tea.Batch(loadPRsCmd(), m.spinner.Tick)
	case progress.FrameMsg:
		// Forward animation frames to the progress component so its spring
		// settles smoothly between SetPercent calls.
		var cmd tea.Cmd
		m.progress, cmd = m.progress.Update(msg)
		return m, cmd
	case spinner.TickMsg:
		// Only keep ticking while a non-bulk async op is in flight; otherwise
		// drop the tick so the spinner naturally stops between operations.
		if !m.loading || m.bulkActive {
			return m, nil
		}
		var cmd tea.Cmd
		m.spinner, cmd = m.spinner.Update(msg)
		return m, cmd
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
	// While a bulk approve is running, only allow Quit and Help so the user
	// can bail out or check the keymap without interfering with the in-flight
	// approvals.
	if m.bulkActive {
		switch {
		case key.Matches(msg, m.keys.Quit):
			return m, tea.Quit
		case key.Matches(msg, m.keys.Help):
			m.help.ShowAll = !m.help.ShowAll
			m.resizeChildren()
			return m, nil
		}
		return m, nil
	}

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
		// Toggling help changes the footer's height, so re-flow the children
		// (the content pane shrinks to make room for the expanded help text).
		m.resizeChildren()
		return m, nil
	case key.Matches(msg, m.keys.Refresh):
		if m.loading {
			return m, nil
		}
		m.loading = true
		m.loadingLabel = "fetching review requests..."
		m.status = "refreshing..."
		m.err = ""
		return m, tea.Batch(loadPRsCmd(), m.spinner.Tick)
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
		m.loadingLabel = fmt.Sprintf("loading diff for %s#%d...", pr.Repository, pr.Number)
		m.status = "loading diff..."
		m.err = ""
		return m, tea.Batch(loadDiffCmd(pr), m.spinner.Tick)
	}
	if key.Matches(msg, m.keys.ToggleSelect) {
		pr, ok := m.selectedPR()
		if !ok {
			return m, nil
		}
		if m.selected[pr.URL] {
			delete(m.selected, pr.URL)
		} else {
			m.selected[pr.URL] = true
		}
		// Rebuild list items so the checkbox marker reflects the new state.
		// Cheaper than mutating in place because items are stored by value.
		idx := m.list.Index()
		items := m.list.Items()
		next := make([]list.Item, 0, len(items))
		for _, it := range items {
			pi, ok := it.(prItem)
			if !ok {
				next = append(next, it)
				continue
			}
			pi.selected = m.selected[pi.pr.URL]
			next = append(next, pi)
		}
		cmd := m.list.SetItems(next)
		m.list.Select(idx)
		m.status = fmt.Sprintf("%d selected", len(m.selected))
		return m, cmd
	}
	if key.Matches(msg, m.keys.ApproveSelected) {
		if len(m.selected) == 0 {
			m.status = "nothing selected"
			return m, nil
		}
		// Snapshot the selection in list order so progress messages arrive in
		// the same order the user sees them.
		prs := make([]pullRequest, 0, len(m.selected))
		for _, it := range m.list.Items() {
			pi, ok := it.(prItem)
			if !ok {
				continue
			}
			if m.selected[pi.pr.URL] {
				prs = append(prs, pi.pr)
			}
		}
		if len(prs) == 0 {
			return m, nil
		}
		return m, bulkApproveCmd(prs)
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
		m.loadingLabel = fmt.Sprintf("approving %s#%d...", pr.Repository, pr.Number)
		m.status = "approving..."
		m.err = ""
		return m, tea.Batch(approveCmd(pr), m.spinner.Tick)
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
	v := tea.NewView(m.renderFrame())
	v.AltScreen = true
	v.MouseMode = tea.MouseModeCellMotion
	v.WindowTitle = "gh review"
	// Hide the cursor on list/diff screens by leaving Cursor nil; when set to
	// non-nil it would be shown at the given position.
	v.Cursor = nil
	// Mirror bulk-approve progress to the terminal's native progress bar
	// (OSC 9;4) so terminals like Windows Terminal / WezTerm can surface it
	// outside the TUI. Value is the integer percent; the field is only set
	// while a bulk approve is in flight.
	if m.bulkActive && m.bulkTotal > 0 {
		pct := int(float64(m.bulkDone) / float64(m.bulkTotal) * 100)
		v.ProgressBar = &tea.ProgressBar{
			State: tea.ProgressBarDefault,
			Value: pct,
		}
	}
	return v
}

// renderFrame composes the header, content, and footer panes vertically.
// On terminals too small to host borders cleanly it falls back to the
// previous plain-text layout so nothing gets clipped or wrapped weirdly.
func (m model) renderFrame() string {
	headerText := m.renderHeader()
	bodyText := m.renderBody()
	footerText := m.renderFooter()

	if !m.borderedLayout() {
		// Plain fallback for tiny terminals — keep the prior look so the app
		// degrades gracefully on small panes / CI environments.
		return headerText + "\n\n" + bodyText + "\n" + footerText
	}

	frame := paneStyle.Width(m.width)
	header := frame.Render(headerText)
	footer := frame.Render(footerText)

	// We want header, content, footer to stack to exactly m.height rows.
	// lipgloss's Width/Height set the *outer* block size (borders included),
	// so the content frame's Height is the leftover rows after the header
	// and footer have claimed theirs.
	contentH := m.height - lipgloss.Height(header) - lipgloss.Height(footer)
	if contentH < borderV+1 {
		contentH = borderV + 1
	}
	content := frame.Height(contentH).Render(bodyText)

	return lipgloss.JoinVertical(lipgloss.Left, header, content, footer)
}

// borderedLayout reports whether the current terminal size has room for the
// rounded outer frame. Below the thresholds we drop borders so the content
// still fits.
func (m model) borderedLayout() bool {
	return m.width >= minBorderedWidth && m.height >= minBorderedHeight
}

func (m model) renderHeader() string {
	parts := []string{titleStyle.Render("gh review")}
	if m.status != "" {
		parts = append(parts, mutedStyle.Render(m.status))
	}
	// Show the animated spinner + a contextual label while a non-bulk async
	// command is in flight. Bulk approve owns the progress bar below, so we
	// suppress the spinner there to avoid two competing indicators.
	if m.loading && !m.bulkActive {
		label := m.loadingLabel
		if label == "" {
			label = "working..."
		}
		parts = append(parts, m.spinner.View()+mutedStyle.Render(label))
	}
	line := strings.Join(parts, "  ")
	if m.bulkActive {
		// Stack the gradient progress bar under the status line so it's
		// visible at a glance without crowding the title row.
		line += "\n" + m.progress.View()
	}
	return line
}

// renderBody returns the inner content for the middle pane, dispatching on
// the active screen. Errors short-circuit to a single line so the surrounding
// frame stays intact.
func (m model) renderBody() string {
	if m.err != "" {
		return errorStyle.Render(m.err)
	}
	switch m.screen {
	case screenDiff:
		return m.renderDiff()
	default:
		return m.list.View()
	}
}

func (m model) renderDiff() string {
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

// resizeChildren resizes the list / viewport / sidebar to fit the available
// terminal space, after accounting for the outer header / content / footer
// frames as well as the bordered sidebar and diff panes on the diff screen.
//
// The math is fiddly enough that the layout would silently desync if we
// hand-rolled it inline twice (once here, once in renderFrame). To keep them
// in lockstep we measure the actually rendered header and footer with
// lipgloss.Height instead of guessing.
func (m *model) resizeChildren() {
	if m.width <= 0 || m.height <= 0 {
		return
	}
	m.help.SetWidth(m.width)
	// Keep the progress bar narrower than the header pane so it never wraps
	// inside the rounded frame. The pane consumes paneOverheadH columns of
	// border + padding, and we leave a small slack for the status text.
	pw := m.width - paneOverheadH - 4
	if pw < 10 {
		pw = 10
	}
	m.progress.SetWidth(pw)

	if !m.borderedLayout() {
		// Plain fallback path: mirror the previous (pre-border) sizing so the
		// children still get a sensible viewport on tiny terminals.
		body := m.height - 4 // header (1) + blank (1) + footer (1) + slack (1)
		if body < 1 {
			body = 1
		}
		m.diff.SetWidth(m.width)
		m.diff.SetHeight(body)
		m.list.SetSize(m.width, body)
		m.fileList.SetSize(0, 0)
		return
	}

	// Width available inside the outer rounded frame:
	//   m.width - border(2) - padding(2)
	innerW := m.width - paneOverheadH
	if innerW < 1 {
		innerW = 1
	}

	// Measure the rendered header + footer to compute the leftover content
	// height. The header itself is always a single line; the footer expands
	// to two-ish lines when help.ShowAll is true. Each occupies +2 rows for
	// its own top/bottom border (padding is 0,1 so no vertical padding).
	headerRendered := paneStyle.Width(m.width).Render(m.renderHeader())
	footerRendered := paneStyle.Width(m.width).Render(m.renderFooter())
	innerH := m.height - lipgloss.Height(headerRendered) - lipgloss.Height(footerRendered) - borderV
	if innerH < 1 {
		innerH = 1
	}

	// Diff screen: the inner area is split into a bordered sidebar and a
	// bordered diff body. Each sub-pane has its own rounded border (2 cols /
	// 2 rows of overhead). The sidebar's outer width is sidebarWidth, so its
	// inner text area is sidebarWidth - borderH.
	if m.sidebarVisible() {
		sidebarInnerW := sidebarWidth - borderH
		if sidebarInnerW < 1 {
			sidebarInnerW = 1
		}
		diffInnerW := innerW - sidebarWidth - sidebarGap - borderH
		if diffInnerW < 1 {
			diffInnerW = 1
		}
		// Both sub-panes share the same inner height. They sit under a single
		// title line, so subtract one more row for that header text.
		subInnerH := innerH - 1 - borderV
		if subInnerH < 1 {
			subInnerH = 1
		}
		m.diff.SetWidth(diffInnerW)
		m.diff.SetHeight(subInnerH)
		m.fileList.SetSize(sidebarInnerW, subInnerH)
	} else {
		// No sidebar: diff body fills the inner width minus its own border,
		// under the title line.
		diffInnerW := innerW - borderH
		if diffInnerW < 1 {
			diffInnerW = 1
		}
		subInnerH := innerH - 1
		if subInnerH < 1 {
			subInnerH = 1
		}
		m.diff.SetWidth(diffInnerW)
		m.diff.SetHeight(subInnerH)
		m.fileList.SetSize(0, 0)
	}

	// List screen: the list fills the full inner area of the content frame.
	m.list.SetSize(innerW, innerH)
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
