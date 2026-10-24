//! Tests of the checker: suggestions for names that name nothing.

use super::*;

/// Each error of `text` with its suggestion, as the text it replaces,
/// read from `text` at its span, and its replacement.
pub(super) fn suggested(text: &str) -> Vec<(String, Option<(String, String)>)> {
    diagnostics(text)
        .into_iter()
        .map(|diagnostic| {
            let suggestion = diagnostic.suggestion.map(|found| {
                let span = found.span.start as usize..found.span.end as usize;
                (text[span].to_owned(), found.replacement)
            });
            (diagnostic.message, suggestion)
        })
        .collect()
}

/// The suggestion of the one error of `text`, as [`suggested`] shows it.
pub(super) fn suggestion(text: &str) -> Option<(String, String)> {
    let found = suggested(text);
    assert_eq!(found.len(), 1, "{text}: {found:?}");
    found
        .into_iter()
        .next()
        .and_then(|(_, suggestion)| suggestion)
}

/// The suggestion to replace `written` with `replacement`, as
/// [`suggestion`] gives it: `written -> replacement`.
pub(super) fn fix(fix: &str) -> Option<(String, String)> {
    let (written, replacement) = fix.split_once(" -> ")?;
    Some((written.to_owned(), replacement.to_owned()))
}

#[test]
fn an_undefined_variable_suggests_the_nearest_name_in_scope() {
    let found = suggested("let version = 1\nx = versoin + 1");
    assert_eq!(
        found,
        [(
            "undefined variable `versoin`".to_owned(),
            fix("versoin -> version")
        )]
    );
    // A parameter, a field of the block being built, a name of the
    // prelude.
    assert_eq!(
        suggestion("fn f(count: Int): Int { cuont }\nr = f(1)"),
        fix("cuont -> count")
    );
    assert_eq!(
        suggestion("r = { port = 80; next = prot + 1 }"),
        fix("prot -> port")
    );
    assert_eq!(suggestion("n = lne(\"abc\")"), fix("lne -> len"));
    // Differing in case only, which always qualifies.
    assert_eq!(
        suggestion("let Total = 1\nx = total"),
        fix("total -> Total")
    );
    // Too far from every name: none.
    assert_eq!(suggestion("let version = 1\nx = vrsn"), None);
    // A short name: never another short one, only a longer one an edit
    // away.
    assert_eq!(suggestion("let y = 1\nx = z"), None);
    assert_eq!(
        suggestion("let ab = 1\nlet idx = 2\nx = id"),
        fix("id -> idx")
    );
    // `x-1` is one name, as its note says, and not a misspelling of `x`.
    assert_eq!(suggestion("let count = 1\nx = count-1"), None);
}

#[test]
fn a_name_not_usable_yet_is_never_suggested() {
    // A `let` declared later in the block.
    assert_eq!(
        suggestion("fn f(): Int {\n    let a = valeu\n    let value = 1\n    a\n}\nr = f()"),
        None
    );
    // A top-level `let` below the function, which the error labels.
    let found = suggested("fn f(): Int { totl }\nlet total = 1\nr = f()");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].1, None);
    // The field being declared, which would read itself.
    assert_eq!(suggestion("count = coutn + 1"), None);
    // But another field, wherever it is declared.
    assert_eq!(
        suggestion("count = countr + 1\ncounter = 1"),
        fix("countr -> counter")
    );
    // The `fileroot` module, which is no value.
    assert_eq!(suggestion("import fileroot\nx = filerootthis"), None);
    // But a module misspelled where it is imported.
    assert_eq!(
        suggestion("import fileroo\nx = 1"),
        fix("fileroo -> fileroot")
    );
}

#[test]
fn a_tie_goes_to_the_nearer_scope_then_the_first_in_alphabetical_order() {
    assert_eq!(
        suggestion("let cart = 1\nlet card = 2\nx = car"),
        fix("car -> card")
    );
    assert_eq!(
        suggestion("let card = 1\nfn f(): Int {\n    let cart = 2\n    car\n}\nr = f()"),
        fix("car -> cart")
    );
}

#[test]
fn a_missing_field_suggests_the_nearest_field_written_as_it_must_be() {
    let found = suggested("let r = {port = 80; host = \"h\"}\nx = r.prot");
    assert_eq!(
        found,
        [(
            "no field `prot` in `{host: String, port: Int}`".to_owned(),
            fix("prot -> port")
        )]
    );
    // Quoted, the quotes replaced too; after a call, the name alone.
    assert_eq!(
        suggestion("let r = {port = 80}\nx = r.\"prot\""),
        fix("\"prot\" -> port")
    );
    assert_eq!(
        suggestion("fn g(): {port: Int} { {port = 80} }\nx = g().prot"),
        fix("prot -> port")
    );
    // A field whose name must be quoted is suggested quoted.
    assert_eq!(
        suggestion("let r = {\"a b\" = 1}\nx = r.\"a c\""),
        fix("\"a c\" -> \"a b\"")
    );
    // Differing in case only; too far.
    assert_eq!(
        suggestion("let r = {Port = 80}\nx = r.port"),
        fix("port -> Port")
    );
    assert_eq!(suggestion("let r = {port = 80}\nx = r.host"), None);
}

#[test]
fn an_unknown_type_suggests_a_type_that_can_be_written_there() {
    assert_eq!(suggestion("let x: Strng = \"a\""), fix("Strng -> String"));
    assert_eq!(suggestion("let x: int = 1"), fix("int -> Int"));
    // `List` takes a type argument: suggested only where one is given.
    assert_eq!(suggestion("let x: Lst<Int> = []"), fix("Lst -> List"));
    assert_eq!(suggestion("let x: Lst = 1"), None);
    assert_eq!(suggestion("let x: Strig<Int> = 1"), None);
}

#[test]
fn an_unknown_argument_suggests_a_parameter_no_other_argument_fills() {
    let text = "fn f(name: String, count: Int): Int { count }\nr = f(name = \"a\", cuont = 1)";
    let found = suggested(text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        found[0],
        ("unknown argument `cuont`".to_owned(), fix("cuont -> count"))
    );
    // `name` is filled by position already: suggesting it would give it
    // twice.
    let found = suggested("fn f(name: String, count: Int): Int { count }\nr = f(\"a\", nmae = 1)");
    assert_eq!(found[0].0, "unknown argument `nmae`");
    assert_eq!(found[0].1, None);
    // A built-in function's parameters.
    let found = suggested("r = exec(args = [\"true\"])");
    assert_eq!(
        found[0],
        ("unknown argument `args`".to_owned(), fix("args -> argv"))
    );
}
