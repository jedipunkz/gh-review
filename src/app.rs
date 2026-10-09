use std::collections::HashSet;
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[allow(unused_imports)]
use crate::cache::CacheEntry;
use crate::cache::DetailCache;
use crate::gh::{self, PullRequest, PullRequestDetail};
use crate::prefetch::{neighbor_prs, top_n, Prefetcher};

pub const MAX_LIST_ITEMS: usize = 10;
pub const TAB_AWAITING: usize = 0;
pub const TAB_REVIEWED: usize = 1;
pub const TAB_COUNT: usize = 2;

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

#[derive(Debug, Clone)]
pub enum Effect {
    LoadPRs,
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
    pub quit: bool,
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
            quit: false,
        }
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

    pub fn pr_matches_tab(&self, pr: &PullRequest) -> bool {
        if self.active_tab == TAB_REVIEWED {
            self.is_reviewed(pr)
        } else {
            !self.is_reviewed(pr)
        }
    }

    pub fn matching_indices(&self) -> Vec<usize> {
        let q = self.search_query();
        let mut out = Vec::with_capacity(self.prs.len());
        for (i, pr) in self.prs.iter().enumerate() {
            if !self.pr_matches_tab(pr) {
                continue;
            }
            if !pr_matches_query(pr, &q) {
                continue;
            }
            out.push(i);
        }
        out
    }

    pub fn tab_counts(&self) -> (usize, usize) {
        let q = self.search_query();
        let mut awaiting = 0;
        let mut reviewed = 0;
        for pr in &self.prs {
            if !pr_matches_query(pr, &q) {
                continue;
            }
            if self.is_reviewed(pr) {
                reviewed += 1;
            } else {
                awaiting += 1;
            }
        }
        (awaiting, reviewed)
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
        if let Some(entry) = self.cache.get_mem(&key) {
            self.inflight_cancel();
            self.debounce_seq += 1;
            self.current_detail = Some(entry.detail);
            self.current_diff = entry.diff;
            self.detail_loading = false;
            return None;
        }
        if let Some(entry) = self.cache.get_disk(&key) {
            self.inflight_cancel();
            self.debounce_seq += 1;
            self.current_detail = Some(entry.detail);
            self.current_diff = entry.diff;
            self.detail_loading = false;
            return Some(Effect::LoadDiff { pr });
        }
        self.debounce_seq += 1;
        Some(Effect::Debounce {
            seq: self.debounce_seq,
            url: pr.url,
        })
    }

    fn inflight_cancel(&mut self) {
        // The main loop aborts the in-flight load task before spawning a new
        // one; nothing to do here beyond bumping the debounce sequence.
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

    pub fn apply_pr_list(&mut self, prs: Vec<PullRequest>) -> Vec<Effect> {
        self.loading = false;
        self.err = String::new();
        let prev_prs = std::mem::take(&mut self.prs);
        let prev_cursor = self.cursor;
        self.prs = prs;
        self.pr_signature = pr_list_signature(&self.prs);
        self.pr_list_loaded = true;
        self.update_notice = None;
        self.prune_marked_prs();
        self.reconcile_cursor(&prev_prs, prev_cursor);
        self.ensure_cursor_visible();
        self.status = format!("{} review request(s)", self.prs.len());
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
                        prev_count: self.prs.len(),
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
                        self.status = format!("{} review request(s)", self.prs.len());
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
                if self.loading || self.detail_loading {
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
        let new_urls = new_pr_urls(&self.prs, &prs);
        for url in &new_urls {
            self.marked_prs.insert(url.clone());
        }
        let prev_prs = std::mem::take(&mut self.prs);
        let prev_cursor = self.cursor;
        self.prs = prs;
        self.pr_signature = current_sig;
        self.reconcile_cursor(&prev_prs, prev_cursor);
        self.ensure_cursor_visible();
        self.popup_seq += 1;
        self.update_notice = Some(UpdateNotice {
            count: self.prs.len(),
            id: self.popup_seq,
        });
        self.status = format!("{} review request(s)", self.prs.len());
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
                vec![Effect::LoadPRs]
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
        let max = self
            .detail_total
            .saturating_sub(self.detail_viewport_height());
        let cur = self.detail_scroll as i64;
        let next = (cur + delta as i64).clamp(0, max as i64);
        self.detail_scroll = next as u16;
    }

    pub fn detail_viewport_height(&self) -> u16 {
        if self.width == 0 || self.height == 0 {
            return 3;
        }
        let list_h = self.compute_list_section_height() as i32;
        let search_h = if self.search_visible() { 1 } else { 0 };
        (self.height as i32 - 3 - list_h - search_h).max(3) as u16
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
        let matched = self.matching_indices();
        if matched.is_empty() {
            self.current_detail = None;
            self.detail_loading = false;
            self.loading_for_url = None;
            self.detail_err = String::new();
            return Vec::new();
        }
        self.cursor = matched[0];
        self.ensure_cursor_visible();
        self.detail_and_prefetch_effects()
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
        if self.active_tab == TAB_REVIEWED {
            return "No reviewed PRs.";
        }
        "No PRs awaiting your review."
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
    lines
}

fn meta_pair(label: &str, value: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{label}:"), fg(colors::YELLOW)),
        Span::raw(format!(" {value}")),
    ]
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

    for raw in body.lines() {
        let trimmed = raw.trim_end();
        if trimmed.trim_start().starts_with("```") || trimmed.trim_start().starts_with("~~~") {
            flush_paragraph(&mut lines, &mut paragraph);
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(Line::from(Span::styled(raw.to_string(), fg(colors::CYAN))));
            continue;
        }
        let t = trimmed.trim_start();
        if t.is_empty() {
            flush_paragraph(&mut lines, &mut paragraph);
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let level = rest.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&level) {
                flush_paragraph(&mut lines, &mut paragraph);
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
            lines.push(Line::from(Span::styled(
                "─".repeat(width.min(80)),
                fg(colors::MUTED),
            )));
            continue;
        }
        if t.starts_with("- ") || t.starts_with("* ") {
            flush_paragraph(&mut lines, &mut paragraph);
            let content = t
                .strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))
                .unwrap_or(t);
            let indent = width.min(2);
            let avail = width.saturating_sub(indent + 2);
            let mut first = true;
            for l in wrap_line_to_width(content, avail.max(10)) {
                let bullet = if first { "• " } else { "  " };
                let pad = repeat_space(indent);
                lines.push(Line::from(vec![
                    Span::styled(format!("{pad}{bullet}"), fg(colors::CYAN)),
                    Span::styled(l, fg(colors::FG)),
                ]));
                first = false;
            }
            continue;
        }
        if t.starts_with("> ") {
            flush_paragraph(&mut lines, &mut paragraph);
            let content = t.strip_prefix("> ").unwrap_or(t);
            for l in wrap_line_to_width(content, width.saturating_sub(2).max(10)) {
                lines.push(Line::from(vec![
                    Span::styled("│ ".to_string(), fg(colors::MUTED)),
                    Span::styled(l, fg(colors::MUTED)),
                ]));
            }
            continue;
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
        let rect = Rect::new(0, y, area.width, detail_rect_h);
        render_detail_section(m, f, rect, vp_h);
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
    let (awaiting, reviewed) = m.tab_counts();
    let labels = [
        format!("Awaiting Review {awaiting}"),
        format!("Reviewed {reviewed}"),
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

    let built = if m.detail_loading || !m.detail_err.is_empty() {
        None
    } else {
        m.current_detail.as_ref().map(|detail| {
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
            (lines, total.min(u16::MAX as usize) as u16)
        })
    };

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if let Some((lines, total)) = built {
        m.detail_total = total;
        let max_scroll = total.saturating_sub(vp_h);
        if m.detail_scroll > max_scroll {
            m.detail_scroll = max_scroll;
        }
        let para = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((m.detail_scroll, 0));
        f.render_widget(para, inner);
        return;
    }

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
    if m.loading || m.detail_loading {
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
        let (awaiting, reviewed) = m.tab_counts();
        assert_eq!((awaiting, reviewed), (2, 1));
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
        assert_eq!(
            m.active_tab, TAB_REVIEWED,
            "activeTab should clamp at tabReviewed"
        );
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
