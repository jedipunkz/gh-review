package app

import "charm.land/bubbles/v2/key"

// keyMap holds every key binding the TUI reacts to. It also implements the
// help.KeyMap interface so the bubbles help component can render contextual
// help text directly from these definitions.
//
// OpenAndApprove and Approve both bind to "a" but only one is enabled at a
// time depending on the current screen: OpenAndApprove on the list screen
// opens the diff (preserving the historical behaviour of treating "a" as a
// shortcut to the diff view), Approve on the diff screen submits an approval.
type keyMap struct {
	Up              key.Binding
	Down            key.Binding
	Open            key.Binding
	OpenAndApprove  key.Binding
	Approve         key.Binding
	ToggleSelect    key.Binding
	ApproveSelected key.Binding
	Refresh         key.Binding
	Back            key.Binding
	NextFile        key.Binding
	PrevFile        key.Binding
	ToggleFocus     key.Binding
	Help            key.Binding
	Quit            key.Binding

	screen screen
}

func newKeyMap() keyMap {
	km := keyMap{
		Up: key.NewBinding(
			key.WithKeys("k", "up"),
			key.WithHelp("k/↑", "up"),
		),
		Down: key.NewBinding(
			key.WithKeys("j", "down"),
			key.WithHelp("j/↓", "down"),
		),
		Open: key.NewBinding(
			key.WithKeys("enter", "d"),
			key.WithHelp("enter/d", "diff"),
		),
		OpenAndApprove: key.NewBinding(
			key.WithKeys("a"),
			key.WithHelp("a", "diff+approve"),
		),
		Approve: key.NewBinding(
			key.WithKeys("a"),
			key.WithHelp("a", "approve"),
		),
		ToggleSelect: key.NewBinding(
			key.WithKeys("space"),
			key.WithHelp("space", "select"),
		),
		ApproveSelected: key.NewBinding(
			key.WithKeys("A"),
			key.WithHelp("A", "approve selected"),
		),
		Refresh: key.NewBinding(
			key.WithKeys("r"),
			key.WithHelp("r", "refresh"),
		),
		Back: key.NewBinding(
			key.WithKeys("esc", "b"),
			key.WithHelp("esc/b", "back"),
		),
		NextFile: key.NewBinding(
			key.WithKeys("n"),
			key.WithHelp("n", "next file"),
		),
		PrevFile: key.NewBinding(
			key.WithKeys("p"),
			key.WithHelp("p", "prev file"),
		),
		ToggleFocus: key.NewBinding(
			key.WithKeys("tab"),
			key.WithHelp("tab", "toggle focus"),
		),
		Help: key.NewBinding(
			key.WithKeys("?"),
			key.WithHelp("?", "help"),
		),
		Quit: key.NewBinding(
			key.WithKeys("q", "ctrl+c"),
			key.WithHelp("q", "quit"),
		),
	}
	km.setScreen(screenList)
	return km
}

// setScreen toggles bindings that are only valid on a specific screen so the
// help view renders the right options and key.Matches won't match disabled
// bindings.
func (k *keyMap) setScreen(s screen) {
	k.screen = s
	switch s {
	case screenList:
		k.OpenAndApprove.SetEnabled(true)
		k.Open.SetEnabled(true)
		k.Approve.SetEnabled(false)
		k.ToggleSelect.SetEnabled(true)
		k.ApproveSelected.SetEnabled(true)
		k.Back.SetEnabled(false)
		k.NextFile.SetEnabled(false)
		k.PrevFile.SetEnabled(false)
		k.ToggleFocus.SetEnabled(false)
	case screenDiff:
		k.OpenAndApprove.SetEnabled(false)
		k.Open.SetEnabled(false)
		k.Approve.SetEnabled(true)
		k.ToggleSelect.SetEnabled(false)
		k.ApproveSelected.SetEnabled(false)
		k.Back.SetEnabled(true)
		k.NextFile.SetEnabled(true)
		k.PrevFile.SetEnabled(true)
		k.ToggleFocus.SetEnabled(true)
	}
}

// ShortHelp implements help.KeyMap.
func (k keyMap) ShortHelp() []key.Binding {
	switch k.screen {
	case screenDiff:
		return []key.Binding{k.Up, k.Down, k.NextFile, k.PrevFile, k.ToggleFocus, k.Approve, k.Back, k.Help, k.Quit}
	default:
		return []key.Binding{k.Up, k.Down, k.Open, k.OpenAndApprove, k.ToggleSelect, k.ApproveSelected, k.Refresh, k.Help, k.Quit}
	}
}

// FullHelp implements help.KeyMap.
func (k keyMap) FullHelp() [][]key.Binding {
	switch k.screen {
	case screenDiff:
		return [][]key.Binding{
			{k.Up, k.Down},
			{k.NextFile, k.PrevFile, k.ToggleFocus},
			{k.Approve, k.Back},
			{k.Refresh, k.Help, k.Quit},
		}
	default:
		return [][]key.Binding{
			{k.Up, k.Down},
			{k.Open, k.OpenAndApprove},
			{k.ToggleSelect, k.ApproveSelected},
			{k.Refresh, k.Help, k.Quit},
		}
	}
}
