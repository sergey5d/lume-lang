//! Source text and span utilities shared across the Rust frontend pipeline.
//!
//! These types keep file contents, human-readable line/column locations, and
//! byte spans together so the lexer, parser, resolver, and type checker can
//! report diagnostics against the original source text consistently.

#[derive(Debug, Clone)]
/// An in-memory source file being compiled or interpreted.
pub struct SourceFile {
    /// Display name used in diagnostics.
    pub name: String,
    /// Full UTF-8 contents of the file.
    pub text: String,
}

impl SourceFile {
    /// Creates a new source file wrapper from a name and raw text.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A 1-based line and column pair used in user-facing diagnostics.
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

impl LineColumn {
    /// Builds a new line/column location.
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A half-open byte range plus its corresponding start/end line-column points.
pub struct Span {
    /// Inclusive start byte offset in the source text.
    pub start: usize,
    /// Exclusive end byte offset in the source text.
    pub end: usize,
    /// Human-readable location for `start`.
    pub start_pos: LineColumn,
    /// Human-readable location for `end`.
    pub end_pos: LineColumn,
}

impl Span {
    /// Builds a span from raw byte offsets and their line/column positions.
    pub const fn new(start: usize, end: usize, start_pos: LineColumn, end_pos: LineColumn) -> Self {
        Self {
            start,
            end,
            start_pos,
            end_pos,
        }
    }

    /// Returns the smallest span that covers both `self` and `other`.
    pub const fn cover(self, other: Span) -> Self {
        let (start, start_pos) = if self.start <= other.start {
            (self.start, self.start_pos)
        } else {
            (other.start, other.start_pos)
        };
        let (end, end_pos) = if self.end >= other.end {
            (self.end, self.end_pos)
        } else {
            (other.end, other.end_pos)
        };
        Self {
            start,
            end,
            start_pos,
            end_pos,
        }
    }
}

/// Returns the stable internal type name used for one anonymous object literal.
pub(crate) fn anonymous_object_type_name(span: Span) -> String {
    format!("__LumeObject_{}_{}", span.start, span.end)
}
