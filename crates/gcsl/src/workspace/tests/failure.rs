//! Tests of workspaces: what a host is told of the failures a program
//! catches, and `raise` (see [Failure](crate::eval::failure#failure)).

use std::cell::RefCell;

use super::effects::run_through;
use super::fileroot::rooted;
use super::*;
use crate::eval::plain::Plain;
use crate::eval::{CaughtFailure, Direct, Effects};

/// A host that keeps each failure it is told a program caught: its
/// message, the text its span covers in `source`, its notes, the input it
/// names and whether it was raised again.
#[derive(Debug)]
struct Told {
    source: String,
    caught: RefCell<Vec<Heard>>,
}

/// A failure a [`Told`] was told of: its message, the text it is located
/// at, its notes, the input it names and whether it was raised again.
type Heard = (String, String, Vec<String>, Option<u32>, bool);

impl Told {
    /// A host of a program whose text is `source`.
    fn of(source: &str) -> Self {
        Self {
            source: source.to_owned(),
            caught: RefCell::new(Vec::new()),
        }
    }

    /// The failures told so far, each its message, the text it is located
    /// at and whether it was raised again.
    fn told(&self) -> Vec<(String, String, bool)> {
        self.caught
            .borrow()
            .iter()
            .map(|(message, at, _, _, again)| (message.clone(), at.clone(), *again))
            .collect()
    }
}

impl Effects for Told {
    fn caught(&self, caught: &CaughtFailure<'_>) {
        let failure = caught.failure;
        let span = failure.span;
        let at = self
            .source
            .get(span.start as usize..span.end as usize)
            .unwrap_or("<outside>")
            .to_owned();
        self.caught.borrow_mut().push((
            failure.message.clone(),
            at,
            failure.notes.clone(),
            failure.input,
            caught.again,
        ));
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        Direct { temp_dirs: None }.temp_dir()
    }
}

#[test]
fn a_host_is_told_of_every_failure_a_program_catches() {
    let text = "fn check(x: Int) -> Int {\n\
                \x20   assert x > 0, \"x must be positive\"\n\
                \x20   = x\n\
                }\n\
                a = recover { check(1) } else { 0 }\n\
                b = recover { check(-1) } else { 0 }\n\
                c = recover { [1, 2][5] } else (failure) { 7 }\n\
                d = recover { recover { panic(\"inner\") } else { panic(\"outer\") } } else { 9 }";
    let host = Told::of(text);
    let value = run_through(text, &host).unwrap();
    assert_eq!(value.to_string(), "{a = 1; b = 0; c = 7; d = 9}");
    // Each as it would have been reported, where it was raised; a body
    // that does not fail tells nothing.
    assert_eq!(
        host.told(),
        [
            (
                "assertion failed: x must be positive".to_owned(),
                "x > 0".to_owned(),
                false
            ),
            (
                "index out of range".to_owned(),
                "[1, 2][5]".to_owned(),
                false
            ),
            ("inner".to_owned(), "panic(\"inner\")".to_owned(), false),
            ("outer".to_owned(), "panic(\"outer\")".to_owned(), false),
        ]
    );
    // Each names its input, the file its span is of.
    assert!(
        host.caught
            .borrow()
            .iter()
            .all(|(_, _, _, input, _)| input.is_some())
    );
}

#[test]
fn a_failure_raised_again_is_the_failure_it_was() {
    let text = "fn step(name: String, body: block -> {}) -> {} {\n\
                \x20   recover { body() } else (failure) {\n\
                \x20       let seen = \"{name}: {failure.message}\"\n\
                \x20       raise\n\
                \x20   }\n\
                }\n\
                fn run(word: String) -> {} {\n\
                \x20   let made = exec([\"sh\", \"-c\", \"echo {word}; exit 3\"])\n\
                \x20   = {}\n\
                }\n\
                a = recover { step(\"one\") { panic(\"boom\") }; = \"passed\" } \
                else (failure) { failure.message }\n\
                b = recover { step(\"two\") { run(\"said\") }; = [\"passed\"] } \
                else (failure) { failure.notes }";
    let host = Told::of(text);
    let value = run_through(text, &host).unwrap();
    // The message and the notes are the first failure's.
    assert_eq!(
        value.to_string(),
        "{a = \"boom\"; b = [\"its standard output: said\"]}"
    );
    // Told twice each: where it was raised both times, the second marked
    // as raised again.
    let told = host.told();
    assert_eq!(told.len(), 4, "{told:?}");
    assert_eq!(
        told[0],
        ("boom".to_owned(), "panic(\"boom\")".to_owned(), false)
    );
    assert_eq!(
        told[1],
        ("boom".to_owned(), "panic(\"boom\")".to_owned(), true)
    );
    assert_eq!((told[2].2, told[3].2), (false, true));
    assert_eq!(told[2].1, told[3].1);
    assert_eq!(host.caught.borrow()[3].2, ["its standard output: said"]);
}

#[test]
fn an_uncaught_failure_raised_again_is_reported_where_it_was_raised() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "app.gcsl",
            "fn step(body: block -> {}) -> {} {\n\
             \x20   recover { body() } else { raise }\n\
             }\n\
             step { assert 1 == 2 }",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let app = workspace.file("app.gcsl").unwrap();
    let error = workspace.run(app).unwrap_err().remove(0);
    assert_eq!(error.message, "assertion failed: `1 == 2`");
    let text = files[1].1;
    assert_eq!(
        &text[error.span.start as usize..error.span.end as usize],
        "1 == 2"
    );
    // The assertion's labels come with it.
    let labels: Vec<&str> = error
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .collect();
    assert_eq!(labels, ["this is `1`", "this is `2`"]);
}

#[test]
fn a_raise_is_for_the_recover_it_is_written_in() {
    // The block is called while `helper`'s own fallback runs: its `raise`
    // is still for the failure of the `recover` it is written in.
    let text = "fn helper(body: block -> {}) -> {} {\n\
                \x20   recover { panic(\"helper's\") } else { body() }\n\
                }\n\
                fn nested() -> String {\n\
                \x20   = recover {\n\
                \x20       recover { panic(\"first\") } else { helper { raise } }\n\
                \x20       = \"unreached\"\n\
                \x20   } else (failure) { failure.message }\n\
                }\n\
                fn inner() -> String {\n\
                \x20   = recover {\n\
                \x20       recover { panic(\"outer's\") } else {\n\
                \x20           recover { panic(\"inner's\") } else { raise }\n\
                \x20       }\n\
                \x20       = \"unreached\"\n\
                \x20   } else (failure) { failure.message }\n\
                }\n\
                fn again(depth: Int) -> String {\n\
                \x20   = recover {\n\
                \x20       recover { panic(\"at {depth}\") } else {\n\
                \x20           if depth < 2 { let deeper = again(depth + 1) }\n\
                \x20           raise\n\
                \x20       }\n\
                \x20       = \"unreached\"\n\
                \x20   } else (failure) { failure.message }\n\
                }\n\
                a = nested()\n\
                b = inner()\n\
                c = again(0)";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        super::fileroot::rooted_text(&files, "app.gcsl"),
        "{a = \"first\"; b = \"inner's\"; c = \"at 0\"}"
    );
}

#[test]
fn a_variable_named_raise_is_read_as_ever() {
    let text = "let raise = 3\n\
                a = raise + 1\n\
                b = recover { panic(\"boom\") } else { raise }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        super::fileroot::rooted_text(&files, "app.gcsl"),
        "{a = 4; b = 3}"
    );
}

#[test]
fn a_unit_in_graph_mode_tells_its_host_what_it_catches() {
    // The step stops for an operation the host fails: the failure is
    // caught in the step, raised again, and caught by the test.
    let app = "#[def_type]\n\
               def test(name: String, f: fn() -> String) -> String { f() }\n\
               fn op(word: String) -> String { exec([\"op\", word]).stdout }\n\
               fn step(name: String, body: block -> {}) -> {} {\n\
               \x20   recover { body() } else { raise }\n\
               }\n\
               def test one() {\n\
               \x20   let mut seen = op(\"first\")\n\
               \x20   let outcome = recover {\n\
               \x20       step(\"a\") { seen := seen + op(\"second\") }\n\
               \x20       = \"passed\"\n\
               \x20   } else (failure) { \"failed: {failure.message}\" }\n\
               \x20   = \"{seen} {outcome}\"\n\
               }";
    let mut workspace = rooted(&[("workspace.gcsl", ""), ("app.gcsl", app)]).unwrap();
    let host = Told::of(app);
    let never = |_: &[crate::check::Attribute]| false;
    let graph_host = || GraphHost {
        defers: &never,
        effects: &host,
    };
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace
        .run_prepared(&call, &mut graph, graph_host())
        .unwrap()
    else {
        panic!("the unit did not stop at its first operation");
    };
    let answered = Plain::Record(vec![
        ("status".to_owned(), Plain::Int(0)),
        ("stdout".to_owned(), Plain::Str("one".to_owned())),
        ("stderr".to_owned(), Plain::Str(String::new())),
    ]);
    let value = workspace.from_plain(&answered);
    assert!(graph.resolve(nodes[0], Ok(value)));
    let Progress::Paused(nodes) = workspace
        .run_prepared(&call, &mut graph, graph_host())
        .unwrap()
    else {
        panic!("the unit did not stop at its second operation");
    };
    // Nothing is caught while the unit waits.
    assert!(host.told().is_empty());
    let gone = Diagnostic::error("the guest is gone", Span::new(0, 0)).with_note("vm 3");
    assert!(graph.resolve(nodes[0], Err(gone)));
    let done = workspace
        .run_prepared(&call, &mut graph, graph_host())
        .unwrap();
    assert_eq!(
        done,
        Progress::Done(Value::Str(Rc::from("one failed: the guest is gone")))
    );
    // Once in the step and once in the test, at the operation's call.
    let told = host.told();
    assert_eq!(told.len(), 2, "{told:?}");
    assert_eq!(
        told[0],
        (
            "the guest is gone".to_owned(),
            "exec([\"op\", word])".to_owned(),
            false
        )
    );
    assert!(told[1].2);
    assert_eq!(told[1].1, told[0].1);
    assert_eq!(host.caught.borrow()[1].2, ["vm 3"]);
}

#[test]
fn a_try_goes_on_from_a_failure_and_the_host_is_told() {
    let text = "fn step(name: String, body: block -> {}) -> {} {\n\
                \x20   recover { body() } else { raise }\n\
                }\n\
                fn parse(word: String) -> Int {\n\
                \x20   = if word == \"seven\" { 7 } else { panic(\"no number: {word}\") }\n\
                }\n\
                let mut ran = \"\"\n\
                try step(\"first\") { panic(\"first failed\") }\n\
                try step(\"second\") { ran := ran + \"second;\" }\n\
                let seven = try parse(\"seven\")\n\
                let none = try parse(\"eight\") + 1\n\
                let both = try { let a = parse(\"seven\"); a * 2 }\n\
                out = {ran = ran; seven = seven; none = none is {}; both = both}";
    let host = Told::of(text);
    let value = run_through(text, &host).unwrap();
    assert_eq!(
        value.to_string(),
        "{out = {ran = \"second;\"; seven = 7; none = true; both = 14}}"
    );
    // The step's failure once in the step and once at the `try`, raised
    // again; the failed parse once.
    assert_eq!(
        host.told(),
        [
            (
                "first failed".to_owned(),
                "panic(\"first failed\")".to_owned(),
                false
            ),
            (
                "first failed".to_owned(),
                "panic(\"first failed\")".to_owned(),
                true
            ),
            (
                "no number: eight".to_owned(),
                "panic(\"no number: {word}\")".to_owned(),
                false
            ),
        ]
    );
}

#[test]
fn a_try_catches_what_its_operand_defers() {
    // The call's result is a future whose body fails: the `try` awaits it,
    // so the failure is inside it.
    let text = "fn later() -> Future<Int> { lazy panic(\"deferred\") }\n\
                a = try later()\n\
                b = (try later()) is {}";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        super::fileroot::rooted_text(&files, "app.gcsl"),
        "{a = {}; b = true}"
    );
}
