//! Tests of workspaces: running units in graph mode, as a host that runs
//! their steps does (see [Graph mode](crate::eval::graph#graph-mode)).

use super::*;
use crate::eval::graph::{Graph, GraphHost, NodeId, NodeKind, NodeState};
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
         def build(name: String, f: fn(): {v: Int}) -> {v: Int} { f() }\n\
         def build main() { v = 1 }\n\
         #[def_type]\n\
         def bumped(name: String, f: fn(): Int) -> Int { f() + ^bump }\n\
         def bumped one() { 1 }",
    ),
];

#[test]
fn a_prepared_invocation_is_a_unit_that_runs_until_it_is_done() {
    let mut workspace = rooted(&TARGETS).unwrap();
    let call = workspace.prepare_call(("build", "main"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    // The invocation is a step: the def-type defers.
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
        match workspace.run_step(&mut graph, step, inline).unwrap() {
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
