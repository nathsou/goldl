//! Lexing, parsing and source positions.

pub mod ast;
pub mod format;
pub mod lexer;
pub mod parser;

/// Byte range in the source text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }
    pub fn join(self, o: Span) -> Span {
        Span { start: self.start.min(o.start), end: self.end.max(o.end) }
    }
    pub fn contains(self, off: u32) -> bool {
        off >= self.start && off <= self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub span: Span,
    pub severity: Severity,
    pub message: String,
    pub notes: Vec<(Span, String)>,
}

impl Diag {
    pub fn error(span: Span, msg: impl Into<String>) -> Self {
        Diag { span, severity: Severity::Error, message: msg.into(), notes: vec![] }
    }
    pub fn warning(span: Span, msg: impl Into<String>) -> Self {
        Diag { span, severity: Severity::Warning, message: msg.into(), notes: vec![] }
    }
    pub fn with_note(mut self, span: Span, msg: impl Into<String>) -> Self {
        self.notes.push((span, msg.into()));
        self
    }
}

/// Line/column (0-based, UTF-16 columns as required by LSP) from byte offsets.
pub struct LineIndex {
    starts: Vec<u32>,
    text: String,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i as u32 + 1);
            }
        }
        LineIndex { starts, text: text.to_string() }
    }
    pub fn line_col(&self, off: u32) -> (u32, u32) {
        let line = match self.starts.binary_search(&off) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let ls = self.starts[line] as usize;
        let off = (off as usize).min(self.text.len());
        let col: usize = self.text.get(ls..off).map(|s| s.encode_utf16().count()).unwrap_or(0);
        (line as u32, col as u32)
    }
    pub fn offset(&self, line: u32, col: u32) -> u32 {
        let Some(&ls) = self.starts.get(line as usize) else { return self.text.len() as u32 };
        let end = self.starts.get(line as usize + 1).copied().unwrap_or(self.text.len() as u32);
        let s = &self.text[ls as usize..end as usize];
        let mut units = 0u32;
        for (i, ch) in s.char_indices() {
            if units >= col {
                return ls + i as u32;
            }
            units += ch.len_utf16() as u32;
        }
        end.min(ls + s.trim_end_matches('\n').len() as u32)
    }
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
}
