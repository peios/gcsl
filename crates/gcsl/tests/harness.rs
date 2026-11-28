//! A test harness as a host of GCSL, written against the public API alone:
//! every test of a run is a unit of one evaluator in graph mode, each a
//! definition whose steps are blocks, over operations the host answers.
//! It uses together what such a host needs: block parameters, `try` and
//! `raise`, a parameter typed by its default, operations and handles,
//! slices, and the failures a program catches.

use std::cell::RefCell;
use std::time::Duration;

use gcsl::check::Attribute;
use gcsl::eval::graph::{Graph, GraphHost, NodeId};
use gcsl::eval::plain::Plain;
use gcsl::eval::{CaughtFailure, Effects};
use gcsl::workspace::graph::PreparedCall;
use gcsl::workspace::{InvokeError, Progress, Turn, Workspace, check_lone_with_standard};
use gcsl::{Diagnostic, SourceFile, Span};

/// A library of a harness, and four tests written with it.
const PROGRAM: &str = r#"
#[def_type]
def test(name: String, body: fn() -> {}) -> {} { body() }

extern type Vm = "vm.machine"

extern fn boot(profile: String) -> Vm = "vm.boot"
extern fn (m: Vm) run(...argv: String) -> {status: Int, stdout: String} = "vm.run"
extern fn begin(name: String) -> {} = "step.begin"
extern fn end(name: String, failure: String) -> {} = "step.end"

# A step: its block runs in the test's scope, and its failure is recorded
# and raised again as it was.
fn step(name: String, body: block -> {}) -> {} {
    begin(name)
    recover { body() } else (failure) {
        end(name, failure.message)
        raise
    }
    end(name, "")
}

# A fixture: built once, however many tests take it.
fn machine() ->> Vm { boot("peios") }

def test passes(m = machine()) {
    let mut pid = ""
    step("starts") {
        pid := m.run("start").stdout
    }
    step("is the same process") {
        assert m.run("pid").stdout == pid
    }
}

def test goes_on(m = machine(), wanted = "ready") {
    let mut seen = ""
    try step("fails") {
        seen := m.run("state").stdout
        assert seen == wanted, "the service is {seen}"
    }
    step("still runs") {
        m.run("after {seen}")
    }
}

def test stops(m = machine()) {
    step("crashes") { m.run("crash") }
    step("is never reached") { m.run("unreached") }
}

def test spins(m = machine()) {
    let mut n = 0
    step("counts") {
        for n < 300000 { n := n + 1 }
    }
    step("reports") { m.run("counted {n}") }
}
"#;

/// The host: the machines it booted, every operation it was asked for, by
/// its unit, and every failure a program caught.
#[derive(Debug, Default)]
struct Host {
    booted: RefCell<i64>,
    asked: RefCell<Vec<(usize, String)>>,
    caught: RefCell<Vec<(String, String, bool)>>,
}

impl Host {
    /// Performs the operation `name` given `args`, for the unit `unit`.
    fn perform(&self, unit: usize, name: &str, args: &[Plain]) -> Result<Plain, Diagnostic> {
        let words = |plain: &Plain| -> String {
            match plain {
                Plain::Str(text) => text.clone(),
                Plain::List(items) => items
                    .iter()
                    .map(|item| match item {
                        Plain::Str(text) => text.clone(),
                        other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                other => format!("{other:?}"),
            }
        };
        let shown: Vec<String> = args
            .iter()
            .filter(|arg| arg.as_handle().is_none())
            .map(words)
            .collect();
        self.asked.borrow_mut().push((
            unit,
            format!("{name} {}", shown.join(" / ")).trim().to_owned(),
        ));
        match (name, args) {
            ("vm.boot", [Plain::Str(_)]) => {
                *self.booted.borrow_mut() += 1;
                Ok(Plain::handle("vm.machine", *self.booted.borrow()))
            }
            ("vm.run", [machine, Plain::List(_)]) => {
                assert_eq!(machine.as_handle(), Some(("vm.machine", 1)));
                let said = match shown.first().map(String::as_str) {
                    Some("start" | "pid") => "4711".to_owned(),
                    Some("state") => "starting".to_owned(),
                    Some("crash") => {
                        return Err(Diagnostic::error("the guest is gone", Span::new(0, 0))
                            .with_note("machine 1 stopped answering"));
                    }
                    Some(other) => other.to_owned(),
                    None => String::new(),
                };
                Ok(Plain::Record(vec![
                    ("status".to_owned(), Plain::Int(0)),
                    ("stdout".to_owned(), Plain::Str(said)),
                ]))
            }
            ("step.begin" | "step.end", _) => Ok(Plain::Empty),
            _ => Err(Diagnostic::error(
                format!("this host has no operation `{name}`"),
                Span::new(0, 0),
            )),
        }
    }
}

impl Effects for Host {
    fn caught(&self, caught: &CaughtFailure<'_>) {
        let span = caught.failure.span;
        let at = PROGRAM
            .get(span.start as usize..span.end as usize)
            .unwrap_or("<elsewhere>");
        self.caught.borrow_mut().push((
            caught.failure.message.clone(),
            at.to_owned(),
            caught.again,
        ));
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

/// How a test ended.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed(String),
}

/// Runs the tests `names` together, as a scheduler would: each a unit, run
/// in slices of `slice`, one operation answered at a time, the units taken
/// in turn. Returns how each ended and how often any unit yielded.
fn run(host: &Host, names: &[&str], slice: Duration) -> (Vec<Outcome>, usize) {
    let source = SourceFile::new("tests.gcsl", PROGRAM).unwrap();
    let (_, checked) = check_lone_with_standard(&source, "/");
    let mut workspace: Workspace = checked.unwrap_or_else(|errors| panic!("{errors:?}"));
    let never = |_: &[Attribute]| false;
    let calls: Vec<PreparedCall> = names
        .iter()
        .map(|name| workspace.prepare_call(("test", name), &[], &[]).unwrap())
        .collect();
    let mut graph = Graph::new();
    let mut outcomes: Vec<Option<Outcome>> = names.iter().map(|_| None).collect();
    let mut pending: Vec<Option<NodeId>> = names.iter().map(|_| None).collect();
    let mut yields = 0;
    while outcomes.iter().any(Option::is_none) {
        for (unit, call) in calls.iter().enumerate() {
            if outcomes[unit].is_some() {
                continue;
            }
            // Answer what the unit stopped at, then run it on.
            if let Some(node) = pending[unit].take() {
                let operation = workspace.operation(&graph, node).expect("an operation");
                let answer = host
                    .perform(unit, &operation.name, &operation.args)
                    .map(|plain| workspace.from_plain(&plain));
                assert!(graph.resolve(node, answer));
            }
            let graph_host = GraphHost {
                defers: &never,
                effects: host,
            };
            match workspace.run_prepared_for(call, &mut graph, graph_host, slice) {
                Ok(Turn::Yielded) => yields += 1,
                Ok(Turn::Progress(Progress::Done(_))) => outcomes[unit] = Some(Outcome::Passed),
                Ok(Turn::Progress(Progress::Paused(nodes))) => {
                    assert_eq!(nodes.len(), 1, "operations come one at a time");
                    pending[unit] = Some(nodes[0]);
                }
                Ok(Turn::Progress(Progress::Waiting(_))) => {}
                Err(InvokeError::Diagnostics(errors)) => {
                    outcomes[unit] = Some(Outcome::Failed(errors[0].message.clone()));
                }
                Err(InvokeError::Request(error)) => panic!("{error:?}"),
            }
        }
    }
    (outcomes.into_iter().flatten().collect(), yields)
}

/// The operations the host was asked for by the unit `unit`, in order.
fn asked_by(host: &Host, unit: usize) -> Vec<String> {
    host.asked
        .borrow()
        .iter()
        .filter(|(by, _)| *by == unit)
        .map(|(_, asked)| asked.clone())
        .collect()
}

#[test]
fn tests_run_as_units_of_one_evaluator() {
    let host = Host::default();
    let names = ["passes", "goes_on", "stops", "spins"];
    let (outcomes, yields) = run(&host, &names, Duration::from_micros(200));
    assert_eq!(
        outcomes,
        [
            Outcome::Passed,
            // A tried step lets the test go on: the host, told of the
            // failure, is what fails it.
            Outcome::Passed,
            Outcome::Failed("the guest is gone".to_owned()),
            Outcome::Passed,
        ]
    );
    // The fixture was built once, by whichever test reached it first.
    assert_eq!(*host.booted.borrow(), 1);
    let boots = host
        .asked
        .borrow()
        .iter()
        .filter(|(_, asked)| asked.starts_with("vm.boot"))
        .count();
    assert_eq!(boots, 1);
    // Each test's operations came in its program's order, a step's state
    // carried to the next. (The fixture's boot is left out: one test made
    // it.)
    let own = |unit: usize| -> Vec<String> {
        asked_by(&host, unit)
            .into_iter()
            .filter(|asked| !asked.starts_with("vm.boot"))
            .collect()
    };
    assert_eq!(
        own(0),
        [
            "step.begin starts",
            "vm.run start",
            "step.end starts /",
            "step.begin is the same process",
            "vm.run pid",
            "step.end is the same process /",
        ]
    );
    assert_eq!(
        own(1),
        [
            "step.begin fails",
            "vm.run state",
            "step.end fails / assertion failed: the service is starting",
            "step.begin still runs",
            "vm.run after starting",
            "step.end still runs /",
        ]
    );
    // A failed step ends its test: the next step never began.
    assert_eq!(
        own(2),
        [
            "step.begin crashes",
            "vm.run crash",
            "step.end crashes / the guest is gone",
        ]
    );
    assert_eq!(
        own(3),
        [
            "step.begin counts",
            "step.end counts /",
            "step.begin reports",
            "vm.run counted 300000",
            "step.end reports /",
        ]
    );
    // The test that loops gave way to the others many times.
    assert!(yields > 3, "{yields} yields");
    // Every failure a program caught was told: the tried step's twice, the
    // second raised again, and the crashed step's once, in its step.
    let caught = host.caught.borrow();
    let tried: Vec<&(String, String, bool)> = caught
        .iter()
        .filter(|(message, _, _)| message == "assertion failed: the service is starting")
        .collect();
    assert_eq!(tried.len(), 2, "{caught:?}");
    assert_eq!((tried[0].2, tried[1].2), (false, true));
    assert_eq!(tried[0].1, "seen == wanted");
    let crashed: Vec<&(String, String, bool)> = caught
        .iter()
        .filter(|(message, _, _)| message == "the guest is gone")
        .collect();
    assert_eq!(crashed.len(), 1, "{caught:?}");
    assert_eq!(crashed[0].1, "m.run(\"crash\")");
    assert!(!crashed[0].2);
}

#[test]
fn the_outcome_does_not_depend_on_the_slice() {
    let whole = Host::default();
    let sliced = Host::default();
    let names = ["passes", "goes_on", "stops", "spins"];
    let (unsliced, none) = run(&whole, &names, Duration::from_secs(3600));
    let (in_slices, some) = run(&sliced, &names, Duration::ZERO);
    assert_eq!(unsliced, in_slices);
    assert_eq!(none, 0);
    assert!(some > 0);
    for (unit, name) in names.iter().enumerate() {
        let own = |host: &Host| -> Vec<String> {
            asked_by(host, unit)
                .into_iter()
                .filter(|asked| !asked.starts_with("vm.boot"))
                .collect()
        };
        assert_eq!(own(&whole), own(&sliced), "{name}");
    }
    assert_eq!(*whole.caught.borrow(), *sliced.caught.borrow());
}
