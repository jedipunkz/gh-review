package app

import (
	"context"
	"fmt"
	"os/exec"
	"runtime"
	"sort"
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

const popupDismissDelay = 6 * time.Second

type model struct {
	loading        bool
	status         string
	err            string
	prs            []pullRequest
	prSignature    string
	prListLoaded   bool
	cursor         int
	currentDetail  *pullRequestDetail
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
}

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
	tokyoNightSelected   = lipgloss.Color("#283457")
	tokyoNightSelectedFg = lipgloss.Color("#ffffff")

	titleStyle          = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightBlue)
	selectedStyle       = lipgloss.NewStyle().Foreground(tokyoNightFG).Background(tokyoNightSelected)
	selectedTextStyle   = lipgloss.NewStyle().Foreground(tokyoNightSelectedFg).Background(tokyoNightSelected)
	selectedMutedStyle  = lipgloss.NewStyle().Foreground(tokyoNightYellow).Background(tokyoNightSelected)
	selectedOKStyle     = lipgloss.NewStyle().Foreground(tokyoNightGreen).Background(tokyoNightSelected)
	selectedErrorStyle  = lipgloss.NewStyle().Foreground(tokyoNightRed).Background(tokyoNightSelected)
	mutedStyle          = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	footerStyle         = lipgloss.NewStyle().Foreground(tokyoNightFG)
	errorStyle          = lipgloss.NewStyle().Foreground(tokyoNightRed)
	okStyle             = lipgloss.NewStyle().Foreground(tokyoNightGreen)
	listMeTitleStyle    = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightCyan)
	listTeamTitleStyle  = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightOrange)
	listHeaderStyle     = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	listRepoStyle       = lipgloss.NewStyle().Foreground(tokyoNightCyan)
	listNumStyle        = lipgloss.NewStyle().Foreground(tokyoNightBlue)
	listTitleStyle      = lipgloss.NewStyle().Foreground(tokyoNightFG)
	listAuthorStyle     = lipgloss.NewStyle().Foreground(tokyoNightMagenta)
	detailTitleStyle    = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightMagenta)
	detailMetaKeyStyle  = lipgloss.NewStyle().Foreground(tokyoNightYellow)
	detailMetaTextStyle = lipgloss.NewStyle().Foreground(tokyoNightFG)
	detailRuleStyle     = lipgloss.NewStyle().Foreground(tokyoNightMuted)
	framePink           = lipgloss.Color("#f7768e")
	frameGray           = lipgloss.Color("#a9b1d6")
	meFrameStyle        = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(framePink)
	teamFrameStyle      = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(framePink)
	detailFrameStyle    = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(frameGray).PaddingLeft(1)
	updateNoticeStyle   = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).BorderForeground(tokyoNightYellow).Padding(0, 1)
	markStyle           = lipgloss.NewStyle().Bold(true).Foreground(tokyoNightYellow)
)

const updateCheckInterval = time.Minute

func newModel() model {
	vp := viewport.New()
	vp.SoftWrap = false
	return model{
		loading:   true,
		status:    "loading review requests...",
		detailVP:  vp,
		approved:  make(map[string]bool),
		markedPRs: make(map[string]bool),
	}
}

func (m model) Init() tea.Cmd {
	return tea.Batch(loadPRsCmd(), updateCheckTickCmd())
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
		m.prs = msg.prs
		m.prSignature = msg.currentSignature
		if m.cursor >= len(m.prs) {
			m.cursor = max(0, len(m.prs)-1)
		}
		m.resizeViewport()
		m.popupSeq++
		m.updateNotice = &updateNotice{count: msg.count, id: m.popupSeq}
		m.status = fmt.Sprintf("%d review request(s)", len(m.prs))
		var cmds []tea.Cmd
		cmds = append(cmds, playNotifySoundCmd(), dismissPopupCmd(m.popupSeq))
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
		m.prs = msg.prs
		m.prSignature = prListSignature(msg.prs)
		m.prListLoaded = true
		m.updateNotice = nil
		m.pruneMarkedPRs()
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
			m.status = fmt.Sprintf("Approve %s? yes/no", prLabel(pr))
			return m, nil
		}
	}
	return m, nil
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
	case "n", "no", "esc":
		m.pendingApprove = nil
		m.status = "approval canceled"
		m.err = ""
		return m, nil
	default:
		m.status = fmt.Sprintf("Approve %s? yes/no", prLabel(pr))
		return m, nil
	}
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
	view := strings.Join(parts, "\n")
	if m.updateNotice != nil {
		view = m.overlayUpdateNotice(view)
	}
	return tea.NewView(view)
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
		lines = append(lines, listMeTitleStyle.Render("Me"))
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
		lines = append(lines, listTeamTitleStyle.Render("Team"))
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
	colMarkW    = 3
	colRepoW    = 28
	colNumW     = 6
	colAuthorW  = 15
	colApproveW = 11
)

func listTitleWidth(boxW int) int {
	// budget: leading/trailing space (2) + mark + repo + num + title + author + approve columns and separators.
	fixed := 2 + colMarkW + 1 + colRepoW + 2 + colNumW + 2 + 2 + 1 + colAuthorW + 2 + colApproveW
	return max(10, boxW-fixed)
}

func (m model) renderListHeader(boxW int) string {
	titleW := listTitleWidth(boxW)
	line := fmt.Sprintf("%s %s  %s  %s   %s  %s",
		padRight("", colMarkW),
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

	line := fmt.Sprintf("%s %s  %s  %s  %s  %s",
		m.markCell(pr, selected),
		m.listRepoStyle(selected).Render(padRight(pr.Repository, colRepoW)),
		m.listNumStyle(selected).Render(padRight(fmt.Sprintf("#%d", pr.Number), colNumW)),
		m.listTitleStyle(selected).Render(padRight(pr.Title, titleW)),
		m.listAuthorStyle(selected).Render("@"+padRight(pr.Author, colAuthorW)),
		m.approveStyle(approve, selected).Render(padRight(approve, colApproveW)),
	)
	if selected {
		return selectedStyle.Render(" " + line + " ")
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
	return strings.Repeat(" ", colMarkW)
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
	if m.width > 0 {
		return m.width
	}
	return 40
}

func frameContentWidth(style lipgloss.Style, width int) int {
	return max(1, width-style.GetHorizontalFrameSize())
}

func (m model) renderFooter() string {
	if m.pendingApprove != nil {
		return footerStyle.Render("confirm approve: y/yes approve  n/no cancel")
	}
	return footerStyle.Render("ctrl+n/p list  j/k scroll  pgup/pgdn page  a approve  r refresh  q quit")
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

func loadDiffCmd(pr pullRequest) tea.Cmd {
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		detail, err := loadPRDetail(ctx, pr)
		if err != nil {
			return diffMsg{pr: pr, err: err}
		}
		diff, err := loadDiff(ctx, pr)
		if isPRDiffTooLargeError(err) {
			diff = mutedStyle.Render("Diff omitted because GitHub reports this PR diff is too large to display.")
			err = nil
		}
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

var notifySoundFile = "/System/Library/Sounds/Pop.aiff"

func playNotifySoundCmd() tea.Cmd {
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

func renderDiffContent(detail pullRequestDetail, diff string) string {
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
	b.WriteString(detailRuleStyle.Render(strings.Repeat("-", 80)))
	b.WriteString("\n\n")
	b.WriteString(diff)
	return b.String()
}

func formatMeta(label, value string) string {
	return detailMetaKeyStyle.Render(label+":") + " " + detailMetaTextStyle.Render(value)
}

func formatReviewMeta(decision string) string {
	value := nonEmpty(decision, "none")
	return detailMetaKeyStyle.Render("Review:") + " " + reviewDecisionStyle(decision).Render(value)
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
