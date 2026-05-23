package app

import (
	"context"
	"fmt"
	"strings"
	"time"

	"charm.land/bubbles/v2/help"
	"charm.land/bubbles/v2/key"
	"charm.land/bubbles/v2/viewport"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

type screen int

const (
	screenList screen = iota
	screenDiff
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

type model struct {
	screen   screen
	loading  bool
	status   string
	err      string
	prs      []pullRequest
	cursor   int
	diffPR   *pullRequest
	diff     viewport.Model
	keys     keyMap
	help     help.Model
	width    int
	height   int
	approved map[string]bool
}

var (
	titleStyle    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("39"))
	selectedStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("230")).Background(lipgloss.Color("62"))
	mutedStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
	errorStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("196"))
	okStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
)

func newModel() model {
	vp := viewport.New()
	vp.SoftWrap = false
	return model{
		loading:  true,
		status:   "loading review requests...",
		diff:     vp,
		keys:     newKeyMap(),
		help:     help.New(),
		approved: make(map[string]bool),
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
		m.resizeViewport()
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
		m.prs = msg.prs
		if m.cursor >= len(m.prs) {
			m.cursor = max(0, len(m.prs)-1)
		}
		m.status = fmt.Sprintf("%d review request(s)", len(m.prs))
		return m, nil
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
		m.diff.SetContent(msg.diff)
		m.diff.GotoTop()
		m.status = "press a to approve, esc to go back"
		return m, nil
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
	return m, nil
}

func (m model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
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
	switch {
	case key.Matches(msg, m.keys.Down):
		if m.cursor < len(m.prs)-1 {
			m.cursor++
		}
	case key.Matches(msg, m.keys.Up):
		if m.cursor > 0 {
			m.cursor--
		}
	case key.Matches(msg, m.keys.Open, m.keys.OpenAndApprove):
		if len(m.prs) == 0 {
			return m, nil
		}
		m.loading = true
		m.status = "loading diff..."
		m.err = ""
		return m, loadDiffCmd(m.prs[m.cursor])
	}
	return m, nil
}

func (m model) handleDiffKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case key.Matches(msg, m.keys.Back):
		m.screen = screenList
		m.keys.setScreen(screenList)
		m.diffPR = nil
		m.status = fmt.Sprintf("%d review request(s)", len(m.prs))
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
	}

	var cmd tea.Cmd
	m.diff, cmd = m.diff.Update(msg)
	return m, cmd
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
	if len(m.prs) == 0 {
		return mutedStyle.Render("No open PRs are requesting your review.")
	}

	var b strings.Builder
	for i, pr := range m.prs {
		line := fmt.Sprintf("%s  #%d  %s  %s  %s",
			truncate(pr.Repository, 28),
			pr.Number,
			truncate(pr.Title, max(20, m.width-70)),
			mutedStyle.Render("@"+pr.Author),
			mutedStyle.Render(pr.Request),
		)
		if m.approved[pr.URL] {
			line += " " + okStyle.Render("approved")
		}
		if i == m.cursor {
			line = selectedStyle.Render(" " + line + " ")
		} else {
			line = " " + line
		}
		b.WriteString(line)
		if i < len(m.prs)-1 {
			b.WriteByte('\n')
		}
	}
	return b.String()
}

func (m model) renderDiff() string {
	if m.err != "" {
		return errorStyle.Render(m.err)
	}
	if m.diffPR == nil {
		return mutedStyle.Render("No PR selected.")
	}
	header := fmt.Sprintf("%s  #%d  %s", m.diffPR.Repository, m.diffPR.Number, m.diffPR.Title)
	return titleStyle.Render(header) + "\n" + m.diff.View()
}

func (m model) renderFooter() string {
	return m.help.View(m.keys)
}

func (m *model) resizeViewport() {
	if m.width > 0 {
		m.diff.SetWidth(m.width)
		m.help.SetWidth(m.width)
	}
	if m.height > 6 {
		m.diff.SetHeight(m.height - 6)
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

func truncate(s string, maxWidth int) string {
	runes := []rune(s)
	if maxWidth <= 3 || len(runes) <= maxWidth {
		return s
	}
	return string(runes[:maxWidth-3]) + "..."
}
