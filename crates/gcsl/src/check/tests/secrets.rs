//! Tests that every kind of expression is known for how it treats a secret
//! (see [Secrets](crate::check::secrets#secrets)): a kind of expression added
//! later fails to compile here until it is classified, and each class is
//! checked against what the checker does with a sample of it.

use super::*;

/// How an expression treats a secret it is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Treatment {
    /// It computes on the secret's value, so what it gives is a secret.
    Computes,
    /// It would decide what runs by the secret, which it refuses.
    Refuses,
    /// It passes a secret on, or holds one, as it is, never computing on
    /// its value: a variable, a block, a call, a list.
    Carries,
    /// It is made of nothing that may be a secret.
    Plain,
}

/// How an expression of `kind` treats a secret: one arm for every kind, so
/// that a new kind is classified before anything compiles. `Binary` is
/// classified by its arithmetic and comparisons, which compute; `and` and
/// `or`, which decide whether their right side runs, refuse.
const fn treatment(kind: &ExprKind) -> Treatment {
    match kind {
        ExprKind::Unary { .. }
        | ExprKind::Binary { .. }
        | ExprKind::Interp(_)
        | ExprKind::Field { .. }
        | ExprKind::Index { .. } => Treatment::Computes,
        ExprKind::If { .. }
        | ExprKind::Is { .. }
        | ExprKind::As { .. }
        | ExprKind::For(_)
        | ExprKind::Assert { .. } => Treatment::Refuses,
        ExprKind::Paren(_)
        | ExprKind::Name(_)
        | ExprKind::Block { .. }
        | ExprKind::Ascribe { .. }
        | ExprKind::Fn(_)
        | ExprKind::Call { .. }
        | ExprKind::Lazy(_)
        | ExprKind::List(_)
        | ExprKind::Variant { .. }
        | ExprKind::Recover(_)
        | ExprKind::ContextRead(_)
        | ExprKind::With { .. } => Treatment::Carries,
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Duration(_)
        | ExprKind::Size(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::DefTypeRef(_)
        | ExprKind::Error
        | ExprKind::NoField(_)
        | ExprKind::Unclosed { .. } => Treatment::Plain,
    }
}

/// The secrets the samples are made of.
const SECRETS: &str = "let s = secret(\"pw\", \"id\")\n\
                       let n = secret(1, \"n\")\n\
                       let r = secret({a = 1}, \"r\")\n\
                       let xs = secret([1, 2], \"xs\")\n";

#[test]
fn every_kind_of_expression_treats_a_secret_as_it_is_classified() {
    for (sample, expected) in [
        ("= s + \"!\"", Treatment::Computes),
        ("= -n", Treatment::Computes),
        ("= \"a{s}\"", Treatment::Computes),
        ("= r.a", Treatment::Computes),
        ("= xs[0]", Treatment::Computes),
        ("= s == \"pw\"", Treatment::Computes),
        ("= if s == \"pw\" { 1 } else { 2 }", Treatment::Refuses),
        ("= s is String", Treatment::Refuses),
        ("= s as String", Treatment::Refuses),
        ("= for x in xs { x }", Treatment::Refuses),
        ("= (s)", Treatment::Carries),
        ("= s", Treatment::Carries),
        ("= { s }", Treatment::Carries),
        ("= [s]", Treatment::Carries),
        ("= secret(s.reveal(), \"again\")", Treatment::Carries),
        ("= \"plain\"", Treatment::Plain),
        ("= 1", Treatment::Plain),
    ] {
        let text = format!("{SECRETS}{sample}");
        let ast = parse_text(&text);
        let kind = &ast.get(result_expr(&ast)).kind;
        assert_eq!(treatment(kind), expected, "{sample}: classified otherwise");
        let (ast, types, output) = check_prelude(&text);
        match expected {
            Treatment::Computes => {
                let output = output.unwrap_or_else(|errors| panic!("{sample}: {errors:?}"));
                let ty = output.checked.types.get(result_expr(&ast));
                assert!(types.secret_value(ty).is_some(), "{sample}: no secret");
            }
            Treatment::Refuses => {
                assert!(output.is_err(), "{sample}: not refused");
            }
            Treatment::Carries => {
                let output = output.unwrap_or_else(|errors| panic!("{sample}: {errors:?}"));
                let ty = output.checked.types.get(result_expr(&ast));
                assert!(types.holds_secret(ty), "{sample}: the secret is lost");
            }
            Treatment::Plain => {
                let output = output.unwrap_or_else(|errors| panic!("{sample}: {errors:?}"));
                let ty = output.checked.types.get(result_expr(&ast));
                assert!(!types.holds_secret(ty), "{sample}: a secret from nowhere");
            }
        }
    }
    // `and` and `or` refuse, whatever their kind's class.
    assert_eq!(
        messages(&format!("{SECRETS}= (s == \"pw\") and true")),
        ["a secret cannot decide what runs"]
    );
}
