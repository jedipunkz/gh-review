use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[allow(unused_imports)]
use crate::cache::CacheEntry;
use crate::cache::DetailCache;
use crate::gh::{self, HistoryState, PullRequest, PullRequestDetail};
use crate::prefetch::{neighbor_prs, top_n, Prefetcher};

pub const MAX_LIST_ITEMS: usize = 10;
pub const TAB_AWAITING: usize = 0;
pub const TAB_REVIEWED: usize = 1;
pub const TAB_MERGED: usize = 2;
pub const TAB_CLOSED: usize = 3;
pub const TAB_COUNT: usize = 4;

pub const POPUP_DISMISS_DELAY: std::time::Duration = std::time::Duration::from_secs(6);
pub const DETAIL_LOAD_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(80);
pub const UPDATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

const COL_MARK_W: usize = 3;
const COL_TYPE_W: usize = 6;
const COL_REPO_W: usize = 28;
const COL_NUM_W: usize = 6;
const COL_AUTHOR_W: usize = 15;
const COL_APPROVE_W: usize = 11;

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn color(hex: &str) -> Color {
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    Color::Rgb(r, g, b)
}

pub mod colors {
    pub const FG: &str = "c0caf5";
    pub const MUTED: &str = "565f89";
    pub const BLUE: &str = "7aa2f7";
    pub const CYAN: &str = "7dcfff";
    pub const GREEN: &str = "9ece6a";
    pub const MAGENTA: &str = "bb9af7";
    pub const ORANGE: &str = "ff9e64";
    pub const RED: &str = "f7768e";
    pub const YELLOW: &str = "e0af68";
    pub const SELECTED: &str = "3d59a1";
    pub const SELECTED_FG: &str = "ffffff";
    pub const BAR_BG: &str = "1f2335";
    pub const INK: &str = "1a1b26";
    pub const FRAME: &str = "414868";
}

fn c(name: &str) -> Color {
    color(name)
}

#[derive(Debug, Clone)]
pub struct UpdateNotice {
    pub count: usize,
    pub id: u64,
}

pub struct DetailViewCache {
    pub content_width: usize,
    pub total: u16,
    pub lines: Vec<Line<'static>>,
    pub detail: PullRequestDetail,
    pub diff: String,
}

pub struct MatchesCache {
    pub search_value: String,
    pub prs_version: u64,
    pub approved_generation: u64,
    pub indices: Vec<usize>,
    pub counts: [usize; TAB_COUNT],
}

#[derive(Debug, Clone)]
pub enum Effect {
    LoadPRs,
    LoadHistory { state: HistoryState },
    CheckUpdates { prev_sig: String, prev_count: usize },
    LoadDiff { pr: PullRequest },
    Debounce { seq: u64, url: String },
    Approve { pr: PullRequest },
    CopyURL { pr: PullRequest },
    PlaySound,
    DismissPopup { id: u64 },
    PrefetchHint { prs: Vec<PullRequest> },
}

pub enum Msg {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    PRList(Result<Vec<PullRequest>, String>),
    HistoryList {
        state: HistoryState,
        result: Result<Vec<PullRequest>, String>,
    },
    DiffDone(Box<DiffDoneMsg>),
    ApproveDone {
        pr: PullRequest,
        err: Option<String>,
    },
    CopyDone {
        pr: PullRequest,
        err: Option<String>,
    },
    UpdateCheckTick,
    UpdateCheck {
        prev_sig: String,
        prev_count: usize,
        result: Result<Vec<PullRequest>, String>,
    },
    PopupDismiss {
        id: u64,
    },
    DebounceFire {
        seq: u64,
        url: String,
    },
    SpinnerTick,
}

pub struct DiffDoneMsg {
    pub pr: PullRequest,
    pub result: Result<(PullRequestDetail, String), String>,
}

pub struct Model {
    pub loading: bool,
    pub status: String,
    pub err: String,
    pub prs: Vec<PullRequest>,
    pub pr_signature: String,
    pub pr_list_loaded: bool,
    pub cursor: usize,
    pub list_offset: usize,
    pub current_detail: Option<PullRequestDetail>,
    pub current_diff: String,
    pub detail_loading: bool,
    pub detail_err: String,
    pub loading_for_url: Option<String>,
    pub detail_scroll: u16,
    pub detail_total: u16,
    pub width: u16,
    pub height: u16,
    pub approved: HashSet<String>,
    pub pending_approve: Option<PullRequest>,
    pub update_notice: Option<UpdateNotice>,
    pub marked_prs: HashSet<String>,
    pub popup_seq: u64,
    pub cache: Arc<DetailCache>,
    pub prefetcher: Arc<Prefetcher>,
    pub debounce_seq: u64,
    pub search_active: bool,
    pub search_value: String,
    pub search_cursor: usize,
    pub spinner_frame: usize,
    pub active_tab: usize,
    /// Per-tab fetch state of the Merged / Closed tabs, indexed by TAB_*.
    /// These tabs are fetched on first visit, and again on visit after `r`.
    pub history_loading: [bool; TAB_COUNT],
    pub history_loaded: [bool; TAB_COUNT],
    pub history_stale: [bool; TAB_COUNT],
    pub quit: bool,
    pub prs_version: u64,
    pub approved_generation: u64,
    pub detail_view_cache: Option<DetailViewCache>,
    pub matches_cache: RefCell<Option<MatchesCache>>,
}

impl Model {
    pub fn new(cache: Arc<DetailCache>, prefetcher: Arc<Prefetcher>) -> Self {
        Model {
            loading: true,
            status: "loading review requests...".to_string(),
            err: String::new(),
            prs: Vec::new(),
            pr_signature: String::new(),
            pr_list_loaded: false,
            cursor: 0,
            list_offset: 0,
            current_detail: None,
            current_diff: String::new(),
            detail_loading: false,
            detail_err: String::new(),
            loading_for_url: None,
            detail_scroll: 0,
            detail_total: 0,
            width: 0,
            height: 0,
            approved: HashSet::new(),
            pending_approve: None,
            update_notice: None,
            marked_prs: HashSet::new(),
            popup_seq: 0,
            cache,
            prefetcher,
            debounce_seq: 0,
            search_active: false,
            search_value: String::new(),
            search_cursor: 0,
            spinner_frame: 0,
            active_tab: TAB_AWAITING,
            history_loading: [false; TAB_COUNT],
            history_loaded: [false; TAB_COUNT],
            history_stale: [false; TAB_COUNT],
            quit: false,
            prs_version: 0,
            approved_generation: 0,
            detail_view_cache: None,
            matches_cache: RefCell::new(None),
        }
    }

    pub fn any_history_loading(&self) -> bool {
        self.history_loading.iter().any(|&b| b)
    }

    /// Starts fetching the active tab when it is a Merged / Closed tab that
    /// has not been fetched yet (or was marked stale by `r`).
    fn load_active_history_tab(&mut self) -> Option<Effect> {
        let tab = self.active_tab;
        let state = history_state_of_tab(tab)?;
        if self.history_loading[tab] || (self.history_loaded[tab] && !self.history_stale[tab]) {
            return None;
        }
        self.history_loading[tab] = true;
        Some(Effect::LoadHistory { state })
    }

    pub fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    pub fn frame_width(&self) -> usize {
        if self.width > 0 {
            self.width as usize
        } else {
            40
        }
    }

    pub fn search_query(&self) -> String {
        self.search_value.trim().to_lowercase()
    }

    pub fn is_reviewed(&self, pr: &PullRequest) -> bool {
        self.approved.contains(&pr.url) || pr.review_decision == "APPROVED"
    }

    pub fn tab_of(&self, pr: &PullRequest) -> usize {
        match pr.state.as_str() {
            "MERGED" => TAB_MERGED,
            "CLOSED" => TAB_CLOSED,
            _ if self.is_reviewed(pr) => TAB_REVIEWED,
            _ => TAB_AWAITING,
        }
    }

    pub fn pr_matches_tab(&self, pr: &PullRequest) -> bool {
        self.tab_of(pr) == self.active_tab
    }

    /// Open PRs (the review-request list) in list order.
    pub fn open_prs(&self) -> Vec<PullRequest> {
        self.prs
            .iter()
            .filter(|p| !is_history(p))
            .cloned()
            .collect()
    }

    /// Merged / closed PRs in list order.
    pub fn history_prs(&self) -> Vec<PullRequest> {
        self.prs.iter().filter(|p| is_history(p)).cloned().collect()
    }

    pub fn open_count(&self) -> usize {
        self.prs.iter().filter(|p| !is_history(p)).count()
    }

    pub fn matching_indices(&self) -> Vec<usize> {
        let (indices, _) = self.match_summary();
        indices
            .into_iter()
            .filter(|&i| self.pr_matches_tab(&self.prs[i]))
            .collect()
    }

    /// Query-matching PR counts per tab, indexed by TAB_*.
    pub fn tab_counts(&self) -> [usize; TAB_COUNT] {
        let (_, counts) = self.match_summary();
        counts
    }

    /// Query-matching PR indices (all tabs) plus per-tab counts.
    /// Memoized; invalidated by search value, PR list, or approve-state changes.
    fn match_summary(&self) -> (Vec<usize>, [usize; TAB_COUNT]) {
        {
            let guard = self.matches_cache.borrow();
            if let Some(c) = guard.as_ref() {
                if c.search_value == self.search_value
                    && c.prs_version == self.prs_version
                    && c.approved_generation == self.approved_generation
                {
                    return (c.indices.clone(), c.counts);
                }
            }
        }
        let q = self.search_query();
        let mut indices = Vec::with_capacity(self.prs.len());
        let mut counts = [0usize; TAB_COUNT];
        for (i, pr) in self.prs.iter().enumerate() {
            if !pr_matches_query(pr, &q) {
                continue;
            }
            counts[self.tab_of(pr)] += 1;
            indices.push(i);
        }
        let out = (indices.clone(), counts);
        *self.matches_cache.borrow_mut() = Some(MatchesCache {
            search_value: self.search_value.clone(),
            prs_version: self.prs_version,
            approved_generation: self.approved_generation,
            indices,
            counts,
        });
        out
    }

    pub fn visible_pr_indices(&self) -> Vec<usize> {
        let matched = self.matching_indices();
        let n = matched.len();
        if n <= MAX_LIST_ITEMS {
            return matched;
        }
        let mut start = self.list_offset;
        if start > n - MAX_LIST_ITEMS {
            start = n - MAX_LIST_ITEMS;
        }
        matched[start..start + MAX_LIST_ITEMS].to_vec()
    }

    pub fn advance_cursor(&mut self, delta: i32) -> bool {
        let matched = self.matching_indices();
        if matched.is_empty() {
            return false;
        }
        let pos = cursor_position(&matched, self.cursor);
        let pos = match pos {
            Some(p) => p,
            None => {
                self.cursor = matched[0];
                return true;
            }
        };
        let new_pos = pos as i64 + delta as i64;
        if new_pos < 0 || new_pos >= matched.len() as i64 {
            return false;
        }
        self.cursor = matched[new_pos as usize];
        true
    }

    pub fn ensure_cursor_visible(&mut self) {
        let matched = self.matching_indices();
        let n = matched.len();
        if n == 0 {
            self.list_offset = 0;
            return;
        }
        let pos = match cursor_position(&matched, self.cursor) {
            Some(p) => p,
            None => {
                let mut found = None;
                for (i, &idx) in matched.iter().enumerate() {
                    if idx >= self.cursor {
                        found = Some((i, idx));
                        break;
                    }
                }
                match found {
                    Some((i, idx)) => {
                        self.cursor = idx;
                        i
                    }
                    None => {
                        self.cursor = matched[n - 1];
                        n - 1
                    }
                }
            }
        };
        if n <= MAX_LIST_ITEMS {
            self.list_offset = 0;
            return;
        }
        if pos < self.list_offset {
            self.list_offset = pos;
        } else if pos >= self.list_offset + MAX_LIST_ITEMS {
            self.list_offset = pos - MAX_LIST_ITEMS + 1;
        }
        let max_offset = n - MAX_LIST_ITEMS;
        if self.list_offset > max_offset {
            self.list_offset = max_offset;
        }
    }

    pub fn reconcile_cursor(&mut self, prev_prs: &[PullRequest], prev_cursor: usize) {
        if self.prs.is_empty() {
            self.cursor = 0;
            return;
        }
        if prev_cursor < prev_prs.len() {
            let prev_url = prev_prs[prev_cursor].url.clone();
            if let Some(idx) = index_of_pr_url(&self.prs, &prev_url) {
                self.cursor = idx;
                return;
            }
            for pr in prev_prs.iter().skip(prev_cursor + 1) {
                if let Some(idx) = index_of_pr_url(&self.prs, &pr.url) {
                    self.cursor = idx;
                    return;
                }
            }
            for pr in prev_prs.iter().take(prev_cursor).rev() {
                if let Some(idx) = index_of_pr_url(&self.prs, &pr.url) {
                    self.cursor = idx;
                    return;
                }
            }
        }
        if self.cursor >= self.prs.len() {
            self.cursor = self.prs.len() - 1;
        }
    }

    pub fn refresh_detail_if_needed(&mut self) -> Option<Effect> {
        if self.prs.is_empty() {
            self.current_detail = None;
            self.detail_loading = false;
            self.loading_for_url = None;
            return None;
        }
        if self.cursor >= self.prs.len() {
            self.cursor = self.prs.len() - 1;
        }
        if Some(self.prs[self.cursor].url.clone()) == self.loading_for_url {
            return None;
        }
        self.trigger_detail_load()
    }

    pub fn clear_mark_on_selected(&mut self) {
        if self.marked_prs.is_empty() || self.prs.is_empty() {
            return;
        }
        if self.cursor >= self.prs.len() {
            return;
        }
        let url = self.prs[self.cursor].url.clone();
        self.marked_prs.remove(&url);
    }

    pub fn prune_marked_prs(&mut self) {
        if self.marked_prs.is_empty() {
            return;
        }
        let alive: HashSet<String> = self.prs.iter().map(|p| p.url.clone()).collect();
        self.marked_prs.retain(|url| alive.contains(url));
    }

    pub fn trigger_detail_load(&mut self) -> Option<Effect> {
        if self.prs.is_empty() {
            return None;
        }
        let pr = self.prs[self.cursor].clone();
        self.loading_for_url = Some(pr.url.clone());
        self.current_detail = None;
        self.detail_loading = true;
        self.detail_err = String::new();
        self.detail_scroll = 0;

        let key = gh::cache_key_of(&pr);
        let cached = match self.cache.get_mem(&key) {
            Some(entry) => Some((entry, None)),
            None => self
                .cache
                .get_disk(&key)
                .map(|entry| (entry, Some(Effect::LoadDiff { pr: pr.clone() }))),
        };
        if let Some((entry, effect)) = cached {
            self.debounce_seq += 1;
            self.current_detail = Some(entry.detail);
            self.current_diff = entry.diff;
            self.detail_loading = false;
            return effect;
        }
        self.debounce_seq += 1;
        Some(Effect::Debounce {
            seq: self.debounce_seq,
            url: pr.url,
        })
    }

    pub fn detail_and_prefetch_effects(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(e) = self.trigger_detail_load() {
            effects.push(e);
        }
        let neighbors = neighbor_prs(&self.prs, self.cursor);
        effects.push(Effect::PrefetchHint { prs: neighbors });
        effects
    }

    /// Moves the cursor to the first PR of the active tab when it points
    /// outside the tab, unless both are open-PR tabs (awaiting / reviewed),
    /// where ensure_cursor_visible keeps the nearest position instead.
    fn snap_cursor_into_tab_group(&mut self) {
        if self.cursor >= self.prs.len() {
            return;
        }
        let pr = &self.prs[self.cursor];
        if self.tab_of(pr) == self.active_tab {
            return;
        }
        let tab_is_history = history_state_of_tab(self.active_tab).is_some();
        if !is_history(pr) && !tab_is_history {
            return;
        }
        if let Some(&first) = self.matching_indices().first() {
            self.cursor = first;
        }
    }

    pub fn apply_pr_list(&mut self, prs: Vec<PullRequest>) -> Vec<Effect> {
        self.loading = false;
        self.err = String::new();
        let prev_prs = std::mem::take(&mut self.prs);
        let prev_cursor = self.cursor;
        let history: Vec<PullRequest> =
            prev_prs.iter().filter(|p| is_history(p)).cloned().collect();
        self.pr_signature = pr_list_signature(&prs);
        self.prs = combine_pr_lists(prs, history);
        self.prs_version += 1;
        self.pr_list_loaded = true;
        self.update_notice = None;
        self.prune_marked_prs();
        self.reconcile_cursor(&prev_prs, prev_cursor);
        self.snap_cursor_into_tab_group();
        self.ensure_cursor_visible();
        self.status = format!("{} review request(s)", self.open_count());
        let mut effects = Vec::new();
        if !self.prs.is_empty() {
            if let Some(e) = self.trigger_detail_load() {
                effects.push(e);
            }
            let mut candidates = top_n(&self.prs, 3);
            candidates.retain(|p| p.url != self.prs[self.cursor].url);
            effects.push(Effect::PrefetchHint { prs: candidates });
        }
        effects
    }

    pub fn apply_history_list(
        &mut self,
        state: HistoryState,
        fetched: Vec<PullRequest>,
    ) -> Vec<Effect> {
        let tab = tab_of_history_state(state);
        self.history_loading[tab] = false;
        self.history_loaded[tab] = true;
        self.history_stale[tab] = false;
        let prev_prs = std::mem::take(&mut self.prs);
        let prev_cursor = self.cursor;
        let open: Vec<PullRequest> = prev_prs
            .iter()
            .filter(|p| !is_history(p))
            .cloned()
            .collect();
        // Keep the other history tab's PRs; replace only this state's.
        let mut history: Vec<PullRequest> = prev_prs
            .iter()
            .filter(|p| is_history(p) && p.state != state.pr_state())
            .cloned()
            .collect();
        history.extend(fetched.into_iter().filter(|p| p.state == state.pr_state()));
        self.prs = combine_pr_lists(open, history);
        self.prs_version += 1;
        self.prune_marked_prs();
        self.reconcile_cursor(&prev_prs, prev_cursor);
        self.snap_cursor_into_tab_group();
        self.ensure_cursor_visible();
        if self.matching_indices().is_empty() {
            self.clear_detail();
            return Vec::new();
        }
        match self.refresh_detail_if_needed() {
            Some(e) => vec![e],
            None => Vec::new(),
        }
    }

    fn clear_detail(&mut self) {
        self.current_detail = None;
        self.detail_loading = false;
        self.loading_for_url = None;
        self.detail_err = String::new();
    }

    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Key(k) => {
                if k.kind == crossterm::event::KeyEventKind::Release {
                    return Vec::new();
                }
                self.handle_key(k)
            }
            Msg::Resize(w, h) => {
                self.width = w;
                self.height = h;
                Vec::new()
            }
            Msg::PRList(result) => match result {
                Err(e) => {
                    self.loading = false;
                    self.err = e;
                    self.status = "failed to load review requests".to_string();
                    Vec::new()
                }
                Ok(prs) => self.apply_pr_list(prs),
            },
            Msg::HistoryList { state, result } => match result {
                Err(e) => {
                    self.history_loading[tab_of_history_state(state)] = false;
                    self.err = e;
                    self.status = format!("failed to load {} PRs", state.pr_state().to_lowercase());
                    Vec::new()
                }
                Ok(prs) => self.apply_history_list(state, prs),
            },
            Msg::DiffDone(boxed) => {
                let DiffDoneMsg { pr, result } = *boxed;
                if Some(pr.url.clone()) != self.loading_for_url {
                    return Vec::new();
                }
                self.detail_loading = false;
                match result {
                    Err(e) => {
                        self.detail_err = e;
                        Vec::new()
                    }
                    Ok((detail, diff)) => {
                        self.detail_err = String::new();
                        self.current_detail = Some(detail);
                        self.current_diff = diff;
                        self.detail_scroll = 0;
                        self.detail_total = 0;
                        Vec::new()
                    }
                }
            }
            Msg::ApproveDone { pr, err } => {
                self.loading = false;
                if let Some(e) = err {
                    self.err = e;
                    self.status = "failed to approve".to_string();
                    return Vec::new();
                }
                self.err = String::new();
                self.approved.insert(pr.url.clone());
                self.approved_generation += 1;
                self.status = format!("approved {}", pr_label(&pr));
                self.ensure_cursor_visible();
                match self.refresh_detail_if_needed() {
                    Some(e) => vec![e],
                    None => Vec::new(),
                }
            }
            Msg::CopyDone { pr, err } => {
                if let Some(e) = err {
                    self.err = e;
                    self.status = "failed to copy URL".to_string();
                    return Vec::new();
                }
                self.err = String::new();
                self.status = format!("copied {} URL", pr_label(&pr));
                Vec::new()
            }
            Msg::UpdateCheckTick => {
                let mut effects = Vec::new();
                if !self.loading && self.update_notice.is_none() && self.pr_list_loaded {
                    effects.push(Effect::CheckUpdates {
                        prev_sig: self.pr_signature.clone(),
                        prev_count: self.open_count(),
                    });
                }
                effects
            }
            Msg::UpdateCheck {
                prev_sig,
                prev_count,
                result,
            } => self.handle_update_check(prev_sig, prev_count, result),
            Msg::PopupDismiss { id } => {
                if let Some(notice) = &self.update_notice {
                    if notice.id == id {
                        self.update_notice = None;
                        self.status = format!("{} review request(s)", self.open_count());
                    }
                }
                Vec::new()
            }
            Msg::DebounceFire { seq, url } => {
                if seq != self.debounce_seq {
                    return Vec::new();
                }
                if self.prs.is_empty() || self.cursor >= self.prs.len() {
                    return Vec::new();
                }
                let pr = self.prs[self.cursor].clone();
                if pr.url != url || Some(pr.url.clone()) != self.loading_for_url {
                    return Vec::new();
                }
                vec![Effect::LoadDiff { pr }]
            }
            Msg::SpinnerTick => {
                if self.loading || self.detail_loading || self.any_history_loading() {
                    self.spinner_frame = self.spinner_frame.wrapping_add(1);
                }
                Vec::new()
            }
        }
    }

    fn handle_update_check(
        &mut self,
        prev_sig: String,
        prev_count: usize,
        result: Result<Vec<PullRequest>, String>,
    ) -> Vec<Effect> {
        let prs = match result {
            Err(_) => return Vec::new(),
            Ok(prs) => prs,
        };
        if prev_sig != self.pr_signature {
            return Vec::new();
        }
        let current_sig = pr_list_signature(&prs);
        if current_sig == self.pr_signature {
            return Vec::new();
        }
        if prs.len() < prev_count {
            return self.apply_pr_list(prs);
        }
        let new_urls = new_pr_urls(&self.open_prs(), &prs);
        for url in &new_urls {
            self.marked_prs.insert(url.clone());
        }
        let prev_prs = std::mem::take(&mut self.prs);
        let prev_cursor = self.cursor;
        let history: Vec<PullRequest> =
            prev_prs.iter().filter(|p| is_history(p)).cloned().collect();
        self.prs = combine_pr_lists(prs, history);
        self.prs_version += 1;
        self.pr_signature = current_sig;
        self.reconcile_cursor(&prev_prs, prev_cursor);
        self.ensure_cursor_visible();
        self.popup_seq += 1;
        let open_count = self.open_count();
        self.update_notice = Some(UpdateNotice {
            count: open_count,
            id: self.popup_seq,
        });
        self.status = format!("{open_count} review request(s)");
        let mut effects = Vec::new();
        if !new_urls.is_empty() {
            effects.push(Effect::PlaySound);
        }
        effects.push(Effect::DismissPopup { id: self.popup_seq });
        if let Some(e) = self.refresh_detail_if_needed() {
            effects.push(e);
        }
        effects
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        if self.search_active {
            return self.handle_search_key(key);
        }
        self.handle_normal_key(key)
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        if self.update_notice.is_some() {
            self.update_notice = None;
        }
        if self.pending_approve.is_some() {
            return self.handle_approve_confirmation(key);
        }
        self.clear_mark_on_selected();

        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        match key.code {
            crossterm::event::KeyCode::Char('c') if ctrl => {
                self.quit = true;
                Vec::new()
            }
            crossterm::event::KeyCode::Char('q') if !ctrl => {
                self.quit = true;
                Vec::new()
            }
            crossterm::event::KeyCode::Char('r') if !ctrl => {
                if self.loading {
                    return Vec::new();
                }
                self.loading = true;
                self.status = "refreshing...".to_string();
                self.err = String::new();
                // Merged / Closed refetch on their next visit; the active
                // one refetches now.
                for tab in [TAB_MERGED, TAB_CLOSED] {
                    if self.history_loaded[tab] {
                        self.history_stale[tab] = true;
                    }
                }
                let mut effects = vec![Effect::LoadPRs];
                effects.extend(self.load_active_history_tab());
                effects
            }
            crossterm::event::KeyCode::Char('n') if ctrl => {
                if self.loading {
                    return Vec::new();
                }
                if self.advance_cursor(1) {
                    self.ensure_cursor_visible();
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Char('p') if ctrl => {
                if self.loading {
                    return Vec::new();
                }
                if self.advance_cursor(-1) {
                    self.ensure_cursor_visible();
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Char('h') if !ctrl => self.switch_tab(-1),
            crossterm::event::KeyCode::Char('l') if !ctrl => self.switch_tab(1),
            crossterm::event::KeyCode::Char('/') if !ctrl => {
                if !self.loading {
                    self.search_active = true;
                    self.search_cursor = self.search_value.chars().count();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Char('j') if !ctrl => {
                self.scroll_detail(1);
                Vec::new()
            }
            crossterm::event::KeyCode::Char('k') if !ctrl => {
                self.scroll_detail(-1);
                Vec::new()
            }
            crossterm::event::KeyCode::PageDown => {
                self.scroll_detail(self.detail_viewport_height() as i32);
                Vec::new()
            }
            crossterm::event::KeyCode::PageUp => {
                self.scroll_detail(-(self.detail_viewport_height() as i32));
                Vec::new()
            }
            crossterm::event::KeyCode::Char('y') if !ctrl => {
                if !self.loading && !self.prs.is_empty() && self.cursor < self.prs.len() {
                    let pr = self.prs[self.cursor].clone();
                    self.status = "copying URL...".to_string();
                    self.err = String::new();
                    return vec![Effect::CopyURL { pr }];
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Char('a') if !ctrl => {
                if self.current_detail.is_some() && !self.loading && !self.detail_loading {
                    let pr = match self.current_detail.as_ref() {
                        Some(d) => d.base.clone(),
                        None => return Vec::new(),
                    };
                    if is_history(&pr) {
                        self.status = "cannot approve a merged or closed PR".to_string();
                        return Vec::new();
                    }
                    self.err = String::new();
                    self.pending_approve = Some(pr);
                    self.status = "approval confirmation open".to_string();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn scroll_detail(&mut self, delta: i32) {
        let vp = self.detail_viewport_height();
        if vp == 0 {
            // Detail section is not visible (no room); keep content anchored.
            self.detail_scroll = 0;
            return;
        }
        let max = self.detail_total.saturating_sub(vp);
        let cur = self.detail_scroll as i64;
        let next = (cur + delta as i64).clamp(0, max as i64);
        self.detail_scroll = next as u16;
    }

    pub fn detail_viewport_height(&self) -> u16 {
        if self.width == 0 || self.height == 0 {
            return 3;
        }
        let h = self.height as i32;
        let search_h = if self.search_visible() { 1i32 } else { 0 };
        // Mirror render(): the list takes at most the rows left above the footer.
        let list_full = self.compute_list_section_height();
        let list_cap = (h.max(0) as usize).saturating_sub((search_h + 1) as usize);
        let list_h = list_full.min(list_cap) as i32;
        let avail = h - 1 - search_h - list_h;
        if avail <= 2 {
            return 0;
        }
        (avail - 2) as u16
    }

    fn handle_search_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        match key.code {
            crossterm::event::KeyCode::Char('c') if ctrl => {
                self.quit = true;
                Vec::new()
            }
            crossterm::event::KeyCode::Esc => {
                self.search_active = false;
                self.search_value.clear();
                self.search_cursor = 0;
                self.ensure_cursor_visible();
                Vec::new()
            }
            crossterm::event::KeyCode::Enter => {
                self.search_active = false;
                Vec::new()
            }
            crossterm::event::KeyCode::Char('n') if ctrl => {
                if !self.loading && self.advance_cursor(1) {
                    self.ensure_cursor_visible();
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Char('p') if ctrl => {
                if !self.loading && self.advance_cursor(-1) {
                    self.ensure_cursor_visible();
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Backspace => {
                let prev_cursor = self.cursor;
                if self.search_cursor > 0 {
                    let byte_idx = self.char_to_byte(self.search_cursor - 1);
                    self.search_value
                        .replace_range(byte_idx..self.char_to_byte(self.search_cursor), "");
                    self.search_cursor -= 1;
                }
                self.ensure_cursor_visible();
                if self.cursor != prev_cursor && !self.loading && !self.prs.is_empty() {
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Delete => {
                let prev_cursor = self.cursor;
                if self.search_cursor < self.search_value.chars().count() {
                    let byte_idx = self.char_to_byte(self.search_cursor);
                    let end = self.char_to_byte(self.search_cursor + 1);
                    self.search_value.replace_range(byte_idx..end, "");
                }
                self.ensure_cursor_visible();
                if self.cursor != prev_cursor && !self.loading && !self.prs.is_empty() {
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Left => {
                if self.search_cursor > 0 {
                    self.search_cursor -= 1;
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Right => {
                if self.search_cursor < self.search_value.chars().count() {
                    self.search_cursor += 1;
                }
                Vec::new()
            }
            crossterm::event::KeyCode::Home => {
                self.search_cursor = 0;
                Vec::new()
            }
            crossterm::event::KeyCode::End => {
                self.search_cursor = self.search_value.chars().count();
                Vec::new()
            }
            crossterm::event::KeyCode::Char(ch)
                if !ctrl && !key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
            {
                let prev_cursor = self.cursor;
                let byte_idx = self.char_to_byte(self.search_cursor);
                self.search_value.insert(byte_idx, ch);
                self.search_cursor += 1;
                self.ensure_cursor_visible();
                if self.cursor != prev_cursor && !self.loading && !self.prs.is_empty() {
                    return self.detail_and_prefetch_effects();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn char_to_byte(&self, char_idx: usize) -> usize {
        self.search_value
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.search_value.len())
    }

    fn handle_approve_confirmation(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        let pr = match self.pending_approve.clone() {
            Some(p) => p,
            None => return Vec::new(),
        };
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        match key.code {
            crossterm::event::KeyCode::Char('c') if ctrl => {
                self.quit = true;
                Vec::new()
            }
            crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
                self.pending_approve = None;
                self.loading = true;
                self.status = "approving...".to_string();
                self.err = String::new();
                vec![Effect::Approve { pr }]
            }
            crossterm::event::KeyCode::Char('c')
            | crossterm::event::KeyCode::Char('C')
            | crossterm::event::KeyCode::Esc => {
                self.pending_approve = None;
                self.status = "approval canceled".to_string();
                self.err = String::new();
                Vec::new()
            }
            _ => {
                self.status = "press y to approve or c to cancel".to_string();
                Vec::new()
            }
        }
    }

    fn switch_tab(&mut self, delta: i32) -> Vec<Effect> {
        if self.loading {
            return Vec::new();
        }
        let next = self.active_tab as i64 + delta as i64;
        if next < 0 || next >= TAB_COUNT as i64 {
            return Vec::new();
        }
        self.active_tab = next as usize;
        self.list_offset = 0;
        let load = self.load_active_history_tab();
        let matched = self.matching_indices();
        let mut effects = if matched.is_empty() {
            self.clear_detail();
            Vec::new()
        } else {
            self.cursor = matched[0];
            self.ensure_cursor_visible();
            self.detail_and_prefetch_effects()
        };
        effects.extend(load);
        effects
    }

    pub fn approve_label(&self, pr: &PullRequest) -> &'static str {
        if self.approved.contains(&pr.url) || pr.review_decision == "APPROVED" {
            return "approved";
        }
        match pr.review_decision.as_str() {
            "CHANGES_REQUESTED" => "changes",
            "REVIEW_REQUIRED" => "required",
            _ => "-",
        }
    }

    pub fn empty_list_message(&self) -> &'static str {
        if !self.search_value.trim().is_empty() {
            return "No PRs match the current filter.";
        }
        match self.active_tab {
            TAB_REVIEWED => "No reviewed PRs.",
            TAB_MERGED if !self.history_loaded[TAB_MERGED] => "Loading merged PRs...",
            TAB_CLOSED if !self.history_loaded[TAB_CLOSED] => "Loading closed PRs...",
            TAB_MERGED => "No merged PRs.",
            TAB_CLOSED => "No closed PRs.",
            _ => "No PRs awaiting your review.",
        }
    }

    pub fn compute_list_section_height(&self) -> usize {
        let items = self.visible_pr_indices().len().max(1);
        let inner = 1 + items;
        1 + 2 + inner
    }

    pub fn search_visible(&self) -> bool {
        self.search_active || !self.search_value.is_empty()
    }
}

fn history_state_of_tab(tab: usize) -> Option<HistoryState> {
    match tab {
        TAB_MERGED => Some(HistoryState::Merged),
        TAB_CLOSED => Some(HistoryState::Closed),
        _ => None,
    }
}

fn tab_of_history_state(state: HistoryState) -> usize {
    match state {
        HistoryState::Merged => TAB_MERGED,
        HistoryState::Closed => TAB_CLOSED,
    }
}

/// Merged or closed PR (shown in the Merged / Closed tabs).
pub fn is_history(pr: &PullRequest) -> bool {
    pr.state == "MERGED" || pr.state == "CLOSED"
}

/// Open PRs first, then history PRs not already present in `open`.
pub fn combine_pr_lists(open: Vec<PullRequest>, history: Vec<PullRequest>) -> Vec<PullRequest> {
    let open_urls: HashSet<String> = open.iter().map(|p| p.url.clone()).collect();
    let mut out = open;
    out.extend(history.into_iter().filter(|p| !open_urls.contains(&p.url)));
    out
}

pub fn pr_matches_query(pr: &PullRequest, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    pr.repository.to_lowercase().contains(q)
        || pr.title.to_lowercase().contains(q)
        || pr.author.to_lowercase().contains(q)
}

fn cursor_position(indices: &[usize], cursor: usize) -> Option<usize> {
    indices.iter().position(|&i| i == cursor)
}

pub fn index_of_pr_url(prs: &[PullRequest], url: &str) -> Option<usize> {
    if url.is_empty() {
        return None;
    }
    prs.iter().position(|p| p.url == url)
}

pub fn new_pr_urls(prev: &[PullRequest], curr: &[PullRequest]) -> Vec<String> {
    let existing: HashSet<&String> = prev.iter().map(|p| &p.url).collect();
    curr.iter()
        .filter(|p| !existing.contains(&p.url))
        .map(|p| p.url.clone())
        .collect()
}

pub fn pr_list_signature(prs: &[PullRequest]) -> String {
    let mut parts: Vec<String> = prs
        .iter()
        .map(|pr| {
            [
                pr.url.clone(),
                gh::format_rfc3339_nanos(pr.updated_at),
                pr.request.clone(),
                pr.review_decision.clone(),
            ]
            .join("\x00")
        })
        .collect();
    parts.sort();
    parts.join("\x01")
}

pub fn pr_label(pr: &PullRequest) -> String {
    format!("{}#{}", pr.repository, pr.number)
}

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

// ---- rendering ----

fn fg(name: &str) -> Style {
    Style::new().fg(c(name))
}

pub fn render_diff_content(
    detail: &PullRequestDetail,
    diff: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let title = format!(
        "{}  #{}  {}",
        detail.base.repository, detail.base.number, detail.base.title
    );
    lines.push(Line::from(Span::styled(
        title,
        Style::new()
            .fg(c(colors::MAGENTA))
            .add_modifier(Modifier::BOLD),
    )));

    let mut meta = meta_pair("Author", &format!("@{}", detail.base.author));
    meta.extend(meta_pair("Review request", &detail.base.request));
    let branch = branch_label(detail);
    if !branch.is_empty() {
        meta.extend(meta_pair("Branch", &branch));
    }
    lines.push(Line::from(meta));

    let mut meta2 = meta_pair("State", &non_empty(&detail.merge_state_status, "unknown"));
    let decision = non_empty(&detail.base.review_decision, "none");
    meta2.push(Span::styled("Review:".to_string(), fg(colors::YELLOW)));
    meta2.push(Span::raw(" "));
    meta2.push(Span::styled(
        decision,
        review_decision_style(&detail.base.review_decision),
    ));
    meta2.extend(meta_pair("Files", &detail.changed_files.to_string()));
    meta2.push(Span::styled(
        format!("+{}", detail.additions),
        fg(colors::GREEN),
    ));
    meta2.push(Span::raw(" "));
    meta2.push(Span::styled(
        format!("-{}", detail.deletions),
        fg(colors::RED),
    ));
    lines.push(Line::from(meta2));

    if !detail.reviewers.is_empty() {
        let mut spans = vec![
            Span::styled("Reviewed by:".to_string(), fg(colors::YELLOW)),
            Span::raw(" "),
        ];
        for (i, reviewer) in detail.reviewers.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(", "));
            }
            spans.push(Span::raw(format!("@{}", reviewer.author)));
            spans.push(Span::styled(
                reviewer.state.clone(),
                review_decision_style(&reviewer.state),
            ));
        }
        lines.push(Line::from(spans));
    }

    if detail.created_at != std::time::SystemTime::UNIX_EPOCH
        || detail.base.updated_at != std::time::SystemTime::UNIX_EPOCH
    {
        let mut spans = Vec::new();
        if detail.created_at != std::time::SystemTime::UNIX_EPOCH {
            spans.extend(meta_pair(
                "Created",
                &crate::gh::format_human(detail.created_at),
            ));
        }
        if detail.base.updated_at != std::time::SystemTime::UNIX_EPOCH {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.extend(meta_pair(
                "Updated",
                &crate::gh::format_human(detail.base.updated_at),
            ));
        }
        lines.push(Line::from(spans));
    }

    if !detail.labels.is_empty() {
        let spans = meta_pair("Labels", &detail.labels.join(", "));
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));

    let body = detail.body.trim();
    if body.is_empty() {
        lines.push(Line::from(Span::styled(
            "No description.".to_string(),
            fg(colors::MUTED),
        )));
    } else {
        lines.extend(render_markdown(body, width));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("─".repeat(80), fg(colors::MUTED))));
    lines.push(Line::from(""));

    lines.extend(highlight_diff(diff));

    // Wrap every logical line to the content width up front. Paragraph would
    // otherwise word-wrap long lines (diff hunks, code fences, long headers)
    // with its own layout, whose row count the scroll-total estimate below
    // cannot reproduce exactly; pre-wrapped lines are never re-wrapped, so
    // one pre-wrapped line == one rendered row and the estimate stays exact.
    lines
        .into_iter()
        .flat_map(|line| wrap_line_spans(line, width))
        .collect()
}

fn meta_pair(label: &str, value: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{label}:"), fg(colors::YELLOW)),
        Span::raw(format!(" {value}")),
    ]
}

/// Wraps a styled line to `width`, preserving span styles, so that every
/// produced line is at most `width` columns wide. ratatui's `Paragraph` never
/// re-wraps lines at or below the area width, which keeps the scroll-total
/// estimate in render_detail_section exact.
fn wrap_line_spans(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
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
        idx = cut;
    }
    out
}

fn non_empty(s: &str, fallback: &str) -> String {
    if s.is_empty() {
        fallback.to_string()
    } else {
        s.to_string()
    }
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

fn highlight_diff(diff: &str) -> Vec<Line<'static>> {
    if diff.is_empty() {
        return Vec::new();
    }
    diff.lines()
        .map(strip_ansi)
        .map(|line| {
            let style = if line.starts_with("diff --git") {
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
                fg(colors::MAGENTA)
            } else if line.starts_with('+') {
                fg(colors::GREEN)
            } else if line.starts_with('-') {
                fg(colors::RED)
            } else {
                Style::new()
            };
            Line::from(Span::styled(line, style))
        })
        .collect()
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

fn render_tab_bar(m: &Model) -> Paragraph<'static> {
    let counts = m.tab_counts();
    // Merged / Closed: no count until first visit, "…" while fetching.
    let history_label = |name: &str, tab: usize| {
        if m.history_loading[tab] {
            format!("{name} …")
        } else if m.history_loaded[tab] {
            format!("{name} {}", counts[tab])
        } else {
            name.to_string()
        }
    };
    let labels = [
        format!("Awaiting Review {}", counts[TAB_AWAITING]),
        format!("Reviewed {}", counts[TAB_REVIEWED]),
        history_label("Merged", TAB_MERGED),
        history_label("Closed", TAB_CLOSED),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if i == m.active_tab {
            Style::new()
                .fg(c(colors::INK))
                .bg(c(colors::BLUE))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(c(colors::MUTED)).bg(c(colors::BAR_BG))
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Paragraph::new(Line::from(spans))
}

fn list_title_width(box_w: usize) -> usize {
    let fixed = 2
        + COL_MARK_W
        + 1
        + COL_TYPE_W
        + 2
        + COL_REPO_W
        + 2
        + COL_NUM_W
        + 2
        + 2
        + 1
        + COL_AUTHOR_W
        + 2
        + COL_APPROVE_W;
    std::cmp::max(10, box_w.saturating_sub(fixed))
}

fn render_list_frame(m: &Model, f: &mut Frame<'_>, rect: Rect) {
    let box_w = m.frame_width();
    let title_w = list_title_width(box_w);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let header = vec![
        Span::styled(format!(" {}", pad_right("", COL_MARK_W)), fg(colors::MUTED)),
        Span::raw(" "),
        Span::styled(pad_right("Type", COL_TYPE_W), fg(colors::MUTED)),
        Span::raw("  "),
        Span::styled(pad_right("Repository", COL_REPO_W), fg(colors::MUTED)),
        Span::raw("  "),
        Span::styled(pad_right("#", COL_NUM_W), fg(colors::MUTED)),
        Span::raw("  "),
        Span::styled(pad_right("Title", title_w), fg(colors::MUTED)),
        Span::raw("   "),
        Span::styled(pad_right("Author", COL_AUTHOR_W), fg(colors::MUTED)),
        Span::raw("  "),
        Span::styled(pad_right("Approve", COL_APPROVE_W), fg(colors::MUTED)),
    ];
    lines.push(Line::from(header));

    let items = m.visible_pr_indices();
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" {}", m.empty_list_message()),
            fg(colors::MUTED),
        )));
    } else {
        for idx in items {
            let pr = &m.prs[idx];
            lines.push(render_pr_line(m, idx, pr, box_w));
        }
    }

    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(fg(colors::FRAME));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_pr_line(m: &Model, idx: usize, pr: &PullRequest, box_w: usize) -> Line<'static> {
    let title_w = list_title_width(box_w);
    let approve = m.approve_label(pr);
    let selected = idx == m.cursor;

    let mut spans: Vec<Span<'static>> = Vec::new();
    if selected {
        spans.push(Span::styled(
            "▌".to_string(),
            Style::new().fg(c(colors::CYAN)).bg(c(colors::SELECTED)),
        ));
    } else {
        spans.push(Span::raw(" "));
    }

    // mark cell
    if m.marked_prs.contains(&pr.url) {
        let style = if selected {
            Style::new()
                .fg(c(colors::YELLOW))
                .bg(c(colors::SELECTED))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
                .fg(c(colors::YELLOW))
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(pad_right("[!]", COL_MARK_W), style));
    } else if selected {
        spans.push(Span::styled(
            repeat_space(COL_MARK_W),
            Style::new().bg(c(colors::SELECTED)),
        ));
    } else {
        spans.push(Span::raw(repeat_space(COL_MARK_W)));
    }
    spans.push(Span::styled(
        " ".to_string(),
        if selected {
            Style::new().bg(c(colors::SELECTED))
        } else {
            Style::new()
        },
    ));

    // type cell
    let is_me = pr.request.contains("@me");
    let type_label = if is_me { "[me]" } else { "[team]" };
    let type_color = if is_me { colors::CYAN } else { colors::ORANGE };
    let type_style = if selected {
        Style::new().fg(c(type_color)).bg(c(colors::SELECTED))
    } else {
        fg(type_color)
    };
    spans.push(Span::styled(pad_right(type_label, COL_TYPE_W), type_style));
    spans.push(Span::styled(
        "  ".to_string(),
        if selected {
            Style::new().bg(c(colors::SELECTED))
        } else {
            Style::new()
        },
    ));

    let repo_style = if selected {
        Style::new()
            .fg(c(colors::SELECTED_FG))
            .bg(c(colors::SELECTED))
    } else {
        fg(colors::CYAN)
    };
    spans.push(Span::styled(
        pad_right(&pr.repository, COL_REPO_W),
        repo_style,
    ));
    spans.push(Span::styled(
        "  ".to_string(),
        if selected {
            Style::new().bg(c(colors::SELECTED))
        } else {
            Style::new()
        },
    ));

    let num_style = if selected {
        Style::new().fg(c(colors::YELLOW)).bg(c(colors::SELECTED))
    } else {
        fg(colors::BLUE)
    };
    spans.push(Span::styled(
        pad_right(&format!("#{}", pr.number), COL_NUM_W),
        num_style,
    ));
    spans.push(Span::styled(
        "  ".to_string(),
        if selected {
            Style::new().bg(c(colors::SELECTED))
        } else {
            Style::new()
        },
    ));

    let title_style = if selected {
        Style::new()
            .fg(c(colors::SELECTED_FG))
            .bg(c(colors::SELECTED))
            .add_modifier(Modifier::BOLD)
    } else {
        fg(colors::FG)
    };
    spans.push(Span::styled(pad_right(&pr.title, title_w), title_style));
    spans.push(Span::styled(
        "  ".to_string(),
        if selected {
            Style::new().bg(c(colors::SELECTED))
        } else {
            Style::new()
        },
    ));

    let author_style = if selected {
        Style::new()
            .fg(c(colors::SELECTED_FG))
            .bg(c(colors::SELECTED))
    } else {
        fg(colors::MAGENTA)
    };
    spans.push(Span::styled(
        pad_right(&format!("@{}", pr.author), COL_AUTHOR_W),
        author_style,
    ));
    spans.push(Span::styled(
        "  ".to_string(),
        if selected {
            Style::new().bg(c(colors::SELECTED))
        } else {
            Style::new()
        },
    ));

    let approve_style = if selected {
        match approve {
            "approved" => Style::new().fg(c(colors::GREEN)).bg(c(colors::SELECTED)),
            "changes" => Style::new().fg(c(colors::RED)).bg(c(colors::SELECTED)),
            _ => Style::new().fg(c(colors::YELLOW)).bg(c(colors::SELECTED)),
        }
    } else {
        match approve {
            "approved" => fg(colors::GREEN),
            "changes" => fg(colors::RED),
            _ => fg(colors::MUTED),
        }
    };
    spans.push(Span::styled(
        pad_right(approve, COL_APPROVE_W),
        approve_style,
    ));

    if selected {
        spans.push(Span::styled(
            " ".to_string(),
            Style::new().bg(c(colors::SELECTED)),
        ));
    }

    Line::from(spans)
}

fn render_detail_section(m: &mut Model, f: &mut Frame<'_>, rect: Rect, vp_h: u16) {
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
            let fresh = m.detail_view_cache.as_ref().is_some_and(|c| {
                c.content_width == content_width && c.detail == *detail && c.diff == m.current_diff
            });
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
                    detail: detail.clone(),
                    diff: m.current_diff.clone(),
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
    let para = Paragraph::new(Text::from(cache.lines.clone()))
        .wrap(Wrap { trim: false })
        .scroll((m.detail_scroll, 0));
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
    let bindings: [(&str, &str); 9] = [
        ("h/l", "tabs"),
        ("ctrl+n/p", "list"),
        ("j/k", "scroll"),
        ("pgup/dn", "page"),
        ("/", "filter"),
        ("y", "copy"),
        ("a", "approve"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(url: &str) -> PullRequest {
        PullRequest {
            url: url.to_string(),
            ..Default::default()
        }
    }

    fn new_model() -> Model {
        let cache = Arc::new(DetailCache::with_dir(None));
        let prefetcher = Arc::new(Prefetcher::new(cache.clone()));
        Model::new(cache, prefetcher)
    }

    fn model_with_loaded_detail() -> Model {
        let mut m = new_model();
        m.loading = false;
        m.prs = vec![PullRequest {
            repository: "owner/repo".into(),
            number: 123,
            title: "Test PR".into(),
            url: "https://example.test/pr/123".into(),
            author: "octocat".into(),
            request: "@me".into(),
            ..Default::default()
        }];
        m.current_detail = Some(PullRequestDetail {
            base: m.prs[0].clone(),
            ..Default::default()
        });
        m
    }

    fn key(code: crossterm::event::KeyCode) -> Msg {
        Msg::Key(crossterm::event::KeyEvent::from(code))
    }

    fn ctrl_key(code: crossterm::event::KeyCode) -> Msg {
        Msg::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::CONTROL,
        ))
    }

    #[test]
    fn test_approve_key_requires_confirmation() {
        let mut m = model_with_loaded_detail();
        let effects = m.update(key(crossterm::event::KeyCode::Char('a')));
        assert!(
            effects.is_empty(),
            "approve key returned command before confirmation"
        );
        assert!(m.pending_approve.is_some());
        assert!(!m.loading);
    }

    #[test]
    fn test_approve_confirmation_yes_starts_approve() {
        let mut m = model_with_loaded_detail();
        m.update(key(crossterm::event::KeyCode::Char('a')));
        let effects = m.update(key(crossterm::event::KeyCode::Char('y')));
        assert!(matches!(effects.first(), Some(Effect::Approve { .. })));
        assert!(m.pending_approve.is_none());
        assert!(m.loading);
    }

    #[test]
    fn test_approve_confirmation_cancel_cancels_approve() {
        let mut m = model_with_loaded_detail();
        m.update(key(crossterm::event::KeyCode::Char('a')));
        let effects = m.update(key(crossterm::event::KeyCode::Char('c')));
        assert!(effects.is_empty());
        assert!(m.pending_approve.is_none());
        assert!(!m.loading);
    }

    #[test]
    fn test_approve_label_uses_review_decision() {
        let m = new_model();
        let cases = [
            ("APPROVED", "approved"),
            ("CHANGES_REQUESTED", "changes"),
            ("REVIEW_REQUIRED", "required"),
            ("", "-"),
        ];
        for (decision, want) in cases {
            let pr = PullRequest {
                review_decision: decision.into(),
                ..Default::default()
            };
            assert_eq!(m.approve_label(&pr), want);
        }
    }

    #[test]
    fn test_approve_label_uses_local_approved_state() {
        let mut m = new_model();
        let pr = pr("https://example.test/pr/1");
        m.approved.insert(pr.url.clone());
        assert_eq!(m.approve_label(&pr), "approved");
    }

    fn sig_prs(urls: &[(&str, u64)]) -> Vec<PullRequest> {
        urls.iter()
            .map(|(url, t)| PullRequest {
                url: url.to_string(),
                updated_at: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(*t),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn test_update_check_shows_notice_when_signature_changes() {
        let mut m = new_model();
        let prs = sig_prs(&[("https://example.test/pr/1", 1747200000)]);
        m.prs = prs.clone();
        m.pr_signature = pr_list_signature(&prs);

        let new_prs = sig_prs(&[("https://example.test/pr/1", 1747200060)]);
        let effects = m.update(Msg::UpdateCheck {
            prev_sig: m.pr_signature.clone(),
            prev_count: 1,
            result: Ok(new_prs.clone()),
        });

        assert!(
            m.update_notice.is_some(),
            "update check did not show notice"
        );
        assert_eq!(m.status, "1 review request(s)");
        assert_eq!(m.prs[0].updated_at, new_prs[0].updated_at);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::DismissPopup { .. })));
    }

    #[test]
    fn test_update_check_plays_sound_only_for_new_review_requests() {
        let mut m = new_model();
        let prs = sig_prs(&[("https://example.test/pr/1", 1747200000)]);
        m.prs = prs.clone();
        m.pr_signature = pr_list_signature(&prs);

        let updated_existing = sig_prs(&[("https://example.test/pr/1", 1747200300)]);
        let mut effects = m.update(Msg::UpdateCheck {
            prev_sig: m.pr_signature.clone(),
            prev_count: 1,
            result: Ok(updated_existing.clone()),
        });
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::PlaySound)),
            "signature-only change should not play sound"
        );

        let mut added = updated_existing;
        added.push(PullRequest {
            url: "https://example.test/pr/2".into(),
            updated_at: std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1747200360),
            request: "@me".into(),
            ..Default::default()
        });
        effects = m.update(Msg::UpdateCheck {
            prev_sig: m.pr_signature.clone(),
            prev_count: 1,
            result: Ok(added),
        });
        assert!(effects.iter().any(|e| matches!(e, Effect::PlaySound)));
    }

    #[test]
    fn test_update_check_silent_reload_when_prs_decrease() {
        let mut m = new_model();
        m.loading = false;
        m.pr_list_loaded = true;
        let prs = sig_prs(&[
            ("https://example.test/pr/1", 1747200000),
            ("https://example.test/pr/2", 1747200001),
        ]);
        m.prs = prs.clone();
        m.pr_signature = pr_list_signature(&prs);

        let new_prs = vec![prs[0].clone()];
        m.update(Msg::UpdateCheck {
            prev_sig: m.pr_signature.clone(),
            prev_count: 2,
            result: Ok(new_prs),
        });

        assert!(
            m.update_notice.is_none(),
            "decreased PR count should not show notice"
        );
        assert_eq!(m.prs.len(), 1, "decreased PR count should reload list");
    }

    #[test]
    fn test_update_check_marks_new_prs() {
        let mut m = new_model();
        let prs = sig_prs(&[("https://example.test/pr/1", 1747200000)]);
        m.prs = prs.clone();
        m.pr_signature = pr_list_signature(&prs);

        let mut new_prs = prs.clone();
        new_prs.push(pr("https://example.test/pr/2"));
        m.update(Msg::UpdateCheck {
            prev_sig: m.pr_signature.clone(),
            prev_count: 1,
            result: Ok(new_prs),
        });

        assert!(m.marked_prs.contains("https://example.test/pr/2"));
        assert!(!m.marked_prs.contains("https://example.test/pr/1"));
    }

    #[test]
    fn test_key_press_clears_mark_on_selected_pr() {
        let mut m = new_model();
        m.loading = false;
        m.prs = vec![
            pr("https://example.test/pr/1"),
            pr("https://example.test/pr/2"),
        ];
        m.marked_prs.insert("https://example.test/pr/1".into());
        m.marked_prs.insert("https://example.test/pr/2".into());
        m.cursor = 1;

        m.update(key(crossterm::event::KeyCode::Char('j')));
        assert!(!m.marked_prs.contains("https://example.test/pr/2"));
        assert!(m.marked_prs.contains("https://example.test/pr/1"));
    }

    #[test]
    fn test_popup_dismiss_msg_clears_current_notice_only() {
        let mut m = new_model();
        m.update_notice = Some(UpdateNotice { count: 1, id: 5 });
        m.popup_seq = 5;
        m.prs = vec![pr("u")];

        m.update(Msg::PopupDismiss { id: 4 });
        assert!(
            m.update_notice.is_some(),
            "stale dismiss should not clear notice"
        );

        m.update(Msg::PopupDismiss { id: 5 });
        assert!(
            m.update_notice.is_none(),
            "matching dismiss should clear notice"
        );
    }

    #[test]
    fn test_key_press_dismisses_update_notice() {
        let mut m = new_model();
        m.loading = false;
        m.prs = vec![pr("https://example.test/pr/1")];
        m.popup_seq = 7;
        m.update_notice = Some(UpdateNotice { count: 1, id: 7 });

        m.update(key(crossterm::event::KeyCode::Char('j')));
        assert!(m.update_notice.is_none());
    }

    #[test]
    fn test_pr_list_msg_prunes_stale_marks() {
        let mut m = new_model();
        m.marked_prs.insert("https://example.test/pr/1".into());
        m.marked_prs.insert("https://example.test/pr/2".into());

        m.update(Msg::PRList(Ok(vec![pr("https://example.test/pr/1")])));
        assert!(!m.marked_prs.contains("https://example.test/pr/2"));
        assert!(m.marked_prs.contains("https://example.test/pr/1"));
    }

    #[test]
    fn test_debounce_fire_msg_with_stale_seq_is_ignored() {
        let mut m = new_model();
        m.loading = false;
        m.prs = vec![pr("https://example.test/pr/1")];
        m.loading_for_url = Some("https://example.test/pr/1".into());
        m.debounce_seq = 5;

        let effects = m.update(Msg::DebounceFire {
            seq: 4,
            url: m.prs[0].url.clone(),
        });
        assert!(
            effects.is_empty(),
            "stale debounce seq should not fire load command"
        );
    }

    #[test]
    fn test_trigger_detail_load_cache_hit_is_immediate() {
        let mut m = new_model();
        let pr = PullRequest {
            url: "https://example.test/pr/1".into(),
            updated_at: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100),
            ..Default::default()
        };
        m.prs = vec![pr.clone()];
        m.cache.put(
            &gh::cache_key_of(&pr),
            CacheEntry {
                detail: PullRequestDetail {
                    base: pr.clone(),
                    ..Default::default()
                },
                diff: "diff body".into(),
            },
        );

        let effect = m.trigger_detail_load();
        assert!(effect.is_none(), "cache hit applies synchronously");
        assert!(!m.detail_loading);
        assert_eq!(m.current_diff, "diff body");
        assert!(m.current_detail.is_some());
    }

    #[test]
    fn test_trigger_detail_load_cache_miss_returns_debounce_tick() {
        let mut m = new_model();
        let pr = PullRequest {
            url: "https://example.test/pr/miss".into(),
            updated_at: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100),
            ..Default::default()
        };
        m.prs = vec![pr];

        let start_seq = m.debounce_seq;
        let effect = m.trigger_detail_load();
        assert!(matches!(effect, Some(Effect::Debounce { .. })));
        assert_eq!(m.debounce_seq, start_seq + 1);
    }

    #[test]
    fn test_search_filter_matches_repository_title_and_author() {
        let cases: Vec<(&str, Vec<usize>)> = vec![
            ("api", vec![0]),
            ("billing", vec![1]),
            ("CAROL", vec![2]),
            (" ", vec![0, 1, 2]),
            ("missing", vec![]),
        ];

        for (query, want) in cases {
            let mut m = model_with_prs_for_search();
            m.search_value = query.to_string();
            let got = m.matching_indices();
            assert_eq!(got, want, "query {query}");
        }
    }

    fn model_with_prs_for_search() -> Model {
        let mut m = new_model();
        m.prs = vec![
            PullRequest {
                repository: "owner/api".into(),
                number: 1,
                title: "Fix API auth".into(),
                url: "https://example.test/pr/1".into(),
                author: "alice".into(),
                request: "@me".into(),
                ..Default::default()
            },
            PullRequest {
                repository: "owner/web".into(),
                number: 2,
                title: "Add billing view".into(),
                url: "https://example.test/pr/2".into(),
                author: "bob".into(),
                request: "@me".into(),
                ..Default::default()
            },
            PullRequest {
                repository: "owner/cli".into(),
                number: 3,
                title: "Fix command output".into(),
                url: "https://example.test/pr/3".into(),
                author: "carol".into(),
                request: "org/team".into(),
                ..Default::default()
            },
        ];
        m
    }

    #[test]
    fn test_reconcile_cursor_keeps_selection_when_item_above_removed() {
        let prev = vec![
            pr("https://example.test/pr/1"),
            pr("https://example.test/pr/2"),
            pr("https://example.test/pr/3"),
        ];
        let mut m = new_model();
        m.prs = vec![prev[0].clone(), prev[2].clone()];
        m.reconcile_cursor(&prev, 2);
        assert_eq!(m.prs[m.cursor].url, "https://example.test/pr/3");
    }

    #[test]
    fn test_reconcile_cursor_falls_forward_when_selected_removed() {
        let prev = vec![
            pr("https://example.test/pr/1"),
            pr("https://example.test/pr/2"),
            pr("https://example.test/pr/3"),
        ];
        let mut m = new_model();
        m.prs = vec![prev[0].clone(), prev[2].clone()];
        m.reconcile_cursor(&prev, 1);
        assert_eq!(m.prs[m.cursor].url, "https://example.test/pr/3");
    }

    #[test]
    fn test_reconcile_cursor_falls_back_when_tail_removed() {
        let prev = vec![
            pr("https://example.test/pr/1"),
            pr("https://example.test/pr/2"),
        ];
        let mut m = new_model();
        m.prs = vec![prev[0].clone()];
        m.reconcile_cursor(&prev, 1);
        assert_eq!(m.prs[m.cursor].url, "https://example.test/pr/1");
    }

    #[test]
    fn test_search_cursor_moves_within_filtered_matches() {
        let mut m = model_with_prs_for_search();
        m.search_value = "fix".into();
        m.cursor = 0;

        assert!(m.advance_cursor(1));
        assert_eq!(m.cursor, 2);
        assert!(
            !m.advance_cursor(1),
            "advanceCursor should stop at the last filtered match"
        );
        assert!(m.advance_cursor(-1));
        assert_eq!(m.cursor, 0);
    }

    #[test]
    fn test_ensure_cursor_visible_snaps_to_nearest_filtered_match() {
        let mut m = model_with_prs_for_search();
        m.search_value = "fix".into();
        m.cursor = 1;

        m.ensure_cursor_visible();

        assert_eq!(m.cursor, 2);
        assert_eq!(m.list_offset, 0);
    }

    fn model_with_mixed_review_state() -> Model {
        let mut m = new_model();
        m.width = 120;
        m.height = 40;
        m.loading = false;
        m.prs = vec![
            PullRequest {
                repository: "owner/api".into(),
                number: 1,
                title: "awaiting one".into(),
                url: "https://example.test/pr/1".into(),
                author: "alice".into(),
                request: "@me".into(),
                review_decision: "REVIEW_REQUIRED".into(),
                ..Default::default()
            },
            PullRequest {
                repository: "owner/web".into(),
                number: 2,
                title: "already approved".into(),
                url: "https://example.test/pr/2".into(),
                author: "bob".into(),
                request: "org/team".into(),
                review_decision: "APPROVED".into(),
                ..Default::default()
            },
            PullRequest {
                repository: "owner/cli".into(),
                number: 3,
                title: "awaiting two".into(),
                url: "https://example.test/pr/3".into(),
                author: "carol".into(),
                request: "@me".into(),
                review_decision: "CHANGES_REQUESTED".into(),
                ..Default::default()
            },
        ];
        m
    }

    #[test]
    fn test_matching_indices_respects_active_tab() {
        let mut m = model_with_mixed_review_state();
        m.active_tab = TAB_AWAITING;
        assert_eq!(m.matching_indices(), vec![0, 2]);
        m.active_tab = TAB_REVIEWED;
        assert_eq!(m.matching_indices(), vec![1]);
    }

    #[test]
    fn test_local_approve_moves_pr_to_reviewed_tab() {
        let mut m = model_with_mixed_review_state();
        m.approved.insert(m.prs[0].url.clone());

        m.active_tab = TAB_AWAITING;
        assert_eq!(m.matching_indices(), vec![2]);
        m.active_tab = TAB_REVIEWED;
        assert_eq!(m.matching_indices(), vec![0, 1]);
    }

    #[test]
    fn test_tab_counts_are_independent_of_active_tab() {
        let m = model_with_mixed_review_state();
        assert_eq!(m.tab_counts(), [2, 1, 0, 0]);
    }

    #[test]
    fn test_switch_tab_moves_cursor_into_new_tab() {
        let mut m = model_with_mixed_review_state();
        m.cursor = 0;

        let effects = m.switch_tab(1);
        assert_eq!(m.active_tab, TAB_REVIEWED);
        assert_eq!(m.cursor, 1, "cursor = first reviewed PR");
        assert!(!effects.is_empty());

        m.switch_tab(1);
        m.switch_tab(1);
        m.switch_tab(1);
        assert_eq!(
            m.active_tab, TAB_CLOSED,
            "activeTab should clamp at TAB_CLOSED"
        );
    }

    fn history_pr(url: &str, state: &str) -> PullRequest {
        PullRequest {
            url: url.to_string(),
            state: state.to_string(),
            ..Default::default()
        }
    }

    fn history_msg(state: HistoryState, prs: Vec<PullRequest>) -> Msg {
        Msg::HistoryList {
            state,
            result: Ok(prs),
        }
    }

    fn has_load_history(effects: &[Effect], want: HistoryState) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHistory { state } if *state == want))
    }

    #[test]
    fn test_history_prs_go_to_merged_and_closed_tabs() {
        let mut m = model_with_mixed_review_state();
        m.update(history_msg(
            HistoryState::Merged,
            vec![
                history_pr("https://example.test/pr/10", "MERGED"),
                history_pr("https://example.test/pr/12", "MERGED"),
            ],
        ));
        m.update(history_msg(
            HistoryState::Closed,
            vec![history_pr("https://example.test/pr/11", "CLOSED")],
        ));
        assert!(m.history_loaded[TAB_MERGED] && m.history_loaded[TAB_CLOSED]);
        assert_eq!(m.tab_counts(), [2, 1, 2, 1]);
        m.active_tab = TAB_MERGED;
        assert_eq!(m.matching_indices(), vec![3, 4]);
        m.active_tab = TAB_CLOSED;
        assert_eq!(m.matching_indices(), vec![5]);
    }

    #[test]
    fn test_history_list_replaces_only_its_state() {
        let mut m = model_with_mixed_review_state();
        m.update(history_msg(
            HistoryState::Closed,
            vec![history_pr("https://example.test/pr/11", "CLOSED")],
        ));
        m.update(history_msg(
            HistoryState::Merged,
            vec![history_pr("https://example.test/pr/10", "MERGED")],
        ));
        m.update(history_msg(
            HistoryState::Merged,
            vec![
                history_pr("https://example.test/pr/12", "MERGED"),
                history_pr("https://example.test/pr/13", "CLOSED"),
            ],
        ));
        assert_eq!(m.tab_counts(), [2, 1, 1, 1]);
        assert!(index_of_pr_url(&m.prs, "https://example.test/pr/10").is_none());
        assert!(index_of_pr_url(&m.prs, "https://example.test/pr/13").is_none());
    }

    #[test]
    fn test_history_list_ignores_open_prs_and_duplicates() {
        let mut m = model_with_mixed_review_state();
        m.update(history_msg(
            HistoryState::Merged,
            vec![
                history_pr("https://example.test/pr/10", "OPEN"),
                history_pr("https://example.test/pr/1", "MERGED"),
            ],
        ));
        assert_eq!(m.prs.len(), 3, "open PRs win over stale history entries");
        assert_eq!(m.tab_counts(), [2, 1, 0, 0]);
    }

    #[test]
    fn test_history_is_not_loaded_until_tab_is_visited() {
        let mut m = model_with_mixed_review_state();
        let effects = m.switch_tab(1);
        assert!(!effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHistory { .. })));

        let effects = m.switch_tab(1);
        assert_eq!(m.active_tab, TAB_MERGED);
        assert!(has_load_history(&effects, HistoryState::Merged));
        assert!(!has_load_history(&effects, HistoryState::Closed));
        assert!(m.history_loading[TAB_MERGED]);
        assert_eq!(m.empty_list_message(), "Loading merged PRs...");

        let effects = m.switch_tab(1);
        assert_eq!(m.active_tab, TAB_CLOSED);
        assert!(has_load_history(&effects, HistoryState::Closed));
    }

    #[test]
    fn test_history_tab_is_fetched_once() {
        let mut m = model_with_mixed_review_state();
        m.active_tab = TAB_REVIEWED;
        m.switch_tab(1);
        m.update(history_msg(
            HistoryState::Merged,
            vec![history_pr("https://example.test/pr/10", "MERGED")],
        ));
        assert_eq!(m.cursor, 3, "cursor = first merged PR after load");
        m.switch_tab(-1);
        let effects = m.switch_tab(1);
        assert!(!has_load_history(&effects, HistoryState::Merged));
    }

    #[test]
    fn test_history_tab_retries_after_error() {
        let mut m = model_with_mixed_review_state();
        m.active_tab = TAB_REVIEWED;
        m.switch_tab(1);
        m.update(Msg::HistoryList {
            state: HistoryState::Merged,
            result: Err("boom".into()),
        });
        assert!(!m.history_loading[TAB_MERGED]);
        assert_eq!(m.status, "failed to load merged PRs");
        m.switch_tab(-1);
        let effects = m.switch_tab(1);
        assert!(has_load_history(&effects, HistoryState::Merged));
    }

    #[test]
    fn test_pr_list_reload_keeps_history() {
        let mut m = model_with_mixed_review_state();
        m.update(history_msg(
            HistoryState::Merged,
            vec![history_pr("https://example.test/pr/10", "MERGED")],
        ));
        m.update(Msg::PRList(Ok(vec![pr("https://example.test/pr/1")])));
        assert_eq!(m.tab_counts(), [1, 0, 1, 0]);
        assert_eq!(m.status, "1 review request(s)");
        assert_eq!(m.pr_signature, pr_list_signature(&m.open_prs()));
    }

    #[test]
    fn test_update_check_keeps_history_and_counts_open_only() {
        let mut m = new_model();
        let prs = sig_prs(&[("https://example.test/pr/1", 1747200000)]);
        m.prs = prs.clone();
        m.pr_signature = pr_list_signature(&prs);
        m.update(history_msg(
            HistoryState::Merged,
            vec![history_pr("https://example.test/pr/10", "MERGED")],
        ));

        let mut new_prs = prs.clone();
        new_prs.push(pr("https://example.test/pr/2"));
        m.update(Msg::UpdateCheck {
            prev_sig: m.pr_signature.clone(),
            prev_count: 1,
            result: Ok(new_prs),
        });
        assert_eq!(m.update_notice.as_ref().map(|n| n.count), Some(2));
        assert!(!m.marked_prs.contains("https://example.test/pr/10"));
        assert_eq!(m.tab_counts(), [2, 0, 1, 0]);
    }

    #[test]
    fn test_first_history_load_does_not_move_cursor_off_open_tab() {
        let mut m = new_model();
        m.loading = false;
        m.update(history_msg(
            HistoryState::Merged,
            vec![history_pr("https://example.test/pr/10", "MERGED")],
        ));
        m.update(Msg::PRList(Ok(vec![
            pr("https://example.test/pr/1"),
            pr("https://example.test/pr/2"),
        ])));
        assert_eq!(m.cursor, 0, "cursor = first awaiting PR");
    }

    #[test]
    fn test_approve_is_blocked_for_history_pr() {
        let mut m = model_with_loaded_detail();
        if let Some(d) = m.current_detail.as_mut() {
            d.base.state = "MERGED".into();
        }
        let effects = m.update(key(crossterm::event::KeyCode::Char('a')));
        assert!(effects.is_empty());
        assert!(m.pending_approve.is_none());
    }

    #[test]
    fn test_tab_bar_shows_merged_and_closed() {
        let mut m = model_with_mixed_review_state();
        let text = |m: &Model| -> String {
            let backend = ratatui::backend::TestBackend::new(80, 1);
            let mut term = ratatui::Terminal::new(backend).unwrap();
            term.draw(|f| f.render_widget(render_tab_bar(m), f.area()))
                .unwrap();
            let buf = term.backend().buffer().clone();
            buf.content().iter().map(|c| c.symbol()).collect()
        };
        let got = text(&m);
        assert!(got.contains(" Merged   Closed "), "{got}");
        m.history_loading[TAB_MERGED] = true;
        assert!(text(&m).contains(" Merged … "));
        m.update(history_msg(
            HistoryState::Closed,
            vec![history_pr("https://example.test/pr/10", "CLOSED")],
        ));
        let got = text(&m);
        assert!(got.contains(" Awaiting Review 2 "), "{got}");
        assert!(got.contains(" Reviewed 1 "), "{got}");
        assert!(got.contains(" Merged … "), "{got}");
        assert!(got.contains(" Closed 1 "), "{got}");
    }

    #[test]
    fn test_refresh_reloads_active_history_tab_and_marks_other_stale() {
        let mut m = model_with_mixed_review_state();
        m.update(history_msg(HistoryState::Merged, Vec::new()));
        m.update(history_msg(HistoryState::Closed, Vec::new()));
        m.active_tab = TAB_MERGED;

        let effects = m.update(key(crossterm::event::KeyCode::Char('r')));
        assert!(effects.iter().any(|e| matches!(e, Effect::LoadPRs)));
        assert!(has_load_history(&effects, HistoryState::Merged));
        assert!(!has_load_history(&effects, HistoryState::Closed));
        assert!(m.history_stale[TAB_CLOSED]);

        m.update(Msg::PRList(Ok(m.open_prs())));
        let effects = m.switch_tab(1);
        assert!(has_load_history(&effects, HistoryState::Closed));
    }

    #[test]
    fn test_refresh_on_open_tab_does_not_fetch_history() {
        let mut m = new_model();
        m.loading = false;
        let effects = m.update(key(crossterm::event::KeyCode::Char('r')));
        assert!(effects.iter().any(|e| matches!(e, Effect::LoadPRs)));
        assert!(!effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHistory { .. })));
    }

    #[test]
    fn test_list_shows_at_most_max_list_items() {
        let mut m = new_model();
        m.width = 120;
        m.height = 40;
        m.loading = false;
        for i in 0..MAX_LIST_ITEMS + 5 {
            m.prs.push(PullRequest {
                repository: "owner/repo".into(),
                number: i as u64 + 1,
                title: "title".into(),
                url: format!("https://example.test/pr/{}", i + 1),
                author: "alice".into(),
                request: "@me".into(),
                ..Default::default()
            });
        }
        let visible = m.visible_pr_indices();
        assert_eq!(visible.len(), MAX_LIST_ITEMS);
        assert_eq!(visible[0], 0);
        assert_eq!(visible[MAX_LIST_ITEMS - 1], MAX_LIST_ITEMS - 1);
    }

    #[test]
    fn test_ctrl_n_scrolls_beyond_max_list_items() {
        let mut m = new_model();
        m.width = 120;
        m.height = 40;
        m.loading = false;
        let total = MAX_LIST_ITEMS + 3;
        for i in 0..total {
            m.prs.push(PullRequest {
                repository: "owner/repo".into(),
                number: i as u64 + 1,
                title: "title".into(),
                url: format!("https://example.test/pr/{}", i + 1),
                author: "alice".into(),
                request: "@me".into(),
                ..Default::default()
            });
        }

        for _ in 0..MAX_LIST_ITEMS {
            m.update(ctrl_key(crossterm::event::KeyCode::Char('n')));
        }
        assert_eq!(m.cursor, MAX_LIST_ITEMS);
        assert_eq!(m.list_offset, 1);
        let visible = m.visible_pr_indices();
        assert_eq!(visible[0], 1);
        // index 10 corresponds to PR #11, matching the Go test's expectation.
        assert_eq!(*visible.last().unwrap(), MAX_LIST_ITEMS);
    }

    #[test]
    fn test_render_diff_content_shows_reviewed_by() {
        let detail = PullRequestDetail {
            base: PullRequest {
                repository: "owner/repo".into(),
                number: 42,
                title: "Add reviewer header".into(),
                author: "octocat".into(),
                request: "@me".into(),
                ..Default::default()
            },
            reviewers: vec![
                crate::gh::ReviewSummary {
                    author: "alice".into(),
                    state: "APPROVED".into(),
                },
                crate::gh::ReviewSummary {
                    author: "bob".into(),
                    state: "CHANGES_REQUESTED".into(),
                },
            ],
            ..Default::default()
        };

        let out = render_diff_content(&detail, "", 80);
        let text: Vec<String> = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect();
        let joined = text.join("\n");
        for want in [
            "Reviewed by:",
            "@alice",
            "APPROVED",
            "@bob",
            "CHANGES_REQUESTED",
        ] {
            assert!(joined.contains(want), "rendered detail missing {want}");
        }
    }

    #[test]
    fn test_wrap_long_word_not_duplicated() {
        let lines = render_markdown("hello waaaaaaaaaaaaaaytoolongword trailing", 10);
        let texts: Vec<String> = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(
            texts,
            vec!["hello", "waaaaaaaaa", "aaaaaytool", "ongword", "trailing"]
        );
    }

    #[test]
    fn test_match_summary_recomputed_after_pr_list_update() {
        let mut m = new_model();
        m.prs = vec![pr("https://example.test/pr/1")];
        assert_eq!(m.tab_counts(), [1, 0, 0, 0]);

        let mut updated = pr("https://example.test/pr/2");
        updated.review_decision = "APPROVED".into();
        m.update(Msg::PRList(Ok(vec![updated])));
        assert_eq!(
            m.tab_counts(),
            [0, 1, 0, 0],
            "cached counts must be invalidated"
        );
        m.active_tab = TAB_REVIEWED;
        assert_eq!(m.matching_indices(), vec![0]);
    }

    #[test]
    fn test_match_summary_recomputed_after_approve() {
        let mut m = model_with_mixed_review_state();
        assert_eq!(m.tab_counts(), [2, 1, 0, 0]);

        m.update(Msg::ApproveDone {
            pr: m.prs[0].clone(),
            err: None,
        });
        assert_eq!(
            m.tab_counts(),
            [1, 2, 0, 0],
            "approve must refresh tab counts"
        );
    }

    #[test]
    fn test_detail_scroll_estimate_matches_wrapped_rows() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::prelude::Widget;

        fn wrapped_rows(text: &str, w: usize) -> usize {
            let mut buf = Buffer::empty(Rect::new(0, 0, w as u16, 500));
            Paragraph::new(text.to_string())
                .wrap(Wrap { trim: false })
                .render(Rect::new(0, 0, w as u16, 500), &mut buf);
            let mut last = 1;
            for y in 0..500 {
                for x in 0..w {
                    if buf[(x as u16, y as u16)].symbol() != " " {
                        last = y + 1;
                        break;
                    }
                }
            }
            last
        }

        let detail = PullRequestDetail {
            base: pr("https://example.test/pr/1"),
            ..Default::default()
        };
        // Prose-like removed lines are the worst case for word wrapping.
        let diff = ["aaaaaa aaaaaa aaaaaa aaaaaa aaaaaa aaaaaa aaaaaa aaaaaaa"; 4].join("\n");
        let width = 40usize;
        let lines = render_diff_content(&detail, &diff, width);

        let estimate: usize = lines
            .iter()
            .map(|l| {
                let lw = l.width();
                if lw == 0 {
                    1
                } else {
                    lw.div_ceil(width)
                }
            })
            .sum();

        let actual: usize = lines
            .iter()
            .map(|l| {
                let text: String = l
                    .spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>();
                if text.is_empty() {
                    1
                } else {
                    wrapped_rows(&text, width)
                }
            })
            .sum();

        assert_eq!(estimate, actual, "scroll estimate must equal rendered rows");
        // Also ensure every produced line fits the content width so Paragraph
        // cannot re-wrap any of it.
        for l in &lines {
            assert!(l.width() <= width);
        }
    }

    #[test]
    fn test_wrap_line_spans_preserves_styles() {
        let line = Line::from(vec![
            Span::styled("Author:", fg(colors::YELLOW)),
            Span::styled(" aaaaaaaa aaaaaaaa aaaaaaaa aaaaaaaa", fg(colors::FG)),
        ]);
        let width = 20usize;
        let out = wrap_line_spans(line, width);
        assert!(out.len() > 1);
        for l in &out {
            assert!(l.width() <= width);
        }
        let joined: String = out
            .iter()
            .flat_map(|l| {
                l.spans
                    .iter()
                    .flat_map(|s| s.content.chars().collect::<Vec<char>>())
            })
            .collect();
        assert!(joined.contains("Author:"));
    }

    #[test]
    fn test_detail_viewport_zero_when_list_fills_screen() {
        let mut m = new_model();
        m.width = 80;
        m.height = 10;
        m.loading = false;
        for i in 0..12 {
            m.prs.push(pr(&format!("https://example.test/pr/{i}")));
        }
        assert_eq!(m.compute_list_section_height(), 4 + MAX_LIST_ITEMS);
        assert_eq!(
            m.detail_viewport_height(),
            0,
            "no viewport when the list consumes the screen"
        );
        m.detail_total = 50;
        m.scroll_detail(10);
        m.scroll_detail(-10);
        assert_eq!(
            m.detail_scroll, 0,
            "j/k must not scroll while the detail is not visible"
        );
    }

    #[test]
    fn test_detail_viewport_matches_render_allocation() {
        // Mirror the layout arithmetic of render() exactly.
        fn render_detail_rows(m: &Model) -> u16 {
            let h = m.height;
            let search_h = if m.search_visible() { 1u16 } else { 0 };
            let list_h = m.compute_list_section_height() as u16;
            let y0 = search_h;
            let list_used = list_h.min(h.saturating_sub(y0 + 1));
            let y = y0 + list_used;
            let footer_y = h.saturating_sub(1);
            if y < footer_y {
                let detail_rect_h = (m.detail_viewport_height() + 2).min(footer_y - y);
                if detail_rect_h > 2 {
                    return detail_rect_h - 2;
                }
            }
            0
        }

        let sizes = [5u16, 6, 8, 10, 12, 14, 17, 18, 19, 20, 40];
        let counts = [0usize, 3, 30];
        for count in counts {
            for h in sizes {
                let mut m = new_model();
                m.width = 80;
                m.height = h;
                m.loading = false;
                for i in 0..count {
                    m.prs.push(pr(&format!("https://example.test/pr/{i}")));
                }
                assert_eq!(
                    m.detail_viewport_height(),
                    render_detail_rows(&m),
                    "viewport mismatch for h={h} prs={count}"
                );
            }
        }
    }

    fn flatten(l: &Line<'static>) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn test_render_markdown_ordered_list_items() {
        let lines = render_markdown("1. first\n2. second\n10) third", 40);
        let text: Vec<String> = lines.iter().map(flatten).collect();
        assert!(text.iter().any(|t| t.contains("first")), "{text:?}");
        assert!(text
            .iter()
            .any(|t| t.starts_with("2. ") && t.contains("second")));
        assert!(text
            .iter()
            .any(|t| t.starts_with("10) ") && t.contains("third")));
        // Items stay on separate lines instead of merging into one paragraph.
        assert!(
            !text
                .iter()
                .any(|t| t.contains("first") && t.contains("second")),
            "items must not join: {text:?}"
        );
    }

    #[test]
    fn test_render_markdown_list_continuation_indents_under_item() {
        // Indented item: bullet at 2 cols, content at 4; continuation aligns.
        let lines = render_markdown("  - item\n  indented tail\n- next", 40);
        let text: Vec<String> = lines.iter().map(flatten).collect();
        assert!(text
            .iter()
            .any(|t| t.starts_with("  • ") && t.contains("item")));
        let cont = text.iter().find(|t| t.contains("indented tail")).unwrap();
        assert!(
            cont.starts_with("    "),
            "continuation must align under item content: {cont:?}"
        );
    }

    #[test]
    fn test_render_markdown_nested_quote_and_bullet() {
        let lines = render_markdown("> outer\n> > inner\n  - inner bullet", 40);
        let text: Vec<String> = lines.iter().map(flatten).collect();
        assert!(
            text.iter()
                .any(|t| t.starts_with("│ │ ") && t.contains("inner")),
            "nested quote must repeat the marker: {text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.starts_with("│ ") && !t.starts_with("│ │ ")),
            "outer quote must render a single marker: {text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.starts_with("  • ") && t.contains("inner bullet")),
            "nested bullet must indent one level: {text:?}"
        );
    }

    #[test]
    fn test_highlight_diff_strips_ansi_and_styles_lines() {
        // Cached entries from the --color=always era carry ANSI escapes.
        let diff = "\x1b[1mdiff --git a/f b/f\x1b[m\n\x1b[32m+added\x1b[m\n\x1b[31m-removed\x1b[m\n context";
        let lines = highlight_diff(diff);
        let texts: Vec<String> = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(
            texts,
            vec!["diff --git a/f b/f", "+added", "-removed", " context"]
        );
        assert_eq!(lines[1].spans[0].style.fg, Some(c(colors::GREEN)));
        assert_eq!(lines[2].spans[0].style.fg, Some(c(colors::RED)));
    }
}
