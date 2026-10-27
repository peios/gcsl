//! Tests of workspaces: units in graph mode that stop where they wait and
//! take up there (see [Suspension](crate::eval::machine#suspension)).

use super::fileroot::rooted;
use super::graph::{every, on_a_thread};
use super::*;
use crate::eval::profile::Profiled;
use crate::ir::FnKind;

/// Two targets whose calls of the memoised `shared` are one call: its
/// body runs a program, an effect, and so stops.
const SHARED: [(&str, &str); 2] = [
    ("workspace.gcsl", ""),
    (
        "app.gcsl",
        "#[def_type]\n\
         def build(name: String, f: fn() -> String) -> String { f() }\n\
         fn shared(word: String) ->> String {\n\
         \x20   let said = exec([\"echo\", word]).stdout.trim()\n\
         \x20   = \"{said}!\"\n\
         }\n\
         def build first() { = shared(\"w\") }\n\
         def build second() { = shared(\"w\") }",
    ),
];

#[test]
fn a_unit_waits_for_the_unit_running_a_memoised_call_it_needs() {
    let mut workspace = rooted(&SHARED).unwrap();
    workspace.start_profiling();
    let first = workspace
        .prepare_call(("build", "first"), &[], &[])
        .unwrap();
    let second = workspace
        .prepare_call(("build", "second"), &[], &[])
        .unwrap();
    let mut graph = Graph::new();
    // The first stops at the program, inside `shared`, its entry running.
    let Progress::Paused(effects) = workspace.run_prepared(&first, &mut graph, every()).unwrap()
    else {
        panic!("the first target did not stop at its program");
    };
    let effect = effects[0];
    // The second needs that entry: it waits for the first's unit, rather
    // than running `shared` again or failing as if it called itself.
    assert_eq!(
        workspace
            .run_prepared(&second, &mut graph, every())
            .unwrap(),
        Progress::Waiting(first.unit())
    );
    let job = workspace.effect_job(&graph, effect).unwrap();
    let value = workspace.from_plain(&on_a_thread(job, None).unwrap());
    assert!(graph.resolve(effect, Ok(value)));
    let done = Progress::Done(Value::Str(Rc::from("w!")));
    assert_eq!(
        workspace.run_prepared(&first, &mut graph, every()).unwrap(),
        done
    );
    assert_eq!(
        workspace
            .run_prepared(&second, &mut graph, every())
            .unwrap(),
        done
    );
    // One program, and one run of `shared`'s body, whose entry the second
    // found.
    assert_eq!(graph.len(), 1);
    let report = workspace.profile_report().unwrap();
    let shared = report
        .rows
        .iter()
        .find(|row| matches!(&row.site, Profiled::Function { name: Some(name), .. } if name == "shared"))
        .map(|row| (row.calls, row.found, matches!(&row.site, Profiled::Function { kind: FnKind::Memo, .. })));
    assert_eq!(shared, Some((2, 1, true)));
}

#[test]
fn a_unit_that_stopped_takes_up_where_it_stopped_after_another_ran() {
    let mut workspace = rooted(&SHARED).unwrap();
    let first = workspace
        .prepare_call(("build", "first"), &[], &[])
        .unwrap();
    let second = workspace
        .prepare_call(("build", "second"), &[], &[])
        .unwrap();
    let mut graph = Graph::new();
    let Progress::Paused(effects) = workspace.run_prepared(&first, &mut graph, every()).unwrap()
    else {
        panic!("the first target did not stop at its program");
    };
    let effect = effects[0];
    let job = workspace.effect_job(&graph, effect).unwrap();
    let value = workspace.from_plain(&on_a_thread(job, None).unwrap());
    assert!(graph.resolve(effect, Ok(value)));
    // The second runs first, its own call finding the first's running
    // entry; then the first takes up, its slots and frames as it left
    // them, and the second finds what it recorded.
    assert_eq!(
        workspace
            .run_prepared(&second, &mut graph, every())
            .unwrap(),
        Progress::Waiting(first.unit())
    );
    let done = Progress::Done(Value::Str(Rc::from("w!")));
    assert_eq!(
        workspace.run_prepared(&first, &mut graph, every()).unwrap(),
        done
    );
    assert_eq!(
        workspace
            .run_prepared(&second, &mut graph, every())
            .unwrap(),
        done
    );
    assert!(workspace.suspended.is_empty());
}

/// Effects a host isolates, every one: they may be met in any order.
#[derive(Debug)]
struct Isolating;

impl crate::eval::Effects for Isolating {
    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }

    fn isolates(&self, _: crate::check::Builtin) -> bool {
        true
    }
}

/// A host that takes every deferring call as a step and isolates every
/// effect.
fn isolating() -> crate::eval::graph::GraphHost<'static> {
    crate::eval::graph::GraphHost {
        defers: &|_: &[crate::check::Attribute]| true,
        effects: &Isolating,
    }
}

/// A function reading each of its paths in a loop, which a run splits.
const READ_ALL: &str = "import fs\n\
                        fn read_all(paths: List<String>) -> List<String> {\n\
                        \x20   = for path in paths { = fs.read(path) }\n\
                        }\n";

/// The text values `texts`, as a list.
fn texts(texts: &[&str]) -> Value {
    Value::List(crate::value::List::from_vec(
        texts
            .iter()
            .map(|text| Value::Str(Rc::from(*text)))
            .collect(),
    ))
}

#[test]
fn a_loop_whose_iterations_wait_meets_every_effect_before_stopping() {
    let (mut workspace, file) =
        super::graph::lone_workspace(&format!("{READ_ALL}= read_all([\"/a\", \"/b\", \"/c\"])"));
    workspace.start_profiling();
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_graph(file, &mut graph, isolating()).unwrap()
    else {
        panic!("the run did not stop for its reads");
    };
    // Each iteration waits apart: all three reads at once.
    assert_eq!(nodes.len(), 3);
    // Given in any order, the values keep the iterations' order.
    for (node, text) in nodes.iter().rev().zip(["C", "B", "A"]) {
        assert!(graph.resolve(*node, Ok(Value::Str(Rc::from(text)))));
    }
    assert_eq!(
        workspace.run_graph(file, &mut graph, isolating()).unwrap(),
        Progress::Done(texts(&["A", "B", "C"]))
    );
    let report = workspace.profile_report().unwrap();
    let read_all = report
        .rows
        .iter()
        .find(|row| matches!(&row.site, Profiled::Function { name: Some(name), .. } if name == "read_all"));
    assert_eq!(read_all.map(|row| row.calls), Some(1));
}

#[test]
fn effects_the_host_does_not_isolate_are_met_in_program_order() {
    let (mut workspace, file) =
        super::graph::lone_workspace(&format!("{READ_ALL}= read_all([\"/a\", \"/b\"])"));
    let mut graph = Graph::new();
    let mut seen = Vec::new();
    let value = loop {
        match workspace.run_graph(file, &mut graph, every()).unwrap() {
            Progress::Paused(nodes) => {
                // One at a time, the earlier iteration's first.
                assert_eq!(nodes.len(), 1, "{nodes:?}");
                let node = nodes[0];
                let path = graph.node(node).unwrap().args()[0].clone();
                seen.push(path.clone());
                assert!(graph.resolve(node, Ok(path)));
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(value) => break value,
        }
    };
    assert_eq!(seen, [text("/a"), text("/b")]);
    assert_eq!(value, texts(&["/a", "/b"]));
}

#[test]
fn the_earliest_iteration_s_failure_is_reported_whichever_fails_first() {
    let (mut workspace, file) =
        super::graph::lone_workspace(&format!("{READ_ALL}= read_all([\"/a\", \"/b\", \"/c\"])"));
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_graph(file, &mut graph, isolating()).unwrap()
    else {
        panic!("the run did not stop for its reads");
    };
    let failure = |what: &str| Err(Diagnostic::error(what, crate::source::Span::new(0, 0)));
    // The third fails, then the second.
    assert!(graph.resolve(nodes[2], failure("third")));
    assert!(graph.resolve(nodes[1], failure("second")));
    assert!(graph.resolve(nodes[0], Ok(text("A"))));
    let errors = workspace
        .run_graph(file, &mut graph, isolating())
        .unwrap_err();
    assert_eq!(errors[0].message, "second", "{errors:?}");
}

#[test]
fn an_accumulating_loop_runs_its_iterations_in_order() {
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn joined(paths: List<String>) -> String {\n\
         \x20   let mut all = \"\"\n\
         \x20   for path in paths { all := \"{all}{fs.read(path)}\" }\n\
         \x20   = all\n\
         }\n\
         = joined([\"/a\", \"/b\"])",
    );
    let mut graph = Graph::new();
    let mut stops = 0;
    let value = loop {
        match workspace.run_graph(file, &mut graph, isolating()).unwrap() {
            Progress::Paused(nodes) => {
                assert_eq!(nodes.len(), 1, "{nodes:?}");
                stops += 1;
                let path = graph.node(nodes[0]).unwrap().args()[0].clone();
                assert!(graph.resolve(nodes[0], Ok(path)));
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(value) => break value,
        }
    };
    assert_eq!(stops, 2);
    assert_eq!(value, text("/a/b"));
}

#[test]
fn iterations_that_call_one_memoised_call_share_it() {
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn shared(path: String) ->> String { = fs.read(path) }\n\
         fn both(paths: List<String>) -> List<String> {\n\
         \x20   = for path in paths { = shared(\"/one\") }\n\
         }\n\
         = both([\"/a\", \"/b\"])",
    );
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_graph(file, &mut graph, isolating()).unwrap()
    else {
        panic!("the run did not stop for its read");
    };
    // The second iteration waits for the first's call: one read.
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    assert!(graph.resolve(nodes[0], Ok(text("it"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, isolating()).unwrap(),
        Progress::Done(texts(&["it", "it"]))
    );
}

/// The text `text`, as a value.
fn text(text: &str) -> Value {
    Value::Str(Rc::from(text))
}

/// Runs `file` of `workspace` in graph mode under `host`, giving each read
/// the text of its path, in upper case, until it ends: how many nodes each
/// stop waited for, and what it came to.
fn run_reads(
    workspace: &mut Workspace,
    file: u32,
    host: crate::eval::graph::GraphHost<'static>,
) -> (Vec<usize>, Value) {
    let mut graph = Graph::new();
    let mut stops = Vec::new();
    loop {
        match workspace.run_graph(file, &mut graph, host).unwrap() {
            Progress::Paused(nodes) => {
                stops.push(nodes.len());
                for node in nodes.iter().rev() {
                    let Value::Str(path) = graph.node(*node).unwrap().args()[0].clone() else {
                        panic!("a read of no path");
                    };
                    let upper = path.trim_start_matches('/').to_uppercase();
                    assert!(graph.resolve(*node, Ok(text(&upper))));
                }
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(value) => return (stops, value),
        }
    }
}

#[test]
fn the_pieces_of_lists_interpolations_and_calls_wait_apart() {
    let cases = [
        (
            "List<String>",
            "= [fs.read(\"/a\"), fs.read(\"/b\")]",
            "[\"A\", \"B\"]",
        ),
        ("String", "= \"{fs.read(\"/a\")}-{fs.read(\"/b\")}\"", "A-B"),
        ("String", "= pair(fs.read(\"/a\"), fs.read(\"/b\"))", "A+B"),
        // A built-in's arguments.
        (
            "String",
            "= [fs.read(\"/a\"), fs.read(\"/b\")].join(fs.read(\"/c\"))",
            "ACB",
        ),
        // A default reads the arguments before it, split off: it waits for
        // them.
        (
            "String",
            "= defaulted(fs.read(\"/a\"), fs.read(\"/b\"))",
            "A+B+A!",
        ),
    ];
    for (ty, body, shown) in cases {
        let (mut workspace, file) = super::graph::lone_workspace(&format!(
            "import fs\n\
             fn pair(a: String, b: String) -> String {{ = \"{{a}}+{{b}}\" }}\n\
             fn defaulted(a: String, b: String, c: String = \"{{a}}!\") -> String {{\n\
             \x20   = \"{{a}}+{{b}}+{{c}}\"\n\
             }}\n\
             fn f() -> {ty} {{ {body} }}\n\
             = f()"
        ));
        let (stops, value) = run_reads(&mut workspace, file, isolating());
        // Every read at once, in one stop.
        assert_eq!(stops.len(), 1, "{body}: {stops:?}");
        assert_eq!(value.to_string(), shown, "{body}");
    }
}

#[test]
fn the_earliest_piece_s_failure_is_reported_whichever_fails_first() {
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn f() -> List<String> { = [fs.read(\"/a\"), fs.read(\"/b\"), fs.read(\"/c\")] }\n\
         = f()",
    );
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_graph(file, &mut graph, isolating()).unwrap()
    else {
        panic!("the run did not stop for its reads");
    };
    assert_eq!(nodes.len(), 3);
    let failure = |what: &str| Err(Diagnostic::error(what, crate::source::Span::new(0, 0)));
    assert!(graph.resolve(nodes[2], failure("third")));
    assert!(graph.resolve(nodes[1], failure("second")));
    assert!(graph.resolve(nodes[0], Ok(text("A"))));
    let errors = workspace
        .run_graph(file, &mut graph, isolating())
        .unwrap_err();
    assert_eq!(errors[0].message, "second", "{errors:?}");
}

/// Effects a host isolates: reads, but no others.
#[derive(Debug)]
struct ReadsIsolated;

impl crate::eval::Effects for ReadsIsolated {
    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }

    fn isolates(&self, builtin: crate::check::Builtin) -> bool {
        builtin == crate::check::Builtin::FsRead
    }
}

#[test]
fn an_effect_in_order_after_a_piece_that_failed_is_never_made() {
    let host = crate::eval::graph::GraphHost {
        defers: &|_: &[crate::check::Attribute]| true,
        effects: &ReadsIsolated,
    };
    for (ty, body, reads) in [
        (
            "String",
            "= both(fs.read(\"/a\"), fs.write(\"/w\", \"x\"))",
            1,
        ),
        // Each iteration's read is made, the second's write waiting its
        // turn behind the first iteration.
        (
            "List<String>",
            "= for path in [\"/a\", \"/b\"] { = both(fs.read(path), fs.write(\"/w\", \"x\")) }",
            2,
        ),
    ] {
        let (mut workspace, file) = super::graph::lone_workspace(&format!(
            "import fs\n\
             fn both(a: String, w: {{}}) -> String {{ = a }}\n\
             fn f() -> {ty} {{ {body} }}\n\
             = f()"
        ));
        let mut graph = Graph::new();
        let Progress::Paused(nodes) = workspace.run_graph(file, &mut graph, host).unwrap() else {
            panic!("{body}: the run did not stop for its read");
        };
        // The write waits its turn, behind the first read.
        assert_eq!(nodes.len(), reads, "{body}: {nodes:?}");
        let failure = Diagnostic::error("unreadable", crate::source::Span::new(0, 0));
        assert!(graph.resolve(nodes[0], Err(failure)));
        let errors = workspace.run_graph(file, &mut graph, host).unwrap_err();
        assert_eq!(errors[0].message, "unreadable", "{body}: {errors:?}");
        // A run in order fails at the first read: no write is ever made.
        assert_eq!(graph.len(), reads, "{body}");
    }
}

#[test]
fn what_waits_inside_what_is_split_off_still_meets_every_effect_at_once() {
    let cases = [
        // The second element assigns, so it is not split off: the
        // iteration is, with the first element split off from it.
        (
            "List<List<String>>",
            "= for path in [\"/a\", \"/b\"] {\n\
             \x20   = [fs.read(path), { let mut t = \"\"; t := fs.read(\"{path}2\"); = t }]\n\
             }",
            "[[\"A\", \"A2\"], [\"B\", \"B2\"]]",
        ),
        // The first element waits for its loop's iterations: it is split
        // off, and the second runs.
        (
            "List<List<String>>",
            "= [read_all([\"/a\", \"/b\"]), read_all([\"/c\", \"/d\"])]",
            "[[\"A\", \"B\"], [\"C\", \"D\"]]",
        ),
    ];
    for (ty, body, shown) in cases {
        let (mut workspace, file) =
            super::graph::lone_workspace(&format!("{READ_ALL}fn f() -> {ty} {{ {body} }}\n= f()"));
        let (stops, value) = run_reads(&mut workspace, file, isolating());
        assert_eq!(stops, [4], "{body}");
        assert_eq!(value.to_string(), shown, "{body}");
    }
}

#[test]
fn the_items_of_a_block_and_the_fields_of_a_record_wait_apart() {
    let cases = [
        (
            "String",
            "let a = fs.read(\"/a\")\n let b = fs.read(\"/b\")\n = \"{a}{b}\"",
            vec![2],
            "AB",
        ),
        (
            "{x: String, y: String}",
            "= {x = fs.read(\"/a\"); y = fs.read(\"/b\")}",
            vec![2],
            "{x = \"A\"; y = \"B\"}",
        ),
        // An item that uses what an item set aside declares waits for it.
        (
            "String",
            "let a = fs.read(\"/a\")\n let b = fs.read(\"/{a}x\")\n = b",
            vec![1, 1],
            "AX",
        ),
        // So does one that assigns it, or a variable it shares a slot with.
        (
            "String",
            "let mut t = \"\"\n let a = fs.read(\"/a\")\n let b = fs.read(\"/b\")\n t := a\n = \"{t}{b}\"",
            vec![2],
            "AB",
        ),
    ];
    for (ty, body, stops, shown) in cases {
        let (mut workspace, file) = super::graph::lone_workspace(&format!(
            "import fs\nfn f() -> {ty} {{\n {body}\n}}\n= f()"
        ));
        let (seen, value) = run_reads(&mut workspace, file, isolating());
        assert_eq!(seen, stops, "{body}");
        assert_eq!(value.to_string(), shown, "{body}");
    }
}

#[test]
fn the_earliest_item_s_failure_is_reported_whichever_fails_first() {
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn f() -> String {\n\
         \x20   let a = fs.read(\"/a\")\n\
         \x20   let b = fs.read(\"/b\")\n\
         \x20   = \"{a}{b}\"\n\
         }\n\
         = f()",
    );
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_graph(file, &mut graph, isolating()).unwrap()
    else {
        panic!("the run did not stop for its reads");
    };
    assert_eq!(nodes.len(), 2);
    let failure = |what: &str| Err(Diagnostic::error(what, crate::source::Span::new(0, 0)));
    assert!(graph.resolve(nodes[1], failure("second")));
    assert!(graph.resolve(nodes[0], failure("first")));
    let errors = workspace
        .run_graph(file, &mut graph, isolating())
        .unwrap_err();
    assert_eq!(errors[0].message, "first", "{errors:?}");
}

#[test]
fn an_earlier_task_runs_a_memoised_call_a_later_one_began_and_waits_in() {
    // The second element begins `shared` while the first waits for its
    // read; inside, its write waits its turn behind the first. The first
    // then needs `shared`: the second abandons its run, and the first runs
    // it, as a run in order would, so the write is made once.
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn shared() ->> String {\n\
         \x20   let w = fs.write(\"/w\", \"x\")\n\
         \x20   = \"s\"\n\
         }\n\
         fn g(t: String) -> String { = \"{t}{shared()}\" }\n\
         fn f() -> List<String> { = [g(fs.read(\"/a\")), shared()] }\n\
         = f()",
    );
    let host = crate::eval::graph::GraphHost {
        defers: &|_: &[crate::check::Attribute]| true,
        effects: &ReadsIsolated,
    };
    let mut graph = Graph::new();
    let value = loop {
        match workspace.run_graph(file, &mut graph, host).unwrap() {
            Progress::Paused(nodes) => {
                for node in nodes {
                    let given = match graph.node(node).unwrap().args()[0].clone() {
                        Value::Str(path) if &*path == "/a" => text("A"),
                        _ => Value::Empty,
                    };
                    assert!(graph.resolve(node, Ok(given)));
                }
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(value) => break value,
        }
    };
    assert_eq!(value, texts(&["As", "s"]));
    // One read and one write.
    assert_eq!(graph.len(), 2);
}

#[test]
fn memoised_calls_that_need_each_other_across_tasks_fail_as_a_run_in_order_does() {
    // A run in order calls `p`, then `q` inside it, then `p` inside that:
    // a call that depends on its own result. Split into tasks, each may
    // begin one; the earlier then runs the other's for it.
    for list in ["[p(), q()]", "[q(), p()]"] {
        let (mut workspace, file) = super::graph::lone_workspace(&format!(
            "import fs\n\
             fn p() ->> String {{ = \"{{fs.read(\"/a\")}}{{q()}}\" }}\n\
             fn q() ->> String {{ = \"{{fs.read(\"/b\")}}{{p()}}\" }}\n\
             fn f() -> List<String> {{ = {list} }}\n\
             = f()"
        ));
        let mut graph = Graph::new();
        let errors = loop {
            match workspace.run_graph(file, &mut graph, isolating()) {
                Ok(Progress::Paused(nodes)) => {
                    for node in nodes {
                        assert!(graph.resolve(node, Ok(text("x"))));
                    }
                }
                Ok(other) => panic!("{list}: {other:?}"),
                Err(errors) => break errors,
            }
        };
        assert_eq!(
            errors[0].message, "this call depends on its own result",
            "{list}: {errors:?}"
        );
    }
}

#[test]
fn an_earlier_task_runs_a_lazy_a_later_one_began_and_waits_in() {
    // As for a memoised call: the second element begins the `lazy`, whose
    // write waits its turn behind the first; the first needs it too.
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn g(t: String, s: Future<String>) -> String { = \"{t}{await s}\" }\n\
         fn f() -> List<String> {\n\
         \x20   let s = lazy {\n\
         \x20       let w = fs.write(\"/w\", \"x\")\n\
         \x20       = \"s\"\n\
         \x20   }\n\
         \x20   = [g(fs.read(\"/a\"), s), await s]\n\
         }\n\
         = f()",
    );
    let host = crate::eval::graph::GraphHost {
        defers: &|_: &[crate::check::Attribute]| true,
        effects: &ReadsIsolated,
    };
    let mut graph = Graph::new();
    let value = loop {
        match workspace.run_graph(file, &mut graph, host).unwrap() {
            Progress::Paused(nodes) => {
                for node in nodes {
                    let given = match graph.node(node).unwrap().args()[0].clone() {
                        Value::Str(path) if &*path == "/a" => text("A"),
                        _ => Value::Empty,
                    };
                    assert!(graph.resolve(node, Ok(given)));
                }
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(value) => break value,
        }
    };
    assert_eq!(value, texts(&["As", "s"]));
    assert_eq!(graph.len(), 2);
}

#[test]
fn a_task_on_a_piece_after_one_split_off_is_later_than_it_in_program_order() {
    // The first element is split off at its read; the task goes on with
    // the second, which is never marked, being the last. Its write is
    // after the first element's in program order, so it waits.
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn f() -> List<{}> {\n\
         \x20   = [{ let t = fs.read(\"/a\")\n = fs.write(\"/one\", t) }, fs.write(\"/two\", \"y\")]\n\
         }\n\
         = f()",
    );
    let host = crate::eval::graph::GraphHost {
        defers: &|_: &[crate::check::Attribute]| true,
        effects: &ReadsIsolated,
    };
    let mut graph = Graph::new();
    let mut seen = Vec::new();
    loop {
        match workspace.run_graph(file, &mut graph, host).unwrap() {
            Progress::Paused(nodes) => {
                for node in nodes {
                    let first = graph.node(node).unwrap().args()[0].clone();
                    seen.push(first.clone());
                    let given = if first == text("/a") {
                        text("A")
                    } else {
                        Value::Empty
                    };
                    assert!(graph.resolve(node, Ok(given)));
                }
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(_) => break,
        }
    }
    assert_eq!(seen, [text("/a"), text("/one"), text("/two")]);
}

#[test]
fn what_is_split_off_inside_a_piece_before_the_piece_is_keeps_its_place() {
    // The first statement is split off at its read. The inner list's
    // third element then waits its turn and is split off while the outer
    // list has nothing split off; then the outer list's first element is.
    // Every write keeps program order.
    let (mut workspace, file) = super::graph::lone_workspace(
        "import fs\n\
         fn f() -> List<List<{}>> {\n\
         \x20   let t = fs.read(\"/r\")\n\
         \x20   let l = [[{}, {}, fs.write(\"/a1\", \"x\"), fs.write(\"/a2\", \"x\")], [fs.write(\"/b\", \"x\")]]\n\
         \x20   = l\n\
         }\n\
         = f()",
    );
    let host = crate::eval::graph::GraphHost {
        defers: &|_: &[crate::check::Attribute]| true,
        effects: &ReadsIsolated,
    };
    let mut graph = Graph::new();
    let mut seen = Vec::new();
    loop {
        match workspace.run_graph(file, &mut graph, host).unwrap() {
            Progress::Paused(nodes) => {
                for node in nodes {
                    seen.push(graph.node(node).unwrap().args()[0].clone());
                    assert!(graph.resolve(node, Ok(Value::Empty)));
                }
            }
            Progress::Waiting(unit) => panic!("waits for unit {unit}"),
            Progress::Done(_) => break,
        }
    }
    assert_eq!(seen, [text("/r"), text("/a1"), text("/a2"), text("/b")]);
}
