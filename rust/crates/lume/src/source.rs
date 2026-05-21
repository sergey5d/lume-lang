#[derive(Debug, Clone)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

impl LineColumn {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub start_pos: LineColumn,
    pub end_pos: LineColumn,
}

impl Span {
    pub const fn new(start: usize, end: usize, start_pos: LineColumn, end_pos: LineColumn) -> Self {
        Self {
            start,
            end,
            start_pos,
            end_pos,
        }
    }

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
