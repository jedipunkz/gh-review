# gh-review

`gh review` lists open pull requests that request review from you or your teams, shows the PR diff in a TUI, and approves from the keyboard.

This extension is written in Rust.

## Install

Prerequisites:

- A GitHub CLI (`gh`) installation that is authenticated
- [Rust](https://rustup.rs) 1.75 or later (only for building from source)

Install as a binary extension (uses prebuilt assets attached to GitHub
releases for Linux amd64/arm64, macOS amd64/arm64, and Windows amd64):

```bash
gh extension install jedipunkz/gh-review
```

Build from source instead:

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

## Tabs and filter

The list is split into four tabs: `Awaiting Review`, `Reviewed`, `Merged`, and
`Closed`. A PR counts as reviewed when its GitHub review decision is `APPROVED`,
or after you approve it from this TUI during the current session.

`Merged` and `Closed` list merged PRs and closed-but-unmerged PRs that you
reviewed (`reviewed-by:@me`) or that request review from you or your teams.
Each search fetches only the 100 most recently updated PRs. These tabs load at
startup and on `r`; the per-minute auto refresh does not re-check them. Merged
and closed PRs cannot be approved.

Press `/` to edit a filter. The filter matches repository name, PR title, and
author (case-insensitive substring). While editing:

- `enter` confirms and leaves the filter value in place
- `esc` clears the value and closes the filter
- `ctrl+n` / `ctrl+p` move the selection while the cursor stays in the input

## Keys

### List

- `ctrl+n` / `ctrl+p`: move the selection down / up within the filtered list
- `h` / `l`: switch between the `Awaiting Review`, `Reviewed`, `Merged`, and
  `Closed` tabs
- `/`: edit the filter (see above)
- `y`: copy the selected PR URL to the clipboard
- `a`: open the approve confirmation popup; `y` approves, `c` or `esc` cancels
- `r`: refresh the review list and the merged / closed PRs
- `q` or `ctrl+c`: quit

Key presses also dismiss the update notice popup when it is shown.

### Detail

The detail pane below the list shows the selected PR: metadata, description,
headline diff, and the latest reviews.

- `j` / `k`: scroll the detail
- `pgup` / `pgdn`: page-scroll the detail

## Auto refresh

While running, the extension re-checks your review requests every minute.

- If the new result contains fewer review requests than before, the list is
  replaced silently.
- Otherwise, a notice popup appears; if new review requests arrived for you or
  your teams, a sound plays too (macOS only). The popup closes on any key press
  or after a few seconds on its own.

## Notes

- This extension shells out to `gh`, so it uses the same authentication, host,
  and GitHub Enterprise configuration as GitHub CLI.
- The clipboard (`pbcopy`) and notification sound (`afplay`) are implemented
  with macOS commands; on other platforms `y` reports an error and the sound
  is skipped.
- PR details and diffs are cached on disk under your OS cache directory
  (`~/Library/Caches/gh-review` on macOS, `~/.cache/gh-review` on Linux) with
  an LRU cap of 200 files. The cache is keyed by the PR's `updatedAt`, so a PR
  that has changed is re-fetched automatically.

## Development

```bash
cargo test      # unit tests
cargo clippy    # lints
```
