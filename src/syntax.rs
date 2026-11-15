//! Syntax highlighting for diff code lines. Only syntect's parser is used:
//! scopes map onto the active theme's palette slots, so highlighted code
//! follows the configured theme instead of a separate syntect color theme.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::theme::Slot;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Scope prefixes and the slot they paint, checked innermost scope first.
fn scope_slots() -> &'static [(Scope, Slot)] {
    static TABLE: OnceLock<Vec<(Scope, Slot)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        [
            ("comment", Slot::Muted),
            ("string", Slot::Green),
            ("constant.numeric", Slot::Orange),
            ("constant.language", Slot::Orange),
            ("constant.character", Slot::Orange),
            ("keyword", Slot::Magenta),
            ("storage", Slot::Magenta),
            ("entity.name.function", Slot::Blue),
            ("support.function", Slot::Blue),
            ("entity.name", Slot::Cyan),
            ("support.type", Slot::Cyan),
            ("support.class", Slot::Cyan),
        ]
        .into_iter()
        .filter_map(|(s, slot)| Scope::new(s).ok().map(|s| (s, slot)))
        .collect()
    })
}

fn slot_of(stack: &ScopeStack) -> Option<Slot> {
    stack.as_slice().iter().rev().find_map(|&scope| {
        scope_slots()
            .iter()
            .find(|(prefix, _)| prefix.is_prefix_of(scope))
            .map(|&(_, slot)| slot)
    })
}

fn syntax_for_path(path: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.rsplit('/').next().unwrap_or(path);
    // Whole file names first: syntaxes list "Makefile", "Dockerfile", ...
    // among their extensions.
    set.find_syntax_by_extension(name).or_else(|| {
        let (_, ext) = name.rsplit_once('.')?;
        set.find_syntax_by_extension(ext)
    })
}

/// Parses consecutive code lines of one file. Removed and added lines share
/// one parse state per hunk.
// ponytail: a construct opened on a removed line (e.g. a block comment)
// bleeds into the following added lines; delta-style separate old/new
// states would fix it at twice the parse cost.
#[derive(Default)]
pub struct Highlighter {
    syntax: Option<&'static SyntaxReference>,
    state: Option<(ParseState, ScopeStack)>,
}

impl Highlighter {
    /// Switches to the file at `path`; unknown file types stay plain.
    pub fn set_file(&mut self, path: &str) {
        self.syntax = syntax_for_path(path);
        self.reset();
    }

    /// Restarts parsing, e.g. at a hunk boundary where the skipped lines
    /// leave the previous state meaningless.
    pub fn reset(&mut self) {
        self.state = self.syntax.map(|s| (ParseState::new(s), ScopeStack::new()));
    }

    /// Splits `code` into runs tagged with a palette slot (None: default
    /// color). None when the file type is unknown or parsing failed, in which
    /// case the rest of the file stays plain.
    pub fn highlight(&mut self, code: &str) -> Option<Vec<(String, Option<Slot>)>> {
        let (parser, stack) = self.state.as_mut()?;
        let line = format!("{code}\n");
        let Ok(ops) = parser.parse_line(&line, syntaxes()) else {
            self.state = None;
            return None;
        };
        let mut runs: Vec<(String, Option<Slot>)> = Vec::new();
        let mut push = |text: &str, slot: Option<Slot>| {
            if text.is_empty() {
                return;
            }
            match runs.last_mut() {
                Some((prev, prev_slot)) if *prev_slot == slot => prev.push_str(text),
                _ => runs.push((text.to_string(), slot)),
            }
        };
        let mut pos = 0;
        for (at, op) in ops {
            let at = at.min(code.len());
            if at > pos {
                push(&code[pos..at], slot_of(stack));
                pos = at;
            }
            if stack.apply(&op).is_err() {
                self.state = None;
                return None;
            }
        }
        push(&code[pos..], slot_of(stack));
        Some(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_by_extension_and_keeps_text() {
        let mut h = Highlighter::default();
        h.set_file("src/main.rs");
        let code = r#"fn main() { let s = "hi"; } // done"#;
        let runs = h.highlight(code).expect("rust is known");
        let joined: String = runs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, code);
        let slot = |needle: &str| runs.iter().find(|(t, _)| t.contains(needle)).unwrap().1;
        assert_eq!(slot("fn"), Some(Slot::Magenta));
        assert_eq!(slot("\"hi\""), Some(Slot::Green));
        assert_eq!(slot("// done"), Some(Slot::Muted));
    }

    #[test]
    fn unknown_file_type_is_plain() {
        let mut h = Highlighter::default();
        h.set_file("notes.unknown-ext");
        assert!(h.highlight("anything").is_none());
    }

    #[test]
    fn makefile_is_found_by_name() {
        assert!(syntax_for_path("sub/Makefile").is_some());
    }
}
