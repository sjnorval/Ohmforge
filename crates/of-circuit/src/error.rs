//! Errors with a 1-based line and column.

use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// A circuit-file error with a 1-based position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircuitError {
    /// 1-based line.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// What is wrong.
    pub message: String,
    /// File, when known.
    pub path: Option<PathBuf>,
}

impl CircuitError {
    /// Error at byte `span.start` of `src`.
    pub fn at(src: &str, span: Range<usize>, message: impl Into<String>) -> Self {
        let mut start = span.start.min(src.len());
        while !src.is_char_boundary(start) {
            start -= 1;
        }
        let before = &src[..start];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Self {
            line,
            column,
            message: message.into(),
            path: None,
        }
    }

    /// Error that is not tied to a position in a file (line and column 0).
    pub fn general(message: impl Into<String>) -> Self {
        Self {
            line: 0,
            column: 0,
            message: message.into(),
            path: None,
        }
    }

    /// Attach the file path for display.
    pub fn with_path(mut self, path: &Path) -> Self {
        self.path = Some(path.to_owned());
        self
    }
}

impl fmt::Display for CircuitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(p) = &self.path {
            write!(f, "{}:", p.display())?;
        }
        if self.line > 0 {
            write!(f, "{}:{}: ", self.line, self.column)?;
        } else if self.path.is_some() {
            f.write_str(" ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for CircuitError {}
