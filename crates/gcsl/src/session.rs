//! A stateful session: a sequence of inputs sharing their variables, as in
//! the REPL.
//!
//! Each input is a block body, like a file, run in the context of every
//! earlier successful input: variables declared at their top level are
//! visible, `:=` at top level may rebind them, and new top-level `let`s
//! persist for later inputs. The session keeps three things between inputs:
//!
//! - the [`Names`] table, lent to each stage in turn, so that the same name
//!   in two inputs is the same [`crate::names::Name`];
//! - the checker's outermost [`Scope`]: each top-level variable with its
//!   slot and type, from which [`Checker::with_scope`] checks the next
//!   input exactly as if it continued the same block;
//! - the frame, holding each top-level variable's value in its slot.
//!
//! A failed input, whether it fails to compile or fails at runtime, leaves
//! the session as it was: the scope is only replaced after evaluation
//! succeeds, and names interned by the failed input are forgotten. The frame
//! may have grown, and slots above the last committed variable may hold
//! values from the failed run, but those slots belong to no variable and are
//! always written before they are read.

use std::mem;

use crate::ast::Ast;
use crate::check::{Checker, Scope};
use crate::diagnostic::Diagnostic;
use crate::eval;
use crate::ir::Ir;
use crate::lexer;
use crate::lower;
use crate::names::Names;
use crate::parser;
use crate::source::SourceFile;
use crate::value::Value;

/// A sequence of inputs sharing their top-level variables. See the module
/// documentation.
#[derive(Clone, Debug, Default)]
pub struct Session {
    names: Names,
    scope: Scope,
    frame: Vec<Value>,
}

impl Session {
    /// A session with no variables.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `source` as the next input and returns its value: the value of
    /// its body, `()` if it has no result.
    ///
    /// # Errors
    ///
    /// Returns the diagnostics of the first stage that fails, as
    /// [`crate::run`] does. The session is then unchanged.
    pub fn eval(&mut self, source: &SourceFile) -> Result<Value, Vec<Diagnostic>> {
        let names_before = self.names.len();
        let result = self.eval_uncommitted(source);
        if result.is_err() {
            self.names.truncate(names_before);
        }
        result
    }

    /// Runs `source`, committing its scope only on success. On failure the
    /// caller must forget the names it interned.
    fn eval_uncommitted(&mut self, source: &SourceFile) -> Result<Value, Vec<Diagnostic>> {
        let tokens = lexer::tokenize_with(source, &mut self.names).map_err(|error| vec![error])?;
        let (builder, root) = parser::parse_parts(&tokens, source).map_err(|error| vec![error])?;
        // The tree owns the names while it lives; take them back before
        // anything can fail.
        let ast = builder.finish(root, mem::take(&mut self.names));
        let compiled = compile(&ast, self.scope.clone());
        self.names = ast.into_names();
        let (ir, scope) = compiled?;
        let frame_size = ir.frame_size() as usize;
        if self.frame.len() < frame_size {
            self.frame.resize(frame_size, Value::Unit);
        }
        let value = eval::eval_in(&ir, &mut self.frame).map_err(|error| vec![error])?;
        self.scope = scope;
        Ok(value)
    }
}

/// Checks and lowers `ast` in the context of `scope`, returning the IR and
/// the scope extended by the input's top-level declarations.
fn compile(ast: &Ast, scope: Scope) -> Result<(Ir, Scope), Vec<Diagnostic>> {
    let (checked, scope) = Checker::with_scope(ast, scope).check()?;
    let ir = lower::lower(ast, &checked).map_err(|error| vec![error.into()])?;
    Ok((ir, scope))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(session: &mut Session, text: &str) -> Result<Value, Vec<Diagnostic>> {
        session.eval(&SourceFile::new("<test>", text).unwrap())
    }

    fn value(session: &mut Session, text: &str) -> Value {
        match eval(session, text) {
            Ok(value) => value,
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

    #[test]
    fn three_inputs_share_a_variable() {
        let mut session = Session::new();
        assert_eq!(value(&mut session, "let x = 1"), Value::Unit);
        assert_eq!(value(&mut session, "let y = x + 1"), Value::Unit);
        assert_eq!(value(&mut session, "x + y"), Value::Int(3));
        assert_eq!(value(&mut session, "x * 10"), Value::Int(10));
    }

    #[test]
    fn rebinding_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let x = 1");
        assert_eq!(value(&mut session, "x := x + 10"), Value::Unit);
        assert_eq!(value(&mut session, "x"), Value::Int(11));
        // A rebind is a new binding, so it may change the type.
        value(&mut session, "x := 2.5");
        assert_eq!(value(&mut session, "x"), Value::Float(2.5));
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
        assert_eq!(session.frame.len(), 4);
        // The temporaries' slots are reused by later variables.
        value(&mut session, "let b = 2");
        assert_eq!(session.frame.len(), 4);
        assert_eq!(value(&mut session, "a + b"), Value::Int(3));
    }

    #[test]
    fn inputs_have_values_of_every_type() {
        let mut session = Session::new();
        assert_eq!(value(&mut session, ""), Value::Unit);
        assert_eq!(value(&mut session, "7 / 2"), Value::Float(3.5));
        assert_eq!(value(&mut session, "= 1 + 1"), Value::Int(2));
        assert_eq!(value(&mut session, "let u = {}\nu"), Value::Unit);
    }
}
