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
