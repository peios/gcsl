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

/// Styled output when stderr is a terminal and colour has not been disabled
/// through the `NO_COLOR` convention (any non-empty value disables it).
fn renderer() -> Renderer {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    if io::stderr().is_terminal() && !no_color {
        Renderer::styled()
    } else {
        Renderer::plain()
    }
}
