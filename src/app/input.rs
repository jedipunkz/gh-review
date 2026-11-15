use super::*;

impl Model {
    pub(super) fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        if self.search_active {
            return self.handle_search_key(key);
        }
        self.handle_normal_key(key)
    }

    pub(super) fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        if self.update_notice.is_some() {
            self.update_notice = None;
        }
        if self.pending_approve.is_some() {
            return self.handle_approve_confirmation(key);
        }
        if self.comment_editor.is_some() {
            return self.handle_comment_key(key);
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
            crossterm::event::KeyCode::Char('n') if ctrl => self.move_cursor(1),
            crossterm::event::KeyCode::Char('p') if ctrl => self.move_cursor(-1),
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
            crossterm::event::KeyCode::Char('c') if !ctrl => {
                if self.current_detail.is_some() && !self.loading && !self.detail_loading {
                    let pr = match self.current_detail.as_ref() {
                        Some(d) => d.base.clone(),
                        None => return Vec::new(),
                    };
                    if is_history(&pr) {
                        self.status = "cannot comment on a merged or closed PR".to_string();
                        return Vec::new();
                    }
                    let draft = self
                        .comment_drafts
                        .get(&pr.url)
                        .cloned()
                        .unwrap_or_default();
                    self.err = String::new();
                    self.comment_editor = Some(CommentEditor {
                        pr,
                        area: TextArea::new(&draft),
                    });
                    self.status = "ctrl+enter (or ctrl+s) to send, esc to cancel".to_string();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn move_cursor(&mut self, delta: i32) -> Vec<Effect> {
        if self.loading {
            return Vec::new();
        }
        if self.advance_cursor(delta) {
            self.ensure_cursor_visible();
            return self.detail_and_prefetch_effects();
        }
        Vec::new()
    }

    /// Wheel over the list moves the cursor, wheel elsewhere scrolls the
    /// detail, and a left click on a list row selects it. Row geometry
    /// mirrors render(): optional search line, tab bar, top border, header.
    pub(super) fn handle_mouse(&mut self, ev: crossterm::event::MouseEvent) -> Vec<Effect> {
        use crossterm::event::{MouseButton, MouseEventKind};
        if self.search_active || self.pending_approve.is_some() || self.comment_editor.is_some() {
            return Vec::new();
        }
        let search_h = u16::from(self.search_visible());
        let list_h = (self.compute_list_section_height() as u16)
            .min(self.height.saturating_sub(search_h + 1));
        let in_list = ev.row < search_h + list_h;
        match ev.kind {
            MouseEventKind::ScrollDown if in_list => self.move_cursor(1),
            MouseEventKind::ScrollUp if in_list => self.move_cursor(-1),
            MouseEventKind::ScrollDown => {
                self.scroll_detail(MOUSE_SCROLL_LINES);
                Vec::new()
            }
            MouseEventKind::ScrollUp => {
                self.scroll_detail(-MOUSE_SCROLL_LINES);
                Vec::new()
            }
            MouseEventKind::Down(MouseButton::Left) if in_list && !self.loading => {
                let first_row = search_h + 3;
                let visible = self.visible_pr_indices();
                let hit = ev
                    .row
                    .checked_sub(first_row)
                    .and_then(|i| visible.get(i as usize).copied());
                match hit {
                    Some(idx) if idx != self.cursor => {
                        self.clear_mark_on_selected();
                        self.cursor = idx;
                        self.ensure_cursor_visible();
                        self.detail_and_prefetch_effects()
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn scroll_detail(&mut self, delta: i32) {
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

    pub(super) fn handle_search_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        // Emacs-style editing keys map onto the keys that already edit.
        let code = match key.code {
            KeyCode::Char('h') if ctrl => KeyCode::Backspace,
            KeyCode::Char('b') if ctrl => KeyCode::Left,
            KeyCode::Char('f') if ctrl => KeyCode::Right,
            KeyCode::Char('a') if ctrl => KeyCode::Home,
            KeyCode::Char('e') if ctrl => KeyCode::End,
            other => other,
        };
        match code {
            crossterm::event::KeyCode::Char('c') if ctrl => {
                self.quit = true;
                Vec::new()
            }
            crossterm::event::KeyCode::Esc => {
                let prev_cursor = self.cursor;
                self.search_active = false;
                self.search_value.clear();
                self.search_cursor = 0;
                self.refresh_after_filter_change(prev_cursor)
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
                self.refresh_after_filter_change(prev_cursor)
            }
            crossterm::event::KeyCode::Delete => {
                let prev_cursor = self.cursor;
                if self.search_cursor < self.search_value.chars().count() {
                    let byte_idx = self.char_to_byte(self.search_cursor);
                    let end = self.char_to_byte(self.search_cursor + 1);
                    self.search_value.replace_range(byte_idx..end, "");
                }
                self.refresh_after_filter_change(prev_cursor)
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
                self.refresh_after_filter_change(prev_cursor)
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn char_to_byte(&self, char_idx: usize) -> usize {
        self.search_value
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.search_value.len())
    }

    pub(super) fn refresh_after_filter_change(&mut self, prev_cursor: usize) -> Vec<Effect> {
        self.ensure_cursor_visible();
        if self.matching_indices().is_empty() {
            self.clear_detail();
            return Vec::new();
        }
        if self.cursor != prev_cursor && !self.loading && !self.prs.is_empty() {
            return self.detail_and_prefetch_effects();
        }
        Vec::new()
    }

    pub(super) fn handle_comment_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let editor = match self.comment_editor.as_mut() {
            Some(e) => e,
            None => return Vec::new(),
        };
        match key.code {
            KeyCode::Enter if ctrl => self.submit_comment(),
            KeyCode::Char('s') if ctrl => self.submit_comment(),
            KeyCode::Char('c') if ctrl => {
                self.quit = true;
                Vec::new()
            }
            KeyCode::Esc => {
                if let Some(e) = self.comment_editor.take() {
                    if e.area.text().trim().is_empty() {
                        self.comment_drafts.remove(&e.pr.url);
                    } else {
                        self.comment_drafts
                            .insert(e.pr.url.clone(), e.area.text().to_string());
                    }
                }
                self.status = "comment canceled (draft kept)".to_string();
                Vec::new()
            }
            KeyCode::Enter => {
                editor.area.insert('\n');
                Vec::new()
            }
            KeyCode::Backspace => {
                editor.area.backspace();
                Vec::new()
            }
            KeyCode::Delete => {
                editor.area.delete();
                Vec::new()
            }
            KeyCode::Left => {
                editor.area.left();
                Vec::new()
            }
            KeyCode::Right => {
                editor.area.right();
                Vec::new()
            }
            KeyCode::Up => {
                editor.area.up();
                Vec::new()
            }
            KeyCode::Down => {
                editor.area.down();
                Vec::new()
            }
            KeyCode::Home => {
                editor.area.home();
                Vec::new()
            }
            KeyCode::End => {
                editor.area.end();
                Vec::new()
            }
            KeyCode::Char(ch) if !ctrl && !alt => {
                editor.area.insert(ch);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn submit_comment(&mut self) -> Vec<Effect> {
        let body = match self.comment_editor.as_ref() {
            Some(e) => e.area.text().to_string(),
            None => return Vec::new(),
        };
        if body.trim().is_empty() {
            self.status = "comment is empty".to_string();
            return Vec::new();
        }
        let pr = match self.comment_editor.take() {
            Some(e) => e.pr,
            None => return Vec::new(),
        };
        self.loading = true;
        self.status = "sending comment...".to_string();
        self.err = String::new();
        vec![Effect::Comment { pr, body }]
    }

    pub(super) fn handle_approve_confirmation(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> Vec<Effect> {
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

    pub(super) fn switch_tab(&mut self, delta: i32) -> Vec<Effect> {
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
}
