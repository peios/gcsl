//! Tests of workspaces: parameters written with no type, typed by their
//! defaults (see the checker's [Untyped
//! parameters](crate::check::untyped#untyped-parameters)), across files, in
//! definitions and in invocations.

use super::blocks::{drive, steps};
use super::fileroot::{rooted, rooted_value};
use super::*;

#[test]
fn a_definition_with_a_parameter_of_its_own_written_with_no_type_runs_in_graph_mode() {
    // Its defaults are evaluated where its def-type calls it, each
    // operation of theirs stopping the unit, and its body is given what
    // they came to.
    let mut workspace = steps(
        "fn fixture(name: String) -> String { op(\"make {name}\") }\n\
         def test one(m = fixture(\"vm\"), n = 2, s: String = op(\"typed\")) {\n\
         \x20   let used = op(\"use {m}\")\n\
         \x20   = \"{m} {n} {s} {used}\"\n\
         }\n\
         def test two(m = fixture(\"other\")) { m }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("make vm 2 typed use make vm".to_owned())]);
    assert_eq!(asked, ["make vm", "typed", "use make vm"]);
    // Two units, each with its own.
    let (results, asked) = drive(&mut workspace, &["two", "one"]);
    assert_eq!(
        results,
        [
            Ok("make other".to_owned()),
            Ok("make vm 2 typed use make vm".to_owned())
        ]
    );
    assert_eq!(asked.len(), 4, "{asked:?}");
}

#[test]
fn a_parameter_written_with_no_type_is_given_its_default_across_files() {
    // A `def` of another file, called by its name and through an import,
    // leaving the parameter out and giving it; and a definition of another
    // file with one of its own.
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib.gcsl",
            "let base = 10\n\
             def greet(name = \"world\", times = base * 2) -> String { \"{name} {times}\" }\n\
             #[def_type]\n\
             def test(name: String, f: fn() -> String) -> String { f() }\n\
             def test t(who = greet(\"t\"), n = 1) { \"{who} {n}\" }",
        ),
        (
            "app.gcsl",
            "import lib\n\
             a = greet()\n\
             b = lib.greet(\"you\")\n\
             c = greet(times = 1)\n\
             d = test t()",
        ),
    ];
    assert_eq!(
        rooted_value(&files, "app.gcsl").to_string(),
        "{a = \"world 20\"; b = \"you 20\"; c = \"world 1\"; d = \"t 20 1\"}"
    );
    // A call from another file is checked against the type the default
    // gave.
    let mut wrong = files;
    wrong[2].1 = "a = greet(times = \"x\")";
    let errors = rooted(&wrong).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message, "mismatched types");
    assert_eq!(
        errors[0].label.as_deref(),
        Some("expected Int | Future<Int>, found String")
    );
    // Shown as a host shows a `def`, with the type the default gave.
    let workspace = rooted(&files).unwrap();
    let lib = workspace.file("lib.gcsl").unwrap();
    assert_eq!(
        workspace.top_level(lib, "greet").map(|found| found.ty),
        Some("fn(name: String, times: Int) -> String".to_owned())
    );
}

#[test]
fn a_def_type_s_invocation_parameter_written_with_no_type_is_given() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
             def build(name: String, f: fn(): String, jobs = 4, mode = \"debug\", tags = [\"x\"]) \
             -> String {\n\
             \x20   \"{f()} {jobs} {mode} {len(tags)}\"\n\
             }",
        ),
        (
            "defs.gcsl",
            "def build x() { \"x\" }\nr = build x(jobs = 2, mode = \"m\", tags = [])",
        ),
    ];
    assert_eq!(
        rooted_value(&files, "defs.gcsl").to_string(),
        "{r = \"x 2 m 0\"}"
    );
    let mut workspace = rooted(&files).unwrap();
    let mut invoke = |args: &[(&str, Given)]| {
        let args: Vec<(String, Given)> = args
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect();
        workspace
            .invoke("build", "x", &args)
            .map(|outcome| outcome.value.to_string())
    };
    let error = |result: Result<String, InvokeError>| match result {
        Err(InvokeError::Request(error)) => error.message(),
        other => panic!("{other:?}"),
    };
    // Left out, and given, as the type its default has.
    assert_eq!(invoke(&[]), Ok("x 4 debug 1".to_owned()));
    assert_eq!(
        invoke(&[
            ("jobs", Given::Int(8)),
            ("mode", Given::Str("fast".to_owned()))
        ]),
        Ok("x 8 fast 1".to_owned())
    );
    assert_eq!(
        error(invoke(&[("jobs", Given::Str("many".to_owned()))])),
        "`many` is not an `Int`, the type of `jobs` of `build x`"
    );
    // One that cannot be given is shown with the type its default has.
    assert_eq!(
        error(invoke(&[("tags", Given::Str("t".to_owned()))])),
        "`build`'s parameter `tags` is `List<String>`, but only `String`, `Int`, `Float`, \
         `Bool`, `Duration` and `Size` parameters can be given here yet"
    );
}
