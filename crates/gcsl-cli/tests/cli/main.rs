//! Tests that run the built `gcsl` binary.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use fileroot::crate_dir;
use repl::repl;

/// Resolve Cargo's absolute executable or Bazel's runfiles-relative executable before
/// a child changes its working directory for fixture paths.
fn binary() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join(env!("CARGO_BIN_EXE_gcsl"))
}

/// Runs `gcsl` with `args` from this crate's directory, so fixture paths
/// (and therefore the file names in diagnostics) are stable.
fn gcsl(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .current_dir(crate_dir())
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

mod basics;
mod collections;
mod declarations;
mod diagnostics;
mod fileroot;
mod fs;
mod programs;
mod repl;
mod tools;
mod values;
mod workspaces;
