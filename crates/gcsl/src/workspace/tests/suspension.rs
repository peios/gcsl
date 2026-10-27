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
    let Progress::Paused(effect) = workspace.run_prepared(&first, &mut graph, every()).unwrap()
    else {
        panic!("the first target did not stop at its program");
    };
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
    let Progress::Paused(effect) = workspace.run_prepared(&first, &mut graph, every()).unwrap()
    else {
        panic!("the first target did not stop at its program");
    };
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
