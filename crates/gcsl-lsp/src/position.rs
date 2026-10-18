//! Positions: LSP's `(line, character)` and the byte offsets GCSL's spans
//! are made of, in the encoding negotiated with the client.
//!
//! LSP counts a line's characters in code units of the negotiated encoding
//! ([`Encoding`]): bytes for utf-8, UTF-16 code units for utf-16 (the
//! default, which a character outside the Basic Multilingual Plane, an
//! emoji say, takes two of). Its lines end at `\n`, `\r\n` or a lone `\r`.
//! A [`LineIndex`] of a text converts both ways: a byte offset to a
//! position ([`LineIndex::position`]), for a diagnostic's range, and a
//! position to a byte offset ([`LineIndex::offset`]), for an edit of an
//! incremental change ([`apply`]). A position past a line's end is taken as
//! its end, and one past the last line as the end of the text, as LSP
//! says; one inside a character, between the two halves of a surrogate
//! pair, as the start of the character.

use lsp_types::{Position, PositionEncodingKind, Range, TextDocumentContentChangeEvent};

use gcsl::Span;

/// The encoding positions are counted in, as negotiated at initialisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Bytes of UTF-8.
    Utf8,
    /// UTF-16 code units: LSP's default.
    Utf16,
}

impl Encoding {
    /// The encoding to use for a client offering `offered`: utf-8 if it is
    /// among them, else utf-16, which every client understands.
    #[must_use]
    pub fn negotiate<'a>(offered: impl IntoIterator<Item = &'a str>) -> Self {
        if offered.into_iter().any(|kind| kind == "utf-8") {
            Encoding::Utf8
        } else {
            Encoding::Utf16
        }
    }

    /// The encoding as the server's capabilities name it.
    #[must_use]
    pub fn kind(self) -> PositionEncodingKind {
        match self {
            Encoding::Utf8 => PositionEncodingKind::UTF8,
            Encoding::Utf16 => PositionEncodingKind::UTF16,
        }
    }

    /// How many code units `ch` takes.
    fn width(self, ch: char) -> usize {
        match self {
            Encoding::Utf8 => ch.len_utf8(),
            Encoding::Utf16 => ch.len_utf16(),
        }
    }

    /// How many code units `text` takes.
    fn len(self, text: &str) -> usize {
        match self {
            Encoding::Utf8 => text.len(),
            Encoding::Utf16 => text.chars().map(char::len_utf16).sum(),
        }
    }
}

/// The lines of a text, by the byte offset each starts at: what positions
/// are converted with.
#[derive(Debug)]
pub struct LineIndex<'a> {
    /// The text.
    text: &'a str,
    /// The byte offset at which each line starts: 0 first, then the offset
    /// after each line's end, `\n`, `\r\n` or a lone `\r`.
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// The lines of `text`.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        for (at, &byte) in bytes.iter().enumerate() {
            let ends = byte == b'\n' || (byte == b'\r' && bytes.get(at + 1) != Some(&b'\n'));
            if ends {
                starts.push(at + 1);
            }
        }
        Self { text, starts }
    }

    /// The byte offsets of the start of line `line` and of the end of its
    /// text, before its line's end; `None` past the last line.
    fn line(&self, line: usize) -> Option<(usize, usize)> {
        let start = *self.starts.get(line)?;
        let mut end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        let bytes = self.text.as_bytes();
        if end > start && bytes.get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && bytes.get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some((start, end))
    }

    /// The position of byte `offset` in `encoding`. An offset past the end
    /// is the end; one on a line's end, between `\r` and `\n` say, is the
    /// end of the line's text; one inside a character is its start.
    #[must_use]
    pub fn position(&self, offset: usize, encoding: Encoding) -> Position {
        let offset = self.text.floor_char_boundary(offset.min(self.text.len()));
        let line = self
            .starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let (start, end) = self.line(line).unwrap_or((offset, offset));
        let before = self.text.get(start..offset.min(end)).unwrap_or_default();
        Position::new(to_u32(line), to_u32(encoding.len(before)))
    }

    /// The range of `span` in `encoding` ([`LineIndex::position`] of each
    /// end).
    #[must_use]
    pub fn range(&self, span: Span, encoding: Encoding) -> Range {
        Range::new(
            self.position(span.start as usize, encoding),
            self.position(span.end as usize, encoding),
        )
    }

    /// The byte offset of `position`, counted in `encoding`. A character
    /// past the line's end is the end of its text, and a line past the last
    /// the end of the text; a character inside one, the second half of a
    /// surrogate pair say, is the start of that character.
    #[must_use]
    pub fn offset(&self, position: Position, encoding: Encoding) -> usize {
        let Some((start, end)) = self.line(position.line as usize) else {
            return self.text.len();
        };
        let wanted = position.character as usize;
        let mut units = 0;
        for (at, ch) in self.text.get(start..end).unwrap_or_default().char_indices() {
            let width = encoding.width(ch);
            if units + width > wanted {
                return start + at;
            }
            units += width;
        }
        end
    }
}

/// Applies `change`, one change of a `textDocument/didChange`, to `text`,
/// its positions counted in `encoding`: a change with no range replaces
/// the whole text, and one with a range replaces what is between its ends.
pub fn apply(text: &mut String, change: TextDocumentContentChangeEvent, encoding: Encoding) {
    let Some(range) = change.range else {
        *text = change.text;
        return;
    };
    let (start, end) = {
        let lines = LineIndex::new(text);
        let start = lines.offset(range.start, encoding);
        (start, lines.offset(range.end, encoding).max(start))
    };
    text.replace_range(start..end, &change.text);
}

/// `value` as a `u32`, which LSP's positions are; saturating, for a text
/// longer than any position can reach.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The change replacing the text between `(l1, c1)` and `(l2, c2)` with
    /// `text`.
    fn change(
        (l1, c1): (u32, u32),
        (l2, c2): (u32, u32),
        text: &str,
    ) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(l1, c1), Position::new(l2, c2))),
            range_length: None,
            text: text.to_owned(),
        }
    }

    #[test]
    fn utf_8_is_offered_or_utf_16_is_used() {
        assert_eq!(Encoding::negotiate(["utf-16", "utf-8"]), Encoding::Utf8);
        assert_eq!(Encoding::negotiate(["utf-32", "utf-16"]), Encoding::Utf16);
        assert_eq!(Encoding::negotiate([]), Encoding::Utf16);
        assert_eq!(Encoding::Utf8.kind(), PositionEncodingKind::UTF8);
        assert_eq!(Encoding::Utf16.kind(), PositionEncodingKind::UTF16);
    }

    #[test]
    fn positions_count_code_units_of_the_encoding() {
        // `é` is two bytes and one UTF-16 unit, `😀` four bytes and two.
        let text = "ab\nx = \"é😀\" + 1\n";
        let lines = LineIndex::new(text);
        let plus = text.find('+').unwrap();
        assert_eq!(lines.position(plus, Encoding::Utf8), Position::new(1, 13));
        assert_eq!(lines.position(plus, Encoding::Utf16), Position::new(1, 10));
        assert_eq!(lines.offset(Position::new(1, 13), Encoding::Utf8), plus);
        assert_eq!(lines.offset(Position::new(1, 10), Encoding::Utf16), plus);
        // The second half of the surrogate pair is the emoji's start, and
        // so is a byte inside it.
        let emoji = text.find('😀').unwrap();
        assert_eq!(lines.offset(Position::new(1, 7), Encoding::Utf16), emoji);
        assert_eq!(
            lines.position(emoji + 1, Encoding::Utf8),
            Position::new(1, 7)
        );
    }

    #[test]
    fn lines_end_at_each_kind_of_line_end() {
        let text = "a\r\nb\rc\nd";
        let lines = LineIndex::new(text);
        assert_eq!(lines.position(3, Encoding::Utf8), Position::new(1, 0));
        assert_eq!(lines.position(5, Encoding::Utf8), Position::new(2, 0));
        assert_eq!(lines.position(7, Encoding::Utf8), Position::new(3, 0));
        // Between `\r` and `\n`: the end of the line's text.
        assert_eq!(lines.position(2, Encoding::Utf8), Position::new(0, 1));
        assert_eq!(lines.offset(Position::new(2, 0), Encoding::Utf16), 5);
    }

    #[test]
    fn positions_past_the_end_are_clamped() {
        let text = "ab\ncd";
        let lines = LineIndex::new(text);
        assert_eq!(lines.offset(Position::new(0, 99), Encoding::Utf8), 2);
        assert_eq!(lines.offset(Position::new(9, 0), Encoding::Utf8), 5);
        assert_eq!(lines.position(99, Encoding::Utf16), Position::new(1, 2));
        let empty = LineIndex::new("");
        assert_eq!(empty.position(0, Encoding::Utf8), Position::new(0, 0));
        assert_eq!(empty.offset(Position::new(0, 3), Encoding::Utf8), 0);
    }

    #[test]
    fn changes_apply_in_either_encoding() {
        let mut text = "x = \"😀\" + 1\ny = 2\n".to_owned();
        // After the emoji: character 7 in UTF-16, 9 in UTF-8.
        apply(&mut text, change((0, 7), (0, 7), "!"), Encoding::Utf16);
        assert_eq!(text, "x = \"😀!\" + 1\ny = 2\n");
        apply(&mut text, change((0, 9), (0, 10), ""), Encoding::Utf8);
        assert_eq!(text, "x = \"😀\" + 1\ny = 2\n");
        // Across lines, and the whole text.
        apply(&mut text, change((0, 12), (1, 1), "\nz"), Encoding::Utf16);
        assert_eq!(text, "x = \"😀\" + 1\nz = 2\n");
        let whole = TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "new".to_owned(),
        };
        apply(&mut text, whole, Encoding::Utf8);
        assert_eq!(text, "new");
    }
}
