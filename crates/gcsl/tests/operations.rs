//! A host of operations, written against the public API alone, as an
//! embedder writes one (see the Learn page "Providing Host Operations"):
//! in graph mode, answering each operation a unit stops at, and as an
//! effect context, for a step it runs as plain GCSL.

use std::collections::VecDeque;
use std::sync::Mutex;

use gcsl::check::Attribute;
use gcsl::eval::graph::{Graph, GraphHost, NodeId};
use gcsl::eval::plain::Plain;
use gcsl::eval::{Direct, Effects, OperationFailure};
use gcsl::workspace::{InvokeError, Progress, Workspace, check_lone_with_standard};
use gcsl::{Diagnostic, SourceFile, Span};

/// A program of tests, each a definition of the def-type `test`, over the
/// operations of a host that runs machines.
const PROGRAM: &str = r#"
#[def_type]
def test(name: String, body: fn() -> String) -> String { body() }

extern type Vm = "vm.machine"

extern fn boot(name: String, cpus: Int = 1) -> Vm = "vm.boot"
extern fn (m: Vm) run(...argv: String) -> {status: Int, stdout: String} = "vm.run"
extern fn (m: Vm) halt() -> {}

def test hostname() {
    let m = boot("a")
    let ran = m.run("hostname")
    m.halt()
    = "{ran.stdout} ({ran.status})"
}

def test gone() {
    let m = boot("b", cpus = 2)
    = recover { m.run("crash").stdout } else (failure) { "failed: {failure.message}" }
}

#[attribute] #[defers] type step = {}

#[step]
fn whole(name: String) -> String { boot(name).run("uname").stdout }

def test stepped() { = "ran {whole("c")}" }
"#;

/// The machines a host has booted, by number, each with its name.
#[derive(Debug, Default)]
struct Machines {
    booted: Mutex<Vec<String>>,
}

impl Machines {
    /// Performs the operation `name` given `args`.
    fn perform(&self, name: &str, args: &[Plain]) -> Result<Plain, OperationFailure> {
        match (name, args) {
            ("vm.boot", [Plain::Str(machine), Plain::Int(_)]) => {
                let mut booted = self.booted.lock().unwrap();
                booted.push(machine.clone());
                Ok(Plain::handle(
                    "vm.machine",
                    i64::try_from(booted.len()).unwrap(),
                ))
            }
            ("vm.run", [machine, Plain::List(words)]) => {
                let Some(("vm.machine", id)) = machine.as_handle() else {
                    return Err(OperationFailure::new("`vm.run` needs a machine"));
                };
                let booted = self.booted.lock().unwrap();
                let name = &booted[usize::try_from(id).unwrap() - 1];
                if words.first() == Some(&Plain::Str("crash".to_owned())) {
                    return Err(OperationFailure::new("the guest is gone")
                        .with_note(format!("machine `{name}` stopped answering")));
                }
                Ok(Plain::Record(vec![
                    ("status".to_owned(), Plain::Int(0)),
                    (
                        "stdout".to_owned(),
                        Plain::Str(format!("{name}: {words:?}")),
                    ),
                ]))
            }
            ("halt", [machine]) if machine.as_handle().is_some() => Ok(Plain::Empty),
            _ => Err(OperationFailure::new(format!(
                "this host has no operation `{name}`"
            ))),
        }
    }
}

impl Effects for Machines {
    fn operation(&self, name: &str, args: &[Plain]) -> Result<Plain, OperationFailure> {
        self.perform(name, args)
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

/// An effect context that leaves operations to the default: it has none.
#[derive(Debug)]
struct Bare;

impl Effects for Bare {
    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

/// The program, checked.
fn workspace() -> Workspace {
    let source = SourceFile::new("tests.gcsl", PROGRAM).unwrap();
    let (_, checked) = check_lone_with_standard(&source, "/");
    checked.unwrap_or_else(|errors| panic!("{errors:?}"))
}

/// Answers the node `node` of `graph`: an operation's, by `machines`, with
/// where the program called it noted in `log`.
fn answer(
    workspace: &mut Workspace,
    graph: &mut Graph,
    node: NodeId,
    machines: &Machines,
    log: &mut Vec<String>,
) {
    let operation = workspace.operation(graph, node).expect("an operation");
    let called = &PROGRAM[operation.span.start as usize..operation.span.end as usize];
    log.push(format!("{} at `{called}`", operation.name));
    let outcome = match machines.perform(&operation.name, &operation.args) {
        Ok(plain) => Ok(workspace.from_plain(&plain)),
        Err(failure) => {
            let mut error = Diagnostic::error(failure.message, Span::new(0, 0));
            error.notes = failure.notes;
            Err(error)
        }
    };
    assert!(graph.resolve(node, outcome));
}

#[test]
fn a_host_answers_each_operation_a_unit_stops_at() {
    let mut workspace = workspace();
    let machines = Machines::default();
    let never = |_: &[Attribute]| false;
    let host = || GraphHost {
        defers: &never,
        effects: &Direct { temp_dirs: None },
    };
    let names = ["hostname", "gone"];
    let calls: Vec<_> = names
        .iter()
        .map(|name| workspace.prepare_call(("test", name), &[], &[]).unwrap())
        .collect();
    let mut graph = Graph::new();
    let mut results = vec![String::new(); calls.len()];
    let mut log = Vec::new();
    let mut runnable: VecDeque<usize> = (0..calls.len()).collect();
    while let Some(at) = runnable.pop_front() {
        match workspace.run_prepared(&calls[at], &mut graph, host()) {
            Ok(Progress::Done(value)) => results[at] = value.to_string(),
            Ok(Progress::Paused(nodes)) => {
                for node in nodes {
                    answer(&mut workspace, &mut graph, node, &machines, &mut log);
                }
                runnable.push_back(at);
            }
            Ok(Progress::Waiting(_)) => runnable.push_back(at),
            Err(InvokeError::Diagnostics(errors)) => panic!("{errors:?}"),
            Err(InvokeError::Request(error)) => panic!("{error:?}"),
        }
    }
    assert_eq!(
        results,
        ["a: [Str(\"hostname\")] (0)", "failed: the guest is gone"]
    );
    // The two tests took turns, each in its own order.
    assert_eq!(
        log,
        [
            "vm.boot at `boot(\"a\")`",
            "vm.boot at `boot(\"b\", cpus = 2)`",
            "vm.run at `m.run(\"hostname\")`",
            "vm.run at `m.run(\"crash\")`",
            "halt at `m.halt()`",
        ]
    );
}

#[test]
fn a_step_run_as_plain_gcsl_asks_its_effect_context() {
    let mut workspace = workspace();
    let machines = Machines::default();
    let every = |_: &[Attribute]| true;
    let host = || GraphHost {
        defers: &every,
        effects: &Direct { temp_dirs: None },
    };
    let call = workspace
        .prepare_call(("test", "stepped"), &[], &[])
        .unwrap();
    let mut graph = Graph::new();
    let value = loop {
        match workspace.run_prepared(&call, &mut graph, host()) {
            Ok(Progress::Done(value)) => break value,
            // The step is no operation: its whole body is run here, as
            // plain GCSL, its operations through the effect context.
            Ok(Progress::Paused(nodes)) => {
                for node in nodes {
                    assert!(workspace.operation(&graph, node).is_none());
                    let outcome = workspace.evaluate(&graph, node, &machines);
                    assert!(graph.resolve(node, outcome));
                }
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(value.to_string(), "ran c: [Str(\"uname\")]");
    assert_eq!(*machines.booted.lock().unwrap(), ["c"]);
}

#[test]
fn a_host_with_no_operations_fails_them() {
    let mut workspace = workspace();
    let never = |_: &[Attribute]| false;
    let host = GraphHost {
        defers: &never,
        effects: &Direct { temp_dirs: None },
    };
    let call = workspace
        .prepare_call(("test", "hostname"), &[], &[])
        .unwrap();
    let mut graph = Graph::new();
    let Ok(Progress::Paused(nodes)) = workspace.run_prepared(&call, &mut graph, host) else {
        panic!("the unit did not stop at its operation");
    };
    // The node's job, run under a context that has none.
    let job = workspace.effect_job(&graph, nodes[0]).unwrap();
    let error = job.run(&Bare, None).unwrap_err();
    assert_eq!(error.message, "this host has no operation `vm.boot`");
    assert!(graph.resolve(nodes[0], Err(error)));
    let Err(InvokeError::Diagnostics(errors)) = workspace.run_prepared(&call, &mut graph, host)
    else {
        panic!("the unit did not fail");
    };
    let error = errors
        .iter()
        .find(|error| error.message.starts_with("this host"))
        .unwrap();
    // Reported where the test called the operation.
    assert_eq!(
        &PROGRAM[error.span.start as usize..error.span.end as usize],
        "boot(\"a\")"
    );
}
