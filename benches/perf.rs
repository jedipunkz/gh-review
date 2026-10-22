//! divan benchmarks for gh-review's hot paths: the work every fetch, every
//! detail render, and every keystroke does. Run with `make bench`, or
//! `cargo bench --bench perf`. The bench profile is optimized, so the
//! timings are release-grade.

fn main() {
    divan::main();
}

use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use gh_review::app;
use gh_review::gh::{self, PullRequest, PullRequestDetail};
use gh_review::prefetch;

// A search page the way `gh api graphql` returns it: dozens of pull
// requests with the fields the TUI reads. One page of a busy repository is
// a realistic upper bound for what a single parse does per refresh.
const SEARCH_NODES: usize = 50;

fn search_json() -> &'static Vec<u8> {
    static J: OnceLock<Vec<u8>> = OnceLock::new();
    J.get_or_init(|| {
        let mut nodes = Vec::new();
        for i in 0..SEARCH_NODES {
            let author = format!(r#"{{"login": "contributor{i}"}}"#);
            nodes.push(format!(
                r#"{{"number": {i}, "title": "feat: add ctrl+l to force a full screen repaint {i}",
                    "url": "https://github.com/jedipunkz/gh-review/pull/{num}", "updatedAt": "2026-10-22T09:15:32Z",
                    "reviewDecision": "REVIEW_REQUIRED", "state": "OPEN",
                    "repository": {{"nameWithOwner": "jedipunkz/gh-review"}},
                    "author": {author}}}"#,
                num = 1000 + i
            ));
        }
        format!(
            r#"{{"data": {{"search": {{"pageInfo": {{"hasNextPage": true, "endCursor": "cursor-1"}},
                "nodes": [{}]}}}}}}"#,
            nodes.join(",")
        )
        .into_bytes()
    })
}

fn prs() -> &'static Vec<PullRequest> {
    static P: OnceLock<Vec<PullRequest>> = OnceLock::new();
    P.get_or_init(|| {
        let page =
            gh::parse_search_prs_response(search_json(), "open").expect("sample page parses");
        page.prs
    })
}

fn diff() -> &'static str {
    static D: OnceLock<String> = OnceLock::new();
    D.get_or_init(|| {
        let mut s = String::new();
        for i in 0..400 {
            s.push_str(&format!(
                "@@ -{i},7 +{i},7 @@\n context line {}\n-removed line {}\n+added line {}\n",
                i, i, i
            ));
        }
        s
    })
}

fn body() -> &'static str {
    static B: OnceLock<String> = OnceLock::new();
    B.get_or_init(|| {
        let mut s = String::new();
        s.push_str("## Why\n\nA paragraph with `inline code`, a [link](https://example.com), and ");
        s.push_str("a very long line of prose that the wrapper has to break across the ");
        s.push_str("terminal width, over and over, on every render.\n\n");
        s.push_str("```rust\nfn main() { println!(\"hello\"); }\n```\n\n");
        s.push_str("- list item one\n- list item two\n- list item three\n\n> quoted text\n\n");
        s.push_str("| col a | col b |\n| --- | --- |\n| 1 | 2 |\n\n");
        for i in 0..40 {
            s.push_str(&format!("Paragraph {} continues the document.\n\n", i));
        }
        s
    })
}

fn detail() -> PullRequestDetail {
    PullRequestDetail {
        base: PullRequest {
            repository: "jedipunkz/gh-review".into(),
            number: 56,
            title: "fix: sanitize PR text and diff lines before rendering".into(),
            url: "https://github.com/jedipunkz/gh-review/pull/56".into(),
            author: "jedipunkz".into(),
            updated_at: SystemTime::UNIX_EPOCH,
            request: "open".into(),
            review_decision: "REVIEW_REQUIRED".into(),
            state: "OPEN".into(),
        },
        body: body().to_string(),
        ..Default::default()
    }
}

#[divan::bench]
fn parse_search_page() {
    gh::parse_search_prs_response(search_json(), "open").expect("sample page parses");
}

#[divan::bench]
fn cache_key() {
    let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_760_000_000);
    gh::cache_key_of_updated("https://github.com/jedipunkz/gh-review/pull/56", t);
}

#[divan::bench]
fn parse_rfc3339() {
    gh::parse_rfc3339("2026-10-22T09:15:32.123456789Z");
}

#[divan::bench]
fn render_markdown_80col() {
    app::render_markdown(body(), 80);
}

#[divan::bench]
fn render_diff_120col() {
    let d = detail();
    app::render_diff_content(&d, diff(), 120);
}

#[divan::bench]
fn combine_pr_lists() {
    let open = prs().clone();
    app::combine_pr_lists(open.clone(), open);
}

#[divan::bench]
fn pr_list_signature() {
    app::pr_list_signature(prs());
}

#[divan::bench]
fn prefetch_top_n() {
    prefetch::top_n(prs(), 3);
}

// A model in the steady state a user sees: the list fetched, one pull
// request selected with its detail and diff loaded, search closed.
fn frame_model() -> app::Model {
    let cache = Arc::new(gh_review::cache::DetailCache::with_dir(None));
    let mut m = app::Model::new(cache.clone(), Arc::new(prefetch::Prefetcher::new(cache)));
    m.width = 120;
    m.height = 40;
    m.prs = prs().clone();
    m.pr_list_loaded = true;
    m.cursor = 0;
    m.current_detail = Some(detail());
    m.current_diff = diff().to_string();
    m
}

// First frame with the model: includes the one-time detail rebuild, the
// cost paid on every selection change.
#[divan::bench]
fn render_frame_120x40() {
    let mut m = frame_model();
    let mut term =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).expect("test backend");
    term.draw(|f| app::render(&mut m, f)).expect("draw");
}

// The fixed cost of Terminal::new + one empty draw on this backend: what
// every render_frame bench pays before any widget runs.
#[divan::bench]
fn render_backend_overhead_120x40() {
    let mut term =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).expect("test backend");
    term.draw(|_| {}).expect("draw");
}

// The same model drawn a second time, with the detail view cache warm: this
// is the cost the spinner tick pays ten times a second, not the one-time
// detail rebuild. The terminal is kept across iterations so the number is
// the draw itself, not TestBackend setup.
#[divan::bench]
fn render_frame_steady_120x40() {
    static S: OnceLock<
        std::sync::Mutex<(ratatui::Terminal<ratatui::backend::TestBackend>, app::Model)>,
    > = OnceLock::new();
    let mut g = S
        .get_or_init(|| {
            let mut m = frame_model();
            let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40))
                .expect("test backend");
            term.draw(|f| app::render(&mut m, f)).expect("warm draw");
            std::sync::Mutex::new((term, m))
        })
        .lock()
        .unwrap();
    let (term, m) = &mut *g;
    term.draw(|f| app::render(m, f)).expect("draw");
}
