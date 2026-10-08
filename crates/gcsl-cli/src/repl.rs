//! The interactive REPL: one input per line, sharing variables.

use std::io::{self, Write};
use std::process::ExitCode;

use gcsl::{Session, SourceFile, Value};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use crate::{EXIT_USAGE_OR_IO, output};

/// Name given to each REPL line in diagnostics.
const SOURCE_NAME: &str = "<repl>";
const PROMPT: &str = "> ";

/// Runs the REPL until end of input (Ctrl-D) and returns the exit code.
///
/// Each non-blank line is a block body evaluated in one [`Session`], so
/// variables declared at the top level of a line are visible to later
/// lines. A line prints `value : Type`, with the value as it would be
/// written in a program (a string quoted and escaped, see
/// [`Value::repr`]) and the type the checker gave it, nothing if its value
/// is `{}` (as for a `let`), or its diagnostics; a line with errors changes
/// nothing. A line's warnings go to stderr before its value. A runtime error
/// inside a function an earlier line defined is shown against that line,
/// and a part of a diagnostic in the standard library, which the session
/// runs before the first line, against its file (`<stdlib>/prelude.gcsl`).
/// The prelude's names, `len` among them, are every line's.
/// Ctrl-C discards the current line. History lasts for the session only.
pub fn main() -> ExitCode {
    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(error) => {
            output::error(format_args!("cannot start the REPL: {error}"));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    let mut session = Session::new();
    loop {
        match editor.readline(PROMPT) {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                // Only fails if the history is misconfigured; history is a
                // convenience, so the line is still evaluated.
                let _ = editor.add_history_entry(line.as_str());
                if let Err(error) = evaluate(&mut session, line) {
                    output::error(format_args!("cannot write output: {error}"));
                    return ExitCode::from(EXIT_USAGE_OR_IO);
                }
            }
            Err(ReadlineError::Interrupted) => {}
            Err(ReadlineError::Eof) => return ExitCode::SUCCESS,
            Err(error) => {
                output::error(format_args!("cannot read input: {error}"));
                return ExitCode::from(EXIT_USAGE_OR_IO);
            }
        }
    }
}

/// Evaluates one line in `session`, printing any warnings and then the
/// value and its type, or the diagnostics.
fn evaluate(session: &mut Session, line: String) -> io::Result<()> {
    let source = match SourceFile::new(SOURCE_NAME, line) {
        Ok(source) => source,
        Err(error) => {
            output::error(error);
            return Ok(());
        }
    };
    match session.eval(&source) {
        Ok(outcome) => {
            output::diagnostics(&source, &outcome.warnings);
            if matches!(outcome.value, Value::Empty) {
                return Ok(());
            }
            let ty = session
                .types()
                .display(session.value_type(), session.names());
            let mut out = io::stdout().lock();
            writeln!(out, "{} : {ty}", outcome.value.repr())?;
            out.flush()
        }
        Err(diagnostics) => {
            // A diagnostic from a function of an earlier line refers to
            // that line's source, and a part of one in the standard library
            // to the standard file's, `<stdlib>/prelude.gcsl`, the
            // session's first inputs.
            output::diagnostics_from(&diagnostics, |input| {
                input
                    .and_then(|input| session.source(input))
                    .unwrap_or(&source)
            });
            Ok(())
        }
    }
}
