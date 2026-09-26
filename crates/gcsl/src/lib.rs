//! GCSL: a statically typed scripting language.
//!
//! A program is a block body: a sequence of items (`let` declarations,
//! `:=` rebindings, a `= EXPR` result and expression statements) whose value
//! is its result. It passes through these stages, each exposed as its own
//! module so that tools can stop after any of them:
//!
//! 1. [`lexer::tokenize`]: source text to tokens, interning every name into
//!    a [`names::Names`] table.
//! 2. [`parser::parse`]: tokens to an arena-based [`ast::Ast`], which owns
//!    the names from then on.
//! 3. [`check::check`]: types for every expression and a storage slot for
//!    every variable, in the side tables of a [`check::Checked`].
//! 4. [`lower::lower`]: AST and tables to a type-specialised [`ir::Ir`].
//! 5. [`eval::eval`]: IR to a [`Value`].
//!
//! [`run`] performs all of them on one file. A [`Session`] performs them on
//! a sequence of inputs that share their variables, as the REPL needs. Every
//! failure is a [`Diagnostic`] that can be rendered against the
//! [`SourceFile`]. The library performs no I/O and never prints.

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
pub mod names;
pub mod parser;
pub mod session;
pub mod source;
pub mod types;
pub mod value;

pub use diagnostic::{Diagnostic, Severity};
pub use session::Session;
pub use source::{SourceFile, Span};
pub use types::Ty;
pub use value::Value;

/// The deepest nesting accepted, measured as the height of the syntax tree:
/// a leaf is one level, and every operator, parenthesis, ascription and
/// block adds one to its tallest child (a block's children being its items'
/// expressions). Every stage recurses once per level, so the parser rejects
/// anything deeper with an "expression nested too deeply" error rather than
/// letting a later stage overflow the stack.
///
/// The limit counts left-nested chains too: `1 + 1 + ... + 1` with ten
/// thousand terms is ten thousand levels deep, because each `+` is the left
/// operand of the next. Constructs that nest by recursion in the parser are
/// also limited by its recursion depth, where a nested block counts two
/// levels (its body and the expressions in it), so blocks nest at most
/// about `MAX_DEPTH / 2` deep.
pub const MAX_DEPTH: usize = 10_000;

/// The stack size, in bytes, that guarantees no stage overflows at
/// [`MAX_DEPTH`], even in an unoptimised build. Run the pipeline on a thread
/// with at least this much stack: the `gcsl` binary does, and so should any
/// embedder. Stack memory is reserved, not committed, so an unused
/// reservation costs nothing.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;

/// Runs every stage on `source` as a file and returns the value of its body.
///
/// # Errors
///
/// Returns the diagnostics of the first stage that fails: one for a lexical,
/// syntax or runtime error, and every type error found by the checker.
pub fn run(source: &SourceFile) -> Result<Value, Vec<Diagnostic>> {
    let (tokens, names) = lexer::tokenize(source).map_err(|error| vec![error])?;
    let ast = parser::parse(&tokens, names, source).map_err(|error| vec![error])?;
    let checked = check::check(&ast)?;
    let ir = lower::lower(&ast, &checked).map_err(|error| vec![error.into()])?;
    eval::eval(&ir).map_err(|error| vec![error])
}
