package app

import tea "charm.land/bubbletea/v2"

func Run() error {
	_, err := tea.NewProgram(newModel()).Run()
	return err
}
