//! `gcsl run FILE`: run a file, or dump its intermediate stages.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use gcsl::{Diagnostic, SourceFile, check, eval, lexer, lower, parser};

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
/// last one; with no dumps, prints the value.
fn execute(args: &RunArgs, source: &SourceFile) -> Result<(), Failure> {
    let last = args.last_dump();
    let mut out = io::stdout().lock();

    let tokens = lexer::tokenize(source)?;
    if args.dump_tokens {
        for token in &tokens {
            writeln!(out, "{:?} @{}", token.kind, token.span)?;
        }
    }
    if last == Some(Stage::Tokens) {
        return Ok(out.flush()?);
    }

    let ast = parser::parse(&tokens, source)?;
    if args.dump_ast {
        writeln!(out, "{}", ast.pretty())?;
    }
    if last == Some(Stage::Ast) {
        return Ok(out.flush()?);
    }

    let types = check::check(&ast).map_err(Failure::Program)?;
    if args.dump_types {
        writeln!(out, "{}", ast.pretty_with(&types))?;
    }
    if last == Some(Stage::Types) {
        return Ok(out.flush()?);
    }

    let ir = lower::lower(&ast, &types).map_err(Diagnostic::from)?;
    if args.dump_ir {
        writeln!(out, "{}", ir.pretty())?;
    }
    if last == Some(Stage::Ir) {
        return Ok(out.flush()?);
    }

    let value = eval::eval(&ir)?;
    writeln!(out, "{value}")?;
    Ok(out.flush()?)
}
