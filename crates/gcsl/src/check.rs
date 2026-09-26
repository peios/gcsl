//! The type checker.
//!
//! Types are recorded in a side table, [`Types`], indexed by [`ExprId`].
//!
//! `Int` and `Float` never mix implicitly. The one exception is that integer
//! literals adapt to their context: a literal starts as
//! [`Ty::UntypedInt`], and an operator that needs a concrete type for it
//! decides one. The checker works in two directions:
//!
//! - **Up**: [`Checker::check_expr`] computes each node's type from its
//!   children's. Operators whose operands are all untyped produce
//!   `UntypedInt` themselves where the rules allow, so `1 + 2` stays untyped.
//! - **Down**: when an untyped subtree meets a concrete type, as in
//!   `(1 + 2) * 3.0`, [`Checker::coerce`] walks the untyped subtree and
//!   rewrites every entry in it to the decided type. It stops at nodes that
//!   are already concrete, so each node is rewritten at most once and the
//!   whole check stays linear.
//!
//! Finalisation defaults an untyped root to `Int`. Every untyped node is
//! either coerced by the nearest concrete ancestor or reaches the root through
//! untyped ancestors, so afterwards the table contains no `UntypedInt`.
//!
//! A node whose operator rejects its operands is reported once and given
//! [`Ty::Error`]. Every operator accepts `Error` operands silently and
//! produces `Error` itself, so the mistake does not cascade into a spurious
//! error at each enclosing operator. Independent mistakes elsewhere in the
//! tree are still reported, so one run shows every real error.

use std::fmt;

use crate::ast::{Annotate, Ast, BinaryOp, ExprId, ExprKind, UnaryOp};
use crate::diagnostic::Diagnostic;
use crate::types::Ty;

/// The type of every expression in an [`Ast`], indexed by [`ExprId`].
///
/// A `Types` returned by [`check`] contains neither [`Ty::UntypedInt`] nor
/// [`Ty::Error`] for any node reachable from the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    types: Vec<Ty>,
}

impl Types {
    /// Wraps a raw table. Only the checker (and tests) build tables.
    pub(crate) fn new(types: Vec<Ty>) -> Self {
        Self { types }
    }

    /// The type of expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn get(&self, id: ExprId) -> Ty {
        self.types[id.index()]
    }

    /// Number of entries, equal to the number of nodes in the [`Ast`].
    pub(crate) fn len(&self) -> usize {
        self.types.len()
    }
}

/// Writes ` : Type` after each node, for `--dump-types`.
impl Annotate for Types {
    fn annotate(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, " : {}", self.get(id))
    }
}

/// Type-checks `ast`.
///
/// # Errors
///
/// Returns every type error found. Checking continues after an error by
/// giving the offending node [`Ty::Error`], which every enclosing operator
/// accepts silently, so one run reports each independent error exactly once.
pub fn check(ast: &Ast) -> Result<Types, Vec<Diagnostic>> {
    let mut checker = Checker {
        ast,
        // Placeholder only: every node reachable from the root is written by
        // `check_expr` before it is read.
        types: vec![Ty::Int; ast.len()],
        errors: Vec::new(),
    };
    let root = ast.root();
    checker.check_expr(root);
    // Finalisation: an integer expression with no context defaults to Int.
    checker.coerce(root, Ty::Int);
    if checker.errors.is_empty() {
        Ok(Types::new(checker.types))
    } else {
        Err(checker.errors)
    }
}

struct Checker<'a> {
    ast: &'a Ast,
    types: Vec<Ty>,
    errors: Vec<Diagnostic>,
}

impl Checker<'_> {
    /// Computes, records and returns the type of `id`, bottom-up.
    fn check_expr(&mut self, id: ExprId) -> Ty {
        let expr = self.ast.get(id);
        let ty = match expr.kind {
            ExprKind::Int(_) => Ty::UntypedInt,
            ExprKind::Float(_) => Ty::Float,
            ExprKind::Paren(inner) => self.check_expr(inner),
            // Negation accepts every numeric type and preserves it.
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => self.check_expr(operand),
            ExprKind::Binary { op, lhs, rhs } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match op {
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                        self.arithmetic(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
                    }
                    BinaryOp::Div => {
                        // `/` accepts any mix and always yields Float. Untyped
                        // operands become Float literals; concrete Int
                        // operands are converted by lowering.
                        self.coerce(lhs, Ty::Float);
                        self.coerce(rhs, Ty::Float);
                        if lhs_ty.is_error() || rhs_ty.is_error() {
                            Ty::Error
                        } else {
                            Ty::Float
                        }
                    }
                    BinaryOp::FloorDiv | BinaryOp::Rem => {
                        self.integer_only(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
                    }
                }
            }
        };
        self.types[id.index()] = ty;
        ty
    }

    /// Types `+`, `-` and `*`: both operands must end up the same concrete
    /// type, with untyped operands adapting to a concrete one.
    fn arithmetic(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, Ty),
        (rhs, rhs_ty): (ExprId, Ty),
    ) -> Ty {
        match (lhs_ty, rhs_ty) {
            // An erroneous operand has been reported already. The other
            // operand still needs a concrete type in case it is untyped.
            (Ty::Error, _) => {
                self.coerce(rhs, Ty::Int);
                Ty::Error
            }
            (_, Ty::Error) => {
                self.coerce(lhs, Ty::Int);
                Ty::Error
            }
            (Ty::UntypedInt, Ty::UntypedInt) => Ty::UntypedInt,
            (Ty::UntypedInt, concrete @ (Ty::Int | Ty::Float)) => {
                self.coerce(lhs, concrete);
                concrete
            }
            (concrete @ (Ty::Int | Ty::Float), Ty::UntypedInt) => {
                self.coerce(rhs, concrete);
                concrete
            }
            (Ty::Int, Ty::Int) => Ty::Int,
            (Ty::Float, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => {
                self.errors.push(Diagnostic::error(
                    format!("cannot {} {lhs_ty} and {rhs_ty}", op.verb()),
                    self.ast.get(id).span,
                ));
                Ty::Error
            }
        }
    }

    /// Types `//` and `%`: both operands must be integers, and the result is
    /// a concrete `Int`. Untyped operands are coerced to `Int`; they never
    /// adapt to `Float`, since these operators do not accept it.
    fn integer_only(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, Ty),
        (rhs, rhs_ty): (ExprId, Ty),
    ) -> Ty {
        // Whatever the outcome, untyped operands settle to Int: these
        // operators never accept Float, so nothing else could decide them.
        self.coerce(lhs, Ty::Int);
        self.coerce(rhs, Ty::Int);
        if lhs_ty.is_error() || rhs_ty.is_error() {
            return Ty::Error;
        }
        if lhs_ty == Ty::Float || rhs_ty == Ty::Float {
            self.errors.push(Diagnostic::error(
                format!("cannot {} {}", op.verb(), Ty::Float),
                self.ast.get(id).span,
            ));
            return Ty::Error;
        }
        Ty::Int
    }

    /// Pushes the concrete type `ty` down into the untyped subtree rooted at
    /// `id`. Does nothing if `id` already has a concrete type (or is an
    /// error).
    ///
    /// Only literals, parentheses, negation and `+ - *` can be untyped, and
    /// an untyped node's children are untyped or concrete of the same kind,
    /// so the walk only ever meets those nodes.
    fn coerce(&mut self, id: ExprId, ty: Ty) {
        if self.types[id.index()] != Ty::UntypedInt {
            return;
        }
        self.types[id.index()] = ty;
        match self.ast.get(id).kind {
            ExprKind::Int(_) | ExprKind::Float(_) => {}
            ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
                self.coerce(inner, ty);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.coerce(lhs, ty);
                self.coerce(rhs, ty);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::source::{SourceFile, Span};

    fn parse_text(text: &str) -> Ast {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        parse(&tokenize(&source).unwrap(), &source).unwrap()
    }

    fn root_type(text: &str) -> Ty {
        let ast = parse_text(text);
        check(&ast).unwrap().get(ast.root())
    }

    fn errors(text: &str) -> Vec<(String, Span)> {
        check(&parse_text(text))
            .unwrap_err()
            .into_iter()
            .map(|d| (d.message, d.span))
            .collect()
    }

    fn messages(text: &str) -> Vec<String> {
        errors(text).into_iter().map(|(m, _)| m).collect()
    }

    /// Every node's type in allocation order (children before parents).
    fn all_types(text: &str) -> Vec<Ty> {
        check(&parse_text(text)).unwrap().types
    }

    // A concrete Int operand is written `(7 // 2)`: `//` always produces a
    // concrete Int, so the typing rows with a concrete Int are reachable from
    // source even without variables.

    #[test]
    fn literals() {
        assert_eq!(root_type("1"), Ty::Int);
        assert_eq!(root_type("1.5"), Ty::Float);
    }

    #[test]
    fn untyped_op_untyped_stays_untyped_until_the_root() {
        for op in ["+", "-", "*"] {
            let text = format!("1 {op} 2");
            assert_eq!(all_types(&text), vec![Ty::Int; 3], "{text}");
        }
    }

    #[test]
    fn untyped_op_int_is_int() {
        for op in ["+", "-", "*"] {
            assert_eq!(root_type(&format!("1 {op} (7 // 2)")), Ty::Int);
            assert_eq!(root_type(&format!("(7 // 2) {op} 1")), Ty::Int);
        }
    }

    #[test]
    fn untyped_op_float_is_float() {
        for op in ["+", "-", "*"] {
            let text = format!("1 {op} 2.0");
            assert_eq!(all_types(&text), vec![Ty::Float; 3], "{text}");
            let text = format!("2.0 {op} 1");
            assert_eq!(all_types(&text), vec![Ty::Float; 3], "{text}");
        }
    }

    #[test]
    fn same_concrete_types() {
        assert_eq!(root_type("(7 // 2) + (7 // 2)"), Ty::Int);
        assert_eq!(root_type("1.0 * 2.0"), Ty::Float);
    }

    #[test]
    fn int_and_float_do_not_mix() {
        assert_eq!(
            errors("(7 // 2) + 1.5"),
            vec![("cannot add Int and Float".to_owned(), Span::new(0, 14))]
        );
        assert_eq!(
            messages("1.5 - (7 // 2)"),
            vec!["cannot subtract Float and Int"]
        );
        assert_eq!(
            messages("(7 % 2) * 1.5"),
            vec!["cannot multiply Int and Float"]
        );
    }

    #[test]
    fn negation_preserves_type() {
        assert_eq!(all_types("-1"), vec![Ty::Int; 2]);
        assert_eq!(all_types("-1.5"), vec![Ty::Float; 2]);
        assert_eq!(root_type("-(7 // 2)"), Ty::Int);
        // An untyped negation adapts like a literal.
        assert_eq!(all_types("-1 + 2.0"), vec![Ty::Float; 4]);
    }

    #[test]
    fn division_accepts_any_mix_and_yields_float() {
        assert_eq!(all_types("7 / 2"), vec![Ty::Float; 3]);
        assert_eq!(all_types("7 / 2.0"), vec![Ty::Float; 3]);
        assert_eq!(all_types("7.0 / 2"), vec![Ty::Float; 3]);
        assert_eq!(root_type("(7 // 2) / 2.0"), Ty::Float);
        assert_eq!(root_type("2.0 / (7 // 2)"), Ty::Float);
        assert_eq!(root_type("(7 // 2) / (7 // 2)"), Ty::Float);
    }

    #[test]
    fn division_leaves_concrete_int_operands_int() {
        let ast = parse_text("(7 // 2) / 2");
        let types = check(&ast).unwrap();
        let ExprKind::Binary { lhs, rhs, .. } = ast.get(ast.root()).kind else {
            panic!("expected binary");
        };
        assert_eq!(types.get(lhs), Ty::Int);
        assert_eq!(types.get(rhs), Ty::Float);
    }

    #[test]
    fn floor_division_and_modulo_need_integers() {
        assert_eq!(all_types("7 // 2"), vec![Ty::Int; 3]);
        assert_eq!(all_types("7 % 2"), vec![Ty::Int; 3]);
        assert_eq!(root_type("(7 // 2) // 2"), Ty::Int);
        assert_eq!(
            errors("7.0 // 2"),
            vec![("cannot floor-divide Float".to_owned(), Span::new(0, 8))]
        );
        assert_eq!(messages("7 // 2.0"), vec!["cannot floor-divide Float"]);
        assert_eq!(messages("7.0 % 2"), vec!["cannot take modulo of Float"]);
        assert_eq!(messages("7 % 2.0"), vec!["cannot take modulo of Float"]);
        assert_eq!(messages("7.0 % 2.0"), vec!["cannot take modulo of Float"]);
    }

    #[test]
    fn floor_division_result_does_not_adapt_to_float() {
        assert_eq!(messages("(7 // 2) + 1.0"), vec!["cannot add Int and Float"]);
    }

    #[test]
    fn untyped_subtree_is_coerced_all_the_way_down() {
        // Allocation order: 1, 2, +, paren, 3.0, *.
        assert_eq!(all_types("(1 + 2) * 3.0"), vec![Ty::Float; 6]);
        assert_eq!(all_types("3.0 * -(1 - -2)"), vec![Ty::Float; 8]);
    }

    #[test]
    fn division_makes_untyped_siblings_float() {
        // `2 / 3` is Float, so the untyped `1` beside it adapts to Float.
        assert_eq!(all_types("1 + 2 / 3"), vec![Ty::Float; 5]);
    }

    #[test]
    fn untyped_root_defaults_to_int_throughout() {
        assert_eq!(all_types("-(1 + 2) * 3"), vec![Ty::Int; 7]);
    }

    #[test]
    fn multiple_errors_in_one_run() {
        assert_eq!(
            errors("(7.0 // 2) + (1 % 2.5)"),
            vec![
                ("cannot floor-divide Float".to_owned(), Span::new(1, 9)),
                ("cannot take modulo of Float".to_owned(), Span::new(14, 21)),
            ]
        );
    }

    #[test]
    fn errors_do_not_cascade() {
        // The failed `//` becomes an error type that every enclosing operator
        // accepts, whatever the other operand is.
        for text in [
            "(1.0 // 2) + (7 // 2)",
            "(1.0 // 2) + 1.5",
            "(1.0 // 2) + 1",
            "1.5 * (1.0 // 2)",
            "-(1.0 // 2) // 2",
            "(1.0 // 2) / 2.5",
            "2 % (1.0 // 2)",
        ] {
            assert_eq!(messages(text), vec!["cannot floor-divide Float"], "{text}");
        }
    }

    #[test]
    fn error_type_never_escapes_a_successful_check() {
        // Every node of a successful check has a concrete type.
        for text in ["1", "1.5", "(1 + 2) * 3.0", "7 / 2", "(7 // 2) / 2"] {
            for ty in all_types(text) {
                assert!(!ty.is_error() && ty != Ty::UntypedInt, "{text}: {ty:?}");
            }
        }
    }

    #[test]
    fn annotation_renders_types() {
        let ast = parse_text("1 + 2.0");
        let types = check(&ast).unwrap();
        assert_eq!(
            ast.pretty_with(&types).to_string(),
            "(+ : Float @0..7\n  (int 1 : Float @0..1)\n  (float 2.0 : Float @4..7))"
        );
    }
}
