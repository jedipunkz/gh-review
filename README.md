# gh-review-cli

`gh review-cli` lists open pull requests that request review from you or your teams, shows the PR diff in a TUI, and approves from the keyboard.

## Install

```bash
make build
gh extension install .
```

During development:

```bash
make build
./gh-review-cli
```

## Keys

- `j` / `k`: move selection or scroll diff
- `enter` / `d`: show diff
- `a`: from the list, show the diff first; from the diff view, approve
- `esc` / `b`: return to the list
- `r`: refresh
- `q`: quit

## Notes

This extension shells out to `gh`, so it uses the same authentication, host, and GitHub Enterprise configuration as GitHub CLI.
