//! `gcsl run FILE`: run a file, or dump its intermediate stages.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use clap::Args;
use gcsl::lexer::TokenKind;
use gcsl::strings::Quoted;
use gcsl::{
    Diagnostic, SourceFile, TypeTable, Value, check, diagnostic, eval, lexer, lower, parser,
};

use crate::{EXIT_PROGRAM_ERROR, EXIT_USAGE_OR_IO, output};

/// Arguments of `gcsl run`.
#[derive(Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent command-line flag"
)]
pub struct RunArgs {
    /// The GCSL source file to run.
    file: PathBuf,
    /// Print the tokens.
    #[arg(long)]
    dump_tokens: bool,
    /// Print the syntax tree.
    #[arg(long)]
    dump_ast: bool,
    /// Print the syntax tree annotated with types.
    #[arg(long)]
    dump_types: bool,
    /// Print the lowered IR.
    #[arg(long)]
    dump_ir: bool,
}

impl RunArgs {
    /// The last stage to run, if any dump was requested. Running stops after
    /// it instead of evaluating.
    fn last_dump(&self) -> Option<Stage> {
        [
            (self.dump_ir, Stage::Ir),
            (self.dump_types, Stage::Types),
            (self.dump_ast, Stage::Ast),
            (self.dump_tokens, Stage::Tokens),
        ]
        .into_iter()
        .find_map(|(requested, stage)| requested.then_some(stage))
    }
}

/// The dumpable pipeline stages, in pipeline order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Tokens,
    Ast,
    Types,
    Ir,
}

/// Why a run did not succeed.
enum Failure {
    /// The program has errors: exit code 1.
    Program(Vec<Diagnostic>),
    /// Writing the output failed: exit code 2.
    Output(io::Error),
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Failure::Output(error)
    }
}

impl From<Diagnostic> for Failure {
    fn from(diagnostic: Diagnostic) -> Self {
        Failure::Program(vec![diagnostic])
    }
}

/// Runs `gcsl run` and returns the process exit code.
pub fn main(args: &RunArgs) -> ExitCode {
    let path = args.file.display();
    let text = match fs::read_to_string(&args.file) {
        Ok(text) => text,
        Err(error) => {
            output::error(format_args!("cannot read {path}: {error}"));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    let source = match SourceFile::new(path.to_string(), text) {
        Ok(source) => source,
        Err(error) => {
            output::error(format_args!("cannot load {path}: {error}"));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    match execute(args, &source) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Program(diagnostics)) => {
            output::diagnostics(&source, &diagnostics);
            ExitCode::from(EXIT_PROGRAM_ERROR)
        }
        Err(Failure::Output(error)) => {
            output::error(format_args!("cannot write output: {error}"));
            ExitCode::from(EXIT_USAGE_OR_IO)
        }
    }
}

/// Runs the pipeline, printing each requested dump and stopping after the
/// last one; with no dumps, prints the value, as text, unless it is `{}`.
///
/// Warnings go to stderr once the run has succeeded (after the dumps, or
/// before the value). If a later stage fails, they are returned with its
/// error instead, to be printed together in source order.
fn execute(args: &RunArgs, source: &SourceFile) -> Result<(), Failure> {
    let last = args.last_dump();
    let mut out = io::stdout().lock();

    let (tokens, names, strings) = lexer::tokenize(source)?;
    if args.dump_tokens {
        // Names by their text and string pieces by their quoted text, as in
        // `Ident(x)` and `StrStart("a\n")`.
        for token in &tokens {
            let span = token.span;
            match token.kind {
                TokenKind::Ident(name) => writeln!(out, "Ident({}) @{span}", names.get(name))?,
                TokenKind::Str(id) => writeln!(out, "Str({}) @{span}", Quoted(strings.get(id)))?,
                TokenKind::StrStart(id) => {
                    writeln!(out, "StrStart({}) @{span}", Quoted(strings.get(id)))?;
                }
                TokenKind::StrMid(id) => {
                    writeln!(out, "StrMid({}) @{span}", Quoted(strings.get(id)))?;
                }
                TokenKind::StrEnd(id) => {
                    writeln!(out, "StrEnd({}) @{span}", Quoted(strings.get(id)))?;
                }
                kind => writeln!(out, "{kind:?} @{span}")?,
            }
        }
    }
    if last == Some(Stage::Tokens) {
        return Ok(out.flush()?);
    }

    let ast = parser::parse(&tokens, names, strings, source)?;
    if args.dump_ast {
        writeln!(out, "{}", ast.pretty())?;
    }
    if last == Some(Stage::Ast) {
        return Ok(out.flush()?);
    }

    let mut types = TypeTable::new();
    let (checked, warnings) = check::check(&ast, &mut types).map_err(Failure::Program)?;
    if args.dump_types {
        let annotation = checked.types.annotation(&types, ast.names());
        writeln!(out, "{}", ast.pretty_with(&annotation))?;
    }
    if last == Some(Stage::Types) {
        output::diagnostics(source, &warnings);
        return Ok(out.flush()?);
    }

    let ir = match lower::lower(&ast, &checked, &types, 0) {
        Ok(ir) => Rc::new(ir),
        Err(error) => return Err(with_warnings(error.into(), warnings)),
    };
    if args.dump_ir {
        writeln!(out, "{}", ir.pretty(ast.names()))?;
    }
    if last == Some(Stage::Ir) {
        output::diagnostics(source, &warnings);
        return Ok(out.flush()?);
    }

    let value = match eval::eval(&ir) {
        Ok(value) => value,
        Err(error) => return Err(with_warnings(error, warnings)),
    };
    output::diagnostics(source, &warnings);
    // A string prints as its text, unquoted, so a program can produce
    // output verbatim. A record prints as it would be written.
    if !matches!(value, Value::Empty) {
        writeln!(out, "{value}")?;
    }
    Ok(out.flush()?)
}

/// The failure of a stage after the checker: its `error`, with the
/// `warnings` the checker found, in source order.
fn with_warnings(error: Diagnostic, warnings: Vec<Diagnostic>) -> Failure {
    Failure::Program(diagnostic::merge(vec![error], warnings))
}
