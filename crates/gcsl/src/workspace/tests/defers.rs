//! Tests of workspaces: deferring attributes, whose functions' calls a host
//! may run as steps (see the checker's
//! [Deferring attributes](crate::check::attributes#deferring-attributes)).

use super::*;

use crate::workspace::tests::shell::lone_error;
use crate::workspace::tests::standard::lone;

/// A deferring attribute's declaration.
const STEP: &str = "#[attribute] #[defers] type step = {on: Bool = true}\n";

/// The value of `text`, after [`STEP`], run as a lone file.
fn value_of(text: &str) -> String {
    lone(&format!("{STEP}{text}")).unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
}

#[test]
fn a_deferring_function_s_result_may_be_a_future_and_plain_gcsl_runs_it_at_once() {
    // Its result type is either-shaped, whatever its body returns.
    assert_eq!(
        value_of("#[step] fn c(n: Int) -> Int { n + 1 }\n= c"),
        "fn(n: Int | Future<Int>) -> Int | Future<Int>"
    );
    // Plain GCSL runs each call at once: awaiting its result is a no-op,
    // wherever a value is needed.
    assert_eq!(
        value_of(
            "#[step] fn c(n: Int) -> Int { n + 1 }\n\
             fn p(x: Int): Int { x * 2 }\n\
             let g = c\n\
             let r: {n: Int} = {n = c(5)}\n\
             = [c(1) + 1, p(c(2)), g(3), r.n]"
        ),
        "[3, 6, 4, 6]"
    );
    // Its arguments switched off still defer it: typing depends on the
    // declaration alone, never on a host.
    assert_eq!(
        value_of("#[step(false)] fn c() -> Int { 1 }\n= c"),
        "fn() -> Int | Future<Int>"
    );
    // A function with no deferring attribute keeps a present result.
    assert_eq!(value_of("fn c() -> Int { 1 }\n= c"), "fn() -> Int");
}

#[test]
fn a_def_type_s_definitions_defer_and_keep_its_type() {
    let text = "#[def_type] #[step]\n\
                def build(name: String, f: fn() -> {v: Int}) -> {v: Int} { f() }\n\
                def build main() { v = 1 }\n\
                r = build main\n\
                v = (build main)().v + 1";
    let shown = value_of(text);
    assert!(
        shown.contains("r = fn() -> {v: Int | Future<Int>} | Future<{v: Int | Future<Int>}>"),
        "{shown}"
    );
    assert!(shown.ends_with("v = 2}"), "{shown}");
}

#[test]
fn a_definition_s_own_parameters_have_types_and_defaults_and_its_effect() {
    let error = |text: &str| lone_error(&format!("{STEP}{text}")).message;
    let dt = "#[def_type] def check(name: String, f: fn() -> Int) -> Int { f() }\n";
    assert_eq!(
        error(&format!("{dt}def check one(n: Int) {{ n }}")),
        "`n` is a parameter of the definition's own, which `check` calls with none: give it a \
         default, or leave its type to `check`"
    );
    let two = "#[def_type] def pair(name: String, f: fn(a: Int) -> Int) -> Int { f(1) }\n";
    assert_eq!(
        error(&format!("{two}def pair one(n: Int = 2, a) {{ a + n }}")),
        "the parameters `pair` gives a definition come before its own, which have a type and a \
         default"
    );
    // Of a pure function type, a pure body, given what pure defaults give.
    assert_eq!(
        value_of(
            "#[def_type] def pure(name: String, f: fn(): Int): Int { f() }\n\
             def pure one(n: Int = 2) { n * 3 }\nr = (pure one)()"
        ),
        "{pure = fn(name: String | Future<String>, f: fn(): Int): Int; r = 6}"
    );
    let impure = lone(&format!(
        "{STEP}#[def_type] def pure(name: String, f: fn(): Int): Int {{ f() }}\n\
         def pure one(n: Int = 2) {{ len(exec([\"true\"]).stdout) }}"
    ))
    .unwrap_err();
    assert_eq!(
        impure[0],
        "cannot call an impure function from a pure function"
    );
    // A given parameter and one of its own, its default reading the given.
    assert_eq!(
        value_of(&format!(
            "{two}def pair one(a, n: Int = 2) {{ a + n }}\nr = (pair one)()"
        )),
        "{pair = fn(name: String | Future<String>, f: fn(a: Int) -> Int | Future<Int>) -> Int | \
         Future<Int>; r = 3}"
    );
}

#[test]
fn deferring_attributes_are_refused_where_they_cannot_apply() {
    let error = |text: &str| lone_error(&format!("{STEP}{text}")).message;
    assert_eq!(
        error("#[step] fn c(n: Int): Int { n }"),
        "a pure function cannot defer: its calls are never steps"
    );
    assert_eq!(
        error("#[step] fn c() -> Present<Int> { 1 }"),
        "the result of a deferring function must be able to be a future"
    );
    // A def-type's deferring attribute on definitions of a pure function
    // type, which it gives them.
    assert_eq!(
        error(
            "#[def_type] #[step]\ndef build(name: String, f: fn(): Int) -> Int { f() }\n\
             def build main() { 1 }"
        ),
        "a `build` definition cannot defer: its def-type gives it a pure function type"
    );
    assert_eq!(
        lone_error("#[defers] type nope = {}").message,
        "`#[defers]` marks an attribute's declaration, and `nope` is none"
    );
    assert_eq!(
        lone_error("#[defers] fn f() -> Int { 1 }").message,
        "`#[defers]` applies only to a file's top-level `type`s"
    );
    assert_eq!(
        lone_error("#[defers(1)] type t = {}").message,
        "`#[defers]` takes no arguments"
    );
    assert_eq!(
        lone_error("#[attribute] type defers = {}").message,
        "`defers` is a built-in attribute"
    );
}

#[test]
fn a_deferring_attribute_of_another_file_defers_too() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/attrs.gcsl", STEP),
        (
            "app/main.gcsl",
            "import step from lib/attrs\n#[step] fn c() -> Int { 1 }\nx = c\ny = c() + 1",
        ),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{x = fn() -> Int | Future<Int>; y = 2}"
    );
}
