//! The soundness test of reach (see [Reach](crate::workspace::reach#reach)):
//! every standard item a step's evaluation uses, each function its code
//! runs and each top-level cell it reads, is one its key covers, which the
//! step's function reaches or a function a value it is given or reads
//! reaches (see [`crate::coverage`]).

use std::collections::HashSet;

use crate::coverage;
use crate::eval::graph::Graph;
use crate::value::Value;
use crate::workspace::{Progress, Workspace};

use super::graph::{every, lone_workspace};

/// The standard items reached from each function `value` holds, at any
/// depth, by its file's path and its number among the file's items.
fn reached_by_value(workspace: &Workspace, value: &Value, into: &mut HashSet<(String, usize)>) {
    let mut pending = vec![value.clone()];
    while let Some(value) = pending.pop() {
        match value {
            Value::Fn(closure) => {
                pending.extend(closure.captures.iter().cloned());
                if let Some(reached) = workspace.closure_items_reached(&closure) {
                    into.extend(reached);
                }
            }
            Value::Record(record) => pending.extend(record.values.iter().cloned()),
            Value::List(list) => pending.extend(list.values().iter().cloned()),
            Value::Variant(variant) => pending.push(variant.payload.clone()),
            Value::Future(future) => pending.extend(future.value()),
            _ => {}
        }
    }
}

/// Runs file `file` of `workspace` in graph mode to the end, as
/// [`drive`](super::graph::drive) does, checking each step it runs: what
/// its evaluation used of the standard files is covered by its key. The
/// number of steps checked.
fn drive_checked(workspace: &mut Workspace, file: u32) -> usize {
    let mut graph = Graph::new();
    let direct = crate::eval::Direct { temp_dirs: None };
    let mut checked = 0;
    loop {
        let demands = match workspace.run_graph(file, &mut graph, every()).unwrap() {
            Progress::Paused(nodes) => nodes,
            Progress::Waiting(unit) => panic!("one file's units wait for unit {unit}"),
            Progress::Done(value) => {
                let holes = graph.holes(&value);
                if holes.is_empty() {
                    return checked;
                }
                holes
            }
        };
        loop {
            let ready = graph.ready(&graph.needed(&demands));
            if ready.is_empty() {
                break;
            }
            for node in ready {
                // What its key covers: what its function reaches, and what
                // each function its arguments, captures and context hold
                // does.
                let mut covered = HashSet::new();
                let at = graph.node(node).unwrap();
                if let Some(closure) = at.closure() {
                    if let Some(reached) = workspace.closure_items_reached(closure) {
                        covered.extend(reached);
                    }
                    for capture in &closure.captures {
                        reached_by_value(workspace, capture, &mut covered);
                    }
                }
                for arg in at.args() {
                    reached_by_value(workspace, arg, &mut covered);
                }
                for (_, value) in at.context() {
                    reached_by_value(workspace, &value, &mut covered);
                }
                coverage::start();
                let outcome = workspace.evaluate(&graph, node, &direct);
                let used = coverage::stop();
                let (items, unplaced) = workspace.standard_items_used(&used);
                assert!(unplaced.is_empty(), "uses no item holds: {unplaced:?}");
                let missed: Vec<_> = items.difference(&covered).collect();
                assert!(missed.is_empty(), "used but not reached: {missed:?}");
                checked += 1;
                assert!(graph.resolve(node, outcome));
            }
        }
    }
}

/// Checks the lone file `text`: the number of steps checked.
fn check(text: &str) -> usize {
    let (mut workspace, file) = lone_workspace(text);
    drive_checked(&mut workspace, file)
}

#[test]
fn a_step_s_own_calls_and_reads_are_reached() {
    let steps = check(
        "#[step] fn words(text: String) -> List<String> {\n\
         \x20   = split(trim(text), \" \")\n\
         }\n\
         #[step] fn joined(xs: List<String>) -> String { = join(xs.unique(), \",\") }\n\
         = joined(words(\" b a b \"))",
    );
    assert_eq!(steps, 2);
}

#[test]
fn a_function_a_step_is_given_is_covered_by_its_own_reach() {
    let steps = check(
        "fn tidy(s: String | Future<String>) -> String | Future<String> { = trim(s) }\n\
         #[step] fn apply(f: fn(String) -> String, s: String) -> String { = f(s) }\n\
         #[step] fn by_record(r: {f: fn(String) -> String}, s: String) -> String { = r.f(s) }\n\
         = [apply(tidy, \" a \"), by_record({f = tidy}, \" b \")]",
    );
    assert_eq!(steps, 2);
}

#[test]
fn a_function_a_step_reads_from_its_context_is_covered() {
    let steps = check(
        "fn tidy(s: String | Future<String>) -> String | Future<String> { = trim(s) }\n\
         context zz_trimmer: fn(String) -> String = tidy\n\
         #[step] fn trimmed(s: String) -> String { = (^zz_trimmer)(s) }\n\
         = trimmed(\" c \")",
    );
    assert_eq!(steps, 1);
}

#[test]
fn a_step_s_lambda_and_its_captures_are_covered() {
    let steps = check(
        "#[step] fn each(xs: List<String>) -> List<String> {\n\
         \x20   let clean = fn(x: String) -> String { trim(x) }\n\
         \x20   = for x in xs { clean(x) }\n\
         }\n\
         = each([\" d \", \" e \"])",
    );
    assert_eq!(steps, 1);
}
