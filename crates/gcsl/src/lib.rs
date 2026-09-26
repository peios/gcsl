//! GCSL: a statically typed scripting language.
//!
//! A program passes through these stages, each exposed as its own module so
//! that tools can stop after any of them:
//!
//! 1. [`lexer::tokenize`]: source text to tokens.
//! 2. [`parser::parse`]: tokens to an arena-based [`ast::Ast`].
//! 3. [`check::check`]: types for every AST node, in a [`check::Types`] side
//!    table.
//! 4. [`lower::lower`]: AST and types to a type-specialised [`ir::Ir`].
//! 5. [`eval::eval`]: IR to a [`Value`].
//!
//! [`run`] performs all of them. Every failure is a [`Diagnostic`] that can
//! be rendered against the [`SourceFile`]. The library performs no I/O and
//! never prints.

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

pub mod ast;
pub mod check;
pub mod diagnostic;
pub mod eval;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod source;
pub mod types;
pub mod value;

pub use diagnostic::{Diagnostic, Severity};
pub use source::{SourceFile, Span};
pub use types::Ty;
pub use value::Value;

/// Runs every stage on `source` and returns the value of its expression.
///
/// # Errors
///
/// Returns the diagnostics of the first stage that fails: one for a lexical,
/// syntax or runtime error, and every type error found by the checker.
pub fn run(source: &SourceFile) -> Result<Value, Vec<Diagnostic>> {
    let tokens = lexer::tokenize(source).map_err(|error| vec![error])?;
    let ast = parser::parse(&tokens, source).map_err(|error| vec![error])?;
    let types = check::check(&ast)?;
    let ir = lower::lower(&ast, &types).map_err(|error| vec![error.into()])?;
    eval::eval(&ir).map_err(|error| vec![error])
}
