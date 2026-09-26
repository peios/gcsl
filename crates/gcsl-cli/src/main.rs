//! `gcsl`: the command-line interface to the GCSL language.
//!
//! With no arguments it starts a REPL; `gcsl run FILE` runs a file. All
//! language work is done by the `gcsl` library; this crate only reads input,
//! prints output and chooses exit codes.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro
)]

mod output;
mod repl;
mod run;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Exit code for a program that failed to compile or run.
const EXIT_PROGRAM_ERROR: u8 = 1;
/// Exit code for bad usage or an I/O failure. Also what `clap` uses for
/// usage errors.
const EXIT_USAGE_OR_IO: u8 = 2;

/// Run GCSL programs. With no command, starts an interactive REPL.
#[derive(Parser)]
#[command(name = "gcsl", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run a GCSL file and print its value.
    Run(run::RunArgs),
}

fn main() -> ExitCode {
    // Exits with code 2 on a usage error, and 0 after `--help`/`--version`.
    let cli = Cli::parse();
    match cli.command {
        None => repl::main(),
        Some(Command::Run(args)) => run::main(&args),
    }
}
