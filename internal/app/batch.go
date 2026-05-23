package app

import (
	"context"
	"time"

	tea "charm.land/bubbletea/v2"
)

// bulkApproveStartMsg signals the start of a bulk approve operation. It is
// emitted as the first step of bulkApproveCmd so the model can switch into
// bulk-progress mode (initialise the progress bar, capture the total) before
// any per-PR progress arrives.
type bulkApproveStartMsg struct {
	total int
}

// bulkApproveProgressMsg is emitted after each individual approve attempt
// completes. done is the 1-indexed count of PRs processed so far, err carries
// the per-PR error (nil on success). The model aggregates errors itself; the
// terminating bulkApproveDoneMsg is a pure completion signal.
type bulkApproveProgressMsg struct {
	done int
	pr   pullRequest
	err  error
}

// bulkApproveDoneMsg signals that every PR in the batch has been processed.
// It carries no payload — error aggregation lives on the model so we don't
// have to thread a slice through the tea.Cmd sequence.
type bulkApproveDoneMsg struct{}

// bulkApproveCmd builds a sequential command pipeline that approves each PR
// in order, emitting a progress message after each one. tea.Sequence guarantees
// the start → progress* → done ordering, which keeps the UI's progress bar in
// lockstep with the underlying gh api calls.
func bulkApproveCmd(prs []pullRequest) tea.Cmd {
	cmds := make([]tea.Cmd, 0, len(prs)+2)
	total := len(prs)
	cmds = append(cmds, func() tea.Msg {
		return bulkApproveStartMsg{total: total}
	})
	for i, pr := range prs {
		i, pr := i, pr
		cmds = append(cmds, func() tea.Msg {
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			err := approvePR(ctx, pr)
			return bulkApproveProgressMsg{done: i + 1, pr: pr, err: err}
		})
	}
	cmds = append(cmds, func() tea.Msg { return bulkApproveDoneMsg{} })
	return tea.Sequence(cmds...)
}
