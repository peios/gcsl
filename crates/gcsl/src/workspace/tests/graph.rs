//! Tests of workspaces: running units in graph mode, as a host that runs
//! their steps does (see [Graph mode](crate::eval::graph#graph-mode)).

use super::*;
use crate::eval::graph::{Graph, NodeId, NodeKind, NodeState};
use crate::workspace::Progress;

/// A deferring attribute, and two steps that use it.
const STEPS: &str = "#[attribute] #[defers] type step = {}\n\
                     #[step] fn compile(src: String) -> String { \"{src}.o\" }\n\
                     #[step] fn link(objects: List<String>) -> String { \"app of {len(objects)}\" }\n";

/// A checked workspace of the lone file `text`, after [`STEPS`], with the
/// number of its file.
fn lone_workspace(text: &str) -> (Workspace, u32) {
    let source = SourceFile::new("lone.gcsl", format!("{STEPS}{text}")).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    (checked.unwrap(), count(loaded.len()))
}

/// A host that takes every deferring call as a step.
fn every(_: &[crate::check::Attribute]) -> bool {
    true
}

/// The string `text`, as a node's result.
fn text(text: &str) -> Value {
    Value::Str(Rc::from(text))
}

#[test]
fn independent_steps_are_found_before_any_is_needed() {
    let (mut workspace, file) = lone_workspace(
        "let a = compile(\"a\")\nlet b = compile(\"b\")\nlet c = compile(\"a\")\n= link([a, b, c])",
    );
    let mut graph = Graph::new();
    // One attempt finds the whole graph: an impure function takes futures,
    // so the link is a node whose data edges are the compiles, and the
    // third call, of the first's key, is the first's node.
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, &every).unwrap() else {
        panic!("not done");
    };
    assert_eq!(graph.len(), 3);
    let compile_b = graph.node(NodeId::of(1)).unwrap();
    assert_eq!(compile_b.kind(), NodeKind::Deferred);
    assert_eq!(compile_b.args(), [Value::Str(Rc::from("b"))]);
    assert_eq!(compile_b.after(), Some(NodeId::of(0)));
    assert!(compile_b.data().is_empty());
    let link = graph.node(NodeId::of(2)).unwrap();
    assert_eq!(link.data(), [NodeId::of(0), NodeId::of(1)]);
    // Needing the value needs the link, and through it the compiles, which
    // are ready; the link is once they are given.
    let holes = graph.holes(&value);
    assert_eq!(holes, [NodeId::of(2)]);
    let needed = graph.needed(&holes);
    assert_eq!(needed, [NodeId::of(0), NodeId::of(1), NodeId::of(2)]);
    assert_eq!(graph.ready(&needed), [NodeId::of(0), NodeId::of(1)]);
    assert!(graph.resolve(NodeId::of(0), Ok(text("a.o"))));
    assert!(graph.resolve(NodeId::of(1), Ok(text("b.o"))));
    assert_eq!(graph.ready(&needed), [NodeId::of(2)]);
    assert!(graph.resolve(NodeId::of(2), Ok(text("app"))));
    assert!(graph.holes(&value).is_empty());
    // A node is given its result once.
    assert!(!graph.resolve(NodeId::of(2), Ok(text("again"))));
}

#[test]
fn a_unit_that_needs_a_result_stops_and_runs_again_once_it_is_given() {
    let (mut workspace, file) =
        lone_workspace("let a = compile(\"a\")\nlet b = compile(\"b\")\n= \"{a} and {b}\"");
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    // Both were found before the first was needed.
    assert_eq!(graph.len(), 2);
    assert!(graph.resolve(NodeId::of(0), Ok(text("a.o"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Paused(NodeId::of(1))
    );
    assert!(graph.resolve(NodeId::of(1), Ok(text("b.o"))));
    // The attempts found the same nodes again, and made no others.
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Done(Value::Str(Rc::from("a.o and b.o")))
    );
    assert_eq!(graph.len(), 2);
}

#[test]
fn an_effect_is_a_step_needed_at_once() {
    let (mut workspace, file) = lone_workspace("import fs\n= \"read {fs.read(\"/x\")}\"");
    let mut graph = Graph::new();
    let Progress::Paused(read) = workspace.run_graph(file, &mut graph, &every).unwrap() else {
        panic!("not paused");
    };
    let node = graph.node(read).unwrap();
    assert_eq!(node.kind(), NodeKind::Effect(crate::check::Builtin::FsRead));
    assert_eq!(node.args(), [Value::Str(Rc::from("/x"))]);
    assert!(graph.resolve(read, Ok(text("it"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Done(Value::Str(Rc::from("read it")))
    );
    assert_eq!(
        graph.node(read).unwrap().state(),
        &NodeState::Done(Value::Str(Rc::from("it")))
    );
}

#[test]
fn a_step_s_failure_is_raised_where_its_result_is_needed() {
    let (mut workspace, file) = lone_workspace(
        "let a = compile(\"a\")\n= recover { \"{a}!\" } else (e) { \"failed: {e.message}\" }",
    );
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    let failure = Diagnostic::error("cc exited with 1", Span::new(0, 0));
    assert!(graph.resolve(NodeId::of(0), Err(failure)));
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Done(Value::Str(Rc::from("failed: cc exited with 1")))
    );
}

#[test]
fn a_call_the_host_does_not_take_as_a_step_runs_as_ever() {
    let (mut workspace, file) = lone_workspace("= link([compile(\"a\")])");
    let mut graph = Graph::new();
    let none = |_: &[crate::check::Attribute]| false;
    assert_eq!(
        workspace.run_graph(file, &mut graph, &none).unwrap(),
        Progress::Done(Value::Str(Rc::from("app of 1")))
    );
    assert!(graph.is_empty());
}

#[test]
fn a_memoised_call_that_finds_its_entry_shifts_no_other_effect() {
    // The read inside `m` and the one after it have one key: the second is
    // a node of its own, whether `m`'s body runs or its entry is found.
    let (mut workspace, file) = lone_workspace(
        "import fs\n\
         fn m(p: String) ->> String { fs.read(p) }\n\
         let x = m(\"/a\")\n\
         let y = fs.read(\"/a\")\n\
         = \"{x} {y}\"",
    );
    let mut graph = Graph::new();
    let Progress::Paused(inner) = workspace.run_graph(file, &mut graph, &every).unwrap() else {
        panic!("not paused");
    };
    assert!(graph.resolve(inner, Ok(text("inner"))));
    let Progress::Paused(outer) = workspace.run_graph(file, &mut graph, &every).unwrap() else {
        panic!("not paused");
    };
    assert_ne!(inner, outer);
    assert!(graph.resolve(outer, Ok(text("outer"))));
    // `m`'s entry is found now, its body skipped.
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Done(Value::Str(Rc::from("inner outer")))
    );
    assert_eq!(graph.len(), 2);
}

#[test]
fn a_step_s_context_is_part_of_its_key() {
    let (mut workspace, file) = lone_workspace(
        "context arch: String = \"x86_64\"\n\
         let a = compile(\"a\")\n\
         let b = compile(\"a\") with { arch = \"riscv64\" }\n\
         = [a, b]",
    );
    let mut graph = Graph::new();
    workspace.run_graph(file, &mut graph, &every).unwrap();
    // One call, in two contexts: two steps.
    assert_eq!(graph.len(), 2);
}

#[test]
fn a_unit_waiting_for_a_step_is_never_retried() {
    let (mut workspace, file) = lone_workspace(
        "#[retry(3)] fn r() -> String {\n  let o = compile(\"x\")\n  \"{o}!\"\n}\n= r()",
    );
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    assert_eq!(graph.len(), 1);
    assert!(graph.resolve(NodeId::of(0), Ok(text("x.o"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Done(Value::Str(Rc::from("x.o!")))
    );
}

#[test]
fn a_step_whose_key_changes_between_attempts_is_an_error_not_a_loop() {
    let (mut workspace, file) = lone_workspace("let a = compile(\"a\")\n= \"{a}!\"");
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, &every).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    assert!(graph.resolve(NodeId::of(0), Ok(text("a.o"))));
    // A copy of the file's code, which no closure of the first attempt
    // shares: the call at the same point now has another key.
    let FileRun::Paused(ir) = mem::replace(&mut workspace.runs[file as usize], FileRun::Succeeded)
    else {
        panic!("not paused");
    };
    workspace.runs[file as usize] = FileRun::Paused(Rc::new((*ir).clone()));
    let errors = workspace.run_graph(file, &mut graph, &every).unwrap_err();
    assert!(
        errors[0]
            .message
            .contains("evaluation did not converge: a step met here changed between attempts"),
        "{errors:?}"
    );
}

/// Runs file `file` of `workspace` in graph mode to the end, as a host
/// would that runs every needed step, ready ones first, as plain GCSL runs
/// it: its value with every result filled in, and the graph.
fn drive(workspace: &mut Workspace, file: u32) -> (Value, Graph) {
    let mut graph = Graph::new();
    let direct = crate::eval::Direct { temp_dirs: None };
    loop {
        let demands = match workspace.run_graph(file, &mut graph, &every).unwrap() {
            Progress::Paused(node) => vec![node],
            Progress::Done(value) => {
                let holes = graph.holes(&value);
                if holes.is_empty() {
                    return (value, graph);
                }
                holes
            }
        };
        // Every needed step, until none is waiting.
        loop {
            let ready = graph.ready(&graph.needed(&demands));
            if ready.is_empty() {
                break;
            }
            for node in ready {
                let outcome = workspace.evaluate(&graph, node, &direct);
                assert!(graph.resolve(node, outcome));
            }
        }
    }
}

#[test]
fn a_host_runs_each_step_as_plain_gcsl_runs_it() {
    let (mut workspace, file) = lone_workspace(
        "#[step] fn echo(word: String) -> String { exec([\"echo\", word]).stdout.trim() }\n\
         #[step] fn both(words: List<String>) -> String { words.join(\"+\") }\n\
         #[step] fn nested() -> String { echo(\"inner\") }\n\
         context arch: String = \"x86_64\"\n\
         #[step] fn arch_of() -> String { ^arch }\n\
         let top = exec([\"echo\", \"top\"]).stdout.trim()\n\
         = [both([echo(\"a\"), echo(\"b\")]), top, nested(), arch_of() with { arch = \"riscv64\" }]",
    );
    let (value, graph) = drive(&mut workspace, file);
    // A list of futures, every one given: their values.
    let Value::List(items) = value else {
        panic!("not a list");
    };
    let shown: Vec<String> = items
        .values()
        .iter()
        .map(|item| match item {
            Value::Future(future) => future
                .value()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            other => other.to_string(),
        })
        .collect();
    assert_eq!(shown, ["a+b", "top", "inner", "riscv64"]);
    // The top-level `exec` was an effect's node; `nested`'s `echo` ran
    // inside its step, no node of its own.
    let effects = (0..graph.len())
        .filter(|&at| {
            matches!(
                graph
                    .node(NodeId::of(u32::try_from(at).unwrap()))
                    .unwrap()
                    .kind(),
                NodeKind::Effect(_)
            )
        })
        .count();
    assert_eq!(effects, 1);
    assert_eq!(graph.len(), 6);
}

#[test]
fn a_step_that_fails_fails_where_its_result_is_needed() {
    let (mut workspace, file) = lone_workspace(
        "#[step] fn bad() -> String { panic(\"no compiler\") }\n\
         = recover { \"{bad()}\" } else (e) { \"caught: {e.message}\" }",
    );
    let (value, _) = drive(&mut workspace, file);
    assert_eq!(value, Value::Str(Rc::from("caught: no compiler")));
}
