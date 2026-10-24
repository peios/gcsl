use super::*;
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use crate::CALL_LEVELS;
use crate::ast::FnId;
use crate::check::check;
use crate::ir::{FnKind, LazyShapeId, Operands, ShapedOperands};
use crate::lexer::tokenize;
use crate::lower::lower;
use crate::parser::parse;
use crate::source::SourceFile;
use crate::types::TypeTable;
use crate::value::{Future, FutureState, ValueKind};
use super::memo::{Ambient, Lookup, MIN_SWEEP_THRESHOLD, Mark, Running, key_hash};

use toplevel::assert_cells_scheduled;
use arithmetic::{MAX, MIN, boolean, booleans, error, float, int, run, string};
use calls::{BUDGET_ERROR, BUDGET_NOTE, on_big_stack, run_on};
use futures::{AWAIT_WRAP, await_nesting_kinds, frame_after, labels};
use memo::{fields, hits, same};
use lists::{at, shown};

fn compile(text: &str) -> Rc<Ir> {
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let (tokens, names, strings) = tokenize(&source).unwrap();
    let ast = parse(&tokens, names, strings, &source).unwrap();
    let mut types = TypeTable::new();
    let (checked, _) = check(&ast, &mut types).unwrap();
    let ir = Rc::new(lower(&ast, &checked, &types, 0).unwrap());
    assert_cells_scheduled(&ir, checked.resolution.field_cells(), text);
    ir
}

/// As [`compile`], but checking `text` as a lone file is, beside the
/// standard library, so that the prelude's names, `len` among them, are
/// its: what the tests of `len` compile with. Its frame's first slots
/// are the standard library's, which a run of it alone leaves `{}`: a
/// call of `len` never reads its cell.
fn compile_prelude(text: &str) -> Rc<Ir> {
    let (ast, types, output) = crate::workspace::check_beside_prelude(text);
    let checked = output
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
        .checked;
    let ir = Rc::new(lower(&ast, &checked, &types, 0).unwrap());
    assert_cells_scheduled(&ir, checked.resolution.field_cells(), text);
    ir
}

// ----- Functions -----

// ----- Futures -----

// ----- Memoised functions -----

// ----- Lists -----

// ----- Spreads -----

// ----- Ranges and `for` -----

// ----- Updating an element -----

// ----- Unions -----

// ----- `return`, labelled blocks and loops that return -----

mod arithmetic;
mod calls;
mod control;
mod extensions;
mod futures;
mod lists;
mod memo;
mod mutation;
mod records;
mod toplevel;
mod values;
