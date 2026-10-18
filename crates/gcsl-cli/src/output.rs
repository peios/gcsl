//! Writing to stdout and stderr without panicking.
//!
//! `println!` and `eprintln!` panic when the stream is closed; these helpers
//! return or ignore the error instead.

use std::io::{self, IsTerminal, Write};

use gcsl::diagnostic::Renderer;
use gcsl::{Diagnostic, SourceFile};

/// Writes an error message line to stderr.
///
/// A failure to write to stderr is ignored: there is nowhere left to report
/// it.
pub fn error(message: impl std::fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "error: {message}");
}

/// Renders `diagnostics` against `source` to stderr, separated by blank
/// lines, with colour when stderr is a terminal.
///
/// A failure to write to stderr is ignored: there is nowhere left to report
/// it.
pub fn diagnostics(source: &SourceFile, diagnostics: &[Diagnostic]) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(renderer().render_all(diagnostics, source).as_bytes());
}

/// Renders `diagnostics` to stderr as [`diagnostics`] does, each part of a
/// diagnostic against the source `source_of` gives for the input it names
/// (`None` for none), for diagnostics that do not all refer to one source:
/// a REPL line's, an earlier line's, or a standard file's.
///
/// A stderr failure is ignored because there is nowhere left to report it.
pub fn diagnostics_from<'a>(
    diagnostics: &[Diagnostic],
    source_of: impl Fn(Option<u32>) -> &'a SourceFile,
) {
    let renderer = renderer();
    let text = diagnostics
        .iter()
        .map(|diagnostic| renderer.render_in(diagnostic, &source_of))
        .collect::<Vec<_>>()
        .join("\n");
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
}

/// Renders `diagnostics` to stderr as [`diagnostics`] does, for a
/// workspace: each part of a diagnostic against the source of the input it
/// names, `sources` being indexed by input number, or against `fallback`
/// when it names none.
///
/// A failure to write to stderr is ignored: there is nowhere left to report
/// it.
pub fn diagnostics_in(sources: &[SourceFile], fallback: &SourceFile, diagnostics: &[Diagnostic]) {
    let renderer = renderer();
    let source_of = |input: Option<u32>| {
        input
            .and_then(|input| sources.get(input as usize))
            .unwrap_or(fallback)
    };
    let text = diagnostics
        .iter()
        .map(|diagnostic| renderer.render_in(diagnostic, source_of))
        .collect::<Vec<_>>()
        .join("\n");
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
}

/// Styled output when stderr is a terminal and colour has not been disabled
/// through the `NO_COLOR` convention (any non-empty value disables it).
pub fn renderer() -> Renderer {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    if io::stderr().is_terminal() && !no_color {
        Renderer::styled()
    } else {
        Renderer::plain()
    }
}
