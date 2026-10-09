use unicode_width::UnicodeWidthChar;

/// Multi-line plain-text buffer with a cursor counted in chars.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextArea {
    text: String,
    cursor: usize,
}

impl TextArea {
    pub fn new(text: &str) -> Self {
        TextArea {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.text.len())
    }

    pub fn insert(&mut self, ch: char) {
        let b = self.byte_at(self.cursor);
        self.text.insert(b, ch);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_at(self.cursor - 1);
        let end = self.byte_at(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let start = self.byte_at(self.cursor);
        let end = self.byte_at(self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// (line index, column in chars) of the cursor in logical lines.
    fn line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for ch in self.text.chars().take(self.cursor) {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// Char index of the start of each logical line, and each line's length.
    fn lines(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut len = 0;
        for ch in self.text.chars() {
            if ch == '\n' {
                out.push((start, len));
                start += len + 1;
                len = 0;
            } else {
                len += 1;
            }
        }
        out.push((start, len));
        out
    }

    pub fn home(&mut self) {
        let (line, _) = self.line_col();
        self.cursor = self.lines()[line].0;
    }

    pub fn end(&mut self) {
        let (line, _) = self.line_col();
        let (start, len) = self.lines()[line];
        self.cursor = start + len;
    }

    pub fn up(&mut self) {
        self.move_line(-1);
    }

    pub fn down(&mut self) {
        self.move_line(1);
    }

    fn move_line(&mut self, delta: i64) {
        let (line, col) = self.line_col();
        let lines = self.lines();
        let next = line as i64 + delta;
        if next < 0 || next >= lines.len() as i64 {
            return;
        }
        let (start, len) = lines[next as usize];
        self.cursor = start + col.min(len);
    }

    /// Wraps the text into rows of at most `width` display columns (wide
    /// chars such as Japanese count as 2) and returns the rows plus the
    /// cursor's (row, column) in display cells.
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut rows = vec![String::new()];
        let mut row_w = 0;
        let mut cur = (0, 0);
        for (i, ch) in self.text.chars().enumerate() {
            if i == self.cursor {
                cur = (rows.len() - 1, row_w);
            }
            if ch == '\n' {
                rows.push(String::new());
                row_w = 0;
                continue;
            }
            let cw = ch.width().unwrap_or(0);
            if row_w + cw > width {
                rows.push(String::new());
                row_w = 0;
                if i == self.cursor {
                    cur = (rows.len() - 1, 0);
                }
            }
            rows.last_mut().expect("rows is non-empty").push(ch);
            row_w += cw;
        }
        if self.cursor >= self.char_count() {
            // Keep the cursor cell inside the box.
            if row_w >= width {
                rows.push(String::new());
                row_w = 0;
            }
            cur = (rows.len() - 1, row_w);
        }
        (rows, cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> TextArea {
        let mut t = TextArea::default();
        for ch in s.chars() {
            t.insert(ch);
        }
        t
    }

    #[test]
    fn insert_and_backspace_multibyte() {
        let mut t = typed("日本語");
        assert_eq!(t.text(), "日本語");
        t.backspace();
        assert_eq!(t.text(), "日本");
        t.left();
        t.insert('x');
        assert_eq!(t.text(), "日x本");
        t.delete();
        assert_eq!(t.text(), "日x");
    }

    #[test]
    fn up_down_home_end() {
        let mut t = typed("abc\nde\nfghij");
        t.up();
        assert_eq!(t.cursor(), 6); // end of "de" (col clamped)
        t.up();
        assert_eq!(t.cursor(), 2);
        t.home();
        assert_eq!(t.cursor(), 0);
        t.end();
        assert_eq!(t.cursor(), 3);
        t.down();
        assert_eq!(t.cursor(), 6); // col 3 clamped to end of "de"
        t.down();
        assert_eq!(t.cursor(), 9); // col 2 of "fghij"
        t.down();
        assert_eq!(t.cursor(), 9);
    }

    #[test]
    fn layout_wraps_wide_chars() {
        let t = typed("あいうえお");
        let (rows, cur) = t.layout(4);
        assert_eq!(rows, vec!["あい", "うえ", "お"]);
        assert_eq!(cur, (2, 2));
    }

    #[test]
    fn layout_cursor_mid_text_and_newline() {
        let mut t = typed("ab\ncd");
        t.left();
        t.left();
        t.left();
        let (rows, cur) = t.layout(10);
        assert_eq!(rows, vec!["ab", "cd"]);
        assert_eq!(cur, (0, 2));
    }

    #[test]
    fn layout_cursor_at_full_row_moves_to_next_row() {
        let t = typed("abcd");
        let (rows, cur) = t.layout(4);
        assert_eq!(rows, vec!["abcd", ""]);
        assert_eq!(cur, (1, 0));
    }
}
