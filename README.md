# gh-review

`gh review` lists open pull requests that request review from you or your teams, shows the PR diff in a TUI, and approves from the keyboard.

## Install

```bash
make build
gh extension install .
```

The repository or checkout directory must be named `gh-review` because `gh`
derives the command name from the extension repository name.

During development:

```bash
make install-dev
gh review
```

`make install-dev` symlinks the current checkout to GitHub CLI's extension
directory as `gh-review`, so it works from git worktrees whose directory names
are not `gh-review`.

## Keys

- `j` / `k`: move selection or scroll diff
- `enter` / `d`: show diff
- `a`: from the list, show the diff first; from the diff view, approve
- `esc` / `b`: return to the list
- `r`: refresh
- `q`: quit

## Notes

This extension shells out to `gh`, so it uses the same authentication, host, and GitHub Enterprise configuration as GitHub CLI.
