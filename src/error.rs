use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum GhiError {
    Gh(String),
    Other(String),
}

impl fmt::Display for GhiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GhiError::Gh(m) => write!(f, "gh: {m}"),
            GhiError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for GhiError {}

pub type Result<T> = std::result::Result<T, GhiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_prefixes_gh_errors_only() {
        assert_eq!(
            GhiError::Gh("pr diff failed: HTTP 404".into()).to_string(),
            "gh: pr diff failed: HTTP 404"
        );
        assert_eq!(
            GhiError::Other("parse failed".into()).to_string(),
            "parse failed"
        );
    }
}
