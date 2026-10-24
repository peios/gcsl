//! Tests of workspaces: what contexts functions need, inferred (see
//! [Needs](crate::check::needs#needs)).

use super::*;

use crate::workspace::tests::shell::lone_error;
use crate::workspace::tests::standard::lone;

/// The contexts the tests read: `arch` with no default, `os` with one.
const CONTEXTS: &str = "context arch: String\ncontext os: String = \"linux\"\n";

/// The value of `text`, after [`CONTEXTS`], run as a lone file.
fn value_of(text: &str) -> String {
    lone(&format!("{CONTEXTS}{text}")).unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
}

/// The one diagnostic of `text`, after [`CONTEXTS`], run as a lone file,
/// its message and its notes.
fn error_of(text: &str) -> (String, Vec<String>) {
    let error = lone_error(&format!("{CONTEXTS}{text}"));
    (error.message, error.notes)
}

#[test]
fn a_function_needs_what_it_reads_and_what_it_calls_needs() {
    // A `with` around the call, or inside the function, binds it.
    assert_eq!(
        value_of(
            "fn f(): String { ^arch }\n\
             fn g(): String { f() }\n\
             fn h(): String { f() with { arch = \"inner\" } }\n\
             = [g() with { arch = \"outer\" }, h(), with { arch = \"block\" } { g() }]"
        ),
        "[\"outer\", \"inner\", \"block\"]"
    );
    // A context with a default needs nothing.
    assert_eq!(value_of("fn f(): String { ^os }\n= f()"), "linux");
    // A call that nothing covers, through any number of functions.
    let (message, notes) = error_of("fn f(): String { ^arch }\nfn g(): String { f() }\nx = g()");
    assert_eq!(message, "context `arch` has no value here");
    assert_eq!(
        notes,
        ["`g` needs it, and no `with` around this call sets it"]
    );
    // A method, and a function value, need what they read too.
    assert_eq!(
        error_of("fn (s: String) tag(): String { \"{s}-{^arch}\" }\nx = \"a\".tag()").1,
        ["`tag` needs it, and no `with` around this call sets it"]
    );
    assert_eq!(
        error_of("let h = fn(): String { ^arch }\nx = [h][0]()").1,
        ["the function called needs it, and no `with` around this call sets it"]
    );
}

#[test]
fn a_parameter_s_default_needs_what_it_reads_at_the_call() {
    // `^arch` as a parameter: a call that passes it needs nothing.
    assert_eq!(
        value_of(
            "fn f(^arch): String { arch }\n= [f(arch = \"given\"), f() with { arch = \"bound\" }]"
        ),
        "[\"given\", \"bound\"]"
    );
    let (message, notes) = error_of("fn f(^arch): String { arch }\nx = f()");
    assert_eq!(message, "context `arch` has no value here");
    assert_eq!(
        notes,
        [
            "the default of a parameter this call leaves out needs it, and no `with` around the \
          call sets it"
        ]
    );
}

#[test]
fn a_context_s_default_needs_what_it_reads() {
    let text = "context target: String = ^arch\n\
                fn t(): String { ^target }\n\
                = [t() with { arch = \"a\" }, t() with { target = \"t\" }]";
    assert_eq!(value_of(text), "[\"a\", \"t\"]");
    let (message, notes) = error_of("context target: String = ^arch\nx = ^target");
    assert_eq!(message, "context `arch` has no value here");
    assert_eq!(
        notes,
        ["the default of `^target` reads it; no `with` sets it, and it declares no default"]
    );
    // Another file's default, as the type table recorded it.
    let lib =
        "context arch: String\ncontext target: String = ^arch\ndef read(): String { ^target }";
    let files = [
        ("workspace.gcsl", ""),
        ("lib/ctx.gcsl", lib),
        (
            "app/main.gcsl",
            "import lib/ctx\nx = ^target with { arch = \"a\" }\ny = ctx.read() with { arch = \"b\" }",
        ),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{x = \"a\"; y = \"b\"}");
    let files = [
        ("workspace.gcsl", ""),
        ("lib/ctx.gcsl", lib),
        (
            "app/main.gcsl",
            "import lib/ctx\nx = ^target\ny = ctx.read()",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "context `arch` has no value here at app/main.gcsl; note: the default of `^target` \
             reads it; no `with` sets it, and it declares no default",
            "context `arch` has no value here at app/main.gcsl; note: `read` needs it, and no \
             `with` around this call sets it",
        ]
    );
}

#[test]
fn a_lazy_needs_where_it_is_made_and_a_closure_where_it_is_called() {
    // A `lazy` captures the context it is made in.
    assert_eq!(
        value_of(
            "fn later(): Future<String> { lazy { ^arch } }\n\
             fn force(v: String): String { v }\n\
             = force(later() with { arch = \"made\" })"
        ),
        "made"
    );
    assert_eq!(
        error_of("let l = lazy { ^arch }\nx = 1").1,
        ["no `with` sets it, and it declares no default"]
    );
    // A `with` around a closure's definition binds nothing in its body: a
    // call runs in its caller's context.
    let (message, _) = error_of(
        "fn make(): fn(): String { with { arch = \"around\" } { fn(): String { ^arch } } }\n\
         x = make()()",
    );
    assert_eq!(message, "mismatched types");
    let (message, _) =
        error_of("let made = with { arch = \"around\" } { fn(): String { ^arch } }\nx = made()");
    assert_eq!(message, "context `arch` has no value here");
}

#[test]
fn a_function_type_shows_and_fits_by_what_it_needs() {
    // Shown after its parameters.
    assert_eq!(
        value_of("fn f(x: Int): String { \"{x + 1}{^arch}\" }\n= f"),
        "fn(x: Int, ^arch): String"
    );
    // A need read through a default is met by binding either.
    assert_eq!(
        value_of("context target: String = ^arch\nfn t(): String { ^target }\n= t"),
        "fn(^arch | ^target): String"
    );
    // One that needs a context fits no written type, which needs none.
    let text = "fn apply(f: fn(): String): String { f() }\n\
                x = apply(fn(): String { ^arch }) with { arch = \"a\" }";
    let (message, notes) = error_of(text);
    assert_eq!(message, "mismatched types");
    assert_eq!(
        notes,
        [
            "a function that needs `^arch` fits only a function type that needs it too; bind it \
          with a `with` inside the function to need nothing"
        ]
    );
    // Bound inside, it needs nothing.
    assert_eq!(
        value_of(
            "fn apply(f: fn(): String): String { f() }\n\
             = apply(fn(): String { ^arch with { arch = \"inside\" } })"
        ),
        "inside"
    );
    // A join needs what either side needs.
    assert_eq!(
        value_of(
            "fn pick(b: Bool): fn(): String {\n\
               if b { fn(): String { \"plain\" } } else { fn(): String { \"other\" } }\n\
             }\n\
             let f = if true { fn(): String { ^arch } } else { fn(): String { \"x\" } }\n\
             = [pick(true)(), f() with { arch = \"joined\" }]"
        ),
        "[\"plain\", \"joined\"]"
    );
}

#[test]
fn recursion_gives_each_function_of_a_cycle_what_the_cycle_needs() {
    let cycle = "fn a(n: Int): String { if n == 0 { ^arch } else { b(n - 1) } }\n\
                 fn b(n: Int): String { a(n) }\n";
    assert_eq!(
        value_of(&format!("{cycle}= b(3) with {{ arch = \"z\" }}")),
        "z"
    );
    assert_eq!(
        error_of(&format!("{cycle}x = b(2)")).1,
        ["`b` needs it, and no `with` around this call sets it"]
    );
    // Bound inside the cycle, at the call that needs it.
    assert_eq!(
        value_of(
            "fn a(n: Int): String { if n == 0 { ^arch } else { b(n - 1) } }\n\
             fn b(n: Int): String { a(n) with { arch = \"in\" } }\n\
             = b(3)"
        ),
        "in"
    );
}

#[test]
fn a_use_inside_recursion_typed_before_its_needs_are_known_is_an_error() {
    let message = "cannot tell that `b` needs context `arch` here";
    // From a closure inside it, and as a value.
    let nested = "fn b(n: Int): String {\n\
                    let k = fn(): String { b(n) }\n\
                    if n == 0 { ^arch } else { k() }\n\
                  }\nx = 1";
    let (found, notes) = error_of(nested);
    assert_eq!(found, message);
    assert_eq!(
        notes,
        [
            "`b` is called from a nested function or a default inside its own recursion, where \
             its type is known before what it needs is",
            "call it directly from a function of the recursion, or bind `^arch` with a `with` \
             around the call",
        ]
    );
    let valued = "fn a(): String { ^arch }\n\
                  fn b(n: Int): String { let k = b\n if n == 0 { a() } else { k(n - 1) } }\nx = 1";
    assert_eq!(error_of(valued).0, message);
    // A local recursive function too.
    let local = "let f = fn(n: Int): String {\n\
                   if n == 0 { ^arch } else { let k = fn(): String { f(n - 1) }\n k() }\n\
                 }\nx = 1";
    assert_eq!(
        error_of(local).0,
        "cannot tell that `f` needs context `arch` here"
    );
    // Covered, or needing nothing, it is no error.
    assert_eq!(
        value_of(
            "fn b(n: Int): String {\n\
               let k = fn(): String { b(n) with { arch = \"x\" } }\n\
               if n == 0 { ^arch } else { k() }\n\
             }\n= 1"
        ),
        "1"
    );
    assert_eq!(
        value_of(
            "fn b(n: Int): String {\n\
               if n == 0 { \"done\" } else { let k = fn(): String { b(n - 1) }\n k() }\n\
             }\n= b(2)"
        ),
        "done"
    );
}

#[test]
fn a_definition_cannot_need_a_context() {
    let def_type = "#[def_type]\n\
                    def package(name: String, f: fn(): {v: String}): {v: String} { f() }\n";
    let (message, notes) = error_of(&format!("{def_type}def package p() {{ v = ^arch }}"));
    assert_eq!(message, "context `arch` has no value in this definition");
    assert_eq!(
        notes,
        [
            "its def-type calls a definition with no context bound: bind it with a `with` inside \
          the definition, or give the context a default"
        ]
    );
    // Bound inside, it needs nothing.
    let bound = value_of(&format!(
        "{def_type}def package p() {{ v = ^arch with {{ arch = \"bound\" }} }}\n\
         r = (package p)().v"
    ));
    assert!(bound.ends_with("r = \"bound\"}"), "{bound}");
}
