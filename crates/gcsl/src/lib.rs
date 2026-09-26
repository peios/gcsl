//! GCSL: a statically typed scripting language.
//!
//! A program is a block body: a sequence of items (`let` declarations,
//! named functions, `:=` rebindings, fields, a `= EXPR` result and
//! expression statements)
//! whose value is its result, or the record of its fields. It passes
//! through these stages, each exposed as its own module so that tools can
//! stop after any of them:
//!
//! 1. [`lexer::tokenize`]: source text to tokens, interning every name into
//!    a [`names::Names`] table and storing the processed text of every
//!    string literal in a [`strings::Strings`] table.
//! 2. [`parser::parse`]: tokens to an arena-based [`ast::Ast`], which owns
//!    both tables from then on.
//! 3. [`check::check`]: types for every expression, interned into a
//!    [`TypeTable`] the caller owns, and a storage slot for every variable,
//!    in the side tables of a [`check::Checked`]; and any warnings.
//! 4. [`lower::lower`]: AST and tables to a type-specialised [`ir::Ir`],
//!    which holds its own copies of the string literals and record layouts
//!    it uses, and the code of every function.
//! 5. [`eval::eval`]: IR, shared by reference count with the closures made
//!    from it, to a [`Value`].
//!
//! [`run`] performs all of them on one file. A [`Session`] performs them on
//! a sequence of inputs that share their variables, as the REPL needs. Every
//! failure is a [`Diagnostic`] that can be rendered against the
//! [`SourceFile`], and so is every warning, which does not stop a program
//! from running and is returned with its value in an [`Outcome`]. The
//! library performs no I/O and never prints.

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
pub mod strings;
pub mod types;
pub mod value;

use std::rc::Rc;

pub use diagnostic::{Diagnostic, Severity};
pub use session::Session;
pub use source::{SourceFile, Span};
pub use types::{TyId, TypeTable};
pub use value::{Value, ValueKind};

/// The deepest nesting accepted, measured as the height of the syntax tree:
/// a leaf is one level, and every operator, parenthesis, ascription, field
/// access, block, `if`, interpolated string, function expression and call
/// adds one to its tallest child (a block's children being its items'
/// expressions and annotated types, an ascription's its expression and
/// type, a function's its parameter types, return type and body, and a
/// call's its callee and arguments). A type written as a name is one level,
/// a record type one more than its tallest field's type, and a function
/// type one more than the tallest of its parameter and return types. A `let`, field or `:=` with a dotted path is two levels taller than
/// its value for each segment of the path after the first, since later
/// stages build a record for each segment and read the old one's field
/// through all those before it. Every stage recurses once per
/// level, so the parser rejects anything deeper with an "expression nested
/// too deeply" error rather than letting a later stage overflow the stack.
/// The lexer, which recurses through string literals nested in each other's
/// interpolations, rejects those nested more than this many deep with the
/// same error.
///
/// The limit counts left-nested chains too: `1 + 1 + ... + 1` with ten
/// thousand terms is ten thousand levels deep, because each `+` is the left
/// operand of the next. Constructs that nest by recursion in the parser are
/// also limited by its recursion depth, where a nested block counts two
/// levels (its body and the expressions in it), so blocks nest at most
/// about `MAX_DEPTH / 2` deep.
///
/// It is also the budget for nesting at runtime, which calls share with
/// expressions, since calls nest on the Rust stack as expressions do and
/// GCSL does not eliminate tail calls. There, a level is a node of the IR:
/// every node on the path from a frame's body to the node being evaluated
/// is one level, and each call in progress costs [`CALL_LEVELS`] more on
/// top of the nesting around it. A call is made only if the nesting it
/// would reach, the whole height of its callee's body included, stays
/// within this budget; otherwise it fails with "call nested too deeply".
/// Nesting built by calls therefore never exceeds `MAX_DEPTH` levels,
/// whatever shape the recursion has. (The top level's own nesting is not
/// checked: it is bounded by the parser's limit alone.)
pub const MAX_DEPTH: usize = 10_000;

/// The levels of [`MAX_DEPTH`]'s runtime budget that a call costs on top of
/// the nesting around it: the Rust frames a call itself adds, expressed in
/// levels of expression nesting. Measured in an unoptimised build, a call
/// takes about 9 KB of Rust stack, against 2.4 to 3.3 KB for a level of
/// expression nesting, depending on its kind, so a call is charged as four
/// levels.
pub const CALL_LEVELS: usize = 4;

/// The stack size, in bytes, that the pipeline is run with, even in an
/// unoptimised build. Run the pipeline on a thread with at least this much
/// stack: the `gcsl` binary does, and so should any embedder. Stack memory
/// is reserved, not committed, so an unused reservation costs nothing.
///
/// It must cover every stage at [`MAX_DEPTH`], and evaluation at the
/// runtime budget `MAX_DEPTH` sets: `MAX_DEPTH` levels of the costliest kind
/// of nesting, calls included, which the budget guarantees a program never
/// exceeds. The end-to-end tests check both, unoptimised: every stage on
/// trees `MAX_DEPTH` deep, and recursions that use the whole budget with
/// their calls wrapped in each costly kind of nesting.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;

/// What running a program successfully produces: its value, and the
/// warnings found on the way, in source order.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The value of the program's body.
    pub value: Value,
    /// The warnings found, in source order. The program ran regardless.
    pub warnings: Vec<Diagnostic>,
}

/// Runs every stage on `source` as a file and returns the value of its body
/// with any warnings.
///
/// # Errors
///
/// Returns the diagnostics of the first stage that fails: one for a lexical,
/// syntax or runtime error, and every type error found by the checker. Any
/// warnings found before the failure are among them, all in source order.
pub fn run(source: &SourceFile) -> Result<Outcome, Vec<Diagnostic>> {
    let (tokens, names, strings) = lexer::tokenize(source).map_err(|error| vec![error])?;
    let ast = parser::parse(&tokens, names, strings, source).map_err(|error| vec![error])?;
    let mut types = TypeTable::new();
    let (checked, warnings) = check::check(&ast, &mut types)?;
    let value = lower::lower(&ast, &checked, &types, 0)
        .map_err(Diagnostic::from)
        .and_then(|ir| eval::eval(&Rc::new(ir)));
    match value {
        Ok(value) => Ok(Outcome { value, warnings }),
        Err(error) => Err(diagnostic::merge(vec![error], warnings)),
    }
}
