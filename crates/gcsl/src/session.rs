//! A stateful session: a sequence of inputs sharing their variables, as in
//! the REPL.
//!
//! Each input is a block body, like a file, run in the context of every
//! earlier successful input: variables and fields declared at their top
//! level are visible, `:=` at top level may rebind variables, and new
//! top-level declarations persist for later inputs. The session keeps these
//! things between inputs:
//!
//! - the [`Names`] table, lent to each stage in turn, so that the same name
//!   in two inputs is the same [`crate::names::Name`];
//! - the [`TypeTable`], lent to the checker, so that a type interned by one
//!   input is the same [`TyId`] in the next, and the types of the variables
//!   carried over stay meaningful;
//! - the checker's outermost [`Scope`]: each top-level variable with its
//!   slot and type, from which [`Checker::with_scope`] checks the next
//!   input exactly as if it continued the same block;
//! - the value stack, whose bottom is the top level's frame, holding each
//!   top-level variable's value in its slot, and which calls grow and
//!   shrink back while an input runs;
//! - the type of the last successful input's value, for showing it;
//! - the source of every committed input, in order: input `n` is the `n`th
//!   to succeed, counting from 0, and each input is lowered with its
//!   number. A closure or a future outlives the input that made it, keeping
//!   its code alive, and when one made by an earlier input fails, its
//!   diagnostic names that input (see [`crate::Diagnostic::input`]), whose
//!   source [`Session::source`] returns for rendering it. A future that
//!   fails is left unrun, so a later input that awaits it runs it again;
//! - the [`MemoTables`] of every memoised (`->>`) closure any input has
//!   called, from lists of arguments to what the closure returned, so that
//!   a later input calling the closure with equal arguments gets the
//!   earlier result without running the body again (see
//!   [`crate::eval`]). A table lives as long as its closure: the session
//!   holds the closure weakly and drops the tables of dead closures from
//!   time to time; an entry of a live closure is never evicted.
//!
//! A failed input, whether it fails to compile or fails at runtime, leaves
//! the session as it was: the scope is only replaced after evaluation
//! succeeds, names interned by the failed input are forgotten, and its
//! source is not kept, so it takes no number. The frame may have grown, and
//! slots above the last committed variable may hold values from the failed
//! run, but those slots belong to no variable and are always written before
//! they are read. Types the failed input interned are
//! kept: a type is a fact about structure, not about the input, so a stale
//! entry is harmless (see [`crate::types`]); forgetting them would mean
//! rebuilding the table's lookup map. The memo entries the failed input made
//! are forgotten with it (see [`MemoTables::rollback`]): their keys and
//! results may be records with names it interned, or closures of its code,
//! which would mean something else, or nothing, to a later input. So a
//! memoised call the failed input made runs again when a later input makes
//! it, although its effects have happened.

use std::mem;
use std::rc::Rc;

use crate::Outcome;
use crate::ast::Ast;
use crate::check::{CheckOutput, Checker, Scope};
use crate::diagnostic::{self, Diagnostic};
use crate::eval::{self, MemoTables};
use crate::ir::Ir;
use crate::lexer;
use crate::lower;
use crate::names::Names;
use crate::parser;
use crate::source::SourceFile;
use crate::types::{TyId, TypeTable};
use crate::value::Value;

/// A sequence of inputs sharing their top-level variables. See the module
/// documentation.
#[derive(Clone, Debug)]
pub struct Session {
    names: Names,
    types: TypeTable,
    scope: Scope,
    /// The value stack; its bottom is the top level's frame.
    frame: Vec<Value>,
    value_type: TyId,
    /// The source of each committed input, indexed by its number.
    sources: Vec<SourceFile>,
    /// The memo tables of every memoised closure called by any input.
    memo: MemoTables,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session with no variables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            names: Names::new(),
            types: TypeTable::new(),
            scope: Scope::new(),
            frame: Vec::new(),
            value_type: TyId::EMPTY,
            sources: Vec::new(),
            memo: MemoTables::new(),
        }
    }

    /// The source of committed input number `input`, the `input`th to have
    /// succeeded, counting from 0; `None` if no input of that number has
    /// been committed, as for the input being run. Diagnostics whose
    /// [`Diagnostic::input`] is set are rendered against it.
    #[must_use]
    pub fn source(&self, input: u32) -> Option<&SourceFile> {
        self.sources.get(input as usize)
    }

    /// Runs `source` as the next input and returns its value, `{}` if it
    /// has no result or fields, with its warnings in source order.
    ///
    /// # Errors
    ///
    /// Returns the diagnostics of the first stage that fails, as
    /// [`crate::run`] does, with the input's warnings among them in source
    /// order. The session is then unchanged.
    pub fn eval(&mut self, source: &SourceFile) -> Result<Outcome, Vec<Diagnostic>> {
        let names_before = self.names.len();
        let memo_before = self.memo.checkpoint();
        let result = self.eval_uncommitted(source);
        if result.is_err() {
            // What the input memoised may hold its names and closures of
            // its code, which mean nothing once it is forgotten.
            self.memo.rollback(memo_before);
            self.names.truncate(names_before);
        }
        result
    }

    /// The table the session's types are interned into, for showing them.
    #[must_use]
    pub fn types(&self) -> &TypeTable {
        &self.types
    }

    /// The table the session's names are interned into, for showing them.
    #[must_use]
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// The type of the value of the last input that succeeded, as the
    /// checker found it: `{}` before any has. Show it with
    /// [`Session::types`] and [`Session::names`].
    #[must_use]
    pub fn value_type(&self) -> TyId {
        self.value_type
    }

    /// Runs `source`, committing its scope only on success. On failure the
    /// caller must forget the names it interned.
    fn eval_uncommitted(&mut self, source: &SourceFile) -> Result<Outcome, Vec<Diagnostic>> {
        let (tokens, strings) =
            lexer::tokenize_with(source, &mut self.names).map_err(|error| vec![error])?;
        let (builder, root) = parser::parse_parts(&tokens, source, &mut self.names, &strings)
            .map_err(|error| vec![error])?;
        // The tree owns the names while it lives; take them back before
        // anything can fail. The string table belongs to this input alone:
        // lowering copies what the IR needs out of it, and it is dropped
        // with the tree.
        let ast = builder.finish(root, mem::take(&mut self.names), strings);
        let input = u32::try_from(self.sources.len()).unwrap_or(u32::MAX);
        let compiled = compile(&ast, self.scope.clone(), &mut self.types, input);
        self.names = ast.into_names();
        let (ir, output) = compiled?;
        let ir = Rc::new(ir);
        let frame_size = ir.frame_size() as usize;
        if self.frame.len() < frame_size {
            self.frame.resize(frame_size, Value::Empty);
        }
        match eval::eval_with(&ir, &mut self.frame, &mut self.memo) {
            Ok(value) => {
                self.scope = output.scope;
                self.value_type = output.ty;
                self.sources.push(source.clone());
                Ok(Outcome {
                    value,
                    warnings: output.warnings,
                })
            }
            Err(mut error) => {
                // A closure of this input called from a closure of an
                // earlier one stamps this input's number: that is the input
                // being run, which is what no number means.
                if error.input == Some(input) {
                    error.input = None;
                }
                Err(diagnostic::merge(vec![error], output.warnings))
            }
        }
    }
}

/// Checks and lowers `ast`, session input number `input`, in the context
/// of `scope`, interning types into `types`, returning the IR and the
/// checker's output: the scope extended by the input's top-level
/// declarations, the value's type and the warnings.
fn compile(
    ast: &Ast,
    scope: Scope,
    types: &mut TypeTable,
    input: u32,
) -> Result<(Ir, CheckOutput), Vec<Diagnostic>> {
    let output = Checker::with_scope(ast, scope, types).check()?;
    match lower::lower(ast, &output.checked, types, input) {
        Ok(ir) => Ok((ir, output)),
        Err(error) => Err(diagnostic::merge(vec![error.into()], output.warnings)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;

    fn eval(session: &mut Session, text: &str) -> Result<Outcome, Vec<Diagnostic>> {
        session.eval(&SourceFile::new("<test>", text).unwrap())
    }

    fn value(session: &mut Session, text: &str) -> Value {
        match eval(session, text) {
            Ok(outcome) => outcome.value,
            Err(diagnostics) => panic!("{text}: unexpected errors {diagnostics:?}"),
        }
    }

    fn messages(session: &mut Session, text: &str) -> Vec<String> {
        eval(session, text)
            .unwrap_err()
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    /// The type of the last successful input's value, as shown.
    fn value_type(session: &Session) -> String {
        session
            .types()
            .display(session.value_type(), session.names())
            .to_string()
    }

    #[test]
    fn three_inputs_share_a_variable() {
        let mut session = Session::new();
        assert_eq!(value(&mut session, "let x = 1"), Value::Empty);
        assert_eq!(value(&mut session, "let y = x + 1"), Value::Empty);
        assert_eq!(value(&mut session, "x + y"), Value::Int(3));
        assert_eq!(value(&mut session, "x * 10"), Value::Int(10));
    }

    #[test]
    fn rebinding_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let x = 1");
        assert_eq!(value(&mut session, "x := x + 10"), Value::Empty);
        assert_eq!(value(&mut session, "x"), Value::Int(11));
        // A rebind keeps the variable's type; changing it takes a `let`.
        assert_eq!(messages(&mut session, "x := 2.5"), vec!["mismatched types"]);
        assert_eq!(value(&mut session, "x"), Value::Int(11));
        // Only at the top level: an input's block is an inner block.
        assert_eq!(
            messages(&mut session, "{ x := 1.5 }"),
            vec!["cannot rebind `x` here: it is declared in an enclosing block"]
        );
        assert_eq!(
            messages(&mut session, "y := 1"),
            vec!["cannot rebind `y`: no variable named `y` is declared"]
        );
    }

    #[test]
    fn shadowing_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let x = 1");
        value(&mut session, "let x = 2.5");
        assert_eq!(value(&mut session, "x"), Value::Float(2.5));
        assert_eq!(value(&mut session, "{ let x = 7; x }"), Value::Int(7));
        assert_eq!(value(&mut session, "x"), Value::Float(2.5));
    }

    #[test]
    fn a_failing_input_leaves_earlier_variables_intact() {
        let mut session = Session::new();
        value(&mut session, "let x = 5");
        let names = session.names.len();
        let scope = session.scope.clone();

        // A type error, a syntax error, a lexical error and a runtime error.
        assert_eq!(
            messages(&mut session, "let fresh = x + 1.5"),
            vec!["cannot add Int and Float"]
        );
        assert_eq!(
            messages(&mut session, "let other ="),
            vec!["expected expression, found end of input"]
        );
        assert_eq!(
            messages(&mut session, "let third = x-"),
            vec!["`-` here is ambiguous"]
        );
        assert_eq!(
            messages(&mut session, "let y = 1\nlet z = 1 // 0\nx := 0"),
            vec!["division by zero"]
        );
        // A quoted field name interned by a failed parse is forgotten too.
        assert_eq!(
            messages(&mut session, "\"quoted\" = 1 +"),
            vec!["expected expression, found end of input"]
        );
        assert_eq!(session.names.len(), names);
        assert_eq!(session.scope, scope);

        // Nothing the failed inputs declared exists, and `x` still holds its
        // value even though the runtime failure wrote the slot after it.
        assert_eq!(
            messages(&mut session, "fresh"),
            vec!["undefined variable `fresh`"]
        );
        assert_eq!(messages(&mut session, "y"), vec!["undefined variable `y`"]);
        assert_eq!(value(&mut session, "x"), Value::Int(5));
        assert_eq!(value(&mut session, "let w = 7\nw + x"), Value::Int(12));
    }

    #[test]
    fn the_frame_grows_as_needed() {
        let mut session = Session::new();
        value(&mut session, "let a = 1");
        assert_eq!(session.frame.len(), 1);
        value(&mut session, "{ let t = 1; let u = 2; let v = 3 }");
        // After each input, the stack is its top level's frame: the
        // temporaries' slots are gone, and reused by later variables, but
        // the capacity stays.
        assert_eq!(session.frame.len(), 4);
        value(&mut session, "let b = 2");
        assert_eq!(session.frame.len(), 2);
        assert!(session.frame.capacity() >= 4);
        assert_eq!(value(&mut session, "a + b"), Value::Int(3));
    }

    #[test]
    fn inputs_have_values_of_every_type() {
        let mut session = Session::new();
        assert_eq!(value(&mut session, ""), Value::Empty);
        assert_eq!(value_type(&session), "{}");
        assert_eq!(value(&mut session, "7 / 2"), Value::Float(3.5));
        assert_eq!(value_type(&session), "Float");
        assert_eq!(value(&mut session, "= 1 + 1"), Value::Int(2));
        assert_eq!(value_type(&session), "Int");
        assert_eq!(value(&mut session, "let u = {}\nu"), Value::Empty);
        assert_eq!(value(&mut session, "1 < 2"), Value::Bool(true));
        assert_eq!(
            value(&mut session, "\"a\" + \"b\""),
            Value::Str("ab".into())
        );
        assert_eq!(
            value(&mut session, "{ x = 1; y = \"a\" }").to_string(),
            "{x = 1; y = \"a\"}"
        );
        assert_eq!(value_type(&session), "{x: Int, y: String}");
        // A failed input leaves the last type as it was.
        messages(&mut session, "q");
        assert_eq!(value_type(&session), "{x: Int, y: String}");
    }

    #[test]
    fn strings_and_bools_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let greeting = \"hello\"");
        value(&mut session, "let loud = true");
        // The first input's string table is gone; its string lives on in
        // the frame.
        assert_eq!(
            value(&mut session, "\"{greeting}, world\""),
            Value::Str("hello, world".into())
        );
        value(&mut session, "greeting := greeting + \"!\"");
        assert_eq!(
            value(&mut session, "if loud { greeting } else { \"quiet\" }"),
            Value::Str("hello!".into())
        );
        assert_eq!(
            value(&mut session, "greeting == \"hello!\""),
            Value::Bool(true)
        );
        // A failing input leaves string variables intact.
        assert_eq!(
            messages(&mut session, "greeting := greeting + 1"),
            vec!["cannot add String and Int"]
        );
        assert_eq!(value(&mut session, "greeting"), Value::Str("hello!".into()));
    }

    #[test]
    fn records_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let p = { x = 1; inner = { s = \"a\" } }");
        assert_eq!(value(&mut session, "p.x + 1"), Value::Int(2));
        assert_eq!(value(&mut session, "p.inner.s"), Value::Str("a".into()));
        // The type of `p` is the same type in a later input, so equality
        // with a record built there type-checks.
        assert_eq!(
            value(&mut session, "p == { x = 1; inner = { s = \"a\" } }"),
            Value::Bool(true)
        );
        assert_eq!(
            value(&mut session, "let q: {x: Int} = p\nq").to_string(),
            "{x = 1; inner = {s = \"a\"}}"
        );
        assert_eq!(value_type(&session), "{x: Int}");
        // Top-level fields are in scope for later inputs, as fields.
        value(&mut session, "n = 41\nm = n + 1");
        assert_eq!(value(&mut session, "m"), Value::Int(42));
        assert_eq!(
            messages(&mut session, "n := 0"),
            vec!["cannot rebind `n`: it is a field, not a variable"]
        );
        // A failed input's types stay in the table, harmlessly.
        let types = session.types.len();
        messages(&mut session, "let r = { brand-new = 1 }\nr.missing");
        assert!(session.types.len() > types);
        assert_eq!(value(&mut session, "{ other = 2 }.other"), Value::Int(2));
    }

    #[test]
    fn dotted_fields_and_updates_across_inputs() {
        let mut session = Session::new();
        // An input's merged field is one top-level field, visible later with
        // its full type.
        value(&mut session, "server.host = \"h\"\nserver.port = 80");
        assert_eq!(value_type(&session), "{server: {host: String, port: Int}}");
        assert_eq!(value(&mut session, "server.port"), Value::Int(80));
        // A later input's dotted field is a field of that input's record,
        // declared anew, so it builds from nothing.
        assert_eq!(
            value(&mut session, "server.tls = true").to_string(),
            "{server = {tls = true}}"
        );
        assert_eq!(value(&mut session, "server").to_string(), "{tls = true}");
        // A dotted `let` updates a record from an earlier input, which is
        // left as it was for anything holding it.
        value(
            &mut session,
            "let config = { name = \"gcsl\"; build = { jobs = 4 } }",
        );
        value(&mut session, "let before = config");
        value(&mut session, "let config.build.jobs = 8");
        value(&mut session, "let config.build.dry-run = false");
        value(&mut session, "config.build.dry-run := true");
        assert_eq!(
            value(&mut session, "config").to_string(),
            "{name = \"gcsl\"; build = {jobs = 8; dry-run = true}}"
        );
        assert_eq!(
            value_type(&session),
            "{build: {dry-run: Bool, jobs: Int}, name: String}"
        );
        assert_eq!(
            value(&mut session, "before").to_string(),
            "{name = \"gcsl\"; build = {jobs = 4}}"
        );
        // A field of an earlier input cannot be rebound, dotted or not.
        assert_eq!(
            messages(&mut session, "server.tls := false"),
            vec!["cannot rebind `server`: it is a field, not a variable"]
        );
        // A failed update leaves the variable as it was.
        assert_eq!(
            messages(&mut session, "let config.name.first = 1"),
            vec!["cannot update `config.name.first`: `config.name` is String, not a record"]
        );
        assert_eq!(value(&mut session, "config.build.jobs"), Value::Int(8));
    }

    #[test]
    fn functions_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "fn double(n: Int): Int { n * 2 }");
        value(&mut session, "let x = 10");
        value(&mut session, "let get = fn(): Int { x }");
        // A function of an earlier input is called by a later one, and
        // captured what was in scope when it was made.
        assert_eq!(value(&mut session, "double(21)"), Value::Int(42));
        value(&mut session, "x := 20");
        assert_eq!(value(&mut session, "get()"), Value::Int(10));
        assert_eq!(value(&mut session, "x"), Value::Int(20));
        // Its value shows as its type.
        assert_eq!(value(&mut session, "double").to_string(), "fn(n: Int): Int");
        assert_eq!(value_type(&session), "fn(n: Int): Int");
        // A closure made by one input and passed to a function of another.
        value(
            &mut session,
            "fn apply(f: fn(Int): Int, v: Int): Int { f(v) }",
        );
        assert_eq!(
            value(&mut session, "apply(fn(n: Int): Int { n + x }, 1)"),
            Value::Int(21)
        );
    }

    #[test]
    fn an_error_in_a_function_of_an_earlier_input_names_that_input() {
        let mut session = Session::new();
        let first = "fn div(a: Int, b: Int): Int { a // b }";
        value(&mut session, first);
        value(&mut session, "let unrelated = 1");
        // Input 2 fails at runtime inside input 0's function.
        let diagnostics = eval(&mut session, "div(1, 0)").unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.message, "division by zero");
        assert_eq!(diagnostic.input, Some(0));
        let source = session.source(0).unwrap();
        assert_eq!(source.text(), first);
        assert_eq!(source.slice(diagnostic.span), "//");
        assert_eq!(session.source(1).unwrap().text(), "let unrelated = 1");
        // The failed input takes no number, and the one being run has none
        // yet.
        assert!(session.source(2).is_none());
        // An error in the input being run names no input, even from inside
        // a closure of its own called by a function of an earlier input.
        value(&mut session, "fn call(f: fn(): Int): Int { f() }");
        let diagnostics = eval(&mut session, "call(fn(): Int { 1 // 0 })").unwrap_err();
        assert_eq!(diagnostics[0].input, None);
        // The innermost foreign function names its input: input 0's `div`,
        // called by input 3's `bad`, called by input 2's `call`.
        value(&mut session, "fn bad(): Int { div(1, 0) }");
        let diagnostics = eval(&mut session, "call(bad)").unwrap_err();
        assert_eq!(diagnostics[0].input, Some(0));
        let diagnostics = eval(&mut session, "bad()").unwrap_err();
        assert_eq!(diagnostics[0].input, Some(0));
        // A file names no input: everything in it is input 0.
        let file = SourceFile::new("f.gcsl", "fn f(): Int { 1 // 0 }\nf()").unwrap();
        assert_eq!(crate::run(&file).unwrap_err()[0].input, None);
    }

    #[test]
    fn futures_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let r = { a = 20 }");
        value(&mut session, "let f = lazy r.a + 1");
        // The future holds a copy of the record until it has run.
        let Value::Record(record) = session.frame[0].clone() else {
            panic!("expected a record");
        };
        assert_eq!(Rc::strong_count(&record), 3);
        // It shows as its type, and is run by a later input.
        assert_eq!(value(&mut session, "f").to_string(), "Future<Int>");
        assert_eq!(value_type(&session), "Future<Int>");
        assert_eq!(value(&mut session, "f * 2"), Value::Int(42));
        // Having run, it has dropped its captures, and keeps its value.
        assert_eq!(Rc::strong_count(&record), 2);
        assert_eq!(value(&mut session, "await f"), Value::Int(21));
        assert_eq!(value_type(&session), "Int");
    }

    #[test]
    fn a_failing_future_names_its_input_and_runs_again() {
        let mut session = Session::new();
        let first = "let bad = lazy 1 // 0";
        value(&mut session, first);
        // Awaited by a later input, it fails in the input that made it,
        // whose source the spans refer to; there is no await of that input
        // to label.
        for _ in 0..2 {
            let diagnostics = eval(&mut session, "await bad").unwrap_err();
            assert_eq!(diagnostics.len(), 1);
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.message, "division by zero");
            assert_eq!(diagnostic.input, Some(0));
            assert_eq!(session.source(0).unwrap().slice(diagnostic.span), "//");
            assert!(
                diagnostic
                    .labels
                    .iter()
                    .all(|label| label.text != "awaited here")
            );
        }
        // Left unrun, so each await runs it again; one of the input that
        // made it, awaited there, is labelled.
        let diagnostics = eval(&mut session, "let again = lazy 1 // 0\nawait again").unwrap_err();
        assert_eq!(diagnostics[0].input, None);
        assert!(
            diagnostics[0]
                .labels
                .iter()
                .any(|label| label.text == "awaited here")
        );
    }

    #[test]
    fn a_memo_entry_of_one_input_is_found_by_a_later_one() {
        let mut session = Session::new();
        value(&mut session, "fn f(x: Int) ->> {v: Int} { v = x }");
        let Value::Record(first) = value(&mut session, "f(1)") else {
            panic!("expected a record");
        };
        value(&mut session, "let unrelated = 1");
        let Value::Record(again) = value(&mut session, "f(1)") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&first, &again));
        let Value::Record(other) = value(&mut session, "f(2)") else {
            panic!("expected a record");
        };
        assert!(!Rc::ptr_eq(&first, &other));
        assert_eq!(session.memo.table_count(), 1);
    }

    #[test]
    fn a_failed_input_forgets_what_it_memoised() {
        let mut session = Session::new();
        value(&mut session, "fn f(r: {x: Int}) ->> {v: Int} { v = r.x }");
        let Value::Record(first) = value(&mut session, "f({ x = 1 })") else {
            panic!("expected a record");
        };
        // The failed input's entries go, with the table it made for `g`;
        // those of earlier inputs stay.
        assert_eq!(
            messages(
                &mut session,
                "fn g(x: Int) ->> Int { x }\nlet a = g(1)\nlet r = { x = 2; zzz = 3 }\n\
                 let b = f(r)\nlet h = 1 // 0"
            ),
            vec!["division by zero"]
        );
        assert_eq!(session.memo.table_count(), 1);
        let Value::Record(again) = value(&mut session, "f({ x = 1 })") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&first, &again));
        // A name the failed input interned is another input's now: a key
        // holding it would be found by arguments it does not equal.
        let Value::Record(other) = value(&mut session, "let q = { x = 2; qqq = 3 }\nf(q)") else {
            panic!("expected a record");
        };
        assert_eq!(Rc::strong_count(&other), 2);
        let Value::Record(third) = value(&mut session, "let z = { x = 2; zzz = 3 }\nf(z)") else {
            panic!("expected a record");
        };
        assert!(!Rc::ptr_eq(&other, &third));
    }

    #[test]
    fn a_memoised_call_that_fails_runs_again_in_a_later_input() {
        let mut session = Session::new();
        let first = "fn d(x: Int) ->> Int { 10 // x }";
        value(&mut session, first);
        for _ in 0..2 {
            let diagnostics = eval(&mut session, "d(0)").unwrap_err();
            assert_eq!(diagnostics[0].message, "division by zero");
            // In the body, which ran again.
            assert_eq!(session.source(0).unwrap().slice(diagnostics[0].span), "//");
            assert_eq!(session.memo.running_count(), 0);
        }
        assert_eq!(value(&mut session, "d(5)"), Value::Int(2));
        // A call that depends on its own result, likewise: no running mark
        // is left, so the next input runs the body again, and fails in it,
        // at the inner call, with the outer one labelled.
        let text = "fn r(n: Int) ->> Int { if n == 0 { r(0) } else { n } }";
        value(&mut session, text);
        for _ in 0..2 {
            let diagnostics = eval(&mut session, "r(0)").unwrap_err();
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.message, "this call depends on its own result");
            assert_eq!(diagnostic.input, Some(2));
            assert_eq!(session.source(2).unwrap().slice(diagnostic.span), "r(0)");
            // The call running it is of the input being run, and the
            // error's spans are of the function's: it is not labelled.
            assert!(diagnostic.labels.is_empty());
            assert_eq!(session.memo.running_count(), 0);
        }
        assert_eq!(value(&mut session, "r(4)"), Value::Int(4));
    }

    #[test]
    fn lists_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let xs = [1, 2, 3]");
        assert_eq!(value(&mut session, "xs[2]"), Value::Int(3));
        assert_eq!(value_type(&session), "Int");
        value(&mut session, "let ys: Int[] = [...xs, 4]");
        assert_eq!(value(&mut session, "len(ys)"), Value::Int(4));
        assert_eq!(value(&mut session, "ys").to_string(), "[1, 2, 3, 4]");
        assert_eq!(value_type(&session), "List<Int>");
        // A function of one input takes a list of a later one.
        value(
            &mut session,
            "fn total(xs: Int[]): Int { xs[0] + xs[len(xs) - 1] }",
        );
        assert_eq!(value(&mut session, "total(ys)"), Value::Int(5));
        // An index out of range in a function of an earlier input names
        // that input.
        let diagnostics = eval(&mut session, "total([])").unwrap_err();
        assert_eq!(diagnostics[0].message, "index out of range");
        assert_eq!(diagnostics[0].input, Some(5));
        // `len` is in every input's scope, and shadowed as any name.
        value(&mut session, "let len = 5");
        assert_eq!(value(&mut session, "len"), Value::Int(5));
        assert_eq!(
            messages(&mut session, "let n = [1]\nn := [\"a\"]"),
            vec!["mismatched types"]
        );
        // An element that may be a future is awaited deeply when shown.
        value(&mut session, "let zs = [lazy 1, 2]");
        assert_eq!(value(&mut session, "\"{zs}\""), Value::Str("[1, 2]".into()));
    }

    #[test]
    fn warnings_come_back_with_the_value() {
        let mut session = Session::new();
        let outcome = eval(&mut session, "let p: {x: Int} = { x = 1; y = 2 }\np").unwrap();
        assert_eq!(outcome.value.to_string(), "{x = 1; y = 2}");
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].severity, Severity::Warning);
        assert_eq!(
            outcome.warnings[0].message,
            "field `y` is not in `{x: Int}`"
        );
        // With a runtime error, the warnings come back with it, in source
        // order.
        let diagnostics = eval(&mut session, "let r: {} = { z = 1 // 0 }").unwrap_err();
        let shown: Vec<(Severity, &str)> = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.severity, diagnostic.message.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (Severity::Warning, "field `z` is not in `{}`"),
                (Severity::Error, "division by zero")
            ]
        );
        // No warnings, none returned.
        assert!(eval(&mut session, "p.x").unwrap().warnings.is_empty());
    }
}
