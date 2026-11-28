//! Tests of workspaces: blocks in graph mode, whose calls stop inside a
//! callee and take up there (see [Blocks](crate::eval::blocks#blocks)).

use super::fileroot::rooted;
use super::graph::every;
use super::*;
use crate::diagnostic::Severity;
use crate::eval::plain::Plain;

/// A def-type `test`, an operation `op`, which is an `exec` the host
/// answers itself with its word, and `step`, which takes a block.
const STEPS: &str = "#[def_type]\n\
     def test(name: String, f: fn() -> String) -> String { f() }\n\
     fn op(word: String) -> String { exec([\"op\", word]).stdout }\n\
     fn step(name: String, body: block -> {}) -> {} {\n\
     \x20   op(\"begin {name}\")\n\
     \x20   body()\n\
     \x20   op(\"end {name}\")\n\
     \x20   = {}\n\
     }\n";

/// The workspace of one file, [`STEPS`] and then `text`.
pub(super) fn steps(text: &str) -> Workspace {
    let app = format!("{STEPS}{text}");
    rooted(&[("workspace.gcsl", ""), ("app.gcsl", &app)]).unwrap()
}

/// Answers the effect `node` of `graph`, an `op`, with its word, and
/// returns the word.
fn answer(workspace: &mut Workspace, graph: &mut Graph, node: NodeId) -> String {
    let job = workspace.effect_job(graph, node).unwrap();
    let Plain::List(argv) = &job.args()[0] else {
        panic!("an `exec` without its program");
    };
    let Some(Plain::Str(word)) = argv.get(1) else {
        panic!("an `op` without its word");
    };
    let result = Plain::Record(vec![
        ("status".to_owned(), Plain::Int(0)),
        ("stdout".to_owned(), Plain::Str(word.clone())),
        ("stderr".to_owned(), Plain::Str(String::new())),
    ]);
    assert!(graph.resolve(node, Ok(workspace.from_plain(&result))));
    word.clone()
}

/// Runs the units of the definitions `names` of `test` together, one
/// operation answered at a time, the units taken in turn: what each came
/// to, and every operation in the order the host was asked for it.
pub(super) fn drive(
    workspace: &mut Workspace,
    names: &[&str],
) -> (Vec<Result<String, String>>, Vec<String>) {
    let calls: Vec<_> = names
        .iter()
        .map(|name| workspace.prepare_call(("test", name), &[], &[]).unwrap())
        .collect();
    let mut graph = Graph::new();
    let mut results: Vec<Option<Result<String, String>>> = vec![None; calls.len()];
    let mut asked = Vec::new();
    while results.iter().any(Option::is_none) {
        for (at, call) in calls.iter().enumerate() {
            if results[at].is_some() {
                continue;
            }
            match workspace.run_prepared(call, &mut graph, every()) {
                Ok(Progress::Done(value)) => results[at] = Some(Ok(value.to_string())),
                Ok(Progress::Paused(nodes)) => {
                    // Operations are not isolated: one at a time.
                    assert_eq!(nodes.len(), 1, "{nodes:?}");
                    asked.push(answer(workspace, &mut graph, nodes[0]));
                }
                Ok(Progress::Waiting(_)) => {}
                Err(InvokeError::Diagnostics(errors)) => {
                    let failed = errors
                        .iter()
                        .find(|error| error.severity == Severity::Error);
                    results[at] = Some(Err(
                        failed.map_or_else(String::new, |error| error.message.clone())
                    ));
                }
                Err(InvokeError::Request(error)) => panic!("{error:?}"),
            }
        }
    }
    (results.into_iter().flatten().collect(), asked)
}

#[test]
fn a_unit_stops_inside_a_block_and_takes_up_there() {
    // Each operation stops the unit: before the block, in it, and after
    // it, in the middle of `step`. The block's assignment is kept.
    let mut workspace = steps(
        "def test one() {\n\
         \x20   let mut seen = \"\"\n\
         \x20   step(\"a\") {\n\
         \x20       seen := seen + op(\"in a\")\n\
         \x20       seen := seen + op(\"still a\")\n\
         \x20   }\n\
         \x20   step(\"b\") { seen := seen + op(\"in b\") }\n\
         \x20   = seen\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("in astill ain b".to_owned())]);
    assert_eq!(
        asked,
        [
            "begin a", "in a", "still a", "end a", "begin b", "in b", "end b"
        ]
    );
}

#[test]
fn a_block_that_assigns_nothing_keeps_its_effects_in_program_order() {
    // Neither step assigns, so each may wait apart; their operations are
    // still asked for one at a time, in the order of the program.
    let mut workspace = steps(
        "def test one() {\n\
         \x20   step(\"a\") { op(\"in a\") }\n\
         \x20   step(\"b\") { op(\"in b\") }\n\
         \x20   = op(\"last\")\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("last".to_owned())]);
    assert_eq!(
        asked,
        [
            "begin a", "in a", "end a", "begin b", "in b", "end b", "last"
        ]
    );
}

#[test]
fn units_with_blocks_interleave_each_in_its_own_order() {
    let mut workspace = steps(
        "def test one() {\n\
         \x20   let mut n = 0\n\
         \x20   step(\"a1\") { op(\"one a\"); n := n + 1 }\n\
         \x20   step(\"b1\") { op(\"one b\"); n := n + 1 }\n\
         \x20   = \"one {n}\"\n\
         }\n\
         def test two() {\n\
         \x20   let mut n = 10\n\
         \x20   step(\"a2\") { op(\"two a\"); n := n + 1 }\n\
         \x20   = \"two {n}\"\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["one", "two"]);
    assert_eq!(results, [Ok("one 2".to_owned()), Ok("two 11".to_owned())]);
    let of = |unit: &str| -> Vec<&str> {
        asked
            .iter()
            .map(String::as_str)
            .filter(|word| word.contains(unit))
            .collect()
    };
    assert_eq!(of("1").into_iter().chain(of("one")).count(), 6, "{asked:?}");
    let own: Vec<&str> = asked
        .iter()
        .map(String::as_str)
        .filter(|word| word.ends_with('1') || word.starts_with("one"))
        .collect();
    assert_eq!(
        own,
        ["begin a1", "one a", "end a1", "begin b1", "one b", "end b1"]
    );
    let other: Vec<&str> = asked
        .iter()
        .map(String::as_str)
        .filter(|word| word.ends_with('2') || word.starts_with("two"))
        .collect();
    assert_eq!(other, ["begin a2", "two a", "end a2"]);
    // The two units' operations are mixed, not one unit's and then the
    // other's.
    assert_ne!(&asked[..6], &own[..]);
}

#[test]
fn a_block_called_in_a_loop_runs_its_iterations_in_order() {
    // The loop in `each` calls its block, which may assign, so its
    // iterations are not split off: the operations come in order, and the
    // block's variable is the caller's throughout.
    let mut workspace = steps(
        "fn each(words: List<String>, body: block -> {}) -> {} {\n\
         \x20   for word in words {\n\
         \x20       op(\"at {word}\")\n\
         \x20       body()\n\
         \x20   }\n\
         \x20   = {}\n\
         }\n\
         def test one() {\n\
         \x20   let mut seen = \"\"\n\
         \x20   each([\"x\", \"y\", \"z\"]) { seen := seen + op(\"in {seen}\") + \".\" }\n\
         \x20   = seen\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("in .in in ..in in .in in ...".to_owned())]);
    assert_eq!(
        asked,
        ["at x", "in ", "at y", "in in .", "at z", "in in .in in .."]
    );
}

#[test]
fn iterations_that_give_blocks_wait_apart_and_keep_their_own_variables() {
    // The loop's iterations assign nothing outside themselves, so each is
    // split off where it stops; each has its own `said`, in its own copy
    // of the frame, and the operations still come in program order.
    let mut workspace = steps(
        "fn told(word: String) -> String {\n\
         \x20   let mut said = \"\"\n\
         \x20   step(word) { said := op(\"say {word}\") }\n\
         \x20   = said\n\
         }\n\
         fn all(words: List<String>) -> List<String> { for word in words { told(word) } }\n\
         def test one() { = all([\"x\", \"y\"]).join(\",\") }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("say x,say y".to_owned())]);
    assert_eq!(
        asked,
        ["begin x", "say x", "end x", "begin y", "say y", "end y"]
    );
}

#[test]
fn a_failure_in_a_block_is_the_callee_s_to_recover() {
    let mut workspace = steps(
        "fn guarded(body: block -> {}) -> String {\n\
         \x20   = recover { body(); = \"passed\" } else (failure) { \"caught {failure.message}\" }\n\
         }\n\
         def test caught() {\n\
         \x20   = guarded { op(\"before\"); panic(\"boom\") }\n\
         }\n\
         def test uncaught() {\n\
         \x20   step(\"a\") { op(\"in a\"); panic(\"bang\") }\n\
         \x20   = \"unreached\"\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["caught", "uncaught"]);
    assert_eq!(
        results,
        [Ok("caught boom".to_owned()), Err("bang".to_owned())]
    );
    // The step that failed never ended.
    assert!(!asked.contains(&"end a".to_owned()), "{asked:?}");
}

#[test]
fn a_block_passed_on_runs_in_the_frame_that_gave_it() {
    // `outer`'s block calls its own block inside `step`'s: two frames
    // down, it still assigns the test's variable.
    let mut workspace = steps(
        "fn outer(name: String, body: block -> {}) -> {} {\n\
         \x20   let mut local = \"outer's\"\n\
         \x20   step(name) { local := op(\"passing\"); body() }\n\
         \x20   op(\"outer had {local}\")\n\
         \x20   = {}\n\
         }\n\
         def test one() {\n\
         \x20   let mut mine = \"\"\n\
         \x20   outer(\"o\") { mine := op(\"innermost\") }\n\
         \x20   = mine\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("innermost".to_owned())]);
    assert_eq!(
        asked,
        [
            "begin o",
            "passing",
            "innermost",
            "end o",
            "outer had passing"
        ]
    );
}

#[test]
fn a_block_s_value_comes_back_through_a_stop() {
    let mut workspace = steps(
        "fn measure<T>(name: String, body: block -> T) -> T {\n\
         \x20   op(\"start {name}\")\n\
         \x20   let value = body()\n\
         \x20   op(\"stop {name}\")\n\
         \x20   = value\n\
         }\n\
         def test one() {\n\
         \x20   let word = measure(\"m\") { op(\"work\") }\n\
         \x20   = \"{word}!\"\n\
         }",
    );
    let (results, asked) = drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("work!".to_owned())]);
    assert_eq!(asked, ["start m", "work", "stop m"]);
}

#[test]
fn a_block_of_a_memoised_call_s_body_is_run_once_for_two_units() {
    // The fixture's body gives a block; the second unit waits for the
    // first's run of it and finds what it made.
    let mut workspace = steps(
        "fn fixture() ->> String {\n\
         \x20   let mut made = \"\"\n\
         \x20   step(\"build\") { made := op(\"building\") }\n\
         \x20   = made\n\
         }\n\
         def test one() { = fixture() }\n\
         def test two() { = fixture() }",
    );
    let (results, asked) = drive(&mut workspace, &["one", "two"]);
    assert_eq!(
        results,
        [Ok("building".to_owned()), Ok("building".to_owned())]
    );
    assert_eq!(asked, ["begin build", "building", "end build"]);
}
