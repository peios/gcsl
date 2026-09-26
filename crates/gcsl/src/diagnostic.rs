//! Diagnostics: errors reported to the user, and their rendering.
//!
//! Every stage reports problems as [`Diagnostic`] values. A diagnostic has a
//! severity, a message, a primary [`Span`] that the message is about, any
//! number of secondary [`Label`]s that point at related spans with a short
//! text, and notes shown after the source excerpt. Static and runtime errors
//! share this one representation, so they render identically, and so do
//! warnings, which are rendered as `warning: ...` and do not stop a program
//! from running.
//!
//! Rendering is done by [`Renderer`], which wraps the `annotate-snippets`
//! crate (the rendering engine extracted from rustc). Line and column
//! numbers are computed only at render time.
//!
//! A diagnostic's spans refer to one source. Normally that is the source
//! being run, but a function defined by an earlier input of a
//! [`crate::session::Session`] can fail while a later input runs, and its
//! error then refers to the earlier input's source, which the diagnostic
//! records in [`Diagnostic::input`] so that it can be rendered against the
//! right text.

use std::fmt;

use annotate_snippets::{AnnotationKind, Level, Snippet};

use crate::source::{SourceFile, Span};

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// The program cannot be run, or its run failed.
    Error,
    /// Something is probably a mistake, but the program is valid and runs.
    Warning,
}

impl Severity {
    fn level(self) -> Level<'static> {
        match self {
            Severity::Error => Level::ERROR,
            Severity::Warning => Level::WARNING,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        })
    }
}

/// A secondary span with a short text, shown alongside a diagnostic's primary
/// span to point at something related: an operand and its type, an
/// unclosed bracket, the value that overflowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The source region the text is about.
    pub span: Span,
    /// The text shown next to the underline. Short: a type, a value, a
    /// few words. No trailing full stop.
    pub text: String,
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
    /// Text shown next to the primary span's underline, if any.
    pub label: Option<String>,
    /// Related spans, each with its own text.
    pub labels: Vec<Label>,
    /// Additional lines rendered as `= note: ...` after the source excerpt.
    pub notes: Vec<String>,
    /// The session input whose source the spans refer to, when it is not
    /// the input being run: set on a runtime error raised inside a function
    /// that an earlier input of a [`crate::session::Session`] defined.
    /// `None`, as every builder leaves it, means the input being run (or
    /// the file, outside a session).
    pub input: Option<u32>,
}

impl Diagnostic {
    /// Creates an error diagnostic about `span` with no labels or notes.
    #[must_use]
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            label: None,
            labels: Vec::new(),
            notes: Vec::new(),
            input: None,
        }
    }

    /// Creates a warning diagnostic about `span` with no labels or notes.
    #[must_use]
    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(message, span)
        }
    }

    /// Sets the text shown next to the primary span, returning the modified
    /// diagnostic.
    #[must_use]
    pub fn with_label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }

    /// Adds a secondary span with its text, returning the modified
    /// diagnostic.
    #[must_use]
    pub fn with_context(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            text: text.into(),
        });
        self
    }

    /// Appends a note, returning the modified diagnostic.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Renders the diagnostic against `source` without terminal styling.
    /// See [`Renderer`] for styled output.
    #[must_use]
    pub fn render(&self, source: &SourceFile) -> String {
        Renderer::plain().render(self, source)
    }

    /// The first and last byte offsets any of the diagnostic's spans touch.
    fn extent(&self) -> Span {
        self.labels
            .iter()
            .fold(self.span, |extent, label| extent.join(label.span))
    }
}

/// Renders diagnostics as text, in the style of rustc:
///
/// ```text
/// error: cannot add Int and Float
///  --> calc.gcsl:1:10
///   |
/// 1 | (7 // 2) + 1.5
///   | -------- ^ --- Float
///   | |
///   | Int
/// ```
///
/// Long lines are trimmed around the spans to fit the terminal width, and
/// unannotated lines between spans are folded away.
#[derive(Clone, Debug)]
pub struct Renderer {
    inner: annotate_snippets::Renderer,
}

impl Renderer {
    /// A renderer producing plain text with no escape codes.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            inner: annotate_snippets::Renderer::plain(),
        }
    }

    /// A renderer producing text with ANSI colour and emphasis. Use it only
    /// when the output is a terminal that supports them.
    #[must_use]
    pub const fn styled() -> Self {
        Self {
            inner: annotate_snippets::Renderer::styled(),
        }
    }

    /// Sets the width, in columns, to render within. Long source lines are
    /// trimmed around the spans to fit.
    #[must_use]
    pub const fn term_width(mut self, columns: usize) -> Self {
        self.inner = self.inner.term_width(columns);
        self
    }

    /// Renders one diagnostic against the source it refers to. The result
    /// ends in a newline.
    #[must_use]
    pub fn render(&self, diagnostic: &Diagnostic, source: &SourceFile) -> String {
        let excerpt = source.excerpt(diagnostic.extent());
        // Annotation ranges are relative to the excerpt, not the file.
        let relative = |span: Span| {
            let start = span.start.saturating_sub(excerpt.span.start) as usize;
            let end = span.end.saturating_sub(excerpt.span.start) as usize;
            start..end
        };
        let mut snippet = Snippet::source(source.slice(excerpt.span))
            .path(source.name())
            .line_start(excerpt.first_line as usize)
            .annotation(
                AnnotationKind::Primary
                    .span(relative(diagnostic.span))
                    .label(diagnostic.label.as_deref()),
            );
        for label in &diagnostic.labels {
            snippet = snippet.annotation(
                AnnotationKind::Context
                    .span(relative(label.span))
                    .label(label.text.as_str()),
            );
        }
        let mut group = diagnostic
            .severity
            .level()
            .primary_title(diagnostic.message.as_str())
            .element(snippet);
        for note in &diagnostic.notes {
            group = group.element(Level::NOTE.message(note.as_str()));
        }
        let mut text = self.inner.render(&[group]);
        text.push('\n');
        text
    }

    /// Renders several diagnostics, separated by blank lines.
    #[must_use]
    pub fn render_all(&self, diagnostics: &[Diagnostic], source: &SourceFile) -> String {
        diagnostics
            .iter()
            .map(|diagnostic| self.render(diagnostic, source))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Merges `errors` and `warnings` into one vector in source order: sorted by
/// where their primary spans start, keeping the order they were reported in
/// among those that start at the same place. This is how every stage hands
/// back a failure that follows warnings.
#[must_use]
pub fn merge(errors: Vec<Diagnostic>, warnings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut all = errors;
    all.extend(warnings);
    // Stable, so ties keep errors before warnings and each in report order.
    all.sort_by_key(|diagnostic| diagnostic.span.start);
    all
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
    fn renders_primary_span_with_note() {
        let source = file("# sum\n1 + 2.5 * x\n");
        let diagnostic =
            Diagnostic::error("cannot add Int and Float", Span::new(10, 17)).with_note("a note");
        assert_eq!(
            diagnostic.render(&source),
            "error: cannot add Int and Float\n\
             \x20--> calc.gcsl:2:5\n\
             \x20 |\n\
             2 | 1 + 2.5 * x\n\
             \x20 |     ^^^^^^^\n\
             \x20 |\n\
             \x20 = note: a note\n"
        );
    }

    #[test]
    fn renders_labels() {
        let source = file("(7 // 2) + 1.5");
        let diagnostic = Diagnostic::error("cannot add Int and Float", Span::new(9, 10))
            .with_context(Span::new(0, 8), "Int")
            .with_context(Span::new(11, 14), "Float");
        assert_eq!(
            diagnostic.render(&source),
            "error: cannot add Int and Float\n\
             \x20--> calc.gcsl:1:10\n\
             \x20 |\n\
             1 | (7 // 2) + 1.5\n\
             \x20 | -------- ^ --- Float\n\
             \x20 | |\n\
             \x20 | Int\n"
        );
    }

    #[test]
    fn renders_primary_label() {
        let source = file("7 // 0");
        let diagnostic = Diagnostic::error("division by zero", Span::new(2, 4))
            .with_label("here")
            .with_context(Span::new(5, 6), "0");
        assert_eq!(
            diagnostic.render(&source),
            "error: division by zero\n\
             \x20--> calc.gcsl:1:3\n\
             \x20 |\n\
             1 | 7 // 0\n\
             \x20 |   ^^ - 0\n\
             \x20 |   |\n\
             \x20 |   here\n"
        );
    }

    #[test]
    fn gutter_widens_for_two_digit_line_numbers() {
        let source = file(&format!("{}1 + 2", "\n".repeat(11)));
        let diagnostic = Diagnostic::error("oops", Span::new(13, 14)).with_note("n");
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20 --> calc.gcsl:12:3\n\
             \x20  |\n\
             12 | 1 + 2\n\
             \x20  |   ^\n\
             \x20  |\n\
             \x20  = note: n\n"
        );
    }

    #[test]
    fn multi_line_span_is_underlined_across_lines() {
        let source = file("(1 +\n 2)");
        let diagnostic = Diagnostic::error("oops", Span::new(1, 7));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20--> calc.gcsl:1:2\n\
             \x20 |\n\
             1 |   (1 +\n\
             \x20 |  __^\n\
             2 | |  2)\n\
             \x20 | |__^\n"
        );
    }

    #[test]
    fn labels_on_different_lines_fold_the_lines_between() {
        let text = format!("1 +\n{}2.5", "# filler\n".repeat(6));
        let source = file(&text);
        let end = u32::try_from(text.len()).unwrap();
        let diagnostic = Diagnostic::error("cannot add Int and Float", Span::new(2, 3))
            .with_context(Span::new(0, 1), "Int")
            .with_context(Span::new(end - 3, end), "Float");
        assert_eq!(
            diagnostic.render(&source),
            "error: cannot add Int and Float\n\
             \x20--> calc.gcsl:1:3\n\
             \x20 |\n\
             1 | 1 +\n\
             \x20 | - ^\n\
             \x20 | |\n\
             \x20 | Int\n\
             ...\n\
             8 | 2.5\n\
             \x20 | --- Float\n"
        );
    }

    #[test]
    fn empty_span_at_end_of_input() {
        let source = file("1 +");
        let diagnostic = Diagnostic::error("oops", Span::new(3, 3));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20--> calc.gcsl:1:4\n\
             \x20 |\n\
             1 | 1 +\n\
             \x20 |    ^\n"
        );
    }

    #[test]
    fn empty_source() {
        let source = file("");
        let diagnostic = Diagnostic::error("oops", Span::new(0, 0));
        assert_eq!(
            diagnostic.render(&source),
            "error: oops\n\
             \x20--> calc.gcsl:1:1\n\
             \x20 |\n\
             1 |\n\
             \x20 | ^\n"
        );
    }

    #[test]
    fn long_lines_are_trimmed_around_the_span() {
        let text = format!("{}1 + 2.5{}", "1 + ".repeat(60), " + 1".repeat(60));
        let source = file(&text);
        let start = u32::try_from("1 + ".len() * 60).unwrap();
        let diagnostic =
            Diagnostic::error("cannot add Int and Float", Span::new(start + 2, start + 3))
                .with_context(Span::new(start, start + 1), "Int")
                .with_context(Span::new(start + 4, start + 7), "Float");
        let rendered = Renderer::plain()
            .term_width(60)
            .render(&diagnostic, &source);
        assert!(
            rendered.contains("...+ 1 + 1 + 1 + 1 + 1 + 2.5 + 1 + 1 + 1 + 1 + 1 + 1 ..."),
            "{rendered}"
        );
        assert!(rendered.contains("- ^ --- Float"), "{rendered}");
        assert!(
            rendered.lines().all(|line| line.chars().count() <= 60),
            "{rendered}"
        );
    }

    #[test]
    fn render_all_separates_with_blank_lines() {
        let source = file("1 + 2");
        let diagnostics = [
            Diagnostic::error("first", Span::new(0, 1)),
            Diagnostic::error("second", Span::new(4, 5)),
        ];
        let rendered = Renderer::plain().render_all(&diagnostics, &source);
        assert_eq!(
            rendered,
            "error: first\n\
             \x20--> calc.gcsl:1:1\n\
             \x20 |\n\
             1 | 1 + 2\n\
             \x20 | ^\n\
             \n\
             error: second\n\
             \x20--> calc.gcsl:1:5\n\
             \x20 |\n\
             1 | 1 + 2\n\
             \x20 |     ^\n"
        );
    }

    #[test]
    fn styled_output_contains_escape_codes_and_plain_does_not() {
        let source = file("1 +");
        let diagnostic = Diagnostic::error("oops", Span::new(3, 3));
        assert!(
            Renderer::styled()
                .render(&diagnostic, &source)
                .contains("\x1b[")
        );
        assert!(
            !Renderer::plain()
                .render(&diagnostic, &source)
                .contains("\x1b[")
        );
    }

    #[test]
    fn warnings_render_with_their_severity() {
        let source = file("{ x = 1 }");
        let diagnostic = Diagnostic::warning("unused", Span::new(2, 3))
            .with_context(Span::new(6, 7), "here")
            .with_note("a note");
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(Severity::Warning.to_string(), "warning");
        assert_eq!(Severity::Error.to_string(), "error");
        assert_eq!(
            diagnostic.render(&source),
            "warning: unused\n\
             \x20--> calc.gcsl:1:3\n\
             \x20 |\n\
             1 | { x = 1 }\n\
             \x20 |   ^   - here\n\
             \x20 |\n\
             \x20 = note: a note\n"
        );
    }

    #[test]
    fn merging_puts_diagnostics_in_source_order() {
        let errors = vec![
            Diagnostic::error("e5", Span::new(5, 6)),
            Diagnostic::error("e1", Span::new(1, 2)),
            Diagnostic::error("e3", Span::new(3, 4)),
        ];
        let warnings = vec![
            Diagnostic::warning("w3", Span::new(3, 9)),
            Diagnostic::warning("w0", Span::new(0, 1)),
        ];
        let messages: Vec<String> = merge(errors, warnings)
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect();
        assert_eq!(messages, vec!["w0", "e1", "e3", "w3", "e5"]);
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
}
