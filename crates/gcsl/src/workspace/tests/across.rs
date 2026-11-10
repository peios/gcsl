//! Tests of workspaces: units in graph mode that wait for each other's
//! memoised calls and `lazy`s, and runs other units wait for, which keep
//! their own order (see [Across units](crate::eval::machine::tasks#across-units)).

use super::fileroot::rooted;
use super::graph::every;
use super::*;
use crate::eval::graph::{NodeId, NodeKind};
use crate::workspace::graph::PreparedCall;

/// What every fixture's file begins with: `said`, whose read is an effect
/// that [`every`]'s host does not isolate, so that it keeps program order,
/// and `both`, whose arguments are pieces.
const PRELUDE: &str = "import fs\n\
                       #[def_type]\n\
                       def build(name: String, f: fn() -> String) -> String { f() }\n\
                       fn said(word: String) -> String { = fs.read(\"/{word}\") }\n\
                       fn both(first: String, second: String) -> String { = \"{first}+{second}\" }\n";

/// Programs whose targets share memoised calls or a `lazy`, each target a
/// `def build`: its name, its targets, what each comes to, and how many
/// effects the graph ends with, each made once.
struct Fixture {
    /// What it shows.
    name: &'static str,
    /// The program, after [`PRELUDE`].
    program: &'static str,
    /// Its targets, in the order they are prepared.
    targets: &'static [&'static str],
    /// What each target comes to.
    values: &'static [&'static str],
    /// The effects made.
    effects: usize,
}

/// Two targets that each need both of two memoised calls, `outer` calling
/// `inner`. The second's arguments are pieces: its first waits for the
/// first target's run of `outer`, and its second, a later piece, begins
/// `inner`, whose read then waits its turn behind the first piece.
const CROSSED: Fixture = Fixture {
    name: "crossed",
    program: "fn inner() ->> String { = said(\"b\") }\n\
              fn outer() ->> String {\n\
              \x20   let a = said(\"a\")\n\
              \x20   = \"{a}{inner()}\"\n\
              }\n\
              def build first() { = outer() }\n\
              def build second() { = both(outer(), inner()) }",
    targets: &["first", "second"],
    values: &["AB", "AB+B"],
    effects: 2,
};

/// [`CROSSED`], the second target prepared first, so that it is the unit
/// numbered lower: a host that handed a run from a higher unit only to a
/// lower one could not hand `inner` over, nor `outer`, whose read is made.
const COMMITTED: Fixture = Fixture {
    name: "committed",
    targets: &["second", "first"],
    values: &["AB+B", "AB"],
    ..CROSSED
};

/// [`CROSSED`] a call deeper, the second target's later piece beginning
/// the innermost call.
const NESTED_INNER: Fixture = Fixture {
    name: "nested inner",
    program: "fn inner() ->> String { = said(\"c\") }\n\
              fn mid() ->> String {\n\
              \x20   let b = said(\"b\")\n\
              \x20   = \"{b}{inner()}\"\n\
              }\n\
              fn outer() ->> String {\n\
              \x20   let a = said(\"a\")\n\
              \x20   = \"{a}{mid()}\"\n\
              }\n\
              def build first() { = outer() }\n\
              def build second() { = both(outer(), inner()) }",
    targets: &["first", "second"],
    values: &["ABC", "ABC+C"],
    effects: 3,
};

/// [`NESTED_INNER`], the second target's later piece beginning the call
/// in the middle, which calls the innermost.
const NESTED_MID: Fixture = Fixture {
    name: "nested mid",
    program: "fn inner() ->> String { = said(\"c\") }\n\
              fn mid() ->> String {\n\
              \x20   let b = said(\"b\")\n\
              \x20   = \"{b}{inner()}\"\n\
              }\n\
              fn outer() ->> String {\n\
              \x20   let a = said(\"a\")\n\
              \x20   = \"{a}{mid()}\"\n\
              }\n\
              def build first() { = outer() }\n\
              def build second() { = both(outer(), mid()) }",
    targets: &["first", "second"],
    values: &["ABC", "ABC+BC"],
    effects: 3,
};

/// [`CROSSED`] with a third target that needs both calls too, the other
/// way round.
const THREE: Fixture = Fixture {
    name: "three",
    program: "fn inner() ->> String { = said(\"b\") }\n\
              fn outer() ->> String {\n\
              \x20   let a = said(\"a\")\n\
              \x20   = \"{a}{inner()}\"\n\
              }\n\
              def build first() { = outer() }\n\
              def build second() { = both(outer(), inner()) }\n\
              def build third() { = both(inner(), outer()) }",
    targets: &["first", "second", "third"],
    values: &["AB", "AB+B", "B+AB"],
    effects: 2,
};

/// [`CROSSED`] with a top-level `lazy` for `inner`.
const LAZY: Fixture = Fixture {
    name: "lazy",
    program: "let inner = lazy { = said(\"b\") }\n\
              fn outer() ->> String {\n\
              \x20   let a = said(\"a\")\n\
              \x20   = \"{a}{await inner}\"\n\
              }\n\
              def build first() { = outer() }\n\
              def build second() { = both(outer(), await inner) }",
    targets: &["first", "second"],
    values: &["AB", "AB+B"],
    effects: 2,
};

/// A task that waits for a later one's demanded run, which is not
/// wounded: in the second target, the first piece waits for the third
/// target's `x`, and the second begins `d`, which the first target then
/// waits for. Taken up together, the first piece needs `d`, and the second
/// then ends it and waits its turn behind the first, which is woken as the
/// run ends.
const WOKEN: Fixture = Fixture {
    name: "woken",
    program: "fn x() ->> String {\n\
              \x20   let a = said(\"x1\")\n\
              \x20   = \"{a}{said(\"x2\")}\"\n\
              }\n\
              fn d() ->> String { = said(\"d\") }\n\
              fn late(e: String) -> String { = \"{e}{d()}\" }\n\
              fn after(d: String) -> String { = \"{d}{said(\"t\")}\" }\n\
              fn plain() -> String { = \"z\" }\n\
              fn three(a: String, b: String, c: String) -> String { = \"{a}+{b}+{c}\" }\n\
              def build third() { = x() }\n\
              def build second() { = three(late(x()), after(d()), plain()) }\n\
              def build first() { = d() }",
    targets: &["third", "second", "first"],
    values: &["X1X2", "X1X2D+DT+z", "D"],
    effects: 4,
};

/// Every fixture whose targets share runs.
const FIXTURES: [&Fixture; 8] = [
    &CROSSED,
    &COMMITTED,
    &NESTED_INNER,
    &NESTED_MID,
    &THREE,
    &LAZY,
    &WOKEN,
    &UNSHARED,
];

/// One target whose memoised call nothing else waits for: its read keeps
/// program order, after the first piece's.
const UNSHARED: Fixture = Fixture {
    name: "unshared",
    program: "fn m() ->> String { = said(\"m\") }\n\
              def build only() { = both(said(\"p\"), m()) }",
    targets: &["only"],
    values: &["P+M"],
    effects: 2,
};

/// A seeded generator of the orders units run in and effects are given.
struct Rng(u64);

impl Rng {
    /// The next number (xorshift64*).
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number below `bound`, which is not 0.
    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound).unwrap()).unwrap()
    }
}

/// What [`drive`] came to: what each target came to, and the effects the
/// graph made.
#[derive(Debug)]
struct Driven {
    /// What each target came to, or its first error's message.
    values: Vec<Result<Value, String>>,
    /// The effects made.
    effects: usize,
}

/// Gives the effect node `node` of `graph` what a host would: a read of
/// `/word` is `WORD`.
fn give(graph: &mut Graph, node: NodeId) {
    let found = graph.node(node).unwrap();
    let value = match (found.kind(), found.args().first()) {
        (NodeKind::Effect(crate::check::Builtin::FsRead), Some(Value::Str(path))) => {
            Value::Str(Rc::from(path.trim_start_matches('/').to_uppercase()))
        }
        _ => Value::Empty,
    };
    assert!(graph.resolve(node, Ok(value)));
}

/// Runs `calls` of `workspace` as a host does, until each is done: in
/// rounds, each running every unit not yet done once, a unit that stops
/// for effects having them given. With `rng`, the units run in an order it
/// picks each round, and each effect is given at a point of the round it
/// picks, the rest at its end; without, in order, each effect at once.
/// Every unit has at most one effect waiting at a time, since the host
/// isolates none. `Err` if a round moves nothing on: the units wait for
/// each other.
fn drive(
    workspace: &mut Workspace,
    calls: &[&PreparedCall],
    mut rng: Option<&mut Rng>,
) -> Result<Driven, String> {
    let mut graph = Graph::new();
    let mut done: Vec<Option<Result<Value, String>>> = vec![None; calls.len()];
    let mut waiting: Vec<NodeId> = Vec::new();
    let mut rounds = 0;
    loop {
        rounds += 1;
        assert!(rounds < 200, "{rounds} rounds");
        let before = workspace.memo_progress();
        let mut progressed = false;
        let mut order: Vec<usize> = (0..calls.len()).collect();
        if let Some(rng) = rng.as_deref_mut() {
            for at in (1..order.len()).rev() {
                order.swap(at, rng.below(at + 1));
            }
        }
        for at in order {
            if done[at].is_some() {
                continue;
            }
            match workspace.run_prepared(calls[at], &mut graph, every()) {
                Ok(Progress::Done(value)) => {
                    done[at] = Some(Ok(value));
                    progressed = true;
                }
                Ok(Progress::Paused(nodes)) => {
                    assert!(nodes.len() <= 1, "unit {at} waits for {nodes:?} at once");
                    for node in nodes {
                        if !waiting.contains(&node) {
                            waiting.push(node);
                        }
                    }
                }
                Ok(Progress::Waiting(_)) => {}
                Err(InvokeError::Diagnostics(errors)) => {
                    done[at] = Some(Err(errors[0].message.clone()));
                    progressed = true;
                }
                Err(error) => panic!("{error:?}"),
            }
            // Some of the effects waiting are given now.
            let given = match rng.as_deref_mut() {
                Some(rng) => rng.below(waiting.len() + 1),
                None => waiting.len(),
            };
            for node in waiting.drain(..given) {
                give(&mut graph, node);
                progressed = true;
            }
        }
        for node in waiting.drain(..) {
            give(&mut graph, node);
            progressed = true;
        }
        if done.iter().all(Option::is_some) {
            return Ok(Driven {
                values: done.into_iter().flatten().collect(),
                effects: graph.len(),
            });
        }
        if !progressed && workspace.memo_progress() == before {
            return Err("the units wait for each other".to_owned());
        }
    }
}

/// The workspace of `fixture`, and its targets prepared.
fn prepared(fixture: &Fixture) -> (Workspace, Vec<PreparedCall>) {
    let files = [
        ("workspace.gcsl", String::new()),
        ("app.gcsl", format!("{PRELUDE}{}", fixture.program)),
    ];
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (*path, text.as_str()))
        .collect();
    let mut workspace =
        rooted(&files).unwrap_or_else(|errors| panic!("{}: {errors:?}", fixture.name));
    let calls = fixture
        .targets
        .iter()
        .map(|target| workspace.prepare_call(("build", target), &[], &[]).unwrap())
        .collect();
    (workspace, calls)
}

/// Checks that `driven` is what `fixture`'s targets come to, each effect
/// made once, and nothing left suspended.
fn assert_fixture(fixture: &Fixture, workspace: &Workspace, driven: &Driven, what: &str) {
    let values: Vec<Result<Value, String>> = fixture
        .values
        .iter()
        .map(|value| Ok(Value::Str(Rc::from(*value))))
        .collect();
    assert_eq!(driven.values, values, "{}, {what}", fixture.name);
    assert_eq!(driven.effects, fixture.effects, "{}, {what}", fixture.name);
    assert!(workspace.suspended.is_empty(), "{}, {what}", fixture.name);
}

#[test]
fn units_that_cross_over_runs_they_share_finish_each_in_either_order() {
    // Run in order, the first target begins `outer`; the second waits for
    // it, its later piece having begun `inner`, whose read waits its turn
    // behind the earlier piece; and then the first needs `inner`. `inner`
    // is demanded, and so no longer waits for the second's earlier piece.
    // Run the other way, the second runs both, as wound-wait orders them.
    for fixture in [&CROSSED, &COMMITTED] {
        for reversed in [false, true] {
            let (mut workspace, calls) = prepared(fixture);
            let mut order: Vec<&PreparedCall> = calls.iter().collect();
            if reversed {
                order.reverse();
            }
            let mut driven = drive(&mut workspace, &order, None)
                .unwrap_or_else(|error| panic!("{}, reversed {reversed}: {error}", fixture.name));
            if reversed {
                driven.values.reverse();
            }
            assert_fixture(
                fixture,
                &workspace,
                &driven,
                &format!("reversed {reversed}"),
            );
        }
    }
}

#[test]
fn every_fixture_finishes_with_its_units_in_order_and_reversed() {
    for fixture in FIXTURES {
        for reversed in [false, true] {
            let (mut workspace, calls) = prepared(fixture);
            let mut order: Vec<&PreparedCall> = calls.iter().collect();
            if reversed {
                order.reverse();
            }
            let mut driven = drive(&mut workspace, &order, None)
                .unwrap_or_else(|error| panic!("{}, reversed {reversed}: {error}", fixture.name));
            if reversed {
                driven.values.reverse();
            }
            assert_fixture(
                fixture,
                &workspace,
                &driven,
                &format!("reversed {reversed}"),
            );
        }
    }
}

#[test]
fn a_demanded_run_makes_its_effect_only_once_no_other_waits() {
    // The second target's first piece reads, and its read waits; its
    // second begins `inner`, whose read waits its turn. The first target
    // then waits for `inner`, which no longer waits for the first piece,
    // but its read still waits for the first piece's to end: one at a
    // time.
    let fixture = Fixture {
        name: "one at a time",
        program: "fn inner() ->> String { = said(\"b\") }\n\
                  def build first() { = inner() }\n\
                  def build second() { = both(said(\"p\"), inner()) }",
        targets: &["first", "second"],
        values: &["B", "P+B"],
        effects: 2,
    };
    let (mut workspace, calls) = prepared(&fixture);
    let (first, second) = (&calls[0], &calls[1]);
    let mut graph = Graph::new();
    let Ok(Progress::Paused(nodes)) = workspace.run_prepared(second, &mut graph, every()) else {
        panic!("the second target did not stop for its read");
    };
    let [read] = nodes[..] else {
        panic!("the second target waits for {nodes:?}");
    };
    assert_eq!(
        workspace.run_prepared(first, &mut graph, every()).unwrap(),
        Progress::Waiting(second.unit())
    );
    // `inner` is demanded, and its read still waits.
    assert_eq!(
        workspace.run_prepared(second, &mut graph, every()).unwrap(),
        Progress::Paused(vec![read])
    );
    give(&mut graph, read);
    let Ok(Progress::Paused(nodes)) = workspace.run_prepared(second, &mut graph, every()) else {
        panic!("the second target did not stop for `inner`'s read");
    };
    for node in nodes {
        give(&mut graph, node);
    }
    let text = |text: &str| Progress::Done(Value::Str(Rc::from(text)));
    assert_eq!(
        workspace.run_prepared(second, &mut graph, every()).unwrap(),
        text("P+B")
    );
    assert_eq!(
        workspace.run_prepared(first, &mut graph, every()).unwrap(),
        text("B")
    );
    assert_eq!(graph.len(), 2);
}
