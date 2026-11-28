//! Tests of workspaces: host operations and handles, in plain evaluation
//! and in graph mode (see [Host
//! operations](crate::eval::operations#host-operations)).

use std::cell::RefCell;
use std::fmt::Write as _;
use std::sync::Mutex;

use super::fileroot::rooted;
use super::graph::every;
use super::*;
use crate::diagnostic::Severity;
use crate::eval::graph::{Operation, RunExtras};
use crate::eval::plain::Plain;
use crate::eval::{CaughtFailure, Direct, Effects, OperationFailure, eval_rooted};

/// A def-type `test`, whose definitions are the units of a test in graph
/// mode.
const TEST: &str = "#[def_type]\n\
     def test(name: String, f: fn() -> String) -> String { f() }\n";

/// The handle types and the operations of a host that runs machines:
/// [`perform`] is the host.
const VM: &str = "type Profile = {name: String, cpus: Int = 1}\n\
     extern type Vm = \"vm.machine\"\n\
     extern type Network\n\
     extern fn boot(profile: Profile, nics: List<Network> = []) -> Vm = \"vm.boot\"\n\
     extern fn network(name: String) -> Network = \"net.make\"\n\
     extern fn (m: Vm) run(...argv: String) -> {status: Int, stdout: String} = \"vm.run\"\n\
     extern fn (m: Vm) read(path: String, limit: Size = 64MiB) -> String = \"vm.file.read\"\n\
     extern fn (m: Vm) halt() -> {}\n\
     extern fn say(word: String) -> String\n\
     extern fn number(n: Int) -> Int\n\
     extern fn fail(why: String) -> String\n\
     extern fn wrong() -> Vm\n\
     context arch: String = \"x86_64\"\n\
     extern fn build(name: String, ^arch) -> String\n";

/// What a test's host does for the operation `name` given `args`, the
/// handles it makes numbered from `made` on: what it gives back, or why it
/// failed.
fn perform(name: &str, args: &[Plain], made: &mut i64) -> Result<Plain, OperationFailure> {
    let text = |plain: &Plain| match plain {
        Plain::Str(text) => text.clone(),
        other => panic!("expected a string, got {other:?}"),
    };
    match (name, args) {
        ("vm.boot", [Plain::Record(_), Plain::List(_)]) => {
            *made += 1;
            Ok(Plain::handle("vm.machine", *made))
        }
        ("net.make", [Plain::Str(_)]) => {
            *made += 1;
            Ok(Plain::handle("Network", *made))
        }
        ("vm.run", [machine, Plain::List(words)]) => {
            let (kind, id) = machine.as_handle().expect("a handle");
            assert_eq!(kind, "vm.machine");
            let words: Vec<String> = words.iter().map(text).collect();
            Ok(Plain::Record(vec![
                ("status".to_owned(), Plain::Int(id)),
                ("stdout".to_owned(), Plain::Str(words.join(" "))),
            ]))
        }
        ("vm.file.read", [machine, Plain::Str(path), Plain::Quantity(_, limit)]) => {
            let (_, id) = machine.as_handle().expect("a handle");
            Ok(Plain::Str(format!("{path} of {id} up to {limit}")))
        }
        ("halt", [machine]) => {
            machine.as_handle().expect("a handle");
            Ok(Plain::Empty)
        }
        ("say", [word]) => Ok(Plain::Str(text(word))),
        ("build", [name, arch]) => Ok(Plain::Str(format!("{} for {}", text(name), text(arch)))),
        ("number", [Plain::Int(n)]) => Ok(Plain::Int(*n)),
        ("fail", [why]) => Err(OperationFailure::new(text(why)).with_note("the host said so")),
        ("wrong", []) => Ok(Plain::handle("vm.disk", 9)),
        _ => Err(OperationFailure::new(format!(
            "this host has no operation `{name}` of {} arguments",
            args.len()
        ))),
    }
}

/// How a test's host shows the operation `name` given `args`: its name
/// and each argument, a handle as its kind and number.
fn asked(name: &str, args: &[Plain]) -> String {
    fn shown(plain: &Plain) -> String {
        if let Some((kind, id)) = plain.as_handle() {
            return format!("<{kind} {id}>");
        }
        match plain {
            Plain::Int(n) => n.to_string(),
            Plain::Str(text) => format!("{text:?}"),
            Plain::Quantity(_, amount) => format!("{amount}q"),
            Plain::Empty => "{}".to_owned(),
            Plain::List(items) => {
                let items: Vec<String> = items.iter().map(shown).collect();
                format!("[{}]", items.join(", "))
            }
            Plain::Record(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, value)| format!("{name} = {}", shown(value)))
                    .collect();
                format!("{{{}}}", fields.join("; "))
            }
            other => format!("{other:?}"),
        }
    }
    let args: Vec<String> = args.iter().map(shown).collect();
    format!("{name}({})", args.join(", "))
}

/// The workspace of one file, `app.gcsl`, [`TEST`], [`VM`] and then
/// `text`, and a host for it.
fn machines(text: &str) -> (Workspace, Engine) {
    let app = format!("{TEST}{VM}{text}");
    let workspace = rooted(&[("workspace.gcsl", ""), ("app.gcsl", &app)])
        .unwrap_or_else(|errors| panic!("{errors:?}"));
    let engine = Engine {
        file: workspace.file("app.gcsl").unwrap(),
        source: app,
        ..Engine::default()
    };
    (workspace, engine)
}

/// A host of graph mode, of the file `file` whose text is `source`: every
/// operation it was asked for, in order, as [`asked`] shows it, where each
/// was called, as the text there, and how many handles it has made.
#[derive(Default)]
struct Engine {
    file: u32,
    source: String,
    asked: Vec<String>,
    called: Vec<String>,
    made: i64,
}

impl Engine {
    /// Answers the node `node` of `graph`, an operation's, as [`perform`]
    /// does.
    fn answer(&mut self, workspace: &mut Workspace, graph: &mut Graph, node: NodeId) {
        let operation: Operation = workspace
            .operation(graph, node)
            .expect("a node that is an operation's");
        self.asked.push(asked(&operation.name, &operation.args));
        assert_eq!(operation.input, self.file);
        let (start, end) = (operation.span.start as usize, operation.span.end as usize);
        self.called.push(self.source[start..end].to_owned());
        let outcome = match perform(&operation.name, &operation.args, &mut self.made) {
            Ok(plain) => Ok(workspace.from_plain(&plain)),
            Err(failure) => {
                let mut error = Diagnostic::error(failure.message, Span::new(0, 0));
                error.notes = failure.notes;
                Err(error)
            }
        };
        assert!(graph.resolve(node, outcome));
    }

    /// Runs the units of the definitions `names` of `test` of `workspace`
    /// together, one operation answered at a time, the units taken in
    /// turn: what each came to, its value shown or its failure's message.
    fn drive(&mut self, workspace: &mut Workspace, names: &[&str]) -> Vec<Result<String, String>> {
        self.drive_through(workspace, names, every())
    }

    /// [`Engine::drive`], the units' other effects through `host`.
    fn drive_through(
        &mut self,
        workspace: &mut Workspace,
        names: &[&str],
        host: GraphHost<'_>,
    ) -> Vec<Result<String, String>> {
        let calls: Vec<_> = names
            .iter()
            .map(|name| workspace.prepare_call(("test", name), &[], &[]).unwrap())
            .collect();
        let mut graph = Graph::new();
        let mut results: Vec<Option<Result<String, String>>> = vec![None; calls.len()];
        while results.iter().any(Option::is_none) {
            for (at, call) in calls.iter().enumerate() {
                if results[at].is_some() {
                    continue;
                }
                match workspace.run_prepared(call, &mut graph, host) {
                    Ok(Progress::Done(value)) => results[at] = Some(Ok(value.to_string())),
                    Ok(Progress::Paused(nodes)) => {
                        // Operations are not isolated: one at a time.
                        assert_eq!(nodes.len(), 1, "{nodes:?}");
                        self.answer(workspace, &mut graph, nodes[0]);
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
        results.into_iter().flatten().collect()
    }
}

/// A host of plain evaluation, of a program whose text is `source`: every
/// operation it was asked for, as [`asked`] shows it, how many handles it
/// has made, and each failure it was told the program caught, its message,
/// the text it is located at, its notes and whether it was raised again.
#[derive(Debug, Default)]
struct Plainly {
    source: String,
    asked: Mutex<Vec<String>>,
    made: Mutex<i64>,
    caught: Mutex<Vec<Heard>>,
}

/// A failure a host was told a program caught: its message, the text it is
/// located at, its notes and whether it was raised again.
type Heard = (String, String, Vec<String>, bool);

impl Effects for Plainly {
    fn operation(&self, name: &str, args: &[Plain]) -> Result<Plain, OperationFailure> {
        self.asked.lock().unwrap().push(asked(name, args));
        perform(name, args, &mut self.made.lock().unwrap())
    }

    fn caught(&self, caught: &CaughtFailure<'_>) {
        let failure = caught.failure;
        let at = self
            .source
            .get(failure.span.start as usize..failure.span.end as usize)
            .unwrap_or("<outside>")
            .to_owned();
        self.caught.lock().unwrap().push((
            failure.message.clone(),
            at,
            failure.notes.clone(),
            caught.again,
        ));
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        Direct { temp_dirs: None }.temp_dir()
    }
}

/// Runs [`VM`] and then `text`, a lone file, as plain GCSL, its effects
/// through a [`Plainly`], in the names of its workspace: what it came to,
/// and the host.
fn hosted(text: &str) -> (Result<Value, Diagnostic>, Plainly) {
    let app = format!("{VM}{text}");
    let host = Plainly {
        source: app.clone(),
        ..Plainly::default()
    };
    (run_hosted(&app, &host), host)
}

/// Runs `text`, a lone file, as plain GCSL, its effects through `effects`,
/// in the names of its workspace, once the standard files have run.
fn run_hosted(text: &str, effects: &dyn Effects) -> Result<Value, Diagnostic> {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap_or_else(|errors| panic!("{errors:?}"));
    for file in 0..count(loaded.len()) {
        workspace.run(file).unwrap();
    }
    let FileRun::Pending(ir) = mem::replace(&mut workspace.runs[loaded.len()], FileRun::Succeeded)
    else {
        panic!("expected pending IR for the lone file");
    };
    let ir = Rc::new(*ir);
    let size = ir.frame_size() as usize;
    workspace
        .frame
        .resize(size.max(workspace.frame.len()), Value::Empty);
    let names = RefCell::new(mem::take(&mut workspace.scan.names));
    let extras = RunExtras {
        exec_result: Some(&workspace.exec_result),
        dirs: &workspace.scan.dir_texts[..],
        effects,
        names: Some(&names),
        profile: None,
    };
    eval_rooted(
        &ir,
        (&mut workspace.frame, &mut workspace.memo),
        extras,
        (None, None),
    )
}

/// The one error of checking [`VM`] and then `text`, a lone file.
fn check_error(text: &str) -> Diagnostic {
    let app = format!("{VM}{text}");
    let source = SourceFile::new("lone.gcsl", &app).unwrap();
    let errors: Vec<Diagnostic> = check_lone_with_standard(&source, "/")
        .1
        .expect_err("an error")
        .into_iter()
        .filter(|error| error.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors.into_iter().next().unwrap()
}

// ----- Plain evaluation -----

#[test]
fn plain_gcsl_asks_its_effect_context_for_an_operation() {
    let (value, host) = hosted(
        "m = boot({name = \"a\"; cpus = 4}, [network(\"lan\")])\n\
         ran = m.run(\"ls\", \"-l\")\n\
         text = m.read(\"/etc/hostname\")\n\
         stopped = m.halt()\n\
         shown = \"{m} and {show(m)}\"",
    );
    assert_eq!(
        value.unwrap().to_string(),
        "{m = <vm.machine 2>; ran = {status = 2; stdout = \"ls -l\"}; text = \"/etc/hostname of \
         2 up to 67108864\"; stopped = {}; shown = \"<vm.machine 2> and <vm.machine 2>\"}"
    );
    // In program order: the arguments first, defaults and the rest list
    // among them, the receiver before a method's own.
    assert_eq!(
        *host.asked.lock().unwrap(),
        [
            "net.make(\"lan\")",
            "vm.boot({name = \"a\"; cpus = 4}, [<Network 1>])",
            "vm.run(<vm.machine 2>, [\"ls\", \"-l\"])",
            "vm.file.read(<vm.machine 2>, \"/etc/hostname\", 67108864q)",
            "halt(<vm.machine 2>)",
        ]
    );
}

#[test]
fn a_host_with_no_operations_fails_each_at_its_call() {
    let text = format!("{VM}x = [say(\"a\")]");
    let error = run_hosted(&text, &Direct { temp_dirs: None }).unwrap_err();
    assert_eq!(error.message, "this host has no operation `say`");
    let (start, end) = (error.span.start as usize, error.span.end as usize);
    assert_eq!(&text[start..end], "say(\"a\")");
    assert!(error.notes.is_empty());
    // And so does a workspace's own run, as `gcsl run` runs a file.
    let mut workspace = rooted(&[("workspace.gcsl", ""), ("app.gcsl", &text)]).unwrap();
    let index = workspace.file("app.gcsl").unwrap();
    let errors = workspace.run(index).unwrap_err();
    let messages: Vec<&str> = errors.iter().map(|error| error.message.as_str()).collect();
    assert_eq!(messages, ["this host has no operation `say`"]);
}

#[test]
fn an_operation_s_failure_is_raised_at_its_call_and_caught_as_any_is() {
    let (value, host) = hosted(
        "fn guarded() -> String { recover { fail(\"inner\") } else { raise } }\n\
         a = recover { fail(\"the guest is gone\") } else (failure) {\n\
         \x20   \"{failure.message}: {failure.notes}\"\n\
         }\n\
         b = recover { guarded() } else (failure) { failure.message }\n\
         c = try fail(\"quietly\")\n\
         d = say(\"after\")",
    );
    assert_eq!(
        value.unwrap().to_string(),
        "{a = \"the guest is gone: [\\\"the host said so\\\"]\"; b = \"inner\"; c = {}; d = \
         \"after\"}"
    );
    // The host is told of each, at the operation's call, with its notes;
    // one raised again is the failure it was.
    let notes = vec!["the host said so".to_owned()];
    assert_eq!(
        *host.caught.lock().unwrap(),
        [
            (
                "the guest is gone".to_owned(),
                "fail(\"the guest is gone\")".to_owned(),
                notes.clone(),
                false
            ),
            (
                "inner".to_owned(),
                "fail(\"inner\")".to_owned(),
                notes.clone(),
                false
            ),
            (
                "inner".to_owned(),
                "fail(\"inner\")".to_owned(),
                notes.clone(),
                true
            ),
            (
                "quietly".to_owned(),
                "fail(\"quietly\")".to_owned(),
                notes,
                false
            ),
        ]
    );
    // Uncaught, it ends the run where the operation was called.
    let (value, host) = hosted("x = 1\ny = [fail(\"down\")]");
    let error = value.unwrap_err();
    assert_eq!(error.message, "down");
    assert_eq!(error.notes, ["the host said so"]);
    let (start, end) = (error.span.start as usize, error.span.end as usize);
    assert_eq!(&host.source[start..end], "fail(\"down\")");
    assert!(error.labels.is_empty());
}

/// A host that answers every operation with one value.
#[derive(Debug)]
struct Gives(Plain);

impl Effects for Gives {
    fn operation(&self, _: &str, _: &[Plain]) -> Result<Plain, OperationFailure> {
        Ok(self.0.clone())
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

#[test]
fn a_result_of_the_wrong_type_fails_at_the_call() {
    let (value, host) = hosted("m = wrong()");
    let error = value.unwrap_err();
    assert_eq!(error.message, "the result of operation `wrong` is not a Vm");
    assert_eq!(
        error.label.as_deref(),
        Some("it is a handle of the kind `vm.disk`")
    );
    let (start, end) = (error.span.start as usize, error.span.end as usize);
    assert_eq!(&host.source[start..end], "wrong()");
    // Deep in what is given, too, and for every shape.
    let record = |status: Plain| {
        Plain::Record(vec![
            ("status".to_owned(), status),
            ("stdout".to_owned(), Plain::Str(String::new())),
        ])
    };
    for (declared, given, message, label) in [
        (
            "Int",
            Plain::Str("3".to_owned()),
            "the result of operation `f` is not an Int",
            "it is a String",
        ),
        (
            "{status: Int, stdout: String}",
            record(Plain::Str("0".to_owned())),
            "the result of operation `f` is not a `{status: Int, stdout: String}`",
            "it is a record, and its field `status` is a String",
        ),
        (
            "List<Int>",
            Plain::List(vec![Plain::Int(1), Plain::Bool(true)]),
            "the result of operation `f` is not a `List<Int>`",
            "it is a list, and its element 1 is a Bool",
        ),
        (
            "{}",
            Plain::Int(1),
            "the result of operation `f` is not a `{}`",
            "it is an Int",
        ),
        (
            "Vm",
            Plain::Int(1),
            "the result of operation `f` is not a Vm",
            "it is an Int",
        ),
        (
            "Int | String",
            Plain::Empty,
            "the result of operation `f` is not an `Int | String`",
            "it is a record",
        ),
    ] {
        let text = format!("extern type Vm\nextern fn f() -> {declared}\nx = f()");
        let error = run_hosted(&text, &Gives(given)).unwrap_err();
        assert_eq!(error.message, message, "{declared}");
        assert_eq!(error.label.as_deref(), Some(label), "{declared}");
    }
    // What fits is the value, a record with more than is declared too.
    for (declared, given, shown) in [
        ("{}", Plain::Empty, "{x = {}}"),
        ("{}", Plain::Record(Vec::new()), "{x = {}}"),
        ("Int | String", Plain::Int(3), "{x = 3}"),
        (
            "List<Vm>",
            Plain::List(vec![Plain::handle("Vm", 7)]),
            "{x = [<Vm 7>]}",
        ),
        (
            "Size",
            Plain::Quantity(crate::value::Quantity::Size, 1024),
            "{x = 1KiB}",
        ),
    ] {
        let text = format!("extern type Vm\nextern fn f() -> {declared}\nx = f()");
        let value = run_hosted(&text, &Gives(given)).unwrap();
        assert_eq!(value.to_string(), shown, "{declared}");
    }
}

#[test]
fn an_operation_is_a_function_value_like_any_other() {
    let (value, host) = hosted(
        "fn twice(f: fn(String) -> String, word: String) -> String { f(word) + f(word) }\n\
         let held = say\n\
         let table = {speak = say}\n\
         a = twice(say, \"x\")\n\
         b = held(\"y\")\n\
         c = table.speak(word = \"z\")\n\
         d = for n in [1, 2, 3] { number(n) }\n\
         e = \"{say}\"",
    );
    assert_eq!(
        value.unwrap().to_string(),
        "{a = \"xx\"; b = \"y\"; c = \"z\"; d = [1, 2, 3]; e = \"fn(word: String | Future<String>) -> String\"}"
    );
    assert_eq!(
        *host.asked.lock().unwrap(),
        [
            "say(\"x\")",
            "say(\"x\")",
            "say(\"y\")",
            "say(\"z\")",
            "number(1)",
            "number(2)",
            "number(3)"
        ]
    );
}

#[test]
fn a_context_parameter_is_an_argument_its_call_reads() {
    let (value, host) = hosted(
        "fn made() -> String { build(\"a\") }\n\
         plain = made()\n\
         bound = made() with { arch = \"aarch64\" }\n\
         given = build(\"b\", arch = \"riscv64\")",
    );
    assert_eq!(
        value.unwrap().to_string(),
        "{plain = \"a for x86_64\"; bound = \"a for aarch64\"; given = \"b for riscv64\"}"
    );
    assert_eq!(
        *host.asked.lock().unwrap(),
        [
            "build(\"a\", \"x86_64\")",
            "build(\"a\", \"aarch64\")",
            "build(\"b\", \"riscv64\")"
        ]
    );
}

#[test]
fn an_operation_s_arguments_are_awaited_before_the_host_is_asked() {
    let (value, host) = hosted(
        "word = lazy say(\"first\")\n\
         p = lazy { name = \"late\"; cpus = 2 }\n\
         m = boot(p)\n\
         echoed = say(word)\n\
         all = m.run(...[lazy \"a\", \"b\"])",
    );
    value.unwrap();
    assert_eq!(
        *host.asked.lock().unwrap(),
        [
            "vm.boot({name = \"late\"; cpus = 2}, [])",
            "say(\"first\")",
            "say(\"first\")",
            "vm.run(<vm.machine 1>, [\"a\", \"b\"])",
        ]
    );
}

#[test]
fn a_memoised_function_runs_its_operation_once_for_each_list_of_arguments() {
    let (value, host) = hosted(
        "fn fixture(name: String) ->> Vm { boot({name = name}) }\n\
         fn of(m: Vm) ->> String { say(\"{m}\") }\n\
         a = fixture(\"x\")\n\
         b = fixture(\"x\")\n\
         c = fixture(\"y\")\n\
         same = a == b\n\
         other = a == c\n\
         total = a === b and not (a === c)\n\
         shown = [of(a), of(b), of(c)]",
    );
    assert_eq!(
        value.unwrap().to_string(),
        "{a = <vm.machine 1>; b = <vm.machine 1>; c = <vm.machine 2>; same = true; other = \
         false; total = true; shown = [\"<vm.machine 1>\", \"<vm.machine 1>\", \"<vm.machine \
         2>\"]}"
    );
    // A handle is a key of a memoised call, by its kind and number.
    assert_eq!(
        *host.asked.lock().unwrap(),
        [
            "vm.boot({name = \"x\"; cpus = 1}, [])",
            "vm.boot({name = \"y\"; cpus = 1}, [])",
            "say(\"<vm.machine 1>\")",
            "say(\"<vm.machine 2>\")",
        ]
    );
}

#[test]
fn handles_are_data_told_apart_by_their_types() {
    let (value, _) = hosted(
        "type Either = Vm | Network | Int\n\
         fn name(x: Either): String {\n\
         \x20   if x is Vm { \"vm\" } else if x is Network { \"network\" } else { \"int\" }\n\
         }\n\
         m = boot({name = \"a\"})\n\
         n = network(\"lan\")\n\
         kinds = [name(m), name(n), name(3)]\n\
         held = {machines = [m]; any: {*: *} = {m = m; n = n}}\n\
         back = (held.any as {m: Vm}).m.run(\"x\").status\n\
         differ = (m : Either) === (n : Either)",
    );
    assert_eq!(
        value.unwrap().to_string(),
        "{m = <vm.machine 1>; n = <Network 2>; kinds = [\"vm\", \"network\", \"int\"]; held = \
         {machines = [<vm.machine 1>]; any = {m = <vm.machine 1>; n = <Network 2>}}; back = 1; \
         differ = false}"
    );
}

#[test]
fn a_handle_goes_to_plain_data_and_back() {
    let (mut workspace, _) = machines("");
    let plain = Plain::handle("vm.machine", 41);
    assert_eq!(plain.as_handle(), Some(("vm.machine", 41)));
    let value = workspace.from_plain(&plain);
    assert_eq!(value.to_string(), "<vm.machine 41>");
    assert_eq!(workspace.to_plain(&value), Ok(plain.clone()));
    assert_eq!(value, workspace.from_plain(&plain));
    assert_ne!(
        value,
        workspace.from_plain(&Plain::handle("vm.machine", 42))
    );
    assert_ne!(value, workspace.from_plain(&Plain::handle("vm.disk", 41)));
    // Inside other data, and nothing else is a handle.
    let nested = Plain::List(vec![Plain::Record(vec![("m".to_owned(), plain)])]);
    let made = workspace.from_plain(&nested);
    assert_eq!(workspace.to_plain(&made), Ok(nested));
    for other in [
        Plain::Int(41),
        Plain::Str("vm.machine".to_owned()),
        Plain::Variant {
            key: "<variant C of <enum T@a.gcsl>@a.gcsl>".to_owned(),
            of: "<enum T@a.gcsl>".to_owned(),
            shown: "T::C".to_owned(),
            payload: Some(Box::new(Plain::Int(1))),
        },
        Plain::Variant {
            key: "<handle vm.machine>".to_owned(),
            of: "<extern vm.machine>".to_owned(),
            shown: "<vm.machine>".to_owned(),
            payload: None,
        },
        Plain::Variant {
            key: "<handle vm.machine>".to_owned(),
            of: "<extern vm.disk>".to_owned(),
            shown: "<vm.machine>".to_owned(),
            payload: Some(Box::new(Plain::Int(1))),
        },
    ] {
        assert_eq!(other.as_handle(), None, "{other:?}");
    }
}

/// A host whose operation `flaky` fails until it has been asked
/// `after` times, and whose `slow` takes a while.
#[derive(Debug)]
struct Flaky {
    after: usize,
    asked: RefCell<usize>,
}
impl Effects for Flaky {
    fn operation(&self, name: &str, _: &[Plain]) -> Result<Plain, OperationFailure> {
        if name == "slow" {
            std::thread::sleep(std::time::Duration::from_millis(60));
            return Ok(Plain::Int(0));
        }
        *self.asked.borrow_mut() += 1;
        if *self.asked.borrow() < self.after {
            return Err(OperationFailure::new("not yet"));
        }
        Ok(Plain::Int(i64::try_from(*self.asked.borrow()).unwrap()))
    }
    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

#[test]
fn retry_and_timeout_apply_around_an_operation() {
    let text = "extern fn flaky() -> Int\n\
                extern fn slow() -> Int\n\
                #[retry(2)]\n\
                fn patient() -> Int { flaky() }\n\
                #[timeout(20ms)]\n\
                fn hurried() -> Int { slow() + slow() }\n";
    let host = Flaky {
        after: 3,
        asked: RefCell::new(0),
    };
    let value = run_hosted(&format!("{text}x = patient()"), &host).unwrap();
    assert_eq!(value.to_string(), "{x = 3}");
    // One attempt too few, and the last failure is the call's.
    let host = Flaky {
        after: 5,
        asked: RefCell::new(0),
    };
    let error = run_hosted(&format!("{text}x = patient()"), &host).unwrap_err();
    assert_eq!(error.message, "not yet");
    assert_eq!(*host.asked.borrow(), 3);
    // A deadline passes between two operations: the second is never asked.
    let error = run_hosted(&format!("{text}x = hurried()"), &host).unwrap_err();
    assert!(error.message.contains("timed out"), "{}", error.message);
}

// ----- Checking, where a workspace is needed -----

#[test]
fn a_handle_kind_is_declared_once_in_a_workspace() {
    let errors = rooted(&[
        ("workspace.gcsl", ""),
        ("a.gcsl", "extern type Vm = \"vm.machine\""),
        (
            "b/c.gcsl",
            "extern type Machine = \"vm.machine\"\nextern type Other",
        ),
    ])
    .unwrap_err();
    let errors: Vec<&Diagnostic> = errors
        .iter()
        .filter(|error| error.severity == Severity::Error)
        .collect();
    let [error] = errors[..] else {
        panic!("{errors:?}");
    };
    assert_eq!(error.message, "handle kind `vm.machine` is declared twice");
    assert_eq!(error.label.as_deref(), Some("declared here"));
    assert_eq!(error.span, Span::new(12, 19));
    let labels: Vec<(Option<u32>, Span, &str)> = error
        .labels
        .iter()
        .map(|label| (label.input, label.span, label.text.as_str()))
        .collect();
    assert_eq!(labels.len(), 1);
    assert_eq!(
        (labels[0].1, labels[0].2),
        (Span::new(12, 14), "first declared here")
    );
    assert_ne!(labels[0].0, error.input);
    assert_eq!(
        error.notes,
        [
            "two handle types of one kind would be one type under two names: declare a handle type \
          once and import it where it is used"
        ]
    );
    // In one file too, under two names.
    let error = check_error("extern type Machine = \"vm.machine\"");
    assert_eq!(error.message, "handle kind `vm.machine` is declared twice");
    // Declared once and imported, it is one type everywhere.
    let mut workspace = rooted(&[
        ("workspace.gcsl", ""),
        (
            "lib/vm.gcsl",
            "extern type Vm = \"vm.machine\"\n\
             extern def start(name: String) -> Vm = \"vm.start\"\n\
             extern def (m: Vm) name() -> String = \"vm.name\"",
        ),
        (
            "app.gcsl",
            "import Vm, start from lib/vm\n\
             fn keep(m: Vm): List<Vm> { [m] }\n\
             kept = fn() -> List<Vm> { keep(start(\"a\")) }",
        ),
    ])
    .unwrap();
    assert!(workspace.file("app.gcsl").is_some());
    let index = workspace.file("app.gcsl").unwrap();
    workspace.run(index).unwrap();
}

#[test]
fn no_program_makes_or_opens_a_handle() {
    for (text, message) in [
        // No variant is declared for a handle type.
        (
            "enum Vm::Fake<Int>",
            "`Vm` is a handle type: only the host makes its values",
        ),
        // Nor is it anything else's value.
        ("m: Vm = 3", "mismatched types"),
        ("m = 3 as Vm", "this value is never a Vm"),
        ("m: Vm = {}", "mismatched types"),
        ("m = Vm(3)", "undefined variable `Vm`"),
        ("m = Vm::Handle(3)", "no variant `Handle` of `Vm`"),
        (
            "import json\nm: Vm = json.decode(\"3\")",
            "cannot decode JSON as a Vm",
        ),
        // A handle of one kind is none of another.
        ("n: Network = boot({name = \"a\"})", "mismatched types"),
        // Its number is the host's.
        ("fn id(m: Vm): Int { m }", "mismatched types"),
        (
            "fn id(m: Vm): Int { m.payload }",
            "cannot access field `payload` of Vm",
        ),
        ("fn id(m: Vm): Int { m as Int }", "`m` is never an Int"),
    ] {
        let error = check_error(text);
        assert_eq!(error.message, message, "{text}");
    }
    let error = check_error("enum Vm::Fake<Int>");
    let at = VM.find("Vm = \"vm.machine\"").unwrap();
    let at = u32::try_from(at).unwrap();
    let labels: Vec<(Span, &str)> = error
        .labels
        .iter()
        .map(|label| (label.span, label.text.as_str()))
        .collect();
    assert_eq!(labels, [(Span::new(at, at + 2), "declared `extern` here")]);
    assert_eq!(
        error.notes,
        ["an operation of the host's gives a handle; no variant is declared for one"]
    );
}

// ----- Graph mode -----

#[test]
fn operations_arrive_in_program_order_across_steps_and_blocks() {
    let (mut workspace, mut engine) = machines(
        "fn step(name: String, body: block -> {}) -> {} {\n\
         \x20   say(\"begin {name}\")\n\
         \x20   body()\n\
         \x20   say(\"end {name}\")\n\
         \x20   = {}\n\
         }\n\
         def test one() {\n\
         \x20   let m = boot({name = \"a\"})\n\
         \x20   let mut seen = \"\"\n\
         \x20   step(\"run\") {\n\
         \x20       seen := seen + m.run(\"ls\", \"-l\").stdout\n\
         \x20       seen := seen + say(\" then \")\n\
         \x20   }\n\
         \x20   step(\"stop\") { m.halt() }\n\
         \x20   = \"{seen}{m.read(\"/x\")}\"\n\
         }",
    );
    let results = engine.drive(&mut workspace, &["one"]);
    assert_eq!(
        results,
        [Ok("ls -l then /x of 1 up to 67108864".to_owned())]
    );
    assert_eq!(
        engine.asked,
        [
            "vm.boot({name = \"a\"; cpus = 1}, [])",
            "say(\"begin run\")",
            "vm.run(<vm.machine 1>, [\"ls\", \"-l\"])",
            "say(\" then \")",
            "say(\"end run\")",
            "say(\"begin stop\")",
            "halt(<vm.machine 1>)",
            "say(\"end stop\")",
            "vm.file.read(<vm.machine 1>, \"/x\", 67108864q)",
        ]
    );
    // Each is located where the program called the operation.
    assert_eq!(
        engine.called,
        [
            "boot({name = \"a\"})",
            "say(\"begin {name}\")",
            "m.run(\"ls\", \"-l\")",
            "say(\" then \")",
            "say(\"end {name}\")",
            "say(\"begin {name}\")",
            "m.halt()",
            "say(\"end {name}\")",
            "m.read(\"/x\")",
        ]
    );
}

#[test]
fn an_operation_s_node_says_what_it_asks() {
    let (mut workspace, _) = machines(
        "def test one() {\n\
         \x20   let lan = network(\"lan\")\n\
         \x20   = \"{boot({name = \"a\"; cpus = 2}, [lan, lan])}\"\n\
         }",
    );
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_prepared(&call, &mut graph, every()).unwrap()
    else {
        panic!("the unit did not stop at its first operation");
    };
    let operation = workspace.operation(&graph, nodes[0]).unwrap();
    assert_eq!(operation.name, "net.make");
    assert_eq!(operation.args, [Plain::Str("lan".to_owned())]);
    assert_eq!(operation.input, workspace.file("app.gcsl").unwrap());
    // It is an effect's node like any other.
    let node = graph.node(nodes[0]).unwrap();
    assert_eq!(
        node.kind(),
        crate::eval::graph::NodeKind::Effect(crate::check::Builtin::Operation)
    );
    assert_eq!(node.called(), (operation.input, operation.span));
    let lan = workspace.from_plain(&Plain::handle("Network", 5));
    assert!(graph.resolve(nodes[0], Ok(lan)));
    let Progress::Paused(nodes) = workspace.run_prepared(&call, &mut graph, every()).unwrap()
    else {
        panic!("the unit did not stop at its second operation");
    };
    let operation = workspace.operation(&graph, nodes[0]).unwrap();
    assert_eq!(operation.name, "vm.boot");
    let lan = Plain::handle("Network", 5);
    assert_eq!(
        operation.args,
        [
            Plain::Record(vec![
                ("name".to_owned(), Plain::Str("a".to_owned())),
                ("cpus".to_owned(), Plain::Int(2)),
            ]),
            Plain::List(vec![lan.clone(), lan]),
        ]
    );
    // No other node is an operation's, nor one the graph lacks.
    assert!(workspace.operation(&Graph::new(), nodes[0]).is_none());
    let vm = workspace.from_plain(&Plain::handle("vm.machine", 6));
    assert!(graph.resolve(nodes[0], Ok(vm)));
    assert_eq!(
        workspace.run_prepared(&call, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(Rc::from("<vm.machine 6>")))
    );
}

#[test]
fn an_operation_s_job_asks_the_effect_context_it_is_run_under() {
    let (mut workspace, _) = machines(
        "def test one() {\n\
         \x20   = \"{boot({name = \"a\"}).run(\"x\").stdout} {exec([\"true\"], panic = false).status}\"\n\
         }",
    );
    let host = Plainly::default();
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let mut operations = 0;
    let done = loop {
        match workspace.run_prepared(&call, &mut graph, every()).unwrap() {
            Progress::Done(value) => break value,
            Progress::Paused(nodes) => {
                let job = workspace.effect_job(&graph, nodes[0]).unwrap();
                operations += usize::from(job.builtin() == crate::check::Builtin::Operation);
                let plain = job.run(&host, None).unwrap();
                let value = workspace.from_plain(&plain);
                assert!(graph.resolve(nodes[0], Ok(value)));
            }
            Progress::Waiting(_) => panic!("nothing to wait for"),
        }
    };
    assert_eq!(done.to_string(), "x 0");
    assert_eq!(operations, 2);
    assert_eq!(
        *host.asked.lock().unwrap(),
        [
            "vm.boot({name = \"a\"; cpus = 1}, [])",
            "vm.run(<vm.machine 1>, [\"x\"])"
        ]
    );
    // A job's failure is the operation's, with its notes.
    let (mut workspace, _) = machines("def test one() { = fail(\"no\") }");
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_prepared(&call, &mut graph, every()).unwrap()
    else {
        panic!("the unit did not stop at its operation");
    };
    let job = workspace.effect_job(&graph, nodes[0]).unwrap();
    let error = job.run(&host, None).unwrap_err();
    assert_eq!(
        (error.message.as_str(), &error.notes[..]),
        ("no", &["the host said so".to_owned()][..])
    );
    // And a node run as plain GCSL runs it asks the context it is given.
    let value = workspace
        .evaluate(&graph, nodes[0], &Direct { temp_dirs: None })
        .unwrap_err();
    assert_eq!(value.message, "this host has no operation `fail`");
}

#[test]
fn units_interleave_each_in_its_own_order() {
    let (mut workspace, mut engine) = machines(
        "fn work(name: String) -> String {\n\
         \x20   let m = boot({name = name})\n\
         \x20   let a = m.run(name, \"1\").stdout\n\
         \x20   let b = m.run(name, \"2\").stdout\n\
         \x20   m.halt()\n\
         \x20   = \"{a},{b}\"\n\
         }\n\
         def test one() { = work(\"one\") }\n\
         def test two() { = work(\"two\") }\n\
         def test three() { = say(\"three\") }",
    );
    let results = engine.drive(&mut workspace, &["one", "two", "three"]);
    assert_eq!(
        results,
        [
            Ok("one 1,one 2".to_owned()),
            Ok("two 1,two 2".to_owned()),
            Ok("three".to_owned())
        ]
    );
    // Taken in turn, one operation each: every unit's own are in its
    // program's order, on its own machine.
    assert_eq!(
        engine.asked,
        [
            "vm.boot({name = \"one\"; cpus = 1}, [])",
            "vm.boot({name = \"two\"; cpus = 1}, [])",
            "say(\"three\")",
            "vm.run(<vm.machine 1>, [\"one\", \"1\"])",
            "vm.run(<vm.machine 2>, [\"two\", \"1\"])",
            "vm.run(<vm.machine 1>, [\"one\", \"2\"])",
            "vm.run(<vm.machine 2>, [\"two\", \"2\"])",
            "halt(<vm.machine 1>)",
            "halt(<vm.machine 2>)",
        ]
    );
}

#[test]
fn a_host_s_failure_in_graph_mode_is_raised_at_the_call() {
    let (mut workspace, mut engine) = machines(
        "fn step(body: block -> {}) -> {} { recover { body() } else { raise } }\n\
         def test plain() {\n\
         \x20   let a = say(\"a\")\n\
         \x20   let b = fail(\"the guest is gone\")\n\
         \x20   = a + b + say(\"never\")\n\
         }\n\
         def test caught() {\n\
         \x20   let a = say(\"a\")\n\
         \x20   let b = recover {\n\
         \x20       step() { fail(\"the guest is gone\") }\n\
         \x20       = \"passed\"\n\
         \x20   } else (failure) { \"{failure.message}: {failure.notes}\" }\n\
         \x20   = \"{a} {b} {say(\"c\")}\"\n\
         }",
    );
    let host = Plainly {
        source: engine.source.clone(),
        ..Plainly::default()
    };
    let never = |_: &[crate::check::Attribute]| false;
    let graph_host = GraphHost {
        defers: &never,
        effects: &host,
    };
    let results = engine.drive_through(&mut workspace, &["plain", "caught"], graph_host);
    assert_eq!(
        results,
        [
            Err("the guest is gone".to_owned()),
            Ok("a the guest is gone: [\"the host said so\"] c".to_owned())
        ]
    );
    // The uncaught one ended its unit: nothing after it was asked for.
    assert!(!engine.asked.contains(&"say(\"never\")".to_owned()));
    // The host is told of the caught one, in the step and then in the
    // test, at the operation's call, and never asked through its context.
    let notes = vec!["the host said so".to_owned()];
    let at = "fail(\"the guest is gone\")".to_owned();
    assert_eq!(
        *host.caught.lock().unwrap(),
        [
            (
                "the guest is gone".to_owned(),
                at.clone(),
                notes.clone(),
                false
            ),
            ("the guest is gone".to_owned(), at, notes, true),
        ]
    );
    assert!(host.asked.lock().unwrap().is_empty());
}

#[test]
fn an_uncaught_failure_in_graph_mode_is_located_at_the_operation_s_call() {
    let (mut workspace, engine) = machines(
        "def test one() {\n\
         \x20   = say(\"a\") + fail(\"down\")\n\
         }",
    );
    let call = workspace.prepare_call(("test", "one"), &[], &[]).unwrap();
    let mut graph = Graph::new();
    let Progress::Paused(nodes) = workspace.run_prepared(&call, &mut graph, every()).unwrap()
    else {
        panic!("the unit did not stop at its first operation");
    };
    let said = workspace.from_plain(&Plain::Str("a".to_owned()));
    assert!(graph.resolve(nodes[0], Ok(said)));
    let Progress::Paused(nodes) = workspace.run_prepared(&call, &mut graph, every()).unwrap()
    else {
        panic!("the unit did not stop at its second operation");
    };
    // Whatever place the host's diagnostic names, and whatever it labels.
    let down = Diagnostic::error("down", Span::new(3, 9))
        .with_label("somewhere")
        .with_context(Span::new(0, 2), "elsewhere")
        .with_note("vm 3")
        .in_input(7);
    assert!(graph.resolve(nodes[0], Err(down)));
    let Err(InvokeError::Diagnostics(errors)) = workspace.run_prepared(&call, &mut graph, every())
    else {
        panic!("the unit did not fail");
    };
    let error = errors.iter().find(|error| error.message == "down").unwrap();
    let (start, end) = (error.span.start as usize, error.span.end as usize);
    assert_eq!(&engine.source[start..end], "fail(\"down\")");
    assert_eq!(error.input, Some(engine.file));
    assert_eq!(error.notes, ["vm 3"]);
    assert_eq!(error.label, None);
    assert!(error.labels.iter().all(|label| label.text != "elsewhere"));
}

#[test]
fn a_result_of_the_wrong_type_in_graph_mode_fails_at_the_call() {
    let (mut workspace, mut engine) = machines(
        "def test one() {\n\
         \x20   let m = recover { wrong() } else (failure) { boot({name = failure.message}) }\n\
         \x20   = \"{m}\"\n\
         }\n\
         def test two() { = \"{wrong()}\" }",
    );
    let results = engine.drive(&mut workspace, &["one", "two"]);
    assert_eq!(
        results,
        [
            Ok("<vm.machine 1>".to_owned()),
            Err("the result of operation `wrong` is not a Vm".to_owned())
        ]
    );
    assert_eq!(
        engine.asked,
        [
            "wrong()",
            "wrong()",
            "vm.boot({name = \"the result of operation `wrong` is not a Vm\"; cpus = 1}, [])"
        ]
    );
}

#[test]
fn a_memoised_function_s_operation_is_run_once_for_two_units() {
    let (mut workspace, mut engine) = machines(
        "fn fixture() ->> Vm { boot({name = \"shared\"}) }\n\
         def test one() { = fixture().run(\"one\").stdout }\n\
         def test two() { = fixture().run(\"two\").stdout }",
    );
    let results = engine.drive(&mut workspace, &["one", "two"]);
    assert_eq!(results, [Ok("one".to_owned()), Ok("two".to_owned())]);
    assert_eq!(
        engine.asked,
        [
            "vm.boot({name = \"shared\"; cpus = 1}, [])",
            "vm.run(<vm.machine 1>, [\"one\"])",
            "vm.run(<vm.machine 1>, [\"two\"])",
        ]
    );
}

#[test]
fn many_operations_in_a_row_carry_their_state() {
    // The spike's `sequential`: one unit, an operation each iteration.
    let (mut workspace, mut engine) = machines(
        "def test seq() {\n\
         \x20   let mut total = 0\n\
         \x20   for i in 0..300 { total := total + number(i) }\n\
         \x20   = \"{total}\"\n\
         }",
    );
    let results = engine.drive(&mut workspace, &["seq"]);
    assert_eq!(results, [Ok((300 * 299 / 2).to_string())]);
    assert_eq!(engine.asked.len(), 300);
    let in_order = engine
        .asked
        .iter()
        .enumerate()
        .all(|(at, asked)| *asked == format!("number({at})"));
    assert!(in_order);
}

#[test]
fn many_units_share_a_fixture_and_keep_their_own_order() {
    // The spike's `many`: each test with a step and operations of its own
    // number, all sharing one fixture, built once.
    let tests = 40;
    let each = 6;
    let mut text = String::from(
        "fn fixture() ->> Int { number(7) }\n\
         fn step(name: String, body: block -> Int) -> Int { body() }\n",
    );
    for test in 1..=tests {
        write!(
            text,
            "def test t{test}() {{\n\
             \x20   let f = fixture()\n\
             \x20   let a = step(\"first\") {{\n\
             \x20       let mut s = 0\n\
             \x20       for i in 0..3 {{ s := s + number({test} * 1000 + i) }}\n\
             \x20       = s\n\
             \x20   }}\n\
             \x20   let mut total = 0\n\
             \x20   for i in 3..{each} {{ total := total + number({test} * 1000 + i) }}\n\
             \x20   = \"{{a + total + f}}\"\n\
             }}\n"
        )
        .unwrap();
    }
    let (mut workspace, mut engine) = machines(&text);
    let names: Vec<String> = (1..=tests).map(|test| format!("t{test}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let results = engine.drive(&mut workspace, &names);
    for (at, result) in results.iter().enumerate() {
        let number = i64::try_from(at).unwrap() + 1;
        let want = number * 1000 * each + (0..each).sum::<i64>() + 7;
        assert_eq!(*result, Ok(want.to_string()), "t{number}");
    }
    // The fixture once, and each test's operations in its own order.
    assert_eq!(
        engine.asked.len(),
        usize::try_from(tests * each).unwrap() + 1
    );
    assert_eq!(
        engine
            .asked
            .iter()
            .filter(|asked| *asked == "number(7)")
            .count(),
        1
    );
    for test in 1..=tests {
        let own: Vec<&String> = engine
            .asked
            .iter()
            .filter(|asked| {
                let number: i64 = asked["number(".len()..asked.len() - 1].parse().unwrap();
                number / 1000 == test
            })
            .collect();
        let want: Vec<String> = (0..each)
            .map(|i| format!("number({})", test * 1000 + i))
            .collect();
        assert_eq!(own, want.iter().collect::<Vec<_>>(), "t{test}");
    }
}

#[test]
fn a_failed_operation_ends_its_unit_or_is_caught_and_the_unit_goes_on() {
    // The spike's `failure`: the second operation of each fails.
    let (mut workspace, mut engine) = machines(
        "fn op(n: Int) -> Int {\n\
         \x20   let gone = if n == 1 { fail(\"the guest is gone\") } else { \"\" }\n\
         \x20   = number(n)\n\
         }\n\
         def test plain() {\n\
         \x20   let a = op(0)\n\
         \x20   let b = op(1)\n\
         \x20   let c = op(2)\n\
         \x20   = \"{a + b + c}\"\n\
         }\n\
         def test caught() {\n\
         \x20   let a = op(0)\n\
         \x20   let b = recover { op(1) } else (f) { 100 }\n\
         \x20   let c = op(2)\n\
         \x20   = \"{a + b + c}\"\n\
         }",
    );
    let results = engine.drive(&mut workspace, &["plain", "caught"]);
    assert_eq!(
        results,
        [Err("the guest is gone".to_owned()), Ok("102".to_owned())]
    );
    assert_eq!(
        engine.asked,
        [
            "number(0)",
            "number(0)",
            "fail(\"the guest is gone\")",
            "fail(\"the guest is gone\")",
            "number(2)"
        ]
    );
}

#[test]
fn retry_and_timeout_wrap_an_operation_in_graph_mode() {
    let (mut workspace, mut engine) = machines(
        "#[retry(1)]\n\
         fn again() -> String { say(\"try\") + fail(\"once\") }\n\
         #[timeout(1h)]\n\
         fn timed() -> String { say(\"in time\") }\n\
         def test one() { = timed() + (recover { again() } else (f) { \" \" + f.message }) }",
    );
    let results = engine.drive(&mut workspace, &["one"]);
    assert_eq!(results, [Ok("in time once".to_owned())]);
    // The body ran twice, its operations asked for each time.
    assert_eq!(
        engine.asked,
        [
            "say(\"in time\")",
            "say(\"try\")",
            "fail(\"once\")",
            "say(\"try\")",
            "fail(\"once\")"
        ]
    );
}
