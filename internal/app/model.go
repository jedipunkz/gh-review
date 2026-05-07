package app

import (
	"context"
	"fmt"
	"strings"
	"time"

	"charm.land/bubbles/v2/viewport"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/mattn/go-runewidth"
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

type model struct {
	loading       bool
	status        string
	err           string
	prs           []pullRequest
	cursor        int
	currentDetail *pullRequestDetail
	detailLoading bool
	detailErr     string
	loadingForURL string
	detailVP      viewport.Model
	width         int
	height        int
	approved      map[string]bool
}

var (
	titleStyle       = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("39"))
	selectedStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("230")).Background(lipgloss.Color("62"))
	mutedStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("245"))
	errorStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("196"))
	okStyle          = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	meFrameStyle     = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(lipgloss.Color("39"))
	teamFrameStyle   = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(lipgloss.Color("214"))
	detailFrameStyle = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(lipgloss.Color("245")).PaddingLeft(1)
)

func newModel() model {
	vp := viewport.New()
	vp.SoftWrap = false
	return model{
		loading:  true,
		status:   "loading review requests...",
		detailVP: vp,
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
		m.resizeViewport()
		if len(m.prs) > 0 {
			cmd := m.triggerDetailLoad()
			return m, cmd
		}
		return m, nil
	case diffMsg:
		if msg.pr.URL != m.loadingForURL {
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
		m.detailVP.SetContent(renderDiffContent(msg.detail, msg.diff))
		m.detailVP.GotoTop()
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
		return m, loadPRsCmd()
	}
	return m, nil
}

func (m model) handleKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	key := msg.String()
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
		if !m.loading && m.cursor < len(m.prs)-1 {
			m.cursor++
			cmd := m.triggerDetailLoad()
			return m, cmd
		}
	case "ctrl+p":
		if !m.loading && m.cursor > 0 {
			m.cursor--
			cmd := m.triggerDetailLoad()
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
	case "a":
		if m.currentDetail != nil && !m.loading && !m.detailLoading {
			pr := m.currentDetail.pullRequest
			m.loading = true
			m.status = "approving..."
			m.err = ""
			return m, approveCmd(pr)
		}
	}
	return m, nil
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
	return loadDiffCmd(pr)
}

func (m model) View() tea.View {
	parts := []string{
		m.renderHeader(),
		m.renderGroupedList(),
		m.renderDetailSection(),
		m.renderFooter(),
	}
	return tea.NewView(strings.Join(parts, "\n"))
}

func (m model) renderHeader() string {
	parts := []string{titleStyle.Render("gh review")}
	if m.status != "" {
		parts = append(parts, mutedStyle.Render(m.status))
	}
	if m.loading || m.detailLoading {
		parts = append(parts, mutedStyle.Render("working..."))
	}
	return strings.Join(parts, "  ")
}

func (m model) groupPRs() (me, team []pullRequest) {
	for _, pr := range m.prs {
		if strings.Contains(pr.Request, "@me") {
			me = append(me, pr)
		} else {
			team = append(team, pr)
		}
	}
	return
}

func (m model) renderGroupedList() string {
	if m.err != "" && len(m.prs) == 0 {
		return errorStyle.Render(m.err)
	}
	if len(m.prs) == 0 {
		return mutedStyle.Render("No open PRs are requesting your review.")
	}

	boxW := m.frameWidth()
	me, team := m.groupPRsByIndex()

	var sections []string

	if len(me) > 0 {
		contentW := frameContentWidth(meFrameStyle, boxW)
		var lines []string
		lines = append(lines, titleStyle.Render("Me"))
		lines = append(lines, m.renderListHeader(contentW))
		for _, item := range me {
			lines = append(lines, m.renderPRLine(item.idx, item.pr, contentW))
		}
		inner := 2 + len(me)
		frameH := inner + 2
		sections = append(sections, meFrameStyle.Width(boxW).Height(inner).MaxHeight(frameH).Render(strings.Join(lines, "\n")))
	}

	if len(team) > 0 {
		contentW := frameContentWidth(teamFrameStyle, boxW)
		var lines []string
		lines = append(lines, titleStyle.Render("Team"))
		lines = append(lines, m.renderListHeader(contentW))
		for _, item := range team {
			lines = append(lines, m.renderPRLine(item.idx, item.pr, contentW))
		}
		inner := 2 + len(team)
		frameH := inner + 2
		sections = append(sections, teamFrameStyle.Width(boxW).Height(inner).MaxHeight(frameH).Render(strings.Join(lines, "\n")))
	}

	return strings.Join(sections, "\n")
}

type indexedPR struct {
	idx int
	pr  pullRequest
}

func (m model) groupPRsByIndex() (me, team []indexedPR) {
	for i, pr := range m.prs {
		if strings.Contains(pr.Request, "@me") {
			me = append(me, indexedPR{i, pr})
		} else {
			team = append(team, indexedPR{i, pr})
		}
	}
	return
}

const (
	colRepoW   = 28
	colNumW    = 6
	colAuthorW = 15
)

func listTitleWidth(boxW, approvedW int) int {
	// budget: leading/trailing space (2) + repo + "  " (2) + num + "  " (2) + title + "  " (2) + "@" or " " (1) + author + approved
	fixed := 2 + colRepoW + 2 + colNumW + 2 + 2 + 1 + colAuthorW + approvedW
	return max(10, boxW-fixed)
}

func (m model) renderListHeader(boxW int) string {
	titleW := listTitleWidth(boxW, 0)
	line := fmt.Sprintf("%s  %s  %s   %s",
		padRight("Repository", colRepoW),
		padRight("#", colNumW),
		padRight("Title", titleW),
		padRight("Author", colAuthorW),
	)
	return " " + mutedStyle.Render(line)
}

func (m model) renderPRLine(idx int, pr pullRequest, boxW int) string {
	approvedW := 0
	approved := m.approved[pr.URL]
	if approved {
		approvedW = 9 // " approved"
	}
	titleW := listTitleWidth(boxW, approvedW)

	line := fmt.Sprintf("%s  %s  %s  %s",
		padRight(pr.Repository, colRepoW),
		padRight(fmt.Sprintf("#%d", pr.Number), colNumW),
		padRight(pr.Title, titleW),
		mutedStyle.Render("@"+padRight(pr.Author, colAuthorW)),
	)
	if approved {
		line += " " + okStyle.Render("approved")
	}
	if idx == m.cursor {
		return selectedStyle.Render(" " + line + " ")
	}
	return " " + line
}

func (m model) renderDetailSection() string {
	boxW := m.frameWidth()
	vpH := m.detailVP.Height()
	var content string
	switch {
	case m.detailLoading:
		content = mutedStyle.Render("loading detail...")
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
	return max(40, m.width-4)
}

func frameContentWidth(style lipgloss.Style, width int) int {
	return max(1, width-style.GetHorizontalFrameSize())
}

func (m model) renderFooter() string {
	return mutedStyle.Render("ctrl+n/p list  j/k scroll  pgup/pgdn page  a approve  r refresh  q quit")
}

func (m *model) resizeViewport() {
	if m.width == 0 || m.height == 0 {
		return
	}
	listH := m.computeListSectionHeight()
	// layout: header(1) + list(listH) + detailBorder(2) + vpH + footer(1) = 4 + listH + vpH
	vpH := max(3, m.height-4-listH)
	vpW := max(20, frameContentWidth(detailFrameStyle, m.frameWidth()))
	m.detailVP.SetHeight(vpH)
	m.detailVP.SetWidth(vpW)
}

func (m model) computeListSectionHeight() int {
	me, team := m.groupPRs()
	h := 0
	if len(me) > 0 {
		h += 2 + 2 + len(me) // top border + bottom border + title line + header line + items
	}
	if len(team) > 0 {
		if h > 0 {
			h++ // gap between frames
		}
		h += 2 + 2 + len(team)
	}
	if h == 0 {
		h = 1
	}
	return h
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
		detail, err := loadPRDetail(ctx, pr)
		if err != nil {
			return diffMsg{pr: pr, err: err}
		}
		diff, err := loadDiff(ctx, pr)
		return diffMsg{pr: pr, detail: detail, diff: diff, err: err}
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

func renderDiffContent(detail pullRequestDetail, diff string) string {
	var b strings.Builder
	b.WriteString(titleStyle.Render(fmt.Sprintf("%s  #%d  %s", detail.Repository, detail.Number, detail.Title)))
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
	b.WriteString(formatMeta("Review", nonEmpty(detail.ReviewDecision, "none")))
	b.WriteString("  ")
	b.WriteString(formatMeta("Files", fmt.Sprintf("%d", detail.ChangedFiles)))
	b.WriteString("  ")
	b.WriteString(okStyle.Render(fmt.Sprintf("+%d", detail.Additions)))
	b.WriteString(" ")
	b.WriteString(errorStyle.Render(fmt.Sprintf("-%d", detail.Deletions)))
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
		b.WriteString(body)
	}
	b.WriteString("\n\n")
	b.WriteString(mutedStyle.Render(strings.Repeat("-", 80)))
	b.WriteString("\n\n")
	b.WriteString(diff)
	return b.String()
}

func formatMeta(label, value string) string {
	return mutedStyle.Render(label+":") + " " + value
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

func truncate(s string, maxWidth int) string {
	runes := []rune(s)
	if maxWidth <= 3 || len(runes) <= maxWidth {
		return s
	}
	return string(runes[:maxWidth-3]) + "..."
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
