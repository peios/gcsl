//! The interactive REPL: one expression per line.

use std::io::{self, Write};
use std::process::ExitCode;

use gcsl::SourceFile;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use crate::{EXIT_USAGE_OR_IO, output};

/// Name given to each REPL line in diagnostics.
const SOURCE_NAME: &str = "<repl>";
const PROMPT: &str = "> ";

/// Runs the REPL until end of input (Ctrl-D) and returns the exit code.
///
/// Each non-blank line is evaluated on its own and prints `value : Type` or
/// its diagnostics. Ctrl-C discards the current line. History lasts for the
/// session only.
pub fn main() -> ExitCode {
    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(error) => {
            output::error(format_args!("cannot start the REPL: {error}"));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    loop {
        match editor.readline(PROMPT) {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                // Only fails if the history is misconfigured; history is a
                // convenience, so the line is still evaluated.
                let _ = editor.add_history_entry(line.as_str());
                if let Err(error) = evaluate(line) {
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

/// Evaluates one line, printing the value and its type or the diagnostics.
fn evaluate(line: String) -> io::Result<()> {
    let source = match SourceFile::new(SOURCE_NAME, line) {
        Ok(source) => source,
        Err(error) => {
            output::error(error);
            return Ok(());
        }
    };
    match gcsl::run(&source) {
        Ok(value) => {
            let mut out = io::stdout().lock();
            writeln!(out, "{value} : {}", value.ty())?;
            out.flush()
        }
        Err(diagnostics) => {
            output::diagnostics(&source, &diagnostics);
            Ok(())
        }
    }
}
