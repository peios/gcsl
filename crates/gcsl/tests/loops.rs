//! A host's view of a loop of operations in graph mode: one unit, run in
//! slices, one operation answered at a time, as a test harness runs a test.
//! However its body is written, a loop of N operations is answered in N
//! turns of the unit, and the work of each turn does not grow with N.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gcsl::check::Attribute;
use gcsl::eval::Effects;
use gcsl::eval::graph::{Graph, GraphHost};
use gcsl::eval::plain::Plain;
use gcsl::workspace::{InvokeError, Progress, Turn, Workspace, check_lone_with_standard};
use gcsl::{Diagnostic, SourceFile};

/// A counter the host keeps, and tests that call it in loops written each
/// way a test may write one.
const PROGRAM: &str = r#"
#[def_type]
def test(name: String, body: fn() -> {}) -> {} { body() }

extern type Counter = "demo.counter"
extern fn counter() -> Counter = "demo.counter"
extern fn (c: Counter) add(n: Int) -> Int = "demo.add"

fn step(name: String, body: block -> {}) -> {} { body() }

def test bound() {
    let c = counter()
    for i in 0..COUNT {
        let n = c.add(1)
    }
}

def test assigned() {
    let c = counter()
    let mut last = 0
    for i in 0..COUNT {
        last := c.add(1)
    }
}

def test fails() {
    let c = counter()
    for i in 0..COUNT {
        let n = c.add(1)
        assert n != 3, "the third failed"
    }
}

def test stepped() {
    let c = counter()
    step("loop") {
        for i in 0..COUNT {
            let n = c.add(1)
        }
    }
}
"#;

/// The host: the counter's value.
#[derive(Debug, Default)]
struct Host {
    total: Cell<i64>,
}

impl Effects for Host {
    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

impl Host {
    /// Answers the operation `name`, given `args`.
    fn perform(&self, name: &str) -> Plain {
        match name {
            "demo.counter" => Plain::handle("demo.counter", 1),
            "demo.add" => {
                self.total.set(self.total.get() + 1);
                Plain::Int(self.total.get())
            }
            other => panic!("no operation `{other}`"),
        }
    }
}

/// Runs the test `name`, its loops of `count` iterations, to its end: how many
/// operations it asked for and how long it took.
fn run(name: &str, count: i64) -> (i64, Duration) {
    let (asked, took, failure) = run_to_end(name, count);
    if let Some(failure) = failure {
        panic!("{name}: {failure}");
    }
    (asked, took)
}

/// [`run`], and the message of the failure the test ended with, if it
/// failed.
fn run_to_end(name: &str, count: i64) -> (i64, Duration, Option<String>) {
    let text = PROGRAM.replace("COUNT", &count.to_string());
    let source = SourceFile::new("tests.gcsl", text).unwrap();
    let (_, checked) = check_lone_with_standard(&source, "/");
    let mut workspace: Workspace = checked.unwrap_or_else(|errors| panic!("{errors:?}"));
    let never = |_: &[Attribute]| false;
    let host = Host::default();
    let call = workspace.prepare_call(("test", name), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let started = Instant::now();
    let mut asked = 0;
    loop {
        let graph_host = GraphHost {
            defers: &never,
            effects: &host,
        };
        match workspace.run_prepared_for(&call, &mut graph, graph_host, Duration::from_millis(2)) {
            Ok(Turn::Yielded) => {}
            Ok(Turn::Progress(Progress::Done(_))) => break,
            Ok(Turn::Progress(Progress::Paused(nodes))) => {
                assert_eq!(nodes.len(), 1, "operations come one at a time");
                let operation = workspace.operation(&graph, nodes[0]).unwrap();
                let answer: Result<_, Diagnostic> =
                    Ok(workspace.from_plain(&host.perform(&operation.name)));
                assert!(graph.resolve(nodes[0], answer));
                asked += 1;
            }
            Ok(Turn::Progress(Progress::Waiting(nodes))) => {
                panic!("{name}: waiting on {nodes:?} with nothing in flight")
            }
            Err(InvokeError::Diagnostics(errors)) => {
                return (asked, started.elapsed(), Some(errors[0].message.clone()));
            }
            Err(InvokeError::Request(error)) => panic!("{name}: {error:?}"),
        }
    }
    (asked, started.elapsed(), None)
}

#[test]
fn a_loop_of_operations_asks_for_each_once_in_order() {
    for name in ["assigned", "bound", "stepped"] {
        let (asked, _) = run(name, 200);
        assert_eq!(asked, 201, "{name}");
    }
}

#[test]
fn a_failed_iteration_ends_the_loop_and_no_later_one_is_asked_for() {
    // The iterations after the third wait their turn behind it; its
    // failure is the test's, and none of them reaches the host.
    let (asked, _, failure) = run_to_end("fails", 50);
    assert_eq!(
        failure.as_deref(),
        Some("assertion failed: the third failed")
    );
    assert_eq!(asked, 4);
}

/// The timings of each loop at 1,000, 2,000 and 4,000 iterations, printed:
/// run with `cargo test --release --test loops -- --ignored --nocapture`.
#[test]
#[ignore = "a timing: run explicitly"]
fn loops_of_operations_are_timed() {
    for name in ["assigned", "bound", "stepped"] {
        for count in [1_000, 2_000, 4_000] {
            let (asked, took) = run(name, count);
            assert_eq!(asked, count + 1);
            println!(
                "{name:>8} {count:>5}: {:>9.1} ms",
                took.as_secs_f64() * 1000.0
            );
        }
    }
}

/// One loop, `$LOOP` (`bound` by default) of `$COUNT` iterations (2,000 by
/// default), timed and printed: for a profiler.
#[test]
#[ignore = "a timing: run explicitly"]
fn one_loop_is_timed() {
    let name = std::env::var("LOOP").unwrap_or_else(|_| "bound".to_owned());
    let count = std::env::var("COUNT").map_or(2_000, |count| count.parse().unwrap());
    let (asked, took) = run(&name, count);
    assert_eq!(asked, count + 1);
    println!(
        "{name:>8} {count:>5}: {:>9.1} ms",
        took.as_secs_f64() * 1000.0
    );
}
