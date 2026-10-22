## Why

<!-- What problem does this change solve? One or two sentences. -->

## What

<!-- What changed, in one or two sentences? -->

## Reference

<!-- Issue, PR, or "N/A". -->

## Checklist

- [ ] Safe: nothing outside gh-review's own cache and extension paths is written or deleted — name the test that shows it.
- [ ] Tests added; say which of them fail without this change.
- [ ] `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --locked` clean.
- [ ] A long-running gh subprocess call added in the app runs on a task, not the UI thread.
- [ ] Any new dependency is in `Cargo.lock`, and its license is acceptable.
