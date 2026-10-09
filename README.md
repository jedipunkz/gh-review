<div align="center">

# gh-review

**Your review queue, one keystroke away.**

A [GitHub CLI](https://cli.github.com) extension that lists pull requests
waiting for your review, shows the diff in a TUI, and approves from the keyboard.

<p>
  <a href="https://github.com/jedipunkz/gh-review/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jedipunkz/gh-review/ci.yml?branch=main&label=ci&style=flat-square" alt="CI status"></a>
  <a href="https://github.com/jedipunkz/gh-review/releases/latest"><img src="https://img.shields.io/github/v/release/jedipunkz/gh-review?style=flat-square" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/built_with-Rust-dea584?style=flat-square&logo=rust" alt="Built with Rust">
</p>

<p>
  <a href="#-installation">Installation</a> ·
  <a href="#-keys">Keys</a> ·
  <a href="#-configuration">Configuration</a> ·
  <a href="#-development">Development</a>
</p>

</div>

## ✨ Features

- **Review queue at a glance**: open PRs that request review from you or your
  teams, split into `Awaiting Review`, `Reviewed`, `Merged`, and `Closed` tabs.
- **Diff in the terminal**: metadata, description, headline diff, and the
  latest reviews of the selected PR.
- **Approve and comment from the keyboard**: `a` to approve, `c` to write a
  review comment (IME input supported, drafts kept per PR).
- **Auto refresh**: re-checks every minute and notifies you (with a sound on
  macOS) when new review requests arrive.
- **Fast**: PR details and diffs are cached on disk and re-fetched only when
  the PR changes.
- **Themes**: `tokyonight`, `kanagawa-wave`, `solarized`, `gruvbox`,
  `catppuccin-mocha`.
- **Works wherever `gh` works**: same authentication, host, and GitHub
  Enterprise configuration as GitHub CLI.

## 🚀 Installation

### Prerequisites

- A GitHub CLI (`gh`) installation that is authenticated
- [Rust](https://rustup.rs) 1.75 or later (only for building from source)

### Step 1. Install the extension

Prebuilt binaries are attached to GitHub releases for Linux amd64/arm64,
macOS amd64/arm64, and Windows amd64:

```bash
gh extension install jedipunkz/gh-review
```

<details>
<summary>Build from source</summary>

```bash
make build
gh extension install .
```

The repository or checkout directory must be named `gh-review` because `gh`
derives the command name from the extension repository name.

</details>

### Step 2. Run it

```bash
gh review
```

## 🔑 Keys

### List

| Key | Action |
|---|---|
| `ctrl+n` / `ctrl+p` | Move the selection down / up within the filtered list |
| `h` / `l` | Switch between the `Awaiting Review`, `Reviewed`, `Merged`, and `Closed` tabs |
| `/` | Edit the filter (see [Tabs and filter](#tabs-and-filter)) |
| `y` | Copy the selected PR URL to the clipboard |
| `a` | Open the approve confirmation popup; `y` approves, `c` or `esc` cancels |
| `c` | Open the review comment popup (see [Review comment](#review-comment)) |
| `r` | Refresh the review list (and the `Merged` / `Closed` tab) |
| `q` / `ctrl+c` | Quit |

Key presses also dismiss the update notice popup when it is shown.

### Detail

The detail pane below the list shows the selected PR: metadata, description,
headline diff, and the latest reviews.

| Key | Action |
|---|---|
| `j` / `k` | Scroll the detail |
| `pgup` / `pgdn` | Page-scroll the detail |

## 📖 Usage

### Tabs and filter

The list is split into four tabs: `Awaiting Review`, `Reviewed`, `Merged`, and
`Closed`. A PR counts as reviewed when its GitHub review decision is `APPROVED`,
or after you approve it from this TUI during the current session.

`Merged` and `Closed` list merged PRs and closed-but-unmerged PRs that you
reviewed (`reviewed-by:@me`) or that request review from you or your teams.
Each search fetches only the 100 most recently updated PRs. Each of these tabs
is fetched the first time you switch to it (the tab label shows no count until
then). `r` refetches the active one and makes the other refetch on its next
visit; the per-minute auto refresh does not re-check them. Merged and closed
PRs cannot be approved.

Press `/` to edit a filter. The filter matches repository name, PR title, and
author (case-insensitive substring). While editing:

- `enter` confirms and leaves the filter value in place
- `esc` clears the value and closes the filter
- `ctrl+n` / `ctrl+p` move the selection while the cursor stays in the input

### Review comment

`c` opens a popup to write a review comment on the selected open PR. The text
is sent with `gh pr review --comment`.

- Type freely, including Japanese via your IME; `enter` inserts a newline
- `ctrl+enter` or `ctrl+s` sends the comment
- `esc` closes the popup; the unsent text is kept as a draft for that PR and
  restored the next time you press `c` (a failed send also keeps the draft)

> [!NOTE]
> `ctrl+enter` needs a terminal that supports the kitty keyboard protocol
> (kitty, WezTerm, Ghostty, foot, Alacritty, recent iTerm2 with the CSI u
> setting). On other terminals, such as macOS Terminal.app, `ctrl+enter` is
> indistinguishable from `enter`; use `ctrl+s` instead.

### Auto refresh

While running, the extension re-checks your review requests every minute.

- If the new result contains fewer review requests than before, the list is
  replaced silently.
- Otherwise, a notice popup appears; if new review requests arrived for you or
  your teams, a sound plays too (macOS only). The popup closes on any key press
  or after a few seconds on its own.

## 🔧 Configuration

Pick a color theme in `~/.config/gh-review/gh-review.yaml`:

```yaml
theme: kanagawa-wave
```

Available themes: `tokyonight` (default), `kanagawa-wave`, `solarized`,
`gruvbox`, `catppuccin-mocha`. If the file is missing or has no `theme` key,
`tokyonight` is used. An unknown theme name or invalid YAML makes
`gh review` exit with an error.

## 📝 Notes

- This extension shells out to `gh`, so it uses the same authentication, host,
  and GitHub Enterprise configuration as GitHub CLI.
- The clipboard (`pbcopy`) and notification sound (`afplay`) are implemented
  with macOS commands; on other platforms `y` reports an error and the sound
  is skipped.
- PR details and diffs are cached on disk under your OS cache directory
  (`~/Library/Caches/gh-review` on macOS, `~/.cache/gh-review` on Linux) with
  an LRU cap of 200 files. The cache is keyed by the PR's `updatedAt`, so a PR
  that has changed is re-fetched automatically.

## 🔨 Development

```bash
make install-dev
gh review
```

`make install-dev` symlinks the current checkout to GitHub CLI's extension
directory as `gh-review`, so it works from git worktrees whose directory names
are not `gh-review`.

```bash
cargo test      # unit tests
cargo clippy    # lints
```

<details>
<summary>Release</summary>

Run the `release` workflow from the Actions tab (or
`gh workflow run release.yml -f bump=minor`) on `main` and pick `bump`:

- `patch` (default): `v1.2.3` → `v1.2.4`
- `minor`: `v1.2.3` → `v1.3.0`
- `major`: `v1.2.3` → `v2.0.0`

The next version is computed from the latest `vX.Y.Z` tag (`v0.0.0` when none
exists). The workflow creates the tag and a GitHub release with generated
notes, then attaches the built binaries. Pushing a `v*` tag by hand still
works too.

</details>
