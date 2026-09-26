//! Writing to stdout and stderr without panicking.
//!
//! `println!` and `eprintln!` panic when the stream is closed; these helpers
//! return or ignore the error instead.

use std::io::{self, Write};

use gcsl::{Diagnostic, SourceFile};

/// Writes an error message line to stderr.
///
/// A failure to write to stderr is ignored: there is nowhere left to report
/// it.
pub fn error(message: impl std::fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "error: {message}");
}

/// Renders `diagnostics` against `source` to stderr, separated by blank
/// lines.
///
/// A failure to write to stderr is ignored: there is nowhere left to report
/// it.
pub fn diagnostics(source: &SourceFile, diagnostics: &[Diagnostic]) {
    let mut stderr = io::stderr().lock();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let separator = if index == 0 { "" } else { "\n" };
        let _ = write!(stderr, "{separator}{}", diagnostic.render(source));
    }
}
