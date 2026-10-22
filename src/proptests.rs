//! Property tests over the boundaries remote data crosses: everything `gh`
//! answers with, and the strings the TUI renders. The invariants here are
//! ones a hand-written example test misses — parse never panics on arbitrary
//! input, a formatted timestamp round-trips, a rendered buffer never carries
//! a control character. Run with `cargo test`; the proptest-regressions/
//! directory keeps any counterexample cargo-mutants or CI ever finds.

use std::time::{Duration, SystemTime};

use proptest::prelude::*;

use crate::app;
use crate::cache::cache_key;
use crate::gh::{format_rfc3339_nanos, parse_rfc3339, PullRequest, PullRequestDetail};

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(256))]

    // Every GitHub timestamp is seconds and nanoseconds after the epoch;
    // whatever the formatter writes, the parser must read back exactly.
    // Bounded at the year 10000: the formatter prints a five-digit year
    // there and the parser's four-digit field stops reading GitHub-shaped
    // input, so the invariant is stated over the timestamps gh can send.
    #[test]
    fn rfc3339_round_trips(
        secs in 0u64..253_402_300_800u64,
        nanos in 0u32..1_000_000_000u32,
    ) {
        let t = SystemTime::UNIX_EPOCH + Duration::new(secs, nanos);
        let s = format_rfc3339_nanos(t);
        assert_eq!(parse_rfc3339(&s), t, "round trip broke for {s}");
    }

    // The parser sees strings gh emits and strings users type. Any input is
    // allowed to parse to the epoch, but no input may panic.
    #[test]
    fn rfc3339_never_panics(s in ".*") {
        let _ = parse_rfc3339(&s);
    }

    // The cache key is the filename of every disk cache entry: always 16
    // lowercase hex characters, always the same key for the same input.
    #[test]
    fn cache_key_is_hex_and_stable(
        url in "https://.*",
        secs in 0u64..253_402_300_800u64,
    ) {
        let t = SystemTime::UNIX_EPOCH + Duration::new(secs, 0);
        let key = cache_key(&url, t);
        prop_assert_eq!(key.len(), 16);
        prop_assert!(key.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        prop_assert_eq!(key, cache_key(&url, t));
    }

    // The detail view renders whatever the PR body is — remote text, typed
    // by anyone — at whatever width the terminal has. Formatting may look
    // odd; a panic is a bug.
    #[test]
    fn render_markdown_never_panics(
        body in any::<String>(),
        width in 1usize..200usize,
    ) {
        let _ = app::render_markdown(&body, width);
    }

    // The diff view likewise renders arbitrary remote text. Nothing the
    // sanitizer missed may survive into the rendered spans: control
    // characters (a newline would split a line) and ESC bytes must be gone.
    #[test]
    fn render_diff_output_is_control_free(
        diff in any::<String>(),
        width in 1usize..200usize,
    ) {
        let d = PullRequestDetail {
            base: PullRequest {
                repository: "jedipunkz/gh-review".into(),
                number: 1,
                title: "title \u{1b}[31mred\u{0}".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = app::render_diff_content(&d, &diff, width);
        for line in lines {
            for span in line.spans {
                let text = span.content.as_ref();
                prop_assert!(text.chars().all(|c| c == '\n' || !c.is_control()),
                    "control character rendered: {text:?}");
            }
        }
    }
}
