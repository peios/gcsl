//! Diagnostics: errors reported to the user, and their rendering.
//!
//! Every stage reports problems as [`Diagnostic`] values carrying a [`Span`].
//! Static and runtime errors share one representation so they render
//! identically. Rendering is the only place line and column numbers are
//! computed.

use std::fmt::{self, Write as _};

use crate::source::{SourceFile, Span};

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// The program cannot be run, or its run failed.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
        })
    }
}

/// A message about a region of source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// How serious the problem is.
    pub severity: Severity,
    /// The primary message, without a trailing full stop.
    pub message: String,
    /// The source region the message is about.
    pub span: Span,
    /// Additional lines rendered as `= note: ...` after the source excerpt.
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// Creates an error diagnostic with no notes.
    #[must_use]
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            notes: Vec::new(),
        }
    }

    /// Appends a note, returning the modified diagnostic.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Renders the diagnostic against `source`:
    ///
    /// ```text
    /// error: cannot add Int and Float
    ///   --> calc.gcsl:2:5
    ///    |
    ///  2 | 1 + 2.5 * x
    ///    |     ^^^^^^^
    ///    |
    ///    = note: ...
    /// ```
    ///
    /// The carets cover the span; a span that continues past the end of its
    /// first line is underlined to the end of that line. The gutter widens to
    /// fit the line number. The trailing `|` line and the notes appear only if
    /// there are notes. Every line, including the last, ends in `\n`.
    #[must_use]
    pub fn render(&self, source: &SourceFile) -> String {
        Rendered {
            diagnostic: self,
            source,
        }
        .to_string()
    }
}

/// [`Diagnostic::render`]'s implementation, as a `Display` so it can use
/// `write!` without handling the (impossible) errors of writing to a `String`.
struct Rendered<'a> {
    diagnostic: &'a Diagnostic,
    source: &'a SourceFile,
}

impl fmt::Display for Rendered<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Diagnostic {
            severity,
            message,
            span,
            notes,
        } = self.diagnostic;
        let source = self.source;
        let (line, column) = source.line_col(span.start);
        let text = source.line_text(line);
        let width = decimal_width(line);
        // The gutter is one space, the right-aligned line number, one space.
        let gutter = width + 2;

        writeln!(f, "{severity}: {message}")?;
        writeln!(
            f,
            "{:arrow$}--> {}:{line}:{column}",
            "",
            source.name(),
            arrow = width + 1
        )?;
        writeln!(f, "{:gutter$}|", "")?;
        if text.is_empty() {
            writeln!(f, " {line:width$} |")?;
        } else {
            writeln!(f, " {line:width$} | {text}")?;
        }

        // Reproduce tabs in the padding so the carets line up with the text
        // however the terminal expands them.
        write!(f, "{:gutter$}| ", "")?;
        let skip = (column as usize).saturating_sub(1);
        for ch in text.chars().take(skip) {
            f.write_char(if ch == '\t' { '\t' } else { ' ' })?;
        }
        let (end_line, end_column) = source.line_col(span.end);
        let carets = if end_line == line {
            end_column.saturating_sub(column) as usize
        } else {
            text.chars().count().saturating_sub(skip)
        };
        for _ in 0..carets.max(1) {
            f.write_char('^')?;
        }
        f.write_char('\n')?;

        if !notes.is_empty() {
            writeln!(f, "{:gutter$}|", "")?;
            for note in notes {
                writeln!(f, "{:gutter$}= note: {note}", "")?;
            }
        }
        Ok(())
    }
}

/// Number of decimal digits in `n`.
fn decimal_width(n: u32) -> usize {
    n.checked_ilog10().map_or(1, |digits| digits as usize + 1)
}

/// A violated compiler invariant: a bug in GCSL, not in the user's program.
///
/// Stages that the type checker guarantees cannot fail on user input (such as
/// lowering) return this instead of panicking when they meet an impossible
/// case. It converts to a [`Diagnostic`] so it can be shown like any other
/// error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InternalError {
    /// What invariant was violated.
    pub message: &'static str,
    /// The node being processed when the violation was found.
    pub span: Span,
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "internal error: {}", self.message)
    }
}

impl std::error::Error for InternalError {}

impl From<InternalError> for Diagnostic {
    fn from(error: InternalError) -> Self {
        Diagnostic::error(error.to_string(), error.span)
            .with_note("this is a bug in GCSL, not in the program being run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SourceFile {
        SourceFile::new("calc.gcsl", text).unwrap()
    }

    #[test]
    fn renders_single_digit_line_with_note() {
        let source = file("# sum\n1 + 2.5 * x\n");
        let diagnostic =
            Diagnostic::error("cannot add Int and Float", Span::new(10, 17)).with_note("a note");
        assert_eq!(
            diagnostic.render(&source),
            "error: cannot add Int and Float\n\
             \x20 --> calc.gcsl:2:5\n\
             \x20  |\n\
             \x202 | 1 + 2.5 * x\n\
             \x20  |     ^^^^^^^\n\
             \x20  |\n\
             \x20  = note: a note\n"
        );
    }

    #[test]
    fn renders_without_notes() {
        let source = file("7 // 0");
        let diagnostic = Diagnostic::error("division by zero", Span::new(0, 6));
        assert_eq!(
            diagnostic.render(&source),
            "error: division by zero\n\
             \x20 --> calc.gcsl:1:1\n\
             \x20  |\n\
             \x201 | 7 // 0\n\
             \x20  | ^^^^^^\n"
        );
    }

    #[test]
    fn gutter_widens_for_two_digit_line_numbers() {
        let source = file(&format!("{}1 + 2", "\n".repeat(11)));
        let diagnostic = Diagnostic::error("oops", Span::new(13, 14)).with_note("n");
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20  --> calc.gcsl:12:3\n\
             \x20   |\n\
             \x2012 | 1 + 2\n\
             \x20   |   ^\n\
             \x20   |\n\
             \x20   = note: n\n"
        );
    }

    #[test]
    fn multi_line_span_is_underlined_to_end_of_first_line() {
        let source = file("(1 +\n 2)");
        let diagnostic = Diagnostic::error("oops", Span::new(1, 7));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20 --> calc.gcsl:1:2\n\
             \x20  |\n\
             \x201 | (1 +\n\
             \x20  |  ^^^\n"
        );
    }

    #[test]
    fn empty_span_gets_one_caret() {
        let source = file("1 +");
        let diagnostic = Diagnostic::error("oops", Span::new(3, 3));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20 --> calc.gcsl:1:4\n\
             \x20  |\n\
             \x201 | 1 +\n\
             \x20  |    ^\n"
        );
    }

    #[test]
    fn empty_line_has_no_trailing_space() {
        let source = file("1\n\n");
        let diagnostic = Diagnostic::error("oops", Span::new(2, 2));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20 --> calc.gcsl:2:1\n\
             \x20  |\n\
             \x202 |\n\
             \x20  | ^\n"
        );
    }

    #[test]
    fn tabs_and_multibyte_characters_keep_carets_aligned() {
        let source = file("\té + 1");
        // `é` is two bytes: it occupies bytes 1..3.
        let diagnostic = Diagnostic::error("oops", Span::new(1, 3));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20 --> calc.gcsl:1:2\n\
             \x20  |\n\
             \x201 | \té + 1\n\
             \x20  | \t^\n"
        );
    }

    #[test]
    fn internal_error_converts_to_diagnostic() {
        let diagnostic: Diagnostic = InternalError {
            message: "broken",
            span: Span::new(1, 2),
        }
        .into();
        assert_eq!(diagnostic.message, "internal error: broken");
        assert_eq!(diagnostic.span, Span::new(1, 2));
        assert_eq!(diagnostic.notes.len(), 1);
    }

    #[test]
    fn decimal_width_counts_digits() {
        assert_eq!(decimal_width(0), 1);
        assert_eq!(decimal_width(9), 1);
        assert_eq!(decimal_width(10), 2);
        assert_eq!(decimal_width(u32::MAX), 10);
    }
}
