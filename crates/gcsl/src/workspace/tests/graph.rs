//! Tests of workspaces: running units in graph mode, as a host that runs
//! their steps does (see [Graph mode](crate::eval::graph#graph-mode)).

use super::*;
use crate::eval::graph::{Graph, GraphHost, NodeId, NodeKind, NodeState};
use crate::eval::plain::Plain;
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

/// Whether a host takes a deferring call as a step: always.
fn every_call(_: &[crate::check::Attribute]) -> bool {
    true
}

/// A host that takes every deferring call as a step, its other effects
/// plain GCSL's.
fn every() -> GraphHost<'static> {
    GraphHost {
        defers: &every_call,
        effects: &crate::eval::Direct { temp_dirs: None },
    }
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
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
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
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    // Both were found before the first was needed.
    assert_eq!(graph.len(), 2);
    assert!(graph.resolve(NodeId::of(0), Ok(text("a.o"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(1))
    );
    assert!(graph.resolve(NodeId::of(1), Ok(text("b.o"))));
    // The attempts found the same nodes again, and made no others.
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("a.o and b.o")))
    );
    assert_eq!(graph.len(), 2);
}

#[test]
fn an_effect_is_a_step_needed_at_once() {
    let (mut workspace, file) = lone_workspace("import fs\n= \"read {fs.read(\"/x\")}\"");
    let mut graph = Graph::new();
    let Progress::Paused(read) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not paused");
    };
    let node = graph.node(read).unwrap();
    assert_eq!(node.kind(), NodeKind::Effect(crate::check::Builtin::FsRead));
    assert_eq!(node.args(), [Value::Str(Rc::from("/x"))]);
    assert!(graph.resolve(read, Ok(text("it"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
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
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    let failure = Diagnostic::error("cc exited with 1", Span::new(0, 0));
    assert!(graph.resolve(NodeId::of(0), Err(failure)));
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("failed: cc exited with 1")))
    );
}

#[test]
fn a_def_type_s_deferring_attribute_makes_its_definitions_steps_and_not_itself() {
    // As the prelude's `test` is: the definition a step, the def-type's
    // own body, which recovers its failure, not.
    let (mut workspace, file) = lone_workspace(
        "#[def_type] #[step] def check(name: String, f: fn() -> String) -> String {\n\
         \x20   = recover { await f() } else (e) { \"{name} failed: {e.message}\" }\n\
         }\n\
         def check one() { = \"one passed\" }\n\
         result = await (check one)()",
    );
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    assert_eq!(graph.len(), 1);
    let failure = Diagnostic::error("assertion failed", Span::new(0, 0));
    assert!(graph.resolve(NodeId::of(0), Err(failure)));
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    assert!(
        value
            .to_string()
            .ends_with("result = \"one failed: assertion failed\"}"),
        "{value}"
    );
}

#[test]
fn a_definition_s_own_parameters_are_its_step_s_arguments() {
    // As a test is given its program: the default a step of its own, run
    // where the def-type calls the definition, and the body a step named
    // after the definition, given what the default came to.
    let (mut workspace, file) = lone_workspace(
        "#[def_type] #[step] def check(name: String, f: fn() -> String) -> String {\n\
         \x20   = recover { await f() } else (e) { \"{name} failed: {e.message}\" }\n\
         }\n\
         def check one(object: String = compile(\"w\")) { = \"{object} checked\" }\n\
         result = await (check one)()",
    );
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(1))
    );
    assert_eq!(graph.len(), 2);
    assert!(graph.resolve(NodeId::of(0), Ok(text("w.o"))));
    let inputs = workspace.step_inputs(&graph, NodeId::of(1)).unwrap();
    assert_eq!(inputs.args, [Plain::Str("w.o".to_owned())]);
    let name = graph
        .node(NodeId::of(1))
        .and_then(|node| node.closure())
        .and_then(|closure| closure.code.find_function(closure.function))
        .and_then(|code| code.name_text.clone());
    assert_eq!(name.as_deref(), Some("one"));
    assert!(graph.resolve(NodeId::of(1), Ok(text("w.o checked"))));
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    assert!(
        value.to_string().ends_with("result = \"w.o checked\"}"),
        "{value}"
    );
}

#[test]
fn a_call_the_host_does_not_take_as_a_step_runs_as_ever() {
    let (mut workspace, file) = lone_workspace("= link([compile(\"a\")])");
    let mut graph = Graph::new();
    let none = |_: &[crate::check::Attribute]| false;
    let host = GraphHost {
        defers: &none,
        effects: &crate::eval::Direct { temp_dirs: None },
    };
    assert_eq!(
        workspace.run_graph(file, &mut graph, host).unwrap(),
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
    let Progress::Paused(inner) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not paused");
    };
    assert!(graph.resolve(inner, Ok(text("inner"))));
    let Progress::Paused(outer) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not paused");
    };
    assert_ne!(inner, outer);
    assert!(graph.resolve(outer, Ok(text("outer"))));
    // `m`'s entry is found now, its body skipped.
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
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
    workspace.run_graph(file, &mut graph, every()).unwrap();
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
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(0))
    );
    assert_eq!(graph.len(), 1);
    assert!(graph.resolve(NodeId::of(0), Ok(text("x.o"))));
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("x.o!")))
    );
}

#[test]
fn a_step_given_another_s_result_keys_the_same_once_it_is_given() {
    // `link` is given `compile`'s result before it is given; the unit then
    // needs `link`'s, so it runs again with both given.
    let (mut workspace, file) =
        lone_workspace("let o = compile(\"a\")\nlet app = link([o])\n= \"{app}!\"");
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Paused(NodeId::of(1))
    );
    assert!(graph.resolve(NodeId::of(0), Ok(text("a.o"))));
    assert!(graph.resolve(NodeId::of(1), Ok(text("app"))));
    // The same two nodes, `link`'s found by the key it had.
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
        Progress::Done(text("app!"))
    );
    assert_eq!(graph.len(), 2);
}

#[test]
fn a_step_whose_key_changes_between_attempts_is_an_error_not_a_loop() {
    let (mut workspace, file) = lone_workspace("let a = compile(\"a\")\n= \"{a}!\"");
    let mut graph = Graph::new();
    assert_eq!(
        workspace.run_graph(file, &mut graph, every()).unwrap(),
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
    let errors = workspace.run_graph(file, &mut graph, every()).unwrap_err();
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
        let demands = match workspace.run_graph(file, &mut graph, every()).unwrap() {
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

#[test]
fn a_value_made_plain_and_back_is_the_value_it_was() {
    let (mut workspace, file) = lone_workspace(
        "enum Shape extensible\n\
         enum Shape::Square<{side: Int}>\n\
         = {\n\
           n = 1; x = 1.5; ok = true; s = \"text\"; t = 30s; e = {}\n\
           items = [1, 2]; shape = Shape::Square({side = 3}); kind = Shape::Square\n\
           key = secret(\"k3y\", \"api key\"); src = file(\"/src/a.c\"); out = dir(\"/out\")\n\
         }",
    );
    let Progress::Done(value) = workspace
        .run_graph(file, &mut Graph::new(), every())
        .unwrap()
    else {
        panic!("not done");
    };
    let plain = workspace.to_plain(&value).unwrap();
    let back = workspace.from_plain(&plain);
    assert_eq!(back, value);
    // Its files and directories, for staging.
    assert_eq!(plain.files(), [("/src/a.c", false), ("/out", true)]);
    // What is no data has no plain form.
    let (mut workspace, file) = lone_workspace("= {f = fn(): Int { 1 }}");
    let Progress::Done(value) = workspace
        .run_graph(file, &mut Graph::new(), every())
        .unwrap()
    else {
        panic!("not done");
    };
    assert_eq!(
        workspace.to_plain(&value).unwrap_err(),
        "a step's result must be plain data: it holds a function"
    );
}

#[test]
fn a_step_s_inputs_are_what_a_host_keys_and_stages_it_by() {
    let (mut workspace, file) = lone_workspace(
        "context arch: String = \"x86_64\"\n\
         let flags = [\"-O2\"]\n\
         #[step] fn cc(src: File) -> String { \"cc {flags} {src.path}\" }\n\
         = cc(file(\"/src/a.c\")) with { arch = \"riscv64\" }",
    );
    let mut graph = Graph::new();
    workspace.run_graph(file, &mut graph, every()).unwrap();
    let inputs = workspace.step_inputs(&graph, NodeId::of(0)).unwrap();
    assert_eq!(inputs.kind, NodeKind::Deferred);
    assert_eq!(
        inputs.args,
        [crate::eval::plain::Plain::File {
            path: "/src/a.c".to_owned(),
            identity: None
        }]
    );
    // What its code captured, and the context it runs in.
    assert_eq!(
        inputs.captures,
        [crate::eval::plain::Plain::List(vec![
            crate::eval::plain::Plain::Str("-O2".to_owned())
        ])]
    );
    assert_eq!(inputs.context.len(), 1);
    assert!(
        inputs.context[0].0.starts_with("<context arch@"),
        "{:?}",
        inputs.context
    );
    assert_eq!(
        inputs.context[0].1,
        crate::eval::plain::Plain::Str("riscv64".to_owned())
    );
    // Its attribute, and the files its code is made of: its own, and the
    // standard library's.
    assert_eq!(inputs.attributes.len(), 1);
    assert!(inputs.code_files.contains(&file));
    assert!(inputs.code_files.len() > 1);
}

/// A workspace of a deferring def-type `build`, a definition of it, and
/// a def-type `bumped` that needs a context with no default.
const TARGETS: [(&str, &str); 2] = [
    ("workspace.gcsl", ""),
    (
        "app.gcsl",
        "#[attribute] #[defers] type step = {}\n\
         context bump: Int\n\
         #[def_type] #[step]\n\
         def build(name: String, f: fn() -> {v: Int}) -> {v: Int} { f() }\n\
         def build main() { v = 1 }\n\
         #[def_type]\n\
         def bumped(name: String, f: fn(): Int) -> Int { f() + ^bump }\n\
         def bumped one() { 1 }",
    ),
];

#[test]
fn one_call_two_targets_make_is_one_node_met_where_each_made_it() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "app.gcsl",
            "#[attribute] #[defers] type step = {}\n\
             #[step] fn compile(src: String) -> String { \"{src}.o\" }\n\
             #[def_type]\n\
             def build(name: String, f: fn() -> String) -> String { f() }\n\
             def build first() { = compile(\"w\") }\n\
             def build second() {\n\
             \x20   = compile(\"w\")\n\
             }",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let first = workspace
        .prepare_call(("build", "first"), &[], &[])
        .unwrap();
    let second = workspace
        .prepare_call(("build", "second"), &[], &[])
        .unwrap();
    let mut graph = Graph::new();
    for call in [&first, &second] {
        let progress = workspace.run_prepared(call, &mut graph, every()).unwrap();
        assert!(matches!(progress, Progress::Done(_)), "{progress:?}");
    }
    // The definition each is made in is no part of the call: one node.
    assert_eq!(graph.len(), 1);
    let node = NodeId::of(0);
    let (input, at_first) = graph.met_at(first.unit(), node).unwrap();
    let (other, at_second) = graph.met_at(second.unit(), node).unwrap();
    assert_eq!(input, other);
    assert_ne!(at_first, at_second);
    assert_eq!(graph.node(node).unwrap().span(), at_first);
    assert_eq!(graph.met_at(first.unit() + 7, node), None);
    // Named for a host before it runs, by itself, whoever is given it.
    let inputs = workspace.step_identity(&graph, node).unwrap();
    assert!(
        matches!(inputs.callee, Some(Plain::Closure { .. })),
        "{:?}",
        inputs.callee
    );
    assert_eq!(inputs.args, [Plain::Str("w".to_owned())]);
}

#[test]
fn a_step_s_result_names_the_steps_given_it_by_its_node() {
    let (mut workspace, file) = lone_workspace("let o = compile(\"w\")\nresult = link([o])");
    let mut graph = Graph::new();
    let progress = workspace.run_graph(file, &mut graph, every()).unwrap();
    assert!(matches!(progress, Progress::Done(_)), "{progress:?}");
    assert_eq!(graph.len(), 2);
    let link = NodeId::of(1);
    let named = |graph: &Graph| workspace.step_identity(graph, link).unwrap().args;
    assert_eq!(named(&graph), [Plain::List(vec![Plain::Step(0)])]);
    // Its inputs wait for it; its identity is the same once it is given.
    assert!(workspace.step_inputs(&graph, link).is_err());
    assert!(graph.resolve(NodeId::of(0), Ok(text("w.o"))));
    assert_eq!(named(&graph), [Plain::List(vec![Plain::Step(0)])]);
    assert_eq!(
        workspace.step_inputs(&graph, link).unwrap().args,
        [Plain::List(vec![Plain::Str("w.o".to_owned())])]
    );
}

#[test]
fn a_prepared_invocation_is_a_unit_that_runs_until_it_is_done() {
    let mut workspace = rooted(&TARGETS).unwrap();
    let call = workspace.prepare_call(("build", "main"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    // The definition is a step: its def-type's attribute defers it.
    let Progress::Done(value) = workspace.run_prepared(&call, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    let holes = graph.holes(&value);
    assert_eq!(holes.len(), 1);
    let step = graph.node(holes[0]).unwrap();
    assert_eq!(step.unit(), call.unit());
    let direct = crate::eval::Direct { temp_dirs: None };
    let outcome = workspace.evaluate(&graph, holes[0], &direct);
    assert_eq!(
        outcome.as_ref().map(ToString::to_string),
        Ok("{v = 1}".to_owned())
    );
    assert!(graph.resolve(holes[0], outcome));
    assert!(graph.holes(&value).is_empty());
    // Run again, it finds the same step, given.
    let again = workspace.run_prepared(&call, &mut graph, every()).unwrap();
    assert!(matches!(again, Progress::Done(_)));
    assert_eq!(graph.len(), 1);
}

#[test]
fn what_the_host_binds_at_the_root_meets_an_invocation_s_needs() {
    let mut workspace = rooted(&TARGETS).unwrap();
    let bump = vec![("bump".to_owned(), Value::Int(2))];
    let outcome = workspace
        .invoke_with(("bumped", "one"), &[], &bump)
        .unwrap();
    assert_eq!(outcome.value, Value::Int(3));
    // Without it, the call needs what nothing gives.
    let mut workspace = rooted(&TARGETS).unwrap();
    match workspace.invoke_with(("bumped", "one"), &[], &[]) {
        Err(InvokeError::Diagnostics(errors)) => {
            assert_eq!(errors[0].message, "context `bump` has no value here");
        }
        other => panic!("{other:?}"),
    }
}

/// Plain GCSL's effects, shareable between threads: no temporary
/// directories, which a job's test needs none of.
#[derive(Debug)]
struct Shareable;

impl crate::eval::Effects for Shareable {
    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

/// Runs `job` on a thread of its own, as a host's pool would.
fn on_a_thread(
    job: crate::eval::graph::EffectJob,
    deadline: Option<std::time::Instant>,
) -> Result<crate::eval::plain::Plain, Diagnostic> {
    std::thread::spawn(move || job.run(&Shareable, deadline))
        .join()
        .unwrap()
}

#[test]
fn a_step_runs_as_a_unit_whose_effects_are_jobs_on_any_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<crate::eval::graph::EffectJob>();
    let (mut workspace, file) = lone_workspace(
        "#[step] fn both() -> String {\n\
           let a = exec([\"echo\", \"one\"]).stdout.trim()\n\
           let b = exec([\"echo\", \"two\"]).stdout.trim()\n\
           \"{a} {b}\"\n\
         }\n\
         = both()",
    );
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    let step = graph.holes(&value)[0];
    // The step's body is a unit of its own; each `exec` a job.
    let inline = GraphHost {
        defers: &|_: &[crate::check::Attribute]| false,
        effects: &crate::eval::Direct { temp_dirs: None },
    };
    let mut jobs = 0;
    let result = loop {
        match workspace.run_step(&mut graph, step, inline).unwrap().0 {
            Progress::Paused(effect) => {
                let node = graph.node(effect).unwrap();
                assert_eq!(node.unit(), crate::eval::graph::STEP_UNITS + step.number());
                let job = workspace.effect_job(&graph, effect).unwrap();
                assert_eq!(job.builtin(), crate::check::Builtin::Exec);
                let plain = on_a_thread(job, None).unwrap();
                let value = workspace.from_plain(&plain);
                assert!(graph.resolve(effect, Ok(value)));
                jobs += 1;
            }
            Progress::Done(value) => break value,
        }
    };
    assert_eq!(jobs, 2);
    assert_eq!(result, Value::Str(Rc::from("one two")));
}

#[test]
fn a_job_past_its_deadline_is_killed_and_fails() {
    let job = crate::eval::graph::EffectJob::new(
        crate::check::Builtin::Exec,
        vec![
            crate::eval::plain::Plain::List(vec![
                crate::eval::plain::Plain::Str("sleep".to_owned()),
                crate::eval::plain::Plain::Str("5".to_owned()),
            ]),
            crate::eval::plain::Plain::Record(vec![
                (
                    "cwd".to_owned(),
                    crate::eval::plain::Plain::Str(String::new()),
                ),
                ("env".to_owned(), crate::eval::plain::Plain::Empty),
                (
                    "interactive".to_owned(),
                    crate::eval::plain::Plain::Bool(false),
                ),
                ("panic".to_owned(), crate::eval::plain::Plain::Bool(true)),
                (
                    "until".to_owned(),
                    crate::eval::plain::Plain::Str(String::new()),
                ),
            ]),
        ],
        Span::new(0, 0),
    );
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_millis(200);
    let error = on_a_thread(job, Some(deadline)).unwrap_err();
    assert_eq!(error.message, "the step timed out");
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
}

#[test]
fn a_sandbox_is_plain_data_a_step_s_body_passes_on() {
    let (mut workspace, _) = lone_workspace("= 1");
    let sandbox = Value::Sandbox(Rc::new(crate::value::Sandbox {
        root: Rc::from("/tmp/copy"),
        cwd: Rc::from("/work"),
        env: Box::new([(Rc::from("PATH"), Rc::from("/bin"))]),
    }));
    let plain = workspace.to_plain(&sandbox).unwrap();
    assert_eq!(
        plain,
        crate::eval::plain::Plain::Sandbox {
            root: "/tmp/copy".to_owned(),
            cwd: "/work".to_owned(),
            env: vec![("PATH".to_owned(), "/bin".to_owned())],
        }
    );
    let back = workspace.from_plain(&plain);
    let (Value::Sandbox(back), Value::Sandbox(original)) = (&back, &sandbox) else {
        panic!("not a sandbox");
    };
    assert_eq!(
        (&back.root, &back.cwd, &back.env),
        (&original.root, &original.cwd, &original.env)
    );
}

#[test]
fn a_step_s_unit_forgotten_runs_afresh_with_new_effects() {
    let (mut workspace, file) =
        lone_workspace("#[step] fn s() -> String { exec([\"echo\", \"x\"]).stdout }\n= s()");
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    let step = graph.holes(&value)[0];
    let inline = GraphHost {
        defers: &|_: &[crate::check::Attribute]| false,
        effects: &crate::eval::Direct { temp_dirs: None },
    };
    let Progress::Paused(first) = workspace.run_step(&mut graph, step, inline).unwrap().0 else {
        panic!("not paused");
    };
    let failure = Diagnostic::error("echo crashed", Span::new(0, 0));
    assert!(graph.resolve(first, Err(failure)));
    assert!(workspace.run_step(&mut graph, step, inline).is_err());
    // Retried afresh: a new effect, not the failed one.
    graph.forget_unit(crate::eval::graph::STEP_UNITS + step.number());
    let Progress::Paused(second) = workspace.run_step(&mut graph, step, inline).unwrap().0 else {
        panic!("not paused");
    };
    assert_ne!(first, second);
}

#[test]
fn a_failed_step_a_value_holds_is_found() {
    let (mut workspace, file) = lone_workspace("#[step] fn s() -> Int { 1 }\n= [s()]");
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    let step = graph.holes(&value)[0];
    assert!(graph.failures(&value).is_empty());
    let failure = Diagnostic::error("it broke", Span::new(0, 0));
    assert!(graph.resolve(step, Err(failure)));
    assert!(graph.holes(&value).is_empty());
    assert_eq!(graph.failures(&value)[0].message, "it broke");
}

#[test]
fn a_step_s_failure_is_in_the_file_of_the_code_that_raised_it() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib.gcsl",
            "#[attribute] #[defers] type step = {}\n\
             #[def_type] #[step]\n\
             def build(name: String, f: fn() -> {v: Int}) -> {v: Int} { f() }\n\
             def boom() -> {v: Int} { panic(\"boom in lib\") }",
        ),
        ("app.gcsl", "def build main() { = boom() }"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let lib = workspace.file("lib.gcsl").unwrap();
    let call = workspace.prepare_call(("build", "main"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_prepared(&call, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    let step = graph.holes(&value)[0];
    let inline = GraphHost {
        defers: &|_: &[crate::check::Attribute]| false,
        effects: &crate::eval::Direct { temp_dirs: None },
    };
    let error = workspace.run_step(&mut graph, step, inline).unwrap_err();
    assert_eq!(error.message, "boom in lib");
    // In the file of the code that raised it, not the step's own.
    assert_eq!(error.input, Some(lib));
    let text = files[1].1;
    let at = &text[error.span.start as usize..error.span.end as usize];
    assert_eq!(at, "panic(\"boom in lib\")");
}

#[test]
fn a_graph_s_nodes_are_listed_in_the_order_they_were_made() {
    let (mut workspace, file) = lone_workspace("= [link([compile(\"b\")]), compile(\"a\")]");
    let mut graph = Graph::new();
    workspace.run_graph(file, &mut graph, every()).unwrap();
    let listed: Vec<(u32, NodeKind)> = graph
        .nodes()
        .map(|(id, node)| (id.number(), node.kind()))
        .collect();
    assert_eq!(listed.len(), graph.len());
    assert!(
        listed
            .iter()
            .enumerate()
            .all(|(at, (number, _))| *number as usize == at)
    );
}

#[test]
fn a_root_context_is_set_from_a_field_its_target_never_reads() {
    let files = [
        (
            "workspace.gcsl",
            "context label: String = \"default\"\nconf = {label = \"from conf\"; n = 1}",
        ),
        ("app/build.gcsl", "def build x() { = \"{^label}!\" }"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let root = workspace.file("workspace.gcsl").unwrap();
    let field = crate::workspace::graph::RootField {
        context: "label",
        file: root,
        field: "conf",
        path: &["label"],
    };
    let call = workspace
        .prepare_call_with(("build", "x"), &[], &[], &[field])
        .unwrap();
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_prepared(&call, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    assert_eq!(value.to_string(), "from conf!");
    // What a host checks first: whether the field's path is there.
    assert!(workspace.top_level_has(root, "conf", &["label"]));
    assert!(workspace.top_level_has(root, "conf", &[]));
    assert!(!workspace.top_level_has(root, "conf", &["keys"]));
    assert!(!workspace.top_level_has(root, "conf", &["label", "deeper"]));
    assert!(!workspace.top_level_has(root, "nope", &[]));
    // A field of another type is the `with`'s own error.
    let wrong = crate::workspace::graph::RootField {
        path: &["n"],
        ..field
    };
    match workspace.prepare_call_with(("build", "x"), &[], &[], &[wrong]) {
        Err(InvokeError::Diagnostics(errors)) => {
            assert!(
                errors[0].message.starts_with("mismatched types"),
                "{errors:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    // A field there is not.
    let missing = crate::workspace::graph::RootField {
        field: "nope",
        ..field
    };
    match workspace.prepare_call_with(("build", "x"), &[], &[], &[missing]) {
        Err(InvokeError::Request(error)) => {
            assert_eq!(
                error.message(),
                "`workspace.gcsl` has no top-level field `nope`"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_prepared_call_can_be_given_to_a_top_level_function() {
    let files = [
        (
            "workspace.gcsl",
            "context factor: Int = 1\nconf = {factor = 3}\n\
             def scaled(n: Int, ^factor) -> Int { n * factor }\n\
             def offset(n: Int, by: Int) -> Int { n + by }",
        ),
        ("app/build.gcsl", "def build x() { = 7 }"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let root = workspace.file("workspace.gcsl").unwrap();
    let field = crate::workspace::graph::RootField {
        context: "factor",
        file: root,
        field: "conf",
        path: &["factor"],
    };
    let wrap = crate::workspace::graph::RootWrap {
        file: root,
        function: "scaled",
        args: &[],
    };
    // `scaled(build x())`, in the root contexts too.
    let call = workspace
        .prepare_call_wrapped(("build", "x"), &[], &[], (&[field], Some(wrap)))
        .unwrap();
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_prepared(&call, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    assert_eq!(value, Value::Int(21));
    // Given arguments after the invocation's value.
    let given = [crate::workspace::Given::Int(5)];
    let offset = crate::workspace::graph::RootWrap {
        function: "offset",
        args: &given,
        ..wrap
    };
    let call = workspace
        .prepare_call_wrapped(("build", "x"), &[], &[], (&[], Some(offset)))
        .unwrap();
    let Progress::Done(value) = workspace.run_prepared(&call, &mut graph, every()).unwrap() else {
        panic!("not done");
    };
    assert_eq!(value, Value::Int(12));
    // A function there is not.
    let missing = crate::workspace::graph::RootWrap {
        function: "nope",
        ..wrap
    };
    match workspace.prepare_call_wrapped(("build", "x"), &[], &[], (&[], Some(missing))) {
        Err(InvokeError::Request(error)) => {
            assert_eq!(
                error.message(),
                "`workspace.gcsl` has no top-level field `nope`"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// The contexts `inputs` holds, by name, each declaration's key's text
/// `<context NAME@...>`.
fn context_names(inputs: &crate::workspace::StepInputs) -> Vec<String> {
    inputs
        .context
        .iter()
        .map(|(key, _)| {
            let name = key.trim_start_matches("<context ");
            name.split('@').next().unwrap_or(name).to_owned()
        })
        .collect()
}

#[test]
fn a_step_is_keyed_by_no_context_with_no_default_that_it_does_not_need() {
    let (mut workspace, file) = lone_workspace(
        "context arch: String\n\
         context session: String\n\
         context level: Int = 1\n\
         fn flags(): String { \"-m{^arch}\" }\n\
         #[step] fn cc(src: String) -> String { \"{src} {flags()}\" }\n\
         #[step] fn plain(src: String) -> String { src }\n\
         #[step] fn quiet(src: String) -> String { cc(src) with { arch = \"x86\" } }\n\
         #[step] fn rec(n: Int) -> String { if n == 0 { ^arch } else { rec(n - 1) } }\n\
         fn odd(n: Int) -> String { if n == 0 { ^arch } else { even(n - 1) } }\n\
         #[step] fn even(n: Int) -> String { if n == 0 { \"e\" } else { odd(n - 1) } }\n\
         = [cc(\"a\"), plain(\"b\"), quiet(\"c\"), rec(1), even(1)] \
         with { arch = \"arm\"; session = \"s\"; level = 2 }",
    );
    let mut graph = Graph::new();
    workspace.run_graph(file, &mut graph, every()).unwrap();
    // `cc` needs `arch`, through `flags`; `level` has a default, so any
    // step can read it; nothing reads `session`.
    let cc = workspace.step_inputs(&graph, NodeId::of(0)).unwrap();
    assert_eq!(context_names(&cc), ["arch", "level"]);
    let plain = workspace.step_inputs(&graph, NodeId::of(1)).unwrap();
    assert_eq!(context_names(&plain), ["level"]);
    // `quiet` binds `arch` around what it calls, so needs nothing.
    let quiet = workspace.step_inputs(&graph, NodeId::of(2)).unwrap();
    assert_eq!(context_names(&quiet), ["level"]);
    // A step's needs are its own once its group of recursion is checked,
    // whether it calls itself or a function that calls it.
    for node in [3, 4] {
        let inputs = workspace.step_inputs(&graph, NodeId::of(node)).unwrap();
        assert_eq!(context_names(&inputs), ["arch", "level"], "{node}");
    }
}

#[test]
fn a_step_s_context_of_functions_is_keyed_by_their_code() {
    let (mut workspace, file) = lone_workspace(
        "context helpers: List<fn(): Int> = []\n\
         fn one(): Int { 1 }\n\
         = compile(\"a\") with { helpers = [one] }",
    );
    let mut graph = Graph::new();
    workspace.run_graph(file, &mut graph, every()).unwrap();
    let inputs = workspace.step_inputs(&graph, NodeId::of(0)).unwrap();
    let helpers = inputs
        .context
        .iter()
        .find(|(key, _)| key.starts_with("<context helpers@"))
        .map(|(_, value)| value.clone());
    let Some(Plain::List(items)) = helpers else {
        panic!("{:?}", inputs.context);
    };
    let [
        Plain::Closure {
            input, captures, ..
        },
    ] = &items[..]
    else {
        panic!("{items:?}");
    };
    assert_eq!(*input, file);
    assert!(captures.is_empty());
    // Its argument is still data alone.
    assert_eq!(inputs.args, [Plain::Str("a".to_owned())]);
}

#[test]
fn a_step_knows_the_definition_whose_call_made_it() {
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] #[defers] type step = {}\n\
             #[step] def compile(src: String) -> String { \"{src}.o\" }",
        ),
        (
            "app/build.gcsl",
            "def build lib() { = compile(\"lib\") }\n\
             def build app() {\n\
             \x20   let l = build lib()\n\
             \x20   = [l, compile(\"main\")]\n\
             }\n\
             def build other() { = compile(\"main\") }",
        ),
    ];
    let mut workspace = crate::workspace::tests::fileroot::rooted(&files).unwrap();
    let call = workspace.prepare_call(("build", "app"), &[], &[]).unwrap();
    let other = workspace
        .prepare_call(("build", "other"), &[], &[])
        .unwrap();
    let mut graph = Graph::new();
    workspace.run_prepared(&call, &mut graph, every()).unwrap();
    workspace.run_prepared(&other, &mut graph, every()).unwrap();
    let of = |src: &str| {
        let mut found: Vec<String> = graph
            .nodes()
            .filter(|(_, node)| node.args() == [text(src)])
            .filter_map(|(id, _)| workspace.step_definition(&graph, id))
            .map(|definition| definition.qualified())
            .collect();
        found.sort();
        found
    };
    // `lib`'s, made in the call of `build lib`, wherever it is called from;
    // and the prepared calls' own one call, which they share, the first's
    // that made it: the definition is no part of a call.
    assert_eq!(of("lib"), ["app/lib"]);
    assert_eq!(of("main"), ["app/app"]);
    // Nor does the definition key the step.
    let (id, _) = graph
        .nodes()
        .find(|(_, node)| node.args() == [text("lib")])
        .unwrap();
    let inputs = workspace.step_inputs(&graph, id).unwrap();
    assert!(
        inputs.context.iter().all(|(key, _)| key != "<definition>"),
        "{:?}",
        inputs.context
    );
    // Its code's files, known before it runs: `compile`'s, and what that
    // file needs; and the call's own, which run before it.
    let root = workspace.file("workspace.gcsl").unwrap();
    let app = workspace.file("app/build.gcsl").unwrap();
    let code = workspace.node_code_files(&graph, id);
    assert!(code.contains(&root) && !code.contains(&app), "{code:?}");
    assert!(call.files().contains(&app), "{:?}", call.files());
    let needed = workspace.files_needed(app);
    assert!(
        needed.contains(&app) && needed.contains(&root),
        "{needed:?}"
    );
}

#[test]
fn a_step_knows_a_definition_made_with_a_variant() {
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] #[defers] type step = {}\n\
             #[step] def compile(src: String) -> String { \"{src}.o\" }\n\
             enum Tool extensible<{*: String}>\n\
             enum Tool::C<{cc: String}>\n\
             #[def_type]\n\
             def Tool<T>(name: String, kind: Variant<Tool, T>, f: fn() -> T) -> Tool { kind(f()) }\n\
             def Tool::C gcc() { cc = compile(\"gcc\") }",
        ),
        ("app/build.gcsl", "def build app() { = (Tool::C gcc)() }"),
    ];
    let mut workspace = crate::workspace::tests::fileroot::rooted(&files).unwrap();
    let call = workspace.prepare_call(("build", "app"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    workspace.run_prepared(&call, &mut graph, every()).unwrap();
    let (id, _) = graph
        .nodes()
        .find(|(_, node)| node.args() == [text("gcc")])
        .unwrap();
    // Nothing invokes it by name, so its def-type lists none, but its
    // reference's call names it all the same, by its variant.
    let definition = workspace.step_definition(&graph, id).unwrap();
    assert_eq!(definition.def_type, "Tool::C");
    assert_eq!(definition.path, "gcc");
    assert_eq!(definition.dir, "");
}

/// Effects as plain GCSL's, recording each context a step's run tells it
/// it read.
#[derive(Debug, Default)]
struct Told {
    read: std::cell::RefCell<Vec<String>>,
}

impl crate::eval::Effects for Told {
    fn context_read(&self, key: &str) {
        self.read.borrow_mut().push(key.to_owned());
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

/// The context's name in its key's text, `<context NAME@...>`.
fn context_name(key: &str) -> String {
    let name = key.trim_start_matches("<context ");
    name.split('@').next().unwrap_or(name).to_owned()
}

#[test]
fn a_step_s_run_tells_what_it_read_of_its_context() {
    let (mut workspace, file) = lone_workspace(
        "context level: Int = 1\n\
         context arch: String = \"x86\"\n\
         context quiet: Int = 0\n\
         fn label(n: Int) ->> String { \"{n}-{^arch}\" }\n\
         #[step] fn reads(n: Int) -> String {\n\
         \x20   let shadowed = ^quiet with { quiet = 2 }\n\
         \x20   = \"{n} {^level} {label(1)} {shadowed}\"\n\
         }\n\
         #[step] fn none(n: Int) -> Int { n }\n\
         = [label(1), reads(1), none(2), reads(2)] with { level = 3 }",
    );
    let mut graph = Graph::new();
    workspace.run_graph(file, &mut graph, every()).unwrap();
    let told = Told::default();
    let host = GraphHost {
        defers: &|_: &[crate::check::Attribute]| false,
        effects: &told,
    };
    let reads_of = |workspace: &mut Workspace, graph: &mut Graph, node: u32| {
        told.read.borrow_mut().clear();
        let (progress, reads) = workspace.run_step(graph, NodeId::of(node), host).unwrap();
        assert!(matches!(progress, Progress::Done(_)), "{progress:?}");
        let shown: Vec<(String, bool)> = reads
            .keys
            .iter()
            .map(|(key, bound)| (context_name(key), *bound))
            .collect();
        let named: Vec<String> = told
            .read
            .borrow()
            .iter()
            .map(|key| context_name(key))
            .collect();
        (shown, named)
    };
    // `level`, bound; `arch`, its default, read by `label(1)`, which the
    // file's own run computed, so found in its memo table, the hit
    // replaying its read; not `quiet`, which the step binds itself. Each
    // told the host as it was first read.
    let (reads, told_of) = reads_of(&mut workspace, &mut graph, 0);
    assert_eq!(
        reads,
        [("level".to_owned(), true), ("arch".to_owned(), false)]
    );
    assert_eq!(told_of, ["level", "arch"]);
    // A step that reads nothing.
    assert_eq!(reads_of(&mut workspace, &mut graph, 1).0, []);
}

#[test]
fn a_lazy_made_in_a_memoised_call_and_awaited_after_converges() {
    let (mut workspace, file) = lone_workspace(
        "fn made(x: String) ->> {name: String, later: Future<String>} {\n\
         \x20   name = x\n\
         \x20   later = lazy { exec([\"echo\", x]).stdout }\n\
         }\n\
         fn get() -> String {\n\
         \x20   let entry = made(\"a\")\n\
         \x20   = await entry.later\n\
         }\n\
         = get()",
    );
    let mut graph = Graph::new();
    let result = loop {
        match workspace.run_graph(file, &mut graph, every()).unwrap() {
            Progress::Paused(effect) => {
                let job = workspace.effect_job(&graph, effect).unwrap();
                let plain = on_a_thread(job, None).unwrap();
                let value = workspace.from_plain(&plain);
                assert!(graph.resolve(effect, Ok(value)));
            }
            Progress::Done(value) => break value,
        }
    };
    assert_eq!(result, text("a\n"));
}

#[test]
fn a_registered_toolchain_s_definition_converges_in_graph_mode() {
    let (mut workspace, file) = lone_workspace(
        "import toolchain\n\
         import Toolchain from toolchain\n\
         let on = {arch = \"x86_64\"; os = \"linux\"}\n\
         = with { host_platform = on; platform = on } { toolchain.get(Toolchain::C).cc.path }",
    );
    let mut graph = Graph::new();
    let mut attempts = 0;
    let result = loop {
        attempts += 1;
        assert!(attempts < 10);
        match workspace.run_graph(file, &mut graph, every()) {
            Ok(Progress::Paused(effect)) => {
                let job = workspace.effect_job(&graph, effect).unwrap();
                let plain = on_a_thread(job, None).unwrap();
                let value = workspace.from_plain(&plain);
                assert!(graph.resolve(effect, Ok(value)));
            }
            Ok(Progress::Done(value)) => break value,
            Err(errors) => panic!("{errors:?}"),
        }
    };
    assert!(matches!(result, Value::Str(_)), "{result:?}");
}

#[test]
fn a_definition_s_body_is_its_own_wherever_it_runs() {
    // A def-type that runs its definition later, in a closure it gives, as
    // the toolchain registry does: a node the definition's body makes is
    // the definition's, though another definition runs the closure.
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] #[defers] type step = {}\n\
             #[step] def compile(src: String) -> String { \"{src}.o\" }\n\
             #[def_type] def later(name: String, f: fn() -> String) ->> {run: fn() ->> String} {\n\
             \x20   run = fn() ->> String { f() }\n\
             }\n\
             def later tool() { = compile(\"tool\") }",
        ),
        ("app/build.gcsl", "def build app() { = later tool().run() }"),
    ];
    let mut workspace = crate::workspace::tests::fileroot::rooted(&files).unwrap();
    let call = workspace.prepare_call(("build", "app"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    workspace.run_prepared(&call, &mut graph, every()).unwrap();
    let found: Vec<String> = graph
        .nodes()
        .filter(|(_, node)| node.args() == [text("tool")])
        .filter_map(|(id, _)| workspace.step_definition(&graph, id))
        .map(|definition| definition.qualified())
        .collect();
    assert_eq!(found, ["/tool"]);
}
