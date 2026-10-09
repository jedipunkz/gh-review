use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// A named color role used by the UI. Each theme maps every slot to a hex color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Fg,
    Muted,
    Blue,
    Cyan,
    Green,
    Magenta,
    Orange,
    Red,
    Yellow,
    Selected,
    SelectedFg,
    BarBg,
    Ink,
    Frame,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Palette {
    pub name: &'static str,
    pub fg: &'static str,
    pub muted: &'static str,
    pub blue: &'static str,
    pub cyan: &'static str,
    pub green: &'static str,
    pub magenta: &'static str,
    pub orange: &'static str,
    pub red: &'static str,
    pub yellow: &'static str,
    pub selected: &'static str,
    pub selected_fg: &'static str,
    pub bar_bg: &'static str,
    pub ink: &'static str,
    pub frame: &'static str,
}

impl Palette {
    pub fn hex(&self, slot: Slot) -> &'static str {
        match slot {
            Slot::Fg => self.fg,
            Slot::Muted => self.muted,
            Slot::Blue => self.blue,
            Slot::Cyan => self.cyan,
            Slot::Green => self.green,
            Slot::Magenta => self.magenta,
            Slot::Orange => self.orange,
            Slot::Red => self.red,
            Slot::Yellow => self.yellow,
            Slot::Selected => self.selected,
            Slot::SelectedFg => self.selected_fg,
            Slot::BarBg => self.bar_bg,
            Slot::Ink => self.ink,
            Slot::Frame => self.frame,
        }
    }
}

pub const DEFAULT_THEME: &str = "tokyonight";

pub const TOKYONIGHT: Palette = Palette {
    name: "tokyonight",
    fg: "c0caf5",
    muted: "565f89",
    blue: "7aa2f7",
    cyan: "7dcfff",
    green: "9ece6a",
    magenta: "bb9af7",
    orange: "ff9e64",
    red: "f7768e",
    yellow: "e0af68",
    selected: "3d59a1",
    selected_fg: "ffffff",
    bar_bg: "1f2335",
    ink: "1a1b26",
    frame: "414868",
};

pub const KANAGAWA_WAVE: Palette = Palette {
    name: "kanagawa-wave",
    fg: "dcd7ba",
    muted: "727169",
    blue: "7e9cd8",
    cyan: "7fb4ca",
    green: "98bb6c",
    magenta: "957fb8",
    orange: "ffa066",
    red: "e46876",
    yellow: "e6c384",
    selected: "2d4f67",
    selected_fg: "ffffff",
    bar_bg: "2a2a37",
    ink: "1f1f28",
    frame: "54546d",
};

pub const SOLARIZED: Palette = Palette {
    name: "solarized",
    fg: "93a1a1",
    muted: "586e75",
    blue: "268bd2",
    cyan: "2aa198",
    green: "859900",
    magenta: "6c71c4",
    orange: "cb4b16",
    red: "dc322f",
    yellow: "b58900",
    selected: "073642",
    selected_fg: "fdf6e3",
    bar_bg: "073642",
    ink: "002b36",
    frame: "586e75",
};

pub const GRUVBOX: Palette = Palette {
    name: "gruvbox",
    fg: "ebdbb2",
    muted: "928374",
    blue: "83a598",
    cyan: "8ec07c",
    green: "b8bb26",
    magenta: "d3869b",
    orange: "fe8019",
    red: "fb4934",
    yellow: "fabd2f",
    selected: "504945",
    selected_fg: "fbf1c7",
    bar_bg: "3c3836",
    ink: "282828",
    frame: "665c54",
};

pub const CATPPUCCIN_MOCHA: Palette = Palette {
    name: "catppuccin-mocha",
    fg: "cdd6f4",
    muted: "6c7086",
    blue: "89b4fa",
    cyan: "89dceb",
    green: "a6e3a1",
    magenta: "cba6f7",
    orange: "fab387",
    red: "f38ba8",
    yellow: "f9e2af",
    selected: "45475a",
    selected_fg: "ffffff",
    bar_bg: "313244",
    ink: "1e1e2e",
    frame: "585b70",
};

pub const THEMES: [&Palette; 5] = [
    &TOKYONIGHT,
    &KANAGAWA_WAVE,
    &SOLARIZED,
    &GRUVBOX,
    &CATPPUCCIN_MOCHA,
];

pub fn find(name: &str) -> Option<&'static Palette> {
    THEMES.iter().copied().find(|p| p.name == name)
}

static ACTIVE: OnceLock<&'static Palette> = OnceLock::new();

/// Sets the palette used for rendering. Only the first call takes effect.
pub fn set_active(p: &'static Palette) {
    let _ = ACTIVE.set(p);
}

pub fn active() -> &'static Palette {
    ACTIVE.get().copied().unwrap_or(&TOKYONIGHT)
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: Option<String>,
}

/// `~/.config/gh-review/gh-review.yaml`
pub fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("gh-review").join("gh-review.yaml"))
}

/// Resolves the palette from the config file. A missing file selects the
/// default theme; an unreadable file, invalid YAML or unknown theme is an error.
pub fn load_from(path: &Path) -> Result<&'static Palette, String> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(&TOKYONIGHT),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let cfg: Config = if data.trim().is_empty() {
        Config::default()
    } else {
        serde_yaml_ng::from_str(&data).map_err(|e| format!("{}: {e}", path.display()))?
    };
    let name = cfg.theme.as_deref().unwrap_or(DEFAULT_THEME);
    find(name).ok_or_else(|| {
        let names: Vec<&str> = THEMES.iter().map(|p| p.name).collect();
        format!(
            "{}: unknown theme \"{name}\" (available: {})",
            path.display(),
            names.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, body: &str) -> PathBuf {
        let p = dir.path().join("gh-review.yaml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn missing_file_uses_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = load_from(&dir.path().join("nope.yaml")).unwrap();
        assert_eq!(p.name, "tokyonight");
    }

    #[test]
    fn empty_file_uses_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_from(&write(&dir, "")).unwrap().name, "tokyonight");
    }

    #[test]
    fn selects_each_theme() {
        let dir = tempfile::tempdir().unwrap();
        for t in THEMES {
            let p = write(&dir, &format!("theme: {}\n", t.name));
            assert_eq!(load_from(&p).unwrap(), t);
        }
    }

    #[test]
    fn unknown_theme_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_from(&write(&dir, "theme: dracula\n")).unwrap_err();
        assert!(err.contains("unknown theme \"dracula\""), "{err}");
        assert!(err.contains("catppuccin-mocha"), "{err}");
    }

    #[test]
    fn invalid_yaml_is_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from(&write(&dir, "theme: [\n")).is_err());
    }

    #[test]
    fn palettes_are_valid_hex() {
        let slots = [
            Slot::Fg,
            Slot::Muted,
            Slot::Blue,
            Slot::Cyan,
            Slot::Green,
            Slot::Magenta,
            Slot::Orange,
            Slot::Red,
            Slot::Yellow,
            Slot::Selected,
            Slot::SelectedFg,
            Slot::BarBg,
            Slot::Ink,
            Slot::Frame,
        ];
        for t in THEMES {
            for s in slots {
                let h = t.hex(s);
                assert_eq!(h.len(), 6, "{} {s:?}", t.name);
                assert!(u32::from_str_radix(h, 16).is_ok(), "{} {s:?}", t.name);
            }
        }
    }
}
