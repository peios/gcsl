//! `gcsl`: the command-line interface to the GCSL language.
//!
//! With no arguments it starts a REPL; `gcsl run FILE` runs a file. All
//! language work is done by the `gcsl` library, and finding and reading a
//! workspace's files by the `gcsl-disk` crate, which `pekit` shares; this
//! crate only reads its input, prints output and chooses exit codes.

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
    /// Run a GCSL file and print its value, unless it is `{}`.
    Run(run::RunArgs),
}

fn main() -> ExitCode {
    // Exits with code 2 on a usage error, and 0 after `--help`/`--version`.
    let cli = Cli::parse();
    // Every stage of the pipeline recurses once per level of nesting, up to
    // `gcsl::MAX_DEPTH`. The main thread's stack is platform-dependent and
    // often too small for that, so all the work runs on a thread with the
    // stack the library asks for. The memory is only reserved, not used.
    let worker = std::thread::Builder::new()
        .name("gcsl".to_owned())
        .stack_size(gcsl::STACK_SIZE)
        .spawn(move || match cli.command {
            None => repl::main(),
            Some(Command::Run(args)) => run::main(&args),
        });
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => {
            output::error("internal error: the interpreter thread panicked");
            ExitCode::from(EXIT_USAGE_OR_IO)
        }
        Err(error) => {
            output::error(format_args!("cannot start the interpreter thread: {error}"));
            ExitCode::from(EXIT_USAGE_OR_IO)
        }
    }
}
