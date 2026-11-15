use std::borrow::Cow;

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, Clear, HighlightSpacing, Padding, Paragraph, Row, Table, TableState, Tabs, Wrap,
};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::*;

// ---- rendering ----

fn pad_right(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let w = s.width();
    if w > width {
        let mut out = String::new();
        let budget = width.saturating_sub(3);
        let mut used = 0;
        for ch in s.chars() {
            let cw = ch.width().unwrap_or(0);
            if used + cw > budget {
                break;
            }
            out.push(ch);
            used += cw;
        }
        out.push_str("...");
        return out;
    }
    let mut out = s.to_string();
    for _ in w..width {
        out.push(' ');
    }
    out
}

fn repeat_space(n: usize) -> String {
    " ".repeat(n)
}

pub(super) fn fg(slot: theme::Slot) -> Style {
    Style::new().fg(c(slot))
}

pub fn render_diff_content(
    detail: &PullRequestDetail,
    diff: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let title = format!(
        "{}  #{}  {}",
        detail.base.repository,
        detail.base.number,
        sanitize_text(&detail.base.title)
    );
    lines.push(Line::from(Span::styled(
        title,
        Style::new()
            .fg(c(colors::MAGENTA))
            .add_modifier(Modifier::BOLD),
    )));

    // Status pills: merge state, review decision, CI, then the diffstat.
    let mut status = vec![
        state_pill(&detail.merge_state_status),
        Span::raw(" "),
        review_pill(&detail.base.review_decision),
    ];
    let ci = ci_spans(&detail.ci);
    if !ci.is_empty() {
        status.push(Span::raw(" "));
        status.extend(ci);
    }
    status.push(Span::raw("   "));
    status.push(Span::styled(
        format!("{} files ", detail.changed_files),
        fg(colors::MUTED),
    ));
    status.push(Span::styled(
        format!("+{}", detail.additions),
        fg(colors::GREEN),
    ));
    status.push(Span::raw(" "));
    status.push(Span::styled(
        format!("-{}", detail.deletions),
        fg(colors::RED),
    ));
    if let Some((add, del)) = diffstat_blocks(detail.additions, detail.deletions) {
        status.push(Span::raw(" "));
        status.push(Span::styled("■".repeat(add), fg(colors::GREEN)));
        status.push(Span::styled("■".repeat(del), fg(colors::RED)));
    }
    lines.push(Line::from(status));

    // Fields come in related pairs (Author/Requested, Created/Updated) that
    // the grid keeps on one row; long ones (the branch, many reviewers or
    // labels) get full rows below.
    let mut fields: Vec<(&str, Vec<Span<'static>>)> = vec![
        (
            "Author",
            vec![Span::styled(
                format!("@{}", detail.base.author),
                fg(colors::MAGENTA),
            )],
        ),
        ("Requested", vec![Span::raw(detail.base.request.clone())]),
    ];
    for (label, at) in [
        ("Created", detail.created_at),
        ("Updated", detail.base.updated_at),
    ] {
        if at != std::time::SystemTime::UNIX_EPOCH {
            fields.push((label, vec![Span::raw(crate::gh::format_human(at))]));
        }
    }
    if !detail.reviewers.is_empty() {
        let mut spans = Vec::new();
        for (i, reviewer) in detail.reviewers.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(META_GAP));
            }
            spans.push(Span::raw(format!("@{} ", reviewer.author)));
            spans.push(Span::styled(
                reviewer.state.to_lowercase().replace('_', " "),
                review_decision_style(&reviewer.state),
            ));
        }
        fields.push(("Reviewers", spans));
    }
    if !detail.labels.is_empty() {
        let mut spans = Vec::new();
        for (i, label) in detail.labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(pill(&sanitize_text(label), None));
        }
        fields.push(("Labels", spans));
    }
    let branch = branch_label(detail);
    if !branch.is_empty() {
        fields.push(("Branch", vec![Span::styled(branch, fg(colors::CYAN))]));
    }
    lines.extend(field_grid(fields, width));

    lines.push(rule(width));

    let body = sanitize_text(detail.body.trim());
    let body = body.trim();
    if body.is_empty() {
        lines.push(Line::from(Span::styled(
            "No description.".to_string(),
            fg(colors::MUTED),
        )));
    } else {
        lines.extend(render_markdown(body, width));
    }

    lines.push(rule(width));

    lines.extend(highlighted_diff(diff).iter().cloned());

    // Wrap every logical line to the content width up front. Paragraph would
    // otherwise word-wrap long lines (diff hunks, code fences, long headers)
    // with its own layout, whose row count the scroll-total estimate below
    // cannot reproduce exactly; pre-wrapped lines are never re-wrapped, so
    // one pre-wrapped line == one rendered row and the estimate stays exact.
    // A fitting line is kept as-is: a rebuild walks hundreds of lines and a
    // typical diff line already fits, so round-tripping each through
    // wrap_line_spans' per-line Vec is allocation churn.
    let mut wrapped: Vec<Line<'static>> = Vec::new();
    for line in lines {
        if width == 0 || line.width() <= width {
            wrapped.push(line);
        } else {
            wrapped.extend(wrap_line_spans(line, width));
        }
    }
    wrapped
}

/// "CI: failing  ✓3 ✗1 ●2", or None when the head commit has no checks.
/// Gap between the label/value pairs of one metadata line.
const META_GAP: &str = "   ";

/// " text " on a colored background. `None` is the neutral style used for
/// labels and unknown states.
pub(super) fn pill(text: &str, slot: Option<theme::Slot>) -> Span<'static> {
    let style = match slot {
        Some(slot) => Style::new()
            .fg(c(colors::INK))
            .bg(c(slot))
            .add_modifier(Modifier::BOLD),
        None => Style::new().fg(c(colors::FG)).bg(c(colors::FRAME)),
    };
    Span::styled(format!(" {text} "), style)
}

/// GitHub's mergeStateStatus, lowercased, colored by whether it blocks a
/// merge.
fn state_pill(state: &str) -> Span<'static> {
    let slot = match state {
        "CLEAN" | "HAS_HOOKS" => Some(colors::GREEN),
        "UNSTABLE" | "BEHIND" => Some(colors::YELLOW),
        "BLOCKED" | "DIRTY" => Some(colors::RED),
        _ => None,
    };
    let text = if state.is_empty() {
        "unknown".to_string()
    } else {
        state.to_lowercase().replace('_', " ")
    };
    pill(&text, slot)
}

fn review_pill(decision: &str) -> Span<'static> {
    match decision {
        "APPROVED" => pill("approved", Some(colors::GREEN)),
        "CHANGES_REQUESTED" => pill("changes requested", Some(colors::RED)),
        "REVIEW_REQUIRED" => pill("review required", Some(colors::YELLOW)),
        "" => pill("no review", None),
        other => pill(&other.to_lowercase().replace('_', " "), None),
    }
}

/// A "ci failing" pill plus per-outcome counts ("✓3 ✗1 ●2"); empty when the
/// head commit has no checks.
pub(super) fn ci_spans(ci: &gh::CiSummary) -> Vec<Span<'static>> {
    let (word, slot) = if ci.failed > 0 {
        ("ci failing", colors::RED)
    } else if ci.pending > 0 {
        ("ci pending", colors::YELLOW)
    } else if ci.passed > 0 {
        ("ci passing", colors::GREEN)
    } else {
        return Vec::new();
    };
    let mut spans = vec![pill(word, Some(slot))];
    for (n, mark, color) in [
        (ci.passed, "✓", colors::GREEN),
        (ci.failed, "✗", colors::RED),
        (ci.pending, "●", colors::YELLOW),
    ] {
        if n > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!("{mark}{n}"), fg(color)));
        }
    }
    spans
}

const DIFFSTAT_BLOCKS: usize = 5;

/// Splits GitHub's five-block diffstat bar into (green, red) counts by the
/// share of added lines. Any nonzero side keeps at least one block, so a
/// one-line deletion in a large addition is still visible. None when there
/// are no changed lines.
pub(super) fn diffstat_blocks(additions: i64, deletions: i64) -> Option<(usize, usize)> {
    let (add, del) = (additions.max(0) as u64, deletions.max(0) as u64);
    let total = add + del;
    if total == 0 {
        return None;
    }
    let n = DIFFSTAT_BLOCKS as u64;
    let green = ((add * n + total / 2) / total).clamp(u64::from(add > 0), n - u64::from(del > 0));
    Some((green as usize, DIFFSTAT_BLOCKS - green as usize))
}

/// Label column width of the detail header fields ("Reviewers" + gap).
pub(super) const FIELD_W: usize = 11;

/// A "Label      value" row, wrapped to `width` with continuation rows
/// indented to the value column.
fn field_rows(label: &str, value: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let avail = width.saturating_sub(FIELD_W).max(1);
    wrap_line_spans(Line::from(value), avail)
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let head = if i == 0 {
                Span::styled(pad_right(label, FIELD_W), fg(colors::YELLOW))
            } else {
                Span::raw(repeat_space(FIELD_W))
            };
            let mut spans = vec![head];
            spans.extend(row.spans);
            Line::from(spans)
        })
        .collect()
}

/// Gap between grid columns of the detail header fields.
const GRID_GAP: usize = 3;

/// Lays `fields` out row by row as label/value cells, each column as wide
/// as its longest value, in the most columns that fit `width`. The column
/// count is even (or 1), so fields given in pairs share a row. A field
/// wider than half the content gets its own full-width row after the grid,
/// so one long value never stretches a column.
fn field_grid(fields: Vec<(&str, Vec<Span<'static>>)>, width: usize) -> Vec<Line<'static>> {
    let value_w = |v: &[Span<'static>]| v.iter().map(|s| s.width()).sum::<usize>();
    let half = width.saturating_sub(GRID_GAP) / 2;
    let (narrow, wide): (Vec<_>, Vec<_>) = fields
        .into_iter()
        .partition(|(_, v)| FIELD_W + value_w(v) <= half);

    let mut out = Vec::new();
    if !narrow.is_empty() {
        let n = narrow.len();
        let column_widths = |cols: usize| -> Vec<usize> {
            (0..cols.min(n))
                .map(|c| {
                    narrow
                        .iter()
                        .skip(c)
                        .step_by(cols)
                        .map(|(_, v)| value_w(v))
                        .max()
                        .unwrap_or(0)
                })
                .collect()
        };
        let fits = |widths: &[usize]| {
            widths.iter().map(|w| FIELD_W + w).sum::<usize>()
                + GRID_GAP * widths.len().saturating_sub(1)
                <= width
        };
        // Widest even layout that fits; a single column always does.
        let cols = (2..=n.next_multiple_of(2))
            .rev()
            .step_by(2)
            .find(|&cols| fits(&column_widths(cols)))
            .unwrap_or(1);
        let widths = column_widths(cols);
        for row in narrow.chunks(cols) {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, (label, value)) in row.iter().enumerate() {
                if c > 0 {
                    // Pad the previous cell out to its column width.
                    let prev = &row[c - 1].1;
                    spans.push(Span::raw(repeat_space(
                        widths[c - 1] - value_w(prev) + GRID_GAP,
                    )));
                }
                spans.push(Span::styled(pad_right(label, FIELD_W), fg(colors::YELLOW)));
                spans.extend(value.iter().cloned());
            }
            out.push(Line::from(spans));
        }
    }
    for (label, value) in wide {
        out.extend(field_rows(label, value, width));
    }
    out
}

/// A horizontal rule across the content width.
fn rule(width: usize) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width), fg(colors::FRAME)))
}

/// Wraps a styled line to `width`, preserving span styles, so that every
/// produced line is at most `width` columns wide. ratatui's `Paragraph` never
/// re-wraps lines at or below the area width, which keeps the scroll-total
/// estimate in render_detail_section exact.
pub(super) fn wrap_line_spans(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let flat: Vec<(Style, char)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |ch| (s.style, ch)))
        .collect();
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut idx = 0usize;
    while idx < flat.len() {
        let mut used = 0usize;
        let mut last_space: Option<usize> = None;
        let mut end = idx;
        while end < flat.len() {
            let cw = flat[end].1.width().unwrap_or(0);
            if used + cw > width {
                break;
            }
            if flat[end].1 == ' ' && end > idx {
                last_space = Some(end);
            }
            used += cw;
            end += 1;
        }
        let cut = if end >= flat.len() {
            flat.len()
        } else if end > idx {
            match last_space {
                Some(sp) if sp > idx => sp,
                _ => end,
            }
        } else {
            idx + 1
        };
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut run_style: Option<Style> = None;
        let mut run = String::new();
        for (st, ch) in &flat[idx..cut] {
            if run_style != Some(*st) {
                if !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), run_style.unwrap()));
                }
                run_style = Some(*st);
            }
            run.push(*ch);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, run_style.unwrap()));
        }
        out.push(Line::from(spans));
        // A break at a space consumes it, so the next row does not start
        // with a stray blank.
        idx = if cut < flat.len() && flat[cut].1 == ' ' {
            cut + 1
        } else {
            cut
        };
    }
    out
}

fn branch_label(detail: &PullRequestDetail) -> String {
    let head = &detail.head_ref_name;
    let base = &detail.base_ref_name;
    if head.is_empty() && base.is_empty() {
        return String::new();
    }
    if head.is_empty() {
        return base.clone();
    }
    if base.is_empty() {
        return head.clone();
    }
    format!("{head} -> {base}")
}

fn review_decision_style(decision: &str) -> Style {
    match decision {
        "APPROVED" => fg(colors::GREEN),
        "CHANGES_REQUESTED" => fg(colors::RED),
        "REVIEW_REQUIRED" => fg(colors::YELLOW),
        _ => fg(colors::FG),
    }
}

// Normalizes externally sourced text (titles, bodies, labels, diffs) for
// terminal rendering. Tabs become spaces (ratatui silently drops them,
// losing diff indentation), remaining control characters are removed, and
// zero-width / variation-selector code points are stripped: unicode-width,
// ratatui and the terminal disagree on their widths (e.g. "♻️" = base char
// plus U+FE0F), and one disagreement desyncs the physical cursor from
// ratatui's buffer, leaving stale glyphs from earlier frames on screen.
pub(super) fn sanitize_text(s: &str) -> String {
    fn drop_or_expand(ch: char) -> bool {
        matches!(ch,
            '\t'
            | '\u{FE00}'..='\u{FE0F}' // variation selectors
            | '\u{200B}'..='\u{200F}' // zero-width space/joiners, bidi marks
            | '\u{2060}' // word joiner
            | '\u{FEFF}' // BOM
        ) || (ch.is_control() && ch != '\n')
    }
    if !s.chars().any(drop_or_expand) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\t' {
            out.push_str("    ");
        } else if !drop_or_expand(ch) {
            out.push(ch);
        }
    }
    out
}

// Drops ANSI CSI sequences (ESC [ ... final byte). Diffs cached before the
// switch to plain-text `gh pr diff` still carry them, and ratatui renders
// escape bytes as garbage instead of interpreting them.
fn strip_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Highlighted diffs kept for reuse; one entry per recently shown or
/// prefetched PR.
const HIGHLIGHT_CACHE_ENTRIES: usize = 8;

/// `highlight_diff` memoized by diff content. Syntax highlighting is the
/// slow part of a detail rebuild and does not depend on the content width,
/// so a resize reuses it, and background fetches call this first so that
/// selecting a fetched PR finds it ready. Colors resolve against the active
/// theme, which is fixed for the process lifetime.
pub fn highlighted_diff(diff: &str) -> Arc<Vec<Line<'static>>> {
    use std::hash::{Hash, Hasher};
    type Memo =
        std::sync::Mutex<std::collections::VecDeque<((u64, usize), Arc<Vec<Line<'static>>>)>>;
    static MEMO: std::sync::OnceLock<Memo> = std::sync::OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);

    // ponytail: a 64-bit hash plus length stands in for the diff itself;
    // keeping the full text as the key would double the memory per entry.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    diff.hash(&mut h);
    let key = (h.finish(), diff.len());
    if let Some((_, lines)) = memo.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return lines.clone();
    }
    // Computed outside the lock: a slow diff must not block other lookups.
    let lines = Arc::new(highlight_diff(diff));
    let mut memo = memo.lock().unwrap();
    if !memo.iter().any(|(k, _)| *k == key) {
        if memo.len() >= HIGHLIGHT_CACHE_ENTRIES {
            memo.pop_front();
        }
        memo.push_back((key, lines.clone()));
    }
    lines
}

/// Uncached diff styling; callers go through `highlighted_diff`.
pub fn highlight_diff(diff: &str) -> Vec<Line<'static>> {
    if diff.is_empty() {
        return Vec::new();
    }
    let added_bg = Style::new().bg(mix(colors::BAR_BG, colors::GREEN, DIFF_TINT));
    let removed_bg = Style::new().bg(mix(colors::BAR_BG, colors::RED, DIFF_TINT));
    let mut hl = crate::syntax::Highlighter::default();
    let mut budget = HIGHLIGHT_MAX_LINES;
    diff.lines()
        .map(|line| sanitize_text(&strip_ansi(line)))
        .map(|line| {
            let style = if line.starts_with("diff --git") {
                if let Some((_, path)) = line.rsplit_once(" b/") {
                    hl.set_file(path);
                }
                Style::new()
                    .fg(c(colors::CYAN))
                    .add_modifier(Modifier::BOLD)
            } else if line.starts_with("index ")
                || line.starts_with("new file mode")
                || line.starts_with("deleted file mode")
                || line.starts_with("old mode")
                || line.starts_with("new mode")
                || line.starts_with("rename from ")
                || line.starts_with("rename to ")
                || line.starts_with("similarity index")
                || line.starts_with("Binary files")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                fg(colors::MUTED)
            } else if line.starts_with("@@") {
                hl.reset();
                fg(colors::MAGENTA)
            } else {
                let (marker, tint, plain) = match line.chars().next() {
                    Some('+') => ("+", added_bg, fg(colors::GREEN)),
                    Some('-') => ("-", removed_bg, fg(colors::RED)),
                    Some(' ') => (" ", Style::new(), Style::new()),
                    _ => return Line::from(Span::raw(line)),
                };
                let code = &line[1..];
                let runs = (budget > 0).then(|| hl.highlight(code)).flatten();
                budget = budget.saturating_sub(1);
                let Some(runs) = runs else {
                    return Line::from(Span::styled(line, plain));
                };
                let mut spans = vec![Span::styled(marker.to_string(), plain.patch(tint))];
                spans.extend(runs.into_iter().map(|(text, slot)| {
                    Span::styled(text, fg(slot.unwrap_or(colors::FG)).patch(tint))
                }));
                return Line::from(spans);
            };
            Line::from(Span::styled(line, style))
        })
        .collect()
}

/// Code lines per diff that get syntax colors; the rest stay plain.
// ponytail: syntect with the pure-Rust regex engine parses ~30k lines/s, and
// the detail renders on the UI thread, so the cap bounds a cold render to
// ~70 ms. Highlighting off-thread (or the C onig engine, ~3x faster) would
// lift it.
const HIGHLIGHT_MAX_LINES: usize = 2000;

/// Share of the accent color in the added / removed line background.
pub(super) const DIFF_TINT: f32 = 0.2;

/// Blends palette slot `a` toward `b` by `t` (0 = a, 1 = b).
pub(super) fn mix(a: theme::Slot, b: theme::Slot, t: f32) -> Color {
    match (c(a), c(b)) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let ch =
                |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
            Color::Rgb(ch(r1, r2), ch(g1, g2), ch(b1, b2))
        }
        (_, other) => other,
    }
}

fn wrap_line_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for raw in text.split('\n') {
        if raw.width() <= width {
            out.push(raw.to_string());
            continue;
        }
        let mut cur = String::new();
        let mut used = 0;
        for word in raw.split(' ') {
            let ww = word.width();
            if ww > width {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut chunk = String::new();
                let mut chunk_used = 0;
                for ch in word.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if chunk_used + cw > width {
                        out.push(std::mem::take(&mut chunk));
                        chunk_used = 0;
                    }
                    chunk.push(ch);
                    chunk_used += cw;
                }
                cur = chunk;
                used = chunk_used;
                continue;
            }
            if used == 0 {
                cur.push_str(word);
                used = ww;
            } else if used + 1 + ww <= width {
                cur.push(' ');
                cur.push_str(word);
                used += 1 + ww;
            } else {
                out.push(std::mem::take(&mut cur));
                cur.push_str(word);
                used = ww;
            }
        }
        out.push(cur);
    }
    out
}

pub fn render_markdown(body: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code = false;
    let mut paragraph: Vec<String> = Vec::new();

    // Structural context for lazy continuation lines (a non-blank, unmarked
    // line following a list item or blockquote continues it, per markdown).
    enum OpenBlock {
        Item { prefix_w: usize, avail: usize },
        Quote { marker: String },
    }
    let mut open: Option<OpenBlock> = None;

    let flush_paragraph = |lines: &mut Vec<Line<'static>>, para: &mut Vec<String>| {
        if para.is_empty() {
            return;
        }
        let text = para.join(" ");
        for l in wrap_line_to_width(&text, width) {
            lines.push(Line::from(Span::styled(l, fg(colors::FG))));
        }
        para.clear();
    };

    // Leading whitespace width of the raw line (never negative when trailing
    // whitespace is already trimmed away).
    let leading_width = |raw: &str, t: &str| -> usize { raw.width().saturating_sub(t.width()) };

    for raw in body.lines() {
        let trimmed = raw.trim_end();
        if trimmed.trim_start().starts_with("```") || trimmed.trim_start().starts_with("~~~") {
            flush_paragraph(&mut lines, &mut paragraph);
            open = None;
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(Line::from(Span::styled(raw.to_string(), fg(colors::CYAN))));
            continue;
        }
        let t = trimmed.trim_start();
        let indent = leading_width(raw, t) / 2;
        if t.is_empty() {
            flush_paragraph(&mut lines, &mut paragraph);
            open = None;
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let level = rest.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&level) {
                flush_paragraph(&mut lines, &mut paragraph);
                open = None;
                let heading = rest[level..].trim();
                let style = Style::new()
                    .fg(c(colors::BLUE))
                    .add_modifier(Modifier::BOLD);
                for l in wrap_line_to_width(heading, width) {
                    lines.push(Line::from(Span::styled(l, style)));
                }
                continue;
            }
        }
        if t == "---" || t == "***" || t == "___" {
            flush_paragraph(&mut lines, &mut paragraph);
            open = None;
            lines.push(Line::from(Span::styled(
                "─".repeat(width.min(80)),
                fg(colors::MUTED),
            )));
            continue;
        }

        // Ordered list items: "1. ", "12) ", ...
        let item = |lines: &mut Vec<Line<'static>>,
                    marker: &str,
                    content: &str,
                    prefix_w: usize,
                    avail: usize|
         -> () {
            let mut first = true;
            for l in wrap_line_to_width(content, avail) {
                let prefix = if first {
                    format!("{}{marker} ", repeat_space(indent * 2))
                } else {
                    repeat_space(prefix_w)
                };
                lines.push(Line::from(vec![
                    Span::styled(prefix, fg(colors::CYAN)),
                    Span::styled(l, fg(colors::FG)),
                ]));
                first = false;
            }
        };

        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        let ordered = if (1..=4).contains(&digits)
            && t[digits..].starts_with(['.', ')'])
            && t[digits + 1..].starts_with(' ')
        {
            Some(&t[..digits + 1])
        } else {
            None
        };
        if let Some(marker) = ordered {
            flush_paragraph(&mut lines, &mut paragraph);
            let content = t[digits + 2..].to_string();
            let prefix_w = indent * 2 + marker.width() + 1;
            let avail = width.saturating_sub(prefix_w).max(10);
            item(&mut lines, marker, &content, prefix_w, avail);
            open = Some(OpenBlock::Item { prefix_w, avail });
            continue;
        }

        if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
            flush_paragraph(&mut lines, &mut paragraph);
            let content = t[2..].to_string();
            let prefix_w = indent * 2 + 2;
            let avail = width.saturating_sub(prefix_w).max(10);
            item(&mut lines, "•", &content, prefix_w, avail);
            open = Some(OpenBlock::Item { prefix_w, avail });
            continue;
        }

        // Blockquotes; repeated "> " markers nest.
        if t.starts_with('>') {
            flush_paragraph(&mut lines, &mut paragraph);
            let mut level = 0usize;
            let mut rest = t;
            while let Some(r) = rest.strip_prefix('>') {
                level += 1;
                rest = r.strip_prefix(' ').unwrap_or(r);
            }
            let marker = "│ ".repeat(level);
            let prefix_w = marker.width();
            let avail = width.saturating_sub(prefix_w).max(10);
            for l in wrap_line_to_width(rest, avail) {
                lines.push(Line::from(vec![
                    Span::styled(marker.clone(), fg(colors::MUTED)),
                    Span::styled(l, fg(colors::MUTED)),
                ]));
            }
            open = Some(OpenBlock::Quote {
                marker: marker.clone(),
            });
            continue;
        }

        // Lazy continuation of an open list item or blockquote.
        match &open {
            Some(OpenBlock::Item { prefix_w, avail }) => {
                let (prefix_w, avail) = (*prefix_w, *avail);
                for l in wrap_line_to_width(t, avail) {
                    lines.push(Line::from(vec![
                        Span::styled(repeat_space(prefix_w), Style::new()),
                        Span::styled(l, fg(colors::FG)),
                    ]));
                }
                continue;
            }
            Some(OpenBlock::Quote { marker }) => {
                let avail = width.saturating_sub(marker.width()).max(10);
                for l in wrap_line_to_width(t, avail) {
                    lines.push(Line::from(vec![
                        Span::styled(marker.clone(), fg(colors::MUTED)),
                        Span::styled(l, fg(colors::MUTED)),
                    ]));
                }
                continue;
            }
            None => {}
        }

        paragraph.push(t.to_string());
    }
    if in_code {
        // Unterminated fence; nothing more to do.
    }
    flush_paragraph(&mut lines, &mut paragraph);
    lines
}

pub fn render(m: &mut Model, f: &mut Frame<'_>) {
    let area = f.area();
    let w = area.width as usize;
    let h = area.height;

    let search_h = if m.search_visible() { 1u16 } else { 0 };
    let list_h = m.compute_list_section_height() as u16;
    let vp_h = m.detail_viewport_height();
    let detail_h = vp_h + 2;

    let mut y: u16 = 0;
    if search_h == 1 {
        let rect = Rect::new(0, 0, area.width, 1);
        f.render_widget(render_search_line(m), rect);
        y += 1;
    }

    // list section: tab bar + framed rows
    let list_rect = Rect::new(0, y, area.width, list_h.min(h.saturating_sub(y + 1)));
    if list_rect.height > 0 {
        let tab_rect = Rect::new(0, list_rect.y, area.width, 1);
        f.render_widget(render_tab_bar(m), tab_rect);
        if list_rect.height > 1 {
            let frame_rect = Rect::new(0, list_rect.y + 1, area.width, list_rect.height - 1);
            render_list_frame(m, f, frame_rect);
        }
    }
    y += list_rect.height;

    // detail section
    let footer_y = h.saturating_sub(1);
    if y < footer_y {
        let detail_rect_h = detail_h.min(footer_y - y);
        // With fewer than 3 rows there is no room for content; skip instead of
        // drawing border-only noise.
        if detail_rect_h > 2 {
            let rect = Rect::new(0, y, area.width, detail_rect_h);
            render_detail_section(m, f, rect, vp_h);
        }
    }

    // footer
    let footer_rect = Rect::new(0, footer_y, area.width, 1);
    f.render_widget(render_footer(m, w), footer_rect);

    // popups
    if m.update_notice.is_some() {
        render_update_notice_popup(m, f, area);
    }
    if m.pending_approve.is_some() {
        render_approve_popup(m, f, area);
    }
    if m.comment_editor.is_some() {
        render_comment_popup(m, f, area);
    }
}

fn render_search_line(m: &Model) -> Paragraph<'static> {
    let line = if m.search_active {
        let value = &m.search_value;
        let chars: Vec<char> = value.chars().collect();
        let cursor_at_end = m.search_cursor >= chars.len();
        let mut spans = vec![Span::styled("/ ".to_string(), fg(colors::MUTED))];
        let before: String = chars[..m.search_cursor.min(chars.len())].iter().collect();
        if !before.is_empty() {
            spans.push(Span::raw(before));
        }
        if cursor_at_end {
            spans.push(Span::styled("█".to_string(), fg(colors::CYAN)));
        } else {
            let at = chars[m.search_cursor].to_string();
            spans.push(Span::styled(
                at,
                Style::new().fg(c(colors::INK)).bg(c(colors::CYAN)),
            ));
            let after: String = chars[m.search_cursor + 1..].iter().collect();
            if !after.is_empty() {
                spans.push(Span::raw(after));
            }
        }
        Line::from(spans)
    } else {
        Line::from(Span::styled(
            format!("filter: {}  (/ to edit, esc to clear)", m.search_value),
            fg(colors::MUTED),
        ))
    };
    Paragraph::new(line)
}

pub(super) fn render_tab_bar(m: &Model) -> Tabs<'static> {
    let counts = m.tab_counts();
    // Merged / Closed: no count until first visit, "…" while fetching.
    let count = |tab: usize| -> Option<String> {
        if tab == TAB_MERGED || tab == TAB_CLOSED {
            if m.history_loading[tab] {
                return Some("…".to_string());
            }
            if !m.history_loaded[tab] {
                return None;
            }
        }
        Some(counts[tab].to_string())
    };
    let titles = ["Awaiting Review", "Reviewed", "Merged", "Closed"]
        .into_iter()
        .enumerate()
        .map(|(tab, name)| {
            let mut spans = vec![Span::raw(format!(" {name} "))];
            if let Some(n) = count(tab) {
                spans.push(Span::styled(
                    format!("{n} "),
                    fg(colors::CYAN).add_modifier(Modifier::BOLD),
                ));
            }
            Line::from(spans)
        });
    // Titles carry their own padding so the active one reads as a pill.
    Tabs::new(titles)
        .select(m.active_tab)
        .style(fg(colors::MUTED))
        .highlight_style(
            Style::new()
                .fg(c(colors::INK))
                .bg(c(colors::BLUE))
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("│", fg(colors::FRAME)))
        .padding("", "")
}

const COL_SPACING: usize = 2;

/// Title column width: what the fixed columns, the one-column highlight
/// symbol, the six column gaps, and the two border columns leave of `box_w`.
fn list_title_width(box_w: usize) -> usize {
    let fixed = 2
        + 1
        + COL_MARK_W
        + COL_TYPE_W
        + COL_REPO_W
        + COL_NUM_W
        + COL_AUTHOR_W
        + COL_APPROVE_W
        + COL_SPACING * 6;
    std::cmp::max(10, box_w.saturating_sub(fixed))
}

fn render_list_frame(m: &Model, f: &mut Frame<'_>, rect: Rect) {
    let title_w = list_title_width(m.frame_width());
    let items = m.visible_pr_indices();
    // Every other row gets a faint band, halfway between the background and
    // the header bar, so it stays distinct from the selection color in every
    // theme (solarized's selection equals its bar color).
    let zebra = Style::new().bg(mix(colors::INK, colors::BAR_BG, 0.5));
    let rows: Vec<Row<'static>> = items
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let row = render_pr_row(m, idx, &m.prs[idx], title_w);
            if i % 2 == 1 {
                row.style(zebra)
            } else {
                row
            }
        })
        .collect();
    let widths = [
        COL_MARK_W,
        COL_TYPE_W,
        COL_REPO_W,
        COL_NUM_W,
        title_w,
        COL_AUTHOR_W,
        COL_APPROVE_W,
    ]
    .map(|w| Constraint::Length(w as u16));
    let header = Row::new(["", "Type", "Repository", "#", "Title", "Author", "Approve"]).style(
        Style::new()
            .fg(c(colors::MUTED))
            .bg(c(colors::BAR_BG))
            .add_modifier(Modifier::BOLD),
    );
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(fg(colors::FRAME));
    let inner = block.inner(rect);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(COL_SPACING as u16)
        .highlight_symbol(Span::styled("▌", fg(colors::CYAN)))
        .highlight_spacing(HighlightSpacing::Always)
        .row_highlight_style(Style::new().bg(c(colors::SELECTED)))
        .block(block);
    let mut state = TableState::new().with_selected(items.iter().position(|&i| i == m.cursor));
    f.render_stateful_widget(table, rect, &mut state);

    if items.is_empty() && inner.height > 1 {
        let msg = Span::styled(format!(" {}", m.empty_list_message()), fg(colors::MUTED));
        let area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
        f.render_widget(Paragraph::new(Line::from(msg)), area);
    }
}

fn render_pr_row(m: &Model, idx: usize, pr: &PullRequest, title_w: usize) -> Row<'static> {
    let selected = idx == m.cursor;
    // The table paints the selected row's background; plain or muted columns
    // switch to a foreground that stays readable on it.
    let pick = |normal: theme::Slot, sel: theme::Slot| fg(if selected { sel } else { normal });

    let mark = if m.marked_prs.contains(&pr.url) {
        Span::styled("[!]", fg(colors::YELLOW).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("")
    };
    let (type_label, type_color) = if pr.request.contains("@me") {
        ("[me]", colors::CYAN)
    } else {
        ("[team]", colors::ORANGE)
    };
    let title_style = if selected {
        fg(colors::SELECTED_FG).add_modifier(Modifier::BOLD)
    } else {
        fg(colors::FG)
    };
    let approve = m.approve_label(pr);
    let approve_style = match approve {
        "approved" => fg(colors::GREEN),
        "changes" => fg(colors::RED),
        _ => pick(colors::MUTED, colors::YELLOW),
    };

    Row::new(vec![
        Cell::from(mark),
        Cell::from(Span::styled(type_label, fg(type_color))),
        Cell::from(Span::styled(
            pad_right(&pr.repository, COL_REPO_W),
            pick(colors::CYAN, colors::SELECTED_FG),
        )),
        Cell::from(Span::styled(
            format!("#{}", pr.number),
            pick(colors::BLUE, colors::YELLOW),
        )),
        Cell::from(Span::styled(
            pad_right(&sanitize_text(&pr.title), title_w),
            title_style,
        )),
        Cell::from(Span::styled(
            pad_right(&format!("@{}", pr.author), COL_AUTHOR_W),
            pick(colors::MAGENTA, colors::SELECTED_FG),
        )),
        Cell::from(Span::styled(approve, approve_style)),
    ])
}

pub(super) fn render_detail_section(m: &mut Model, f: &mut Frame<'_>, rect: Rect, vp_h: u16) {
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(fg(colors::FRAME))
        .padding(Padding::left(1));

    let content_width = rect.width.saturating_sub(3).max(1) as usize;

    let detail_ready = !m.detail_loading && m.detail_err.is_empty();
    if detail_ready {
        if let Some(detail) = m.current_detail.as_ref() {
            // Rebuild the rendered detail only when its inputs changed; the
            // per-frame cost is then just cloning the cached lines (the slow
            // markdown parse / ANSI strip / style build runs once per change).
            let fresh = m
                .detail_view_cache
                .as_ref()
                .is_some_and(|c| c.content_width == content_width);
            if !fresh {
                let lines = render_diff_content(detail, &m.current_diff, content_width);
                let total: usize = lines
                    .iter()
                    .map(|l| {
                        let lw = l.width();
                        if lw == 0 {
                            1
                        } else {
                            lw.div_ceil(content_width)
                        }
                    })
                    .sum();
                m.detail_view_cache = Some(DetailViewCache {
                    content_width,
                    total: total.min(u16::MAX as usize) as u16,
                    lines,
                });
            }
        }
    }

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if !detail_ready {
        let lines: Vec<Line<'static>> = if m.detail_loading {
            vec![Line::from(vec![
                Span::styled(format!("{} ", m.spinner()), fg(colors::CYAN)),
                Span::styled("loading detail...".to_string(), fg(colors::MUTED)),
            ])]
        } else if !m.detail_err.is_empty() {
            vec![Line::from(Span::styled(
                m.detail_err.clone(),
                fg(colors::RED),
            ))]
        } else {
            vec![Line::from(Span::styled(
                "No detail loaded.".to_string(),
                fg(colors::MUTED),
            ))]
        };
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let total = m.detail_view_cache.as_ref().map(|c| c.total);
    let Some(cache) = m.detail_view_cache.as_ref() else {
        f.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(
                "No detail loaded.".to_string(),
                fg(colors::MUTED),
            ))]),
            inner,
        );
        return;
    };
    if let Some(total) = total {
        m.detail_total = total;
        let max_scroll = total.saturating_sub(vp_h);
        if m.detail_scroll > max_scroll {
            m.detail_scroll = max_scroll;
        }
    }
    // Every cached line is pre-wrapped to the content width, so one cached
    // line renders as exactly one terminal row — the same invariant the
    // scroll-total estimate relies on. That makes the visible part of the
    // view a plain slice of the cache, and the per-frame cost proportional
    // to the viewport instead of the document: a full-text Paragraph with
    // .scroll() lays out every line of the diff on every frame and clones
    // the whole cache into the bargain.
    let line_count = cache.lines.len();
    let start = (m.detail_scroll as usize).min(line_count);
    let end = (start + vp_h as usize).min(line_count);
    let text: Vec<Line<'_>> = cache.lines[start..end]
        .iter()
        .map(|l| Line {
            style: l.style,
            alignment: l.alignment,
            spans: l
                .spans
                .iter()
                .map(|s| Span {
                    style: s.style,
                    content: Cow::Borrowed(s.content.as_ref()),
                })
                .collect(),
        })
        .collect();
    let para = Paragraph::new(text).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_footer(m: &Model, w: usize) -> Paragraph<'static> {
    let status = render_footer_status(m);
    let help = render_help();
    let status_w: usize = status.iter().map(|s| s.content.width()).sum();
    let help_w: usize = help.iter().map(|s| s.content.width()).sum();

    let mut spans: Vec<Span<'static>> = Vec::new();
    if status_w + 1 + help_w > w {
        spans.extend(truncate_spans(status, w));
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        if used < w {
            spans.push(Span::styled(
                repeat_space(w - used),
                Style::new().bg(c(colors::BAR_BG)),
            ));
        }
    } else {
        spans.extend(status);
        let gap = std::cmp::max(1, w - status_w - help_w);
        spans.push(Span::styled(
            repeat_space(gap),
            Style::new().bg(c(colors::BAR_BG)),
        ));
        spans.extend(help);
        spans.push(Span::styled(
            " ".to_string(),
            Style::new().bg(c(colors::BAR_BG)),
        ));
    }
    Paragraph::new(Line::from(spans))
}

fn truncate_spans(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0;
    for s in spans {
        let sw = s.content.width();
        if used + sw <= w {
            used += sw;
            out.push(s);
        } else {
            let budget = w - used;
            let text: String = s
                .content
                .chars()
                .scan(0usize, |acc, ch| {
                    let cw = ch.width().unwrap_or(0);
                    if *acc + cw > budget {
                        None
                    } else {
                        *acc += cw;
                        Some(ch)
                    }
                })
                .collect();
            out.push(Span::styled(text, s.style));
            break;
        }
    }
    out
}

fn render_footer_status(m: &Model) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        " ".to_string(),
        Style::new().bg(c(colors::BAR_BG)),
    )];
    if m.loading || m.detail_loading || m.any_history_loading() {
        spans.push(Span::styled(
            format!("{} ", m.spinner()),
            Style::new().fg(c(colors::CYAN)).bg(c(colors::BAR_BG)),
        ));
    }
    let style = if !m.err.is_empty() {
        Style::new().fg(c(colors::RED)).bg(c(colors::BAR_BG))
    } else {
        Style::new().fg(c(colors::FG)).bg(c(colors::BAR_BG))
    };
    spans.push(Span::styled(m.status.clone(), style));
    spans
}

fn render_help() -> Vec<Span<'static>> {
    let bindings: [(&str, &str); 10] = [
        ("h/l", "tabs"),
        ("ctrl+n/p", "list"),
        ("j/k", "scroll"),
        ("pgup/dn", "page"),
        ("/", "filter"),
        ("y", "copy"),
        ("a", "approve"),
        ("c", "comment"),
        ("r", "refresh"),
        ("q", "quit"),
    ];
    let mut spans = Vec::new();
    for (i, (k, d)) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ".to_string(), fg(colors::FRAME)));
        }
        spans.push(Span::styled(
            k.to_string(),
            Style::new()
                .fg(c(colors::YELLOW))
                .bg(c(colors::BAR_BG))
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {d}"),
            Style::new().fg(c(colors::MUTED)).bg(c(colors::BAR_BG)),
        ));
    }
    spans
}

fn popup_rect(lines_len: usize, max_width: usize, pad_x: usize, area: Rect, corner: bool) -> Rect {
    let pw = (max_width + pad_x * 2 + 2).min(area.width as usize);
    let ph = (lines_len + 2).min(area.height as usize);
    let (x, y) = if corner {
        (
            area.width.saturating_sub(pw as u16 + 1),
            area.height.saturating_sub(ph as u16 + 1),
        )
    } else {
        (
            area.width.saturating_sub(pw as u16) / 2,
            area.height.saturating_sub(ph as u16) / 2,
        )
    };
    Rect::new(x, y, pw as u16, ph as u16)
}

fn render_update_notice_popup(m: &Model, f: &mut Frame<'_>, area: Rect) {
    let count = match &m.update_notice {
        Some(n) => format!("{} review request(s)", n.count),
        None => "review requests changed".to_string(),
    };
    let lines = vec![
        Line::from(Span::styled(
            "Review updated".to_string(),
            Style::new()
                .fg(c(colors::BLUE))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(count, fg(colors::MUTED))),
    ];
    let max_width = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let rect = popup_rect(lines.len(), max_width, 1, area, true);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(fg(colors::YELLOW))
        .padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_approve_popup(m: &Model, f: &mut Frame<'_>, area: Rect) {
    let pr = match &m.pending_approve {
        Some(p) => p.clone(),
        None => return,
    };
    let lines = vec![
        Line::from(Span::styled(
            "Approve pull request?".to_string(),
            Style::new()
                .fg(c(colors::BLUE))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(pr_label(&pr), fg(colors::MUTED))),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "y Yes".to_string(),
                Style::new()
                    .fg(c(colors::GREEN))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                "c Cancel".to_string(),
                Style::new().fg(c(colors::RED)).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    let max_width = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let rect = popup_rect(lines.len() + 2, max_width, 2, area, false);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(fg(colors::GREEN))
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_comment_popup(m: &Model, f: &mut Frame<'_>, area: Rect) {
    let editor = match &m.comment_editor {
        Some(e) => e,
        None => return,
    };
    // Box: up to 80 columns wide, up to 16 rows tall; borders + 1 col padding.
    let pw = (area.width.saturating_sub(4)).min(80);
    let ph = (area.height.saturating_sub(2)).min(16);
    if pw < 10 || ph < 6 {
        return;
    }
    let rect = Rect::new(
        area.x + (area.width - pw) / 2,
        area.y + (area.height - ph) / 2,
        pw,
        ph,
    );
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(fg(colors::BLUE))
        .title(Span::styled(
            format!(" Comment on {} ", pr_label(&editor.pr)),
            Style::new()
                .fg(c(colors::BLUE))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(vec![
            Span::styled(" ctrl+enter".to_string(), fg(colors::YELLOW)),
            Span::styled("/".to_string(), fg(colors::MUTED)),
            Span::styled("ctrl+s".to_string(), fg(colors::YELLOW)),
            Span::styled(" send · ".to_string(), fg(colors::MUTED)),
            Span::styled("enter".to_string(), fg(colors::YELLOW)),
            Span::styled(" newline · ".to_string(), fg(colors::MUTED)),
            Span::styled("esc".to_string(), fg(colors::YELLOW)),
            Span::styled(" cancel ".to_string(), fg(colors::MUTED)),
        ]))
        .padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (rows, (cur_row, cur_col)) = editor.area.layout(inner.width as usize);
    let vp = inner.height as usize;
    let offset = (cur_row + 1).saturating_sub(vp);
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .skip(offset)
        .take(vp)
        .map(|r| Line::from(Span::styled(r, fg(colors::FG))))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    // Place the real terminal cursor so the IME preedit appears in place.
    f.set_cursor_position((
        inner.x + cur_col as u16,
        inner.y + (cur_row - offset) as u16,
    ));
}
