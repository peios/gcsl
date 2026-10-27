//! Source text, byte spans and line/column lookup.
//!
//! Every later stage refers to source positions through [`Span`], a pair of
//! `u32` byte offsets. Line and column numbers are only computed when a
//! diagnostic is rendered, using the line table [`SourceFile`] builds once.

use std::error::Error;
use std::fmt;

/// A half-open byte range `start..end` into a [`SourceFile`]'s text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the first byte covered.
    pub start: u32,
    /// Byte offset one past the last byte covered.
    pub end: u32,
}

impl Span {
    /// Creates a span covering `start..end`.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Returns the smallest span covering both `self` and `other`.
    ///
    /// Callable as `Span::join(a, b)`.
    #[must_use]
    pub const fn join(self, other: Span) -> Span {
        let start = if self.start < other.start {
            self.start
        } else {
            other.start
        };
        let end = if self.end > other.end {
            self.end
        } else {
            other.end
        };
        Span { start, end }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// Error returned when a [`SourceFile`] cannot be constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// The text is longer than `u32::MAX` bytes, so spans cannot address it.
    TooLarge {
        /// Length of the rejected text in bytes.
        len: usize,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::TooLarge { len } => write!(
                f,
                "source is {len} bytes long; the maximum is {} bytes",
                u32::MAX
            ),
        }
    }
}

impl Error for SourceError {}

/// A named source text together with its line table.
#[derive(Clone, Debug)]
pub struct SourceFile {
    name: String,
    text: String,
    /// Where the host found the file's directory, for a library's file
    /// ([`SourceFile::with_dir`]); `None` otherwise.
    dir: Option<String>,
    /// Byte offset at which each line starts. Always non-empty: line 1 starts
    /// at offset 0, and every `\n` starts a new line after it.
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// Creates a source file, building its line table.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::TooLarge`] if `text` is longer than `u32::MAX`
    /// bytes.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Result<Self, SourceError> {
        let mut text = text.into();
        // CRLF line endings are normalised to LF, as rustc does, so that
        // every later stage (item separation, string literals and their
        // indentation stripping, line continuations) sees one kind of line
        // break. Offsets in spans refer to the normalised text.
        if text.contains("\r\n") {
            text = text.replace("\r\n", "\n");
        }
        if u32::try_from(text.len()).is_err() {
            return Err(SourceError::TooLarge { len: text.len() });
        }
        let mut line_starts = vec![0];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, byte)| byte == b'\n')
                // The length check above guarantees every offset + 1 fits.
                .map(|(offset, _)| offset_to_u32(offset + 1)),
        );
        Ok(Self {
            name: name.into(),
            text,
            line_starts,
            dir: None,
        })
    }

    /// The name used for this source in diagnostics, such as a file path.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file, its directory being at `dir`, an absolute path as GCSL code
    /// names paths: where its host found a library, so that a relative path
    /// in the library is in its own directory (see [Where a path
    /// is](crate::eval::files#where-a-path-is)). A workspace's files need
    /// none, being under its root.
    #[must_use]
    pub fn with_dir(mut self, dir: impl Into<String>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Where the host found the file's directory, if it said
    /// ([`SourceFile::with_dir`]).
    #[must_use]
    pub fn dir(&self) -> Option<&str> {
        self.dir.as_deref()
    }

    /// The full source text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The length of the text in bytes. Always fits in `u32`.
    #[must_use]
    pub fn len(&self) -> u32 {
        offset_to_u32(self.text.len())
    }

    /// Returns `true` if the text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The text covered by `span`, or `""` if the span does not lie on
    /// character boundaries within this source.
    #[must_use]
    pub fn slice(&self, span: Span) -> &str {
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    /// Returns the 1-based `(line, column)` of byte `offset`.
    ///
    /// Columns count characters, not bytes. Offsets past the end of the text
    /// are clamped to the end.
    #[must_use]
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let offset = offset.min(self.len());
        let index = self.line_index(offset);
        let line_start = self.line_starts.get(index).copied().unwrap_or(0);
        let before = self
            .text
            .as_bytes()
            .get(line_start as usize..offset as usize)
            .unwrap_or_default();
        (
            offset_to_u32(index + 1),
            offset_to_u32(char_count(before) + 1),
        )
    }

    /// The 0-based index into the line table of the line containing byte
    /// `offset`, which is clamped to the end of the text.
    fn line_index(&self, offset: u32) -> usize {
        let offset = offset.min(self.len());
        // `line_starts[0] == 0 <= offset`, so the partition point is >= 1.
        self.line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1)
    }

    /// The whole lines that `span` touches: from the start of the line
    /// containing `span.start` to the end of the line containing `span.end`,
    /// without that line's terminator.
    #[must_use]
    pub fn excerpt(&self, span: Span) -> Excerpt {
        let first = self.line_index(span.start);
        let last = self.line_index(span.end.max(span.start));
        let start = self.line_starts.get(first).copied().unwrap_or(0);
        let end = self.line_starts.get(last + 1).map_or(self.len(), |&next| {
            // Step back over the terminator that starts the next line.
            let line = self.text.get(..next as usize).unwrap_or("");
            let line = line.strip_suffix('\n').unwrap_or(line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            offset_to_u32(line.len())
        });
        Excerpt {
            first_line: offset_to_u32(first + 1),
            span: Span::new(start, end.max(start)),
        }
    }

    /// Returns the text of 1-based line `line`, without its line terminator
    /// (`\n` or `\r\n`). Returns `""` for a line number out of range.
    #[must_use]
    pub fn line_text(&self, line: u32) -> &str {
        let Some(index) = (line as usize).checked_sub(1) else {
            return "";
        };
        let Some(&start) = self.line_starts.get(index) else {
            return "";
        };
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next as usize);
        let text = self.text.get(start as usize..end).unwrap_or("");
        let text = text.strip_suffix('\n').unwrap_or(text);
        text.strip_suffix('\r').unwrap_or(text)
    }
}

/// A run of whole lines from a [`SourceFile`], as [`SourceFile::excerpt`]
/// returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Excerpt {
    /// The 1-based number of the first line.
    pub first_line: u32,
    /// The bytes covered, from the start of the first line to the end of the
    /// last line's content.
    pub span: Span,
}

/// Converts a byte offset already known to lie within a [`SourceFile`] to
/// `u32`.
///
/// [`SourceFile::new`] rejects texts longer than `u32::MAX` bytes, so every
/// offset into one fits. Saturates rather than panicking if that invariant is
/// ever violated.
pub(crate) fn offset_to_u32(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

/// Counts the UTF-8 characters in `bytes` by counting non-continuation bytes.
///
/// Works on any byte range, including one that does not start or end on a
/// character boundary, so it can never fail.
fn char_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&byte| byte & 0xC0 != 0x80).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SourceFile {
        SourceFile::new("test.gcsl", text).unwrap()
    }

    #[test]
    fn join_covers_both_spans_in_either_order() {
        let a = Span::new(2, 5);
        let b = Span::new(8, 11);
        assert_eq!(Span::join(a, b), Span::new(2, 11));
        assert_eq!(Span::join(b, a), Span::new(2, 11));
        assert_eq!(Span::join(a, Span::new(3, 4)), a);
    }

    #[test]
    fn crlf_is_normalised_to_lf() {
        let source = file("a\r\nb\r\n");
        assert_eq!(source.text(), "a\nb\n");
        assert_eq!(source.line_col(2), (2, 1));
        // A lone `\r` is not a line ending and is kept.
        assert_eq!(file("a\rb").text(), "a\rb");
    }

    #[test]
    fn line_col_is_one_based() {
        let source = file("1 + 2\n  3\n");
        assert_eq!(source.line_col(0), (1, 1));
        assert_eq!(source.line_col(4), (1, 5));
        assert_eq!(source.line_col(5), (1, 6));
        assert_eq!(source.line_col(6), (2, 1));
        assert_eq!(source.line_col(8), (2, 3));
        assert_eq!(source.line_col(10), (3, 1));
    }

    #[test]
    fn line_col_counts_characters_not_bytes() {
        // `é` is two bytes and `€` is three.
        let source = file("é€ + x");
        assert_eq!(source.line_col(5), (1, 3));
        assert_eq!(source.line_col(7), (1, 5));
    }

    #[test]
    fn line_col_clamps_past_the_end() {
        let source = file("ab");
        assert_eq!(source.line_col(100), (1, 3));
    }

    #[test]
    fn line_text_strips_terminators() {
        let source = file("one\r\ntwo\nthree");
        assert_eq!(source.line_text(1), "one");
        assert_eq!(source.line_text(2), "two");
        assert_eq!(source.line_text(3), "three");
        assert_eq!(source.line_text(0), "");
        assert_eq!(source.line_text(4), "");
    }

    #[test]
    fn empty_source_has_one_empty_line() {
        let source = file("");
        assert_eq!(source.line_col(0), (1, 1));
        assert_eq!(source.line_text(1), "");
    }

    #[test]
    fn excerpt_covers_whole_lines() {
        let source = file("one\ntwo\nthree\nfour");
        let lines = |span| {
            let excerpt = source.excerpt(span);
            (excerpt.first_line, source.slice(excerpt.span))
        };
        assert_eq!(lines(Span::new(1, 2)), (1, "one"));
        assert_eq!(lines(Span::new(5, 6)), (2, "two"));
        assert_eq!(lines(Span::new(1, 6)), (1, "one\ntwo"));
        assert_eq!(lines(Span::new(9, 16)), (3, "three\nfour"));
        // An empty span at the very end.
        assert_eq!(lines(Span::new(18, 18)), (4, "four"));
        // The end offset on a terminator belongs to the line it ends.
        assert_eq!(lines(Span::new(0, 3)), (1, "one"));
        assert_eq!(
            source.excerpt(Span::new(0, 0)),
            Excerpt {
                first_line: 1,
                span: Span::new(0, 3)
            }
        );
        assert_eq!(
            file("").excerpt(Span::new(0, 0)),
            Excerpt {
                first_line: 1,
                span: Span::new(0, 0)
            }
        );
    }

    #[test]
    fn slice_returns_covered_text() {
        let source = file("12 + 345");
        assert_eq!(source.slice(Span::new(5, 8)), "345");
        assert_eq!(source.slice(Span::new(5, 80)), "");
    }

    #[test]
    fn too_large_error_message() {
        let error = SourceError::TooLarge { len: 5_000_000_000 };
        assert_eq!(
            error.to_string(),
            "source is 5000000000 bytes long; the maximum is 4294967295 bytes"
        );
    }
}
