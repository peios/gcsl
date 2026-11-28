//! Tests of workspaces: units in graph mode that yield once their slice
//! is spent and carry on where they were (see
//! [Yielding](crate::eval::machine#yielding)), and a host's say over the
//! clocks of a stopped unit (see [Charged
//! time](crate::eval::machine::suspend#charged-time)).

use std::time::{Duration, Instant};

use super::fileroot::rooted;
use super::graph::every;
use super::*;
use crate::diagnostic::Severity;
use crate::eval::machine::set_yield_steps;
use crate::eval::plain::Plain;
use crate::workspace::graph::PreparedCall;

/// A def-type `test`, and an operation `op`, which is an `exec` the host
/// answers itself with its word.
const OPS: &str = "#[def_type]\n\
     def test(name: String, f: fn() -> String) -> String { f() }\n\
     fn op(word: String) -> String { exec([\"op\", word]).stdout }\n";

/// The workspace of one file, [`OPS`] and then `text`.
fn ops(text: &str) -> Workspace {
    let app = format!("{OPS}{text}");
    rooted(&[("workspace.gcsl", ""), ("app.gcsl", &app)]).unwrap()
}

/// The word of the effect `node` of `graph`, an `op`.
fn word(workspace: &Workspace, graph: &Graph, node: NodeId) -> String {
    let job = workspace.effect_job(graph, node).unwrap();
    let Plain::List(argv) = &job.args()[0] else {
        panic!("an `exec` without its program");
    };
    let Some(Plain::Str(word)) = argv.get(1) else {
        panic!("an `op` without its word");
    };
    word.clone()
}

/// Answers the effect `node` of `graph`, an `op`, with its word, and
/// returns the word.
fn answer(workspace: &mut Workspace, graph: &mut Graph, node: NodeId) -> String {
    let word = word(workspace, graph, node);
    let result = Plain::Record(vec![
        ("status".to_owned(), Plain::Int(0)),
        ("stdout".to_owned(), Plain::Str(word.clone())),
        ("stderr".to_owned(), Plain::Str(String::new())),
    ]);
    assert!(graph.resolve(node, Ok(workspace.from_plain(&result))));
    word
}

/// What a unit came to: its value shown, or its error's message.
fn outcome(result: Result<Value, InvokeError>) -> Result<String, String> {
    match result {
        Ok(value) => Ok(value.to_string()),
        Err(InvokeError::Diagnostics(errors)) => Err(errors
            .iter()
            .find(|error| error.severity == Severity::Error)
            .map_or_else(String::new, |error| {
                let mut said = error.message.clone();
                for note in &error.notes {
                    said.push_str("; ");
                    said.push_str(note);
                }
                said
            })),
        Err(InvokeError::Request(error)) => panic!("{error:?}"),
    }
}

/// What driving units together came to.
#[derive(Debug, PartialEq)]
struct Driven {
    /// What each unit came to.
    results: Vec<Result<String, String>>,
    /// Every operation, in the order the host was asked for it.
    asked: Vec<String>,
}

/// Runs the units of the definitions `names` of `test` together, taken in
/// turn, each run given the slice `slice` if there is one, one operation
/// answered at a time: what they came to, and how often each yielded.
fn drive(
    workspace: &mut Workspace,
    names: &[&str],
    slice: Option<Duration>,
) -> (Driven, Vec<usize>) {
    let calls: Vec<_> = names
        .iter()
        .map(|name| workspace.prepare_call(("test", name), &[], &[]).unwrap())
        .collect();
    let mut graph = Graph::new();
    let mut results: Vec<Option<Result<String, String>>> = vec![None; calls.len()];
    let mut yields = vec![0; calls.len()];
    let mut asked = Vec::new();
    while results.iter().any(Option::is_none) {
        for (at, call) in calls.iter().enumerate() {
            if results[at].is_some() {
                continue;
            }
            let turn = match slice {
                Some(slice) => workspace.run_prepared_for(call, &mut graph, every(), slice),
                None => workspace
                    .run_prepared(call, &mut graph, every())
                    .map(Turn::Progress),
            };
            match turn {
                Ok(Turn::Yielded) => yields[at] += 1,
                Ok(Turn::Progress(Progress::Done(value))) => {
                    results[at] = Some(outcome(Ok(value)));
                }
                Ok(Turn::Progress(Progress::Paused(nodes))) => {
                    // Operations are not isolated: one at a time, and none
                    // the host has answered already.
                    assert_eq!(nodes.len(), 1, "{nodes:?}");
                    asked.push(answer(workspace, &mut graph, nodes[0]));
                }
                Ok(Turn::Progress(Progress::Waiting(_))) => {}
                Err(error) => results[at] = Some(outcome(Err(error))),
            }
        }
    }
    let results = results.into_iter().flatten().collect();
    (Driven { results, asked }, yields)
}

/// Drives the units `names` of the workspace of `text` twice, once with
/// no slice and once yielding between every two steps of the machine: what
/// the second came to, which is what the first did, and how often each of
/// its units yielded.
fn at_every_step(text: &str, names: &[&str]) -> (Driven, Vec<usize>) {
    let (plain, none) = drive(&mut ops(text), names, None);
    assert!(none.iter().all(|&yields| yields == 0), "{none:?}");
    set_yield_steps(1);
    let (sliced, yields) = drive(&mut ops(text), names, Some(Duration::ZERO));
    set_yield_steps(1024);
    assert_eq!(sliced, plain);
    (sliced, yields)
}

#[test]
fn a_unit_that_only_computes_yields_and_another_finishes_meanwhile() {
    let mut workspace = ops("def test spin() {\n\
         \x20   let mut n = 0\n\
         \x20   for { n := n + 1 }\n\
         \x20   = \"{n}\"\n\
         }\n\
         def test quick() { = \"quick\" }");
    let spin = workspace.prepare_call(("test", "spin"), &[], &[]).unwrap();
    let quick = workspace.prepare_call(("test", "quick"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let slice = Duration::from_millis(2);
    for _ in 0..5 {
        let began = Instant::now();
        assert_eq!(
            workspace
                .run_prepared_for(&spin, &mut graph, every(), slice)
                .unwrap(),
            Turn::Yielded
        );
        assert!(began.elapsed() >= slice);
    }
    assert_eq!(
        workspace
            .run_prepared_for(&quick, &mut graph, every(), slice)
            .unwrap(),
        Turn::Progress(Progress::Done(Value::Str(Rc::from("quick"))))
    );
    assert_eq!(
        workspace
            .run_prepared_for(&spin, &mut graph, every(), slice)
            .unwrap(),
        Turn::Yielded
    );
    // It waits for nothing, and is given up as any stopped unit is.
    assert_eq!(graph.len(), 0);
    workspace.give_up(spin.unit());
    assert!(workspace.suspended.is_empty());
}

#[test]
fn a_long_loop_yields_several_times_and_gives_its_value() {
    // A slice of zero yields at every look at the clock, and every run
    // takes the steps between two looks first, so the loop ends: its body
    // is simple, run by the loop at once, and counted all the same.
    let text = "def test sum() {\n\
         \x20   let mut sum = 0\n\
         \x20   for i in 0..20000 { sum := sum + i }\n\
         \x20   = \"{sum}\"\n\
         }\n\
         fn add(a: Int, b: Int) -> Int { a + b }\n\
         def test called() {\n\
         \x20   let mut sum = 0\n\
         \x20   for i in 0..5000 { sum := add(sum, i) }\n\
         \x20   = \"{sum}\"\n\
         }";
    let mut workspace = ops(text);
    let (driven, yields) = drive(&mut workspace, &["sum", "called"], Some(Duration::ZERO));
    assert_eq!(
        driven.results,
        [Ok("199990000".to_owned()), Ok("12497500".to_owned())]
    );
    // About one yield for each 1024 iterations of the first, and a few
    // steps to each iteration of the second.
    assert!((15..=25).contains(&yields[0]), "{yields:?}");
    assert!((5..=100).contains(&yields[1]), "{yields:?}");
    assert!(workspace.suspended.is_empty());
}

#[test]
fn a_unit_keeps_its_state_across_yields() {
    // Mutable variables, the places of nested loops, a collecting `for`
    // with a guard, nested calls, and the caller's variables a block
    // assigns from inside its callee.
    let (driven, yields) = at_every_step(
        "fn square(n: Int) -> Int { n * n }\n\
         fn deep(n: Int) -> Int { if n == 0 { 0 } else { 1 + deep(n - 1) } }\n\
         fn twice(body: block -> {}) -> {} {\n\
         \x20   body()\n\
         \x20   body()\n\
         \x20   = {}\n\
         }\n\
         def test state() {\n\
         \x20   let mut total = 0\n\
         \x20   let squares = for i in 0..12 if i % 2 == 0 { square(i) }\n\
         \x20   for i in 0..6 { for j in 0..3 { total := total + i * j } }\n\
         \x20   let counted = for let i = 0; i < 4; i := i + 1 { i * 2 }\n\
         \x20   let mut hits = 0\n\
         \x20   twice() {\n\
         \x20       for i in 0..3 { hits := hits + deep(i) }\n\
         \x20       hits := hits + 0\n\
         \x20   }\n\
         \x20   = \"{total} {squares} {counted} {hits}\"\n\
         }",
        &["state"],
    );
    assert_eq!(
        driven.results,
        [Ok("45 [0, 4, 16, 36, 64, 100] [0, 2, 4, 6] 6".to_owned())]
    );
    assert!(yields[0] > 100, "{yields:?}");
}

#[test]
fn a_yield_is_seen_by_no_recover_with_or_retry() {
    // A yield inside a `recover`'s body and its fallback, inside a
    // `with`, and inside a `#[retry]` function, between its attempts
    // too: each comes to what it does with no yield, and the operations
    // reach the host the same.
    let (driven, yields) = at_every_step(
        "context mode: String = \"none\"\n\
         fn count(to: Int) -> Int {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..to { n := n + 1 }\n\
         \x20   = n\n\
         }\n\
         fn risky(n: Int) -> Int {\n\
         \x20   let counted = count(n)\n\
         \x20   assert counted < 3, \"too many: {counted}\"\n\
         \x20   = counted\n\
         }\n\
         def test recovers() {\n\
         \x20   let fine = recover { risky(2) } else { -1 }\n\
         \x20   let caught = recover { \"{risky(5)}\" } else (failure) {\n\
         \x20       \"{failure.message} after {count(4)}\"\n\
         \x20   }\n\
         \x20   = \"{fine} {caught}\"\n\
         }\n\
         fn reads() -> String {\n\
         \x20   let mut said = \"\"\n\
         \x20   for i in 0..3 { said := said + ^mode }\n\
         \x20   = said\n\
         }\n\
         def test binds() { = \"{reads() with { mode = \"x\" }} {reads()}\" }\n\
         #[retry(2)]\n\
         fn flaky() -> String {\n\
         \x20   let said = op(\"try\")\n\
         \x20   assert count(3) > 5, \"no {said}\"\n\
         \x20   = said\n\
         }\n\
         def test retries() { = flaky() }",
        &["recovers", "binds", "retries"],
    );
    assert_eq!(
        driven.results,
        [
            Ok("2 assertion failed: too many: 5 after 4".to_owned()),
            Ok("xxx nonenonenone".to_owned()),
            Err("assertion failed: no try; `flaky` was tried 3 times".to_owned()),
        ]
    );
    assert_eq!(driven.asked, ["try", "try", "try"]);
    assert!(yields.iter().all(|&yields| yields > 20), "{yields:?}");
}

#[test]
fn a_unit_with_tasks_set_aside_yields_and_keeps_program_order() {
    // Each fetch stops its iteration, which is split off; the loop goes
    // on computing, yielding between every two steps, with its earlier
    // iterations set aside, some waiting for the host and some for their
    // turn.
    let (driven, yields) = at_every_step(
        "fn fetch(n: Int) -> String {\n\
         \x20   let mut pad = 0\n\
         \x20   for i in 0..4 { pad := pad + n }\n\
         \x20   let got = op(\"f{n}\")\n\
         \x20   for i in 0..4 { pad := pad + 1 }\n\
         \x20   = \"{got}:{pad}\"\n\
         }\n\
         def test many() {\n\
         \x20   let got = for n in 0..4 { fetch(n) }\n\
         \x20   = \"{got} {op(\"last\")}\"\n\
         }",
        &["many"],
    );
    assert_eq!(
        driven.results,
        [Ok(
            "[\"f0:4\", \"f1:8\", \"f2:12\", \"f3:16\"] last".to_owned()
        )]
    );
    assert_eq!(driven.asked, ["f0", "f1", "f2", "f3", "last"]);
    assert!(yields[0] > 50, "{yields:?}");
}

#[test]
fn units_that_yield_interleave_with_units_that_wait() {
    let (driven, yields) = at_every_step(
        "fn work(name: String) -> String {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..5 { n := n + i }\n\
         \x20   let first = op(\"{name} one\")\n\
         \x20   for i in 0..5 { n := n + i }\n\
         \x20   = \"{first} {op(\"{name} two\")} {n}\"\n\
         }\n\
         def test a() { = work(\"a\") }\n\
         def test b() { = work(\"b\") }",
        &["a", "b"],
    );
    assert_eq!(
        driven.results,
        [
            Ok("a one a two 20".to_owned()),
            Ok("b one b two 20".to_owned())
        ]
    );
    assert!(yields.iter().all(|&yields| yields > 10), "{yields:?}");
}

#[test]
fn a_unit_yields_then_stops_at_an_effect_then_finishes() {
    let mut workspace = ops("def test mixed() {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..3000 { n := n + 1 }\n\
         \x20   let said = op(\"mid\")\n\
         \x20   for i in 0..3000 { n := n + (if said == \"mid\" { 1 } else { 2 }) }\n\
         \x20   = \"{said} {n}\"\n\
         }");
    let call = workspace.prepare_call(("test", "mixed"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let mut turns = Vec::new();
    let value = loop {
        match workspace
            .run_prepared_for(&call, &mut graph, every(), Duration::ZERO)
            .unwrap()
        {
            Turn::Yielded => turns.push("yielded"),
            Turn::Progress(Progress::Paused(nodes)) => {
                turns.push("paused");
                answer(&mut workspace, &mut graph, nodes[0]);
            }
            Turn::Progress(Progress::Done(value)) => break value,
            Turn::Progress(Progress::Waiting(_)) => panic!("it waits for no unit"),
        }
    };
    assert_eq!(value.to_string(), "mid 6000");
    // Each loop is some three thousand steps, a look at the clock every
    // 1024. The second uses what the operation gave, so it waits for it.
    assert_eq!(
        turns,
        ["yielded", "yielded", "paused", "yielded", "yielded"]
    );
}

#[test]
fn a_run_with_no_slice_never_yields() {
    let text = "def test long() {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..20000 { n := n + 1 }\n\
         \x20   = \"{n} {op(\"after\")}\"\n\
         }";
    // From its start.
    let mut workspace = ops(text);
    let call = workspace.prepare_call(("test", "long"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    assert!(matches!(
        workspace.run_prepared(&call, &mut graph, every()).unwrap(),
        Progress::Paused(_)
    ));
    // And taking up one that yielded, which a run with no slice carries
    // on as far as it waits.
    let mut workspace = ops(text);
    let call = workspace.prepare_call(("test", "long"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    assert_eq!(
        workspace
            .run_prepared_for(&call, &mut graph, every(), Duration::ZERO)
            .unwrap(),
        Turn::Yielded
    );
    let Progress::Paused(nodes) = workspace.run_prepared(&call, &mut graph, every()).unwrap()
    else {
        panic!("it did not run as far as its operation");
    };
    answer(&mut workspace, &mut graph, nodes[0]);
    assert_eq!(
        workspace.run_prepared(&call, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("20000 after")))
    );
}

#[test]
fn another_unit_waits_for_a_shared_run_that_yielded() {
    // The first target yields inside the memoised call, or the `lazy`,
    // that the second needs: the second waits for its unit, as it does
    // when the first waits for the host.
    let memoised = "fn shared(word: String) ->> String {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..3000 { n := n + 1 }\n\
         \x20   = \"{word}{n}\"\n\
         }\n\
         def test first() { = shared(\"w\") }\n\
         def test second() { = shared(\"w\") }";
    let lazy = "let shared = lazy {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..3000 { n := n + 1 }\n\
         \x20   = \"w{n}\"\n\
         }\n\
         def test first() { = await shared }\n\
         def test second() { = await shared }";
    for text in [memoised, lazy] {
        let mut workspace = ops(text);
        let first = workspace.prepare_call(("test", "first"), &[], &[]).unwrap();
        let second = workspace
            .prepare_call(("test", "second"), &[], &[])
            .unwrap();
        let mut graph = Graph::new();
        let zero = Duration::ZERO;
        assert_eq!(
            workspace
                .run_prepared_for(&first, &mut graph, every(), zero)
                .unwrap(),
            Turn::Yielded
        );
        assert_eq!(
            workspace
                .run_prepared_for(&second, &mut graph, every(), zero)
                .unwrap(),
            Turn::Progress(Progress::Waiting(first.unit()))
        );
        let done = Progress::Done(Value::Str(Rc::from("w3000")));
        let mut yields = 1;
        while workspace
            .run_prepared_for(&first, &mut graph, every(), zero)
            .unwrap()
            == Turn::Yielded
        {
            yields += 1;
        }
        assert_eq!(yields, 2, "{text}");
        assert_eq!(
            workspace
                .run_prepared(&second, &mut graph, every())
                .unwrap(),
            done,
            "{text}"
        );
        assert!(workspace.suspended.is_empty());
    }
}

#[test]
fn a_slice_of_zero_moves_a_unit_with_many_waiting_tasks_on() {
    // Every iteration but the first waits for its turn behind an earlier
    // one. A run after a yield wakes none of them, or with so few steps
    // to a run it would spend every run on their looking again.
    let text = "fn fetch(n: Int) -> String { op(\"f{n}\") }\n\
         def test many() {\n\
         \x20   let got = for n in 0..40 { fetch(n) }\n\
         \x20   let mut sum = 0\n\
         \x20   for i in 0..200 { sum := sum + i }\n\
         \x20   = \"{got.len()} {sum}\"\n\
         }";
    let (plain, _) = drive(&mut ops(text), &["many"], None);
    set_yield_steps(4);
    let (sliced, yields) = drive(&mut ops(text), &["many"], Some(Duration::ZERO));
    set_yield_steps(1024);
    assert_eq!(sliced, plain);
    assert_eq!(sliced.results, [Ok("40 19900".to_owned())]);
    assert_eq!(sliced.asked.len(), 40);
    assert!(yields[0] > 40, "{yields:?}");
}

#[test]
fn a_result_given_while_a_unit_yields_is_taken_up_without_a_stop() {
    // Each fetch computes and then asks for its operation. The host gives
    // an operation as soon as it sees one waiting, while the unit is
    // still yielding over the next fetch's computing: the unit finds the
    // result once its tasks all wait, and never stops for one it has.
    let text = "fn fetch(n: Int) -> String {\n\
         \x20   let mut pad = 0\n\
         \x20   for i in 0..6 { pad := pad + n }\n\
         \x20   = \"{op(\"f{n}\")}:{pad}\"\n\
         }\n\
         def test many() {\n\
         \x20   let got = for n in 0..3 { fetch(n) }\n\
         \x20   = \"{got}\"\n\
         }";
    let mut workspace = ops(text);
    let call = workspace.prepare_call(("test", "many"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let mut early = 0;
    let mut stops = 0;
    set_yield_steps(1);
    let value = loop {
        match workspace
            .run_prepared_for(&call, &mut graph, every(), Duration::ZERO)
            .unwrap()
        {
            Turn::Yielded => {
                let waiting: Vec<NodeId> = graph
                    .nodes()
                    .filter(|(_, node)| {
                        matches!(node.state(), crate::eval::graph::NodeState::Waiting)
                    })
                    .map(|(id, _)| id)
                    .collect();
                for node in waiting {
                    answer(&mut workspace, &mut graph, node);
                    early += 1;
                }
            }
            Turn::Progress(Progress::Paused(nodes)) => {
                stops += 1;
                for node in nodes {
                    answer(&mut workspace, &mut graph, node);
                }
            }
            Turn::Progress(Progress::Done(value)) => break value,
            Turn::Progress(Progress::Waiting(_)) => panic!("it waits for no unit"),
        }
    };
    set_yield_steps(1024);
    assert_eq!(value.to_string(), "[\"f0:0\", \"f1:6\", \"f2:12\"]");
    assert_eq!(graph.len(), 3);
    assert!(early >= 1, "{early}");
    assert_eq!(early + stops, 3);
}

#[test]
fn a_timeout_counts_the_time_a_unit_runs_between_yields() {
    // The loop never ends: its function times out once the unit has run
    // for the limit, however long the host leaves it between its runs,
    // and a `recover` around the call catches that.
    let mut workspace = ops("#[timeout(60ms)]\n\
         fn slow() -> String {\n\
         \x20   let mut n = 0\n\
         \x20   for { n := n + 1 }\n\
         \x20   = \"{n}\"\n\
         }\n\
         def test timed() { = recover { slow() } else (failure) { failure.message } }");
    let call = workspace.prepare_call(("test", "timed"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let began = Instant::now();
    let mut ran = Duration::ZERO;
    let mut yields = 0;
    let value = loop {
        let run = Instant::now();
        let turn = workspace
            .run_prepared_for(&call, &mut graph, every(), Duration::from_millis(5))
            .unwrap();
        ran += run.elapsed();
        match turn {
            Turn::Yielded => {
                yields += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Turn::Progress(Progress::Done(value)) => break value,
            Turn::Progress(progress) => panic!("{progress:?}"),
        }
    };
    assert_eq!(value.to_string(), "`slow` timed out after 60ms");
    // Twelve slices of five milliseconds, were none overrun; by the
    // clock on the wall it would have timed out after three.
    assert!((4..=12).contains(&yields), "{yields}");
    assert!(ran >= Duration::from_millis(60), "{ran:?}");
    assert!(began.elapsed() >= Duration::from_millis(60 + 4 * 20));
}

/// A function `timed`, which has 80 milliseconds for its operation, and
/// then `text`.
fn timed(text: &str) -> Workspace {
    ops(&format!(
        "#[timeout(80ms)]\n\
         fn timed(word: String) -> String {{ op(word) }}\n\
         {text}"
    ))
}

/// Runs the unit of `call` as far as its one operation, and returns its
/// node.
fn stop_at_op(workspace: &mut Workspace, call: &PreparedCall, graph: &mut Graph) -> NodeId {
    let Progress::Paused(nodes) = workspace.run_prepared(call, graph, every()).unwrap() else {
        panic!("it did not stop at its operation");
    };
    assert_eq!(nodes.len(), 1);
    nodes[0]
}

/// The failure a host gives an operation it gave up.
fn gave_up() -> Diagnostic {
    Diagnostic::error("the operation was stopped", crate::source::Span::new(0, 0))
}

#[test]
fn time_stopped_counts_toward_no_timeout_until_the_host_charges_it() {
    let mut workspace = timed("def test one() { = timed(\"w\") }");
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    // No unit is stopped, so none has a deadline.
    assert_eq!(workspace.time_left(call.unit()), None);
    let node = stop_at_op(&mut workspace, &call, &mut graph);
    let limit = Duration::from_millis(80);
    let left = workspace.time_left(call.unit()).unwrap();
    assert!(left <= limit && left > limit / 2, "{left:?}");
    // Stopped for longer than its limit, with nothing charged: its clock
    // stood still, and its operation's value is its own.
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(workspace.time_left(call.unit()), Some(left));
    answer(&mut workspace, &mut graph, node);
    assert_eq!(
        workspace.run_prepared(&call, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("w")))
    );
    assert_eq!(workspace.time_left(call.unit()), None);
}

#[test]
fn charges_add_up_and_are_held_to_the_time_stopped() {
    // A limit long enough that the time the test takes passes none.
    let mut workspace = ops("#[timeout(30s)]\n\
         fn long(word: String) -> String { op(word) }\n\
         def test one() { = long(\"w\") }\n\
         def test plain() { = op(\"p\") }");
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let plain = workspace.prepare_call(("test", "plain"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let node = stop_at_op(&mut workspace, &call, &mut graph);
    let left = workspace.time_left(call.unit()).unwrap();
    std::thread::sleep(Duration::from_millis(40));
    // Two charges are one of their sum.
    workspace.charge_stopped(call.unit(), Duration::from_millis(10));
    workspace.charge_stopped(call.unit(), Duration::from_millis(15));
    assert_eq!(
        workspace.time_left(call.unit()),
        Some(left.saturating_sub(Duration::from_millis(25)))
    );
    // More than the time it has been stopped is no more than that: some
    // forty milliseconds of its thirty seconds are gone, not all of them.
    workspace.charge_stopped(call.unit(), Duration::from_secs(3600));
    let held = workspace.time_left(call.unit()).unwrap();
    assert!(
        held > Duration::ZERO && held <= left.saturating_sub(Duration::from_millis(40)),
        "{held:?}"
    );
    answer(&mut workspace, &mut graph, node);
    assert_eq!(
        workspace.run_prepared(&call, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("w")))
    );
    // A unit with no `#[timeout]` running has no time left to tell, and
    // a charge of it, or of a unit that is not stopped, does nothing.
    stop_at_op(&mut workspace, &plain, &mut graph);
    workspace.charge_stopped(plain.unit(), Duration::from_secs(1));
    assert_eq!(workspace.time_left(plain.unit()), None);
    workspace.charge_stopped(call.unit(), Duration::from_secs(1));
    assert_eq!(workspace.time_left(call.unit()), None);
}

#[test]
fn a_deadline_charged_past_its_limit_is_raised_where_the_unit_takes_up() {
    // Whatever the host gives the operation the unit waited for, a
    // failure, a value or nothing: the operation outlived its function's
    // limit, as a program that is killed at its deadline did.
    for given in ["a failure", "a value", "nothing"] {
        let mut workspace = timed("def test one() { = timed(\"w\") }");
        let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
        let mut graph = Graph::new();
        let node = stop_at_op(&mut workspace, &call, &mut graph);
        std::thread::sleep(Duration::from_millis(100));
        workspace.charge_stopped(call.unit(), Duration::from_millis(100));
        assert_eq!(workspace.time_left(call.unit()), Some(Duration::ZERO));
        match given {
            "a failure" => assert!(graph.resolve(node, Err(gave_up()))),
            "a value" => {
                answer(&mut workspace, &mut graph, node);
            }
            _ => {}
        }
        let failed = outcome(
            workspace
                .run_prepared(&call, &mut graph, every())
                .map(|progress| panic!("{given}: {progress:?}")),
        );
        assert_eq!(
            failed,
            Err("`timed` timed out after 80ms".to_owned()),
            "{given}"
        );
        assert!(workspace.suspended.is_empty());
    }
}

#[test]
fn a_charged_timeout_is_caught_outside_its_function_and_retried() {
    // A `recover` around the call catches it, as it catches one raised
    // while the function runs; and its own `#[retry]` tries it again,
    // timed afresh.
    let mut workspace = timed(
        "def test caught() { = recover { timed(\"w\") } else (failure) { failure.message } }\n\
         #[retry(1)] #[timeout(80ms)]\n\
         fn again(word: String) -> String { op(word) }\n\
         def test retried() { = again(\"r\") }",
    );
    let mut graph = Graph::new();
    let caught = workspace
        .prepare_call(("test", "caught"), &[], &[])
        .unwrap();
    let node = stop_at_op(&mut workspace, &caught, &mut graph);
    std::thread::sleep(Duration::from_millis(100));
    workspace.charge_stopped(caught.unit(), Duration::from_millis(100));
    assert!(graph.resolve(node, Err(gave_up())));
    assert_eq!(
        workspace
            .run_prepared(&caught, &mut graph, every())
            .unwrap(),
        Progress::Done(Value::Str(Rc::from("`timed` timed out after 80ms")))
    );
    let retried = workspace
        .prepare_call(("test", "retried"), &[], &[])
        .unwrap();
    let node = stop_at_op(&mut workspace, &retried, &mut graph);
    std::thread::sleep(Duration::from_millis(100));
    workspace.charge_stopped(retried.unit(), Duration::from_millis(100));
    assert!(graph.resolve(node, Err(gave_up())));
    // Its second attempt asks for the operation again, with all of its
    // limit left.
    let again = stop_at_op(&mut workspace, &retried, &mut graph);
    assert_ne!(again, node);
    let left = workspace.time_left(retried.unit()).unwrap();
    assert!(left > Duration::from_millis(40), "{left:?}");
    answer(&mut workspace, &mut graph, again);
    assert_eq!(
        workspace
            .run_prepared(&retried, &mut graph, every())
            .unwrap(),
        Progress::Done(Value::Str(Rc::from("r")))
    );
}

#[test]
fn a_charge_counts_for_every_task_of_the_unit() {
    // The first iteration waits for its operation and the second for its
    // turn, each a task set aside, each inside a call of `timed`: both
    // deadlines pass by what the host charges, and the unit fails with
    // the first in program order, the host having given nothing.
    let mut workspace = timed(
        "def test many() {\n\
         \x20   let got = for n in 0..2 { timed(\"w{n}\") }\n\
         \x20   = \"{got}\"\n\
         }",
    );
    let call = workspace.prepare_call(("test", "many"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let node = stop_at_op(&mut workspace, &call, &mut graph);
    assert_eq!(word(&workspace, &graph, node), "w0");
    let left = workspace.time_left(call.unit()).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    workspace.charge_stopped(call.unit(), Duration::from_millis(30));
    assert_eq!(
        workspace.time_left(call.unit()),
        Some(left.saturating_sub(Duration::from_millis(30)))
    );
    workspace.charge_stopped(call.unit(), Duration::from_millis(70));
    assert_eq!(workspace.time_left(call.unit()), Some(Duration::ZERO));
    let failed = outcome(
        workspace
            .run_prepared(&call, &mut graph, every())
            .map(|progress| panic!("{progress:?}")),
    );
    assert_eq!(failed, Err("`timed` timed out after 80ms".to_owned()));
}

#[test]
fn a_steps_unit_yields_too() {
    let (mut workspace, file) = super::graph::lone_workspace(
        "#[hermetic]\n\
         fn count(name: String) -> String {\n\
         \x20   let mut n = 0\n\
         \x20   for i in 0..3000 { n := n + 1 }\n\
         \x20   = \"{name}{n}\"\n\
         }\n\
         = count(\"a\")",
    );
    let mut graph = Graph::new();
    let Progress::Done(value) = workspace.run_graph(file, &mut graph, every()).unwrap() else {
        panic!("the file did not end with its step's future");
    };
    let step = graph.holes(&value)[0];
    let mut yields = 0;
    let value = loop {
        match workspace
            .run_step_for(&mut graph, step, every(), Duration::ZERO)
            .unwrap()
        {
            (Turn::Yielded, _) => yields += 1,
            (Turn::Progress(Progress::Done(value)), _) => break value,
            (Turn::Progress(progress), _) => panic!("{progress:?}"),
        }
    };
    assert_eq!(yields, 2);
    assert_eq!(value.to_string(), "a3000");
}

/// The loops the benchmarks time, each by its name, its text after
/// [`OPS`] and how many iterations it runs: one whose body is simple,
/// which the machine runs at once; one that calls a function each
/// iteration; and a condition loop.
const LOOPS: [(&str, &str, u32); 3] = [
    (
        "simple",
        "def test bench() {\n\
         \x20   let mut sum = 0\n\
         \x20   for i in 0..20000000 { sum := sum + i }\n\
         \x20   = \"{sum}\"\n\
         }",
        20_000_000,
    ),
    (
        "calling",
        "fn add(a: Int, b: Int) -> Int { a + b }\n\
         def test bench() {\n\
         \x20   let mut sum = 0\n\
         \x20   for i in 0..3000000 { sum := add(sum, i) }\n\
         \x20   = \"{sum}\"\n\
         }",
        3_000_000,
    ),
    (
        "condition",
        "def test bench() {\n\
         \x20   let mut sum = 0\n\
         \x20   for let i = 0; i < 3000000; i := i + 1 { sum := sum + i }\n\
         \x20   = \"{sum}\"\n\
         }",
        3_000_000,
    ),
];

/// How long each of [`LOOPS`] takes with no slice, the best of five runs,
/// in nanoseconds for each iteration: what a host that asks for no slice
/// pays, to compare with the same loops before slices were. Run with
/// `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_loops_with_no_slice() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for (name, text, iterations) in LOOPS {
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let mut workspace = ops(text);
            let call = workspace.prepare_call(("test", "bench"), &[], &[]).unwrap();
            let mut graph = Graph::new();
            let began = Instant::now();
            let progress = workspace.run_prepared(&call, &mut graph, every()).unwrap();
            best = best.min(began.elapsed());
            assert!(matches!(progress, Progress::Done(_)));
        }
        let each = best.as_secs_f64() * 1e9 / f64::from(iterations);
        writeln!(report, "{name}: {each:.2} ns an iteration, no slice").unwrap();
    }
    std::io::stderr().write_all(report.as_bytes()).unwrap();
}

/// How long each of [`LOOPS`] takes in slices of a millisecond, against no
/// slice, and by how much its runs overran their slices, the mean and the
/// most. Run with `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_loops_in_slices() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let slice = Duration::from_millis(1);
    let mut report = String::new();
    for (name, text, iterations) in LOOPS {
        let mut best = Duration::MAX;
        let mut over = Vec::new();
        for _ in 0..5 {
            let mut workspace = ops(text);
            let call = workspace.prepare_call(("test", "bench"), &[], &[]).unwrap();
            let mut graph = Graph::new();
            let mut ran = Duration::ZERO;
            loop {
                let began = Instant::now();
                let turn = workspace
                    .run_prepared_for(&call, &mut graph, every(), slice)
                    .unwrap();
                let took = began.elapsed();
                ran += took;
                if turn != Turn::Yielded {
                    break;
                }
                over.push(took.saturating_sub(slice));
            }
            best = best.min(ran);
        }
        let each = best.as_secs_f64() * 1e9 / f64::from(iterations);
        let most = over.iter().max().copied().unwrap_or_default();
        let mean = over.iter().sum::<Duration>() / u32::try_from(over.len().max(1)).unwrap();
        writeln!(
            report,
            "{name}: {each:.2} ns an iteration in slices of {slice:?}; {} yields, overrun by \
             {mean:?} on average and {most:?} at most",
            over.len()
        )
        .unwrap();
    }
    std::io::stderr().write_all(report.as_bytes()).unwrap();
}
