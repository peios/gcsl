use super::*;
use crate::ast::{AttributeId, ForHead, StrPart};
use crate::diagnostic::Severity;
use crate::lexer::{tokenize, tokenize_with};
use crate::parser::parse;
use crate::source::SourceFile;
use crate::types::{Laziness, ValueTest};
use crate::value::ValueKind;

use scalars::{PRELUDE, VARIABLES};
use functions::{at, captures, last, nth, resolved, uses};
use futures::{IMPLICIT_AWAIT_NOTE, assert_implicit_await_error, marks};
use inference::{
    awaited_paths, capture_note, demands, first_bytes, fn_type, implicit_await_notes, type_at,
    type_of_f,
};
use callbacks::type_of_run;
use unions::warning_messages;
use jumps::diagnostics_or_none;
use defaults::defaults_of;

fn parse_text(text: &str) -> Ast {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    parse(&tokens, names, strings, &source).unwrap()
}

/// The expression of the file's final result or expression item.
fn result_expr(ast: &Ast) -> ExprId {
    let last = *ast.items(ast.root()).last().unwrap();
    match ast.item(last).kind {
        ItemKind::Result(value) | ItemKind::Expr(value) => value,
        other => panic!("the last item has no result: {other:?}"),
    }
}

/// Checks `text` as a file with a fresh type table.
fn check_text(text: &str) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    let ast = parse_text(text);
    let mut types = TypeTable::new();
    let output = Checker::new(&ast, &mut types).check();
    (ast, types, output)
}

/// Checks `text` as a lone file is, beside the standard library, so
/// that the prelude's names, `len` among them, are its: what the
/// helpers that look only at types and diagnostics check with.
fn check_prelude(text: &str) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    crate::workspace::check_beside_prelude(text)
}

/// The number of slots the standard library takes, the first: where a
/// text checked by [`check_prelude`] starts its own.
fn standard_slots() -> u32 {
    let (_, _, output) = check_prelude("");
    u32::try_from(output.unwrap().scope.into_slot_names().len()).unwrap()
}

/// The type of the file's result.
fn result_type(text: &str) -> TyId {
    let (ast, _, output) = check_prelude(text);
    output.unwrap().checked.types.get(result_expr(&ast))
}

/// The type of the file's result, as shown to users.
fn shown_result(text: &str) -> String {
    let (ast, types, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let ty = output.checked.types.get(result_expr(&ast));
    types.display(ty, ast.names()).to_string()
}

/// The type of the file's value, as shown to users.
fn file_type(text: &str) -> String {
    let (ast, types, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    types.display(output.ty, ast.names()).to_string()
}

fn diagnostics(text: &str) -> Vec<Diagnostic> {
    check_prelude(text).2.unwrap_err()
}

/// The warnings of a check that must succeed.
fn warnings(text: &str) -> Vec<Diagnostic> {
    check_prelude(text)
        .2
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
        .warnings
}

fn errors(text: &str) -> Vec<(String, Span)> {
    diagnostics(text)
        .into_iter()
        .map(|d| (d.message, d.span))
        .collect()
}

fn messages(text: &str) -> Vec<String> {
    errors(text).into_iter().map(|(m, _)| m).collect()
}

/// Every node's type in allocation order (children before parents).
fn all_types(text: &str) -> Vec<TyId> {
    check_prelude(text).2.unwrap().checked.types.types
}

/// The labels of a diagnostic as `(span, text)` pairs.
fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
    diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect()
}

// A concrete Int operand is written `(7 // 2)`: `//` always produces a
// concrete Int, so the typing rows with a concrete Int are reachable
// without variables.

// ----- Records -----

// ----- Dotted fields -----

// ----- Dotted `let` and `:=` -----

// ----- Total equality -----

// ----- Functions -----

// ----- Futures -----

// ----- Inferred parameters -----

// ----- Inferred results -----

// ----- Dotted updates of records that may be futures -----

// ----- Awaiting a captured value before a pure function is created -----

// ----- Memoised functions -----

// ----- Narrowed callbacks -----

// ----- Lists -----

// ----- Spreads -----

// ----- Ranges and `for` -----

// ----- Mutable variables -----

// ----- Updating an element -----

// ----- Unions -----

// ----- Loops, `break`, `continue`, labels and Never -----

// ----- `return`, labelled blocks and loops that return -----

// ----- Parameter defaults -----

// ----- Field defaults -----

// ----- Quantities -----

// ----- Attributes -----

// ----- Generics -----

// ----- Methods -----

// ----- Enums and partial records -----

mod basics;
mod builtins;
mod callbacks;
mod defaults;
mod extensions;
mod functions;
mod futures;
mod inference;
mod jumps;
mod lists;
mod loops;
mod memoised;
mod mutation;
mod records;
mod scalars;
mod suggestions;
mod toplevel;
mod unions;
