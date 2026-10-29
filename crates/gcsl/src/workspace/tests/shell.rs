//! Tests of workspaces: `exec`, `panic`, `sh` and their live forms.

use super::*;

/// The summary of slice 29, its `ls` of a directory every machine has.
pub(super) const SHELL_SUMMARY: &str = "r = exec([\"ls\", \"-1d\", \"/\"])\n\
                                 lines = r.stdout\n\
                                 ok = r.status == 0\n\
                                 out = sh(\"echo hi\")\n\
                                 fn pkg(name: String): { build: Future<String> } {\n\
                                     { build = lazy { sh(\"echo make {name}\") } }\n\
                                 }\n\
                                 built = await pkg(\"lib\").build\n\
                                 fn guard(n: Int): Int {\n\
                                     if n < 0 { panic(\"negative size {n}\") }\n\
                                     n\n\
                                 }\n\
                                 size = guard(3)";

/// The one diagnostic of running `text` as a lone file.
pub(super) fn lone_error(text: &str) -> Diagnostic {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let mut errors = crate::run(&source).unwrap_err();
    assert_eq!(errors.len(), 1, "{text}: {errors:?}");
    errors.remove(0)
}

/// The input of `main.gcsl` in a workspace of it and `workspace.gcsl`: the
/// prelude is input 0.
const MAIN: u32 = 1;

/// The type of the prelude's `exec` and `exec_live`, as a value shows it.
const EXEC_TYPE: &str = "fn(argv: List<String>, cwd: String, env: {*: String}, interactive: Bool, \
                         panic: Bool, until: String) -> {status: Int, stderr: String, stdout: \
                         String}";
#[test]
fn the_shell_summary_runs_in_a_workspace_and_alone() {
    let expected = "{r = {status = 0; stdout = \"/\\n\"; stderr = \"\"}; lines = \"/\\n\"; \
                        ok = true; out = \"hi\\n\"; built = \"make lib\\n\"; size = 3}";
    assert_eq!(lone(SHELL_SUMMARY).unwrap(), expected);
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", SHELL_SUMMARY)];
    assert_eq!(value(&files, "app/main.gcsl"), expected);
    assert_eq!(
        lone_error(
            "fn guard(n: Int): Int {\nif n < 0 { panic(\"negative size {n}\") }\nn\n}\n= guard(-2)"
        )
        .message,
        "negative size -2"
    );
}

#[test]
fn exec_reports_what_its_program_did() {
    for (text, expected) in [
        (
            "= exec([\"true\"])",
            "{status = 0; stdout = \"\"; stderr = \"\"}",
        ),
        // With `panic = false`, a non-zero status is data, not an error.
        (
            "= exec([\"false\"], panic = false)",
            "{status = 1; stdout = \"\"; stderr = \"\"}",
        ),
        (
            "= exec([\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"], panic = false)",
            "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
        ),
        ("= exec([\"false\"], panic = false).status != 0", "true"),
        // Killed by a signal: 128 plus its number.
        (
            "= exec([\"sh\", \"-c\", \"kill -9 $$\"], panic = false).status",
            "137",
        ),
        // Its standard input is the null device: `cat` ends at once.
        (
            "= exec([\"cat\"])",
            "{status = 0; stdout = \"\"; stderr = \"\"}",
        ),
        // Invalid UTF-8 is replaced.
        (
            "= exec([\"sh\", \"-c\", \"printf '\\\\377ok'\"]).stdout",
            "\u{FFFD}ok",
        ),
        // The arguments are passed as they are, with no shell.
        (
            "= exec([\"printf\", \"%s|\", \"a b\", \"$HOME\", \"*\"]).stdout",
            "a b|$HOME|*|",
        ),
        // Named, through a variable, a field, a `->` function. A call
        // through a value gives every argument: only a call by the
        // function's own name uses its defaults.
        ("= exec(argv = [\"echo\", \"n\"]).stdout", "n\n"),
        (
            "let e = exec\n= e([\"echo\", \"v\"], \"\", {}, false, false, \"\").stdout",
            "v\n",
        ),
        (
            "let r = { run = exec }\n= r.run([\"echo\", \"f\"], \"\", {}, false, false, \"\").stdout",
            "f\n",
        ),
        (
            "fn run(a: String) -> String { exec([\"echo\", a]).stdout }\n= run(\"i\")",
            "i\n",
        ),
        (
            "import builtins\n\
                 type Options = {cwd: String, env: {*: String}, interactive: Bool, panic: Bool, \
                 until: String}\n\
                 fn pick(f: fn(argv: Present<List<String>>, options: Present<Options>) -> \
                 {status: Int}): fn(Present<List<String>>, Present<Options>) -> {status: Int} \
                 { f }\n\
                 = pick(builtins.exec)([\"false\"], {cwd = \"\"; env = {}; interactive = false; \
                 panic = false; until = \"\"}).status",
            "1",
        ),
        (
            "import exec as run from builtins\n= run([\"true\"], {cwd = \"\"; env = {}; interactive = false; panic = false; until = \"\"}).status",
            "0",
        ),
        // A future argument is awaited, as any call's.
        (
            "let xs = lazy [\"echo\", \"later\"]\n= exec(xs).stdout",
            "later\n",
        ),
        (
            "let xs = [lazy \"echo\", \"deep\"]\n= exec(xs).stdout",
            "deep\n",
        ),
        // The value shows as its type.
        ("= exec", EXEC_TYPE),
        ("= panic", "fn(message: String): Never"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
}

#[test]
fn exec_fails_only_when_its_program_cannot_run() {
    let text = "let none: List<String> = []\nr = exec(none)";
    let error = lone_error(text);
    assert_eq!(error.message, "`exec` needs a program to run");
    assert_eq!(error.span, Span::new(32, 42));
    assert!(error.notes.is_empty());
    let text = "r = exec([\"gcsl-no-such-program-slice-29\", \"x\"])";
    let error = lone_error(text);
    assert_eq!(error.message, "cannot run `gcsl-no-such-program-slice-29`");
    assert_eq!(error.span, Span::new(4, 48));
    assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
    assert_eq!(error.input, None);
    // A directory is not a program.
    let error = lone_error("r = exec([\"/\"])");
    assert_eq!(error.message, "cannot run `/`");
    assert_eq!(error.notes, ["Permission denied (os error 13)"]);
}

#[test]
fn panic_ends_the_run_with_its_message_at_the_call() {
    let text = "x = 1\ny = panic(\"x\")";
    let error = lone_error(text);
    assert_eq!(error.message, "x");
    assert_eq!(error.span, Span::new(10, 20));
    assert!(error.labels.is_empty() && error.notes.is_empty());
    assert_eq!(lone_error("panic(\"one\\ntwo\")").message, "one\ntwo");
    // Wherever it stands, whichever walker meets it: a statement, a
    // `let`, a branch, an operand of each kind, an element, a field, an
    // argument, a callee, an interpolation, a function's result, a
    // `lazy`, a loop.
    for text in [
        "panic(\"p\")",
        "let x = panic(\"p\")",
        "let c = true\nx = if c { panic(\"p\") } else { 1 }",
        "x = 1 + panic(\"p\")",
        "x = 1.5 * panic(\"p\")",
        "x = true and panic(\"p\")",
        "x = \"a\" + panic(\"p\")",
        "x = \"a{panic(\"p\")}\"",
        "x = [1, panic(\"p\")]",
        "x = len(panic(\"p\"))",
        "x = { a = panic(\"p\") }",
        "let r: {a: Int} = panic(\"p\")",
        "x = float(panic(\"p\"))",
        "x = panic(\"p\")(1)",
        "fn f(n: Int): Int { if n > 0 { panic(\"p\") }\nn }\nx = f(1)",
        "fn f(): String { panic(\"p\") }\nx = f() + \"!\"",
        "x = await lazy panic(\"p\")",
        "x = for i in [1] { panic(\"p\") }",
        "def f(n: Int) ->> Int { panic(\"p\") }\nx = f(1)",
        // As a value of a parameter of type `fn(String): Int`.
        "fn apply(f: fn(String): Int): Int { f(\"p\") }\nx = apply(panic)",
        "let p = panic\nx = p(\"p\")",
    ] {
        assert_eq!(lone_error(text).message, "p", "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(messages(&files, "main.gcsl"), ["p"], "{text}");
    }
    // What follows a panic never runs.
    assert_eq!(
        lone_error("let c = true\nif c { panic(\"first\") }\n= 1 // 0").message,
        "first"
    );
    // A message of the program's own wording, built at runtime.
    assert_eq!(
        lone_error("let n = 3\n= panic(\"{n} is {n * 2 // 2}\")").message,
        "3 is 3"
    );
}

#[test]
fn sh_returns_its_output_or_panics_at_its_call() {
    assert_eq!(lone("= sh(\"echo hi\")").unwrap(), "hi\n");
    assert_eq!(lone("= sh(\"printf hi\")").unwrap(), "hi");
    // The panic is the prelude's, but `sh` is `#[track_caller]`: it is
    // located at the call, in the file.
    let text = "x = sh(\"echo nope >&2; exit 2\")";
    let error = lone_error(text);
    assert_eq!(
        error.message,
        "`echo nope >&2; exit 2` exited with status 2: nope"
    );
    assert_eq!(error.input, None);
    assert_eq!(error.span, Span::new(4, 31));
    assert!(error.labels.is_empty());
    // With no standard error, the message ends in ": ".
    assert_eq!(
        lone_error("x = sh(\"exit 2\")").message,
        "`exit 2` exited with status 2: "
    );
    let files = [("workspace.gcsl", ""), ("main.gcsl", "x = sh(\"exit 2\")")];
    assert_eq!(
        run_file(&files, "main.gcsl").unwrap_err(),
        [("`exit 2` exited with status 2: ".to_owned(), Some(MAIN))]
    );
}

#[test]
fn a_memoised_function_around_exec_runs_its_program_once() {
    let path = std::env::temp_dir().join(format!("gcsl-slice-29-{}", std::process::id()));
    let path = path.to_str().unwrap();
    let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
    let _ = std::fs::remove_file(path);
    let memo = format!(
        "fn mark(p: String) ->> Int {{ exec([\"sh\", \"-c\", \"echo x >> \" + p]).status }}\n\
             = [mark(\"{path}\"), mark(\"{path}\")]"
    );
    assert_eq!(lone(&memo).unwrap(), "[0, 0]");
    assert_eq!(count(), 1);
    // An impure function that is not memoised runs it each time.
    let impure = memo.replace("->>", "->");
    assert_eq!(lone(&impure).unwrap(), "[0, 0]");
    assert_eq!(count(), 3);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn exec_panic_and_sh_are_shadowed_silently() {
    for (text, expected) in [
        ("exec = 1\nn = exec + 1", "{exec = 1; n = 2}"),
        ("let panic = \"no\"\nn = panic", "{n = \"no\"}"),
        (
            "def sh(n: Int): Int { n * 2 }\nn = sh(2)",
            "{sh = fn(n: Int): Int; n = 4}",
        ),
        (
            "fn panic(m: String): Int { 7 }\nn = panic(\"x\")",
            "{n = 7}",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
}

#[test]
fn every_record_exec_builds_shares_the_runs_one_layout() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "ra = exec([\"true\"])\n\
                 def run() -> {status: Int, stdout: String, stderr: String} { exec([\"true\"]) }",
        ),
        (
            "b.gcsl",
            "rb = exec([\"false\"], panic = false)\nlet e = exec\ns = e([\"true\"], \"\", {}, false, false, \"\")\nt = run()",
        ),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    // The driver wrote the fifty-eight built-in functions' cells.
    assert_eq!(assert_builtins_written(&workspace).len(), 58);
    let a = workspace.file("a.gcsl").unwrap();
    let b = workspace.file("b.gcsl").unwrap();
    let first = workspace.run(a).unwrap().value;
    let second = workspace.run(b).unwrap().value;
    let layout = |value: &Value, field: &str| {
        let Value::Record(record) = value else {
            panic!("not a record: {value}");
        };
        let name = workspace.scan.names.lookup(field).unwrap();
        let Some(Value::Record(result)) = record.get(name) else {
            panic!("no record {field} in {value}");
        };
        Rc::clone(&result.layout)
    };
    for (value, field) in [
        (&first, "ra"),
        (&second, "rb"),
        (&second, "s"),
        (&second, "t"),
    ] {
        assert!(
            Rc::ptr_eq(&layout(value, field), &workspace.exec_result),
            "{field} of {value}"
        );
    }
    assert_eq!(
        &*workspace.exec_result.texts,
        ["status", "stdout", "stderr"].map(Box::from)
    );
}

#[test]
fn exec_live_gives_the_record_exec_gives() {
    for (argv, expected) in [
        ("[\"true\"]", "{status = 0; stdout = \"\"; stderr = \"\"}"),
        ("[\"false\"]", "{status = 1; stdout = \"\"; stderr = \"\"}"),
        (
            "[\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"]",
            "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
        ),
        // Killed by a signal: 128 plus its number.
        (
            "[\"sh\", \"-c\", \"kill -9 $$\"]",
            "{status = 137; stdout = \"\"; stderr = \"\"}",
        ),
        // Its standard input is the null device: `cat` ends at once.
        ("[\"cat\"]", "{status = 0; stdout = \"\"; stderr = \"\"}"),
        // Invalid UTF-8 is replaced in the record, on either stream.
        (
            "[\"sh\", \"-c\", \"printf '\\\\377ok'; printf 'e\\\\376' >&2\"]",
            "{status = 0; stdout = \"\u{FFFD}ok\"; stderr = \"e\u{FFFD}\"}",
        ),
        // A last line without a newline is captured as it is.
        (
            "[\"sh\", \"-c\", \"printf 'a\\\\nb'\"]",
            "{status = 0; stdout = \"a\\nb\"; stderr = \"\"}",
        ),
        // The arguments are passed as they are, with no shell.
        (
            "[\"printf\", \"%s|\", \"a b\", \"$HOME\", \"*\"]",
            "{status = 0; stdout = \"a b|$HOME|*|\"; stderr = \"\"}",
        ),
    ] {
        for builtin in ["exec", "exec_live"] {
            let text = format!("= {builtin}({argv}, panic = false)");
            assert_eq!(lone(&text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text.as_str())];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }
    // Named, through a variable, a field, an impure function, an import,
    // and a future argument, as `exec`.
    for (text, expected) in [
        ("= exec_live(argv = [\"echo\", \"n\"]).stdout", "n\n"),
        (
            "let e = exec_live\n= e([\"echo\", \"v\"], \"\", {}, false, false, \"\").stdout",
            "v\n",
        ),
        (
            "fn run(a: String) -> String { exec_live([\"echo\", a]).stdout }\n= run(\"i\")",
            "i\n",
        ),
        (
            "import exec_live as run from builtins\n\
             = run([\"true\"], {cwd = \"\"; env = {}; interactive = false; panic = false; until = \"\"}).status",
            "0",
        ),
        (
            "let xs = lazy [\"echo\", \"later\"]\n= exec_live(xs).stdout",
            "later\n",
        ),
        // The value shows as its type, `exec`'s.
        ("= exec_live", EXEC_TYPE),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
    }
}

#[test]
fn exec_live_fails_as_exec_does_naming_itself() {
    let text = "let none: List<String> = []\nr = exec_live(none)";
    let error = lone_error(text);
    assert_eq!(error.message, "`exec_live` needs a program to run");
    assert_eq!(error.span, Span::new(32, 47));
    assert!(error.notes.is_empty());
    let text = "r = exec_live([\"gcsl-no-such-program-slice-46\", \"x\"])";
    let error = lone_error(text);
    assert_eq!(error.message, "cannot run `gcsl-no-such-program-slice-46`");
    assert_eq!(error.span, Span::new(4, 53));
    assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
    let error = lone_error("r = exec_live([\"/\"])");
    assert_eq!(error.message, "cannot run `/`");
    assert_eq!(error.notes, ["Permission denied (os error 13)"]);
    // `exec`'s own message is unchanged.
    assert_eq!(
        lone_error("let none: List<String> = []\nr = exec(none)").message,
        "`exec` needs a program to run"
    );
}

#[test]
fn every_record_exec_live_builds_shares_the_runs_one_layout() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "ra = exec_live([\"true\"])\nlet e = exec_live\ns = e([\"false\"], \"\", {}, false, false, \"\")\nt = \
             exec([\"true\"])",
        ),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    let a = workspace.file("a.gcsl").unwrap();
    let Value::Record(record) = workspace.run(a).unwrap().value else {
        panic!("not a record");
    };
    for field in ["ra", "s", "t"] {
        let name = workspace.scan.names.lookup(field).unwrap();
        let Some(Value::Record(result)) = record.get(name) else {
            panic!("no record {field}");
        };
        assert!(
            Rc::ptr_eq(&result.layout, &workspace.exec_result),
            "{field}"
        );
    }
}

#[test]
fn sh_live_returns_its_output_or_panics_without_its_standard_error() {
    assert_eq!(lone("= sh_live(\"echo hi\")").unwrap(), "hi\n");
    assert_eq!(lone("= sh_live(\"printf hi\")").unwrap(), "hi");
    // The panic is the prelude's, located in it, at `sh_live`'s own
    // `panic`, and has no standard error in it.
    let error = lone_error("x = sh_live(\"echo nope >&2; exit 2\")");
    assert_eq!(
        error.message,
        "`echo nope >&2; exit 2` exited with status 2"
    );
    assert_eq!(error.input, None);
    assert_eq!(error.span, Span::new(4, 36));
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "x = sh_live(\"exit 2\")"),
    ];
    assert_eq!(
        run_file(&files, "main.gcsl").unwrap_err(),
        [("`exit 2` exited with status 2".to_owned(), Some(MAIN))]
    );
    // `sh`'s message is unchanged.
    assert_eq!(
        lone_error("x = sh(\"echo nope >&2; exit 2\")").message,
        "`echo nope >&2; exit 2` exited with status 2: nope"
    );
    // Impure, as `sh`: from a pure function, the existing error.
    let found = lone("fn f(): String { sh_live(\"true\") }\n= f()").unwrap_err();
    assert_eq!(
        found,
        ["cannot call an impure function from a pure function"]
    );
}

#[test]
fn exec_s_options_set_its_program_s_directory_and_environment() {
    assert_eq!(
        lone("= exec([\"pwd\"], cwd = \"/\").stdout").unwrap(),
        "/\n"
    );
    assert_eq!(
        lone("= exec([\"pwd\"], cwd = \"/usr\").stdout").unwrap(),
        "/usr\n"
    );
    // The environment is gcsl's own, the options' set over it.
    assert_eq!(
        lone("= exec([\"printenv\", \"PATH\"]).status").unwrap(),
        "0"
    );
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"echo $HOME\"], env = {HOME = \"/h\"}).stdout").unwrap(),
        "/h\n"
    );
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"echo $A$B\"], env = {A = \"x\"; B = \"y\"}).stdout")
            .unwrap(),
        "xy\n"
    );
    // By name in any order, as any named arguments, and through `exec_live`.
    assert_eq!(
        lone(
            "= exec_live([\"sh\", \"-c\", \"pwd; echo $A\"], env = {A = \"l\"}, cwd = \"/\").stdout"
        )
        .unwrap(),
        "/\nl\n"
    );
    // A directory that is not there is the program's failure to start.
    let text = "r = exec([\"true\"], cwd = \"/gcsl-no-such-directory\")";
    let error = lone_error(text);
    assert_eq!(error.message, "cannot run `true`");
    assert_eq!(error.span, Span::new(4, u32::try_from(text.len()).unwrap()));
}

#[test]
fn exec_fails_unless_its_program_succeeds() {
    // By default, as with `panic = true`.
    let text = "r = exec([\"sh\", \"-c\", \"echo bad >&2; exit 3\"])";
    let error = lone_error(text);
    assert_eq!(error.message, "`sh` exited with status 3");
    assert_eq!(error.notes, ["its standard error: bad"]);
    assert_eq!(error.span, Span::new(4, u32::try_from(text.len()).unwrap()));
    // Its standard output too, first, as a test runner reports there.
    let error = lone_error("r = exec([\"sh\", \"-c\", \"echo said; echo bad >&2; exit 3\"])");
    assert_eq!(
        error.notes,
        ["its standard output: said", "its standard error: bad"]
    );
    // Without output, no note; `exec_live` has shown it already.
    assert!(
        lone_error("r = exec([\"false\"], panic = true)")
            .notes
            .is_empty()
    );
    let error =
        lone_error("r = exec_live([\"sh\", \"-c\", \"echo bad >&2; exit 3\"], panic = true)");
    assert_eq!(error.message, "`sh` exited with status 3");
    assert!(error.notes.is_empty());
    // Success is the record, as without it.
    assert_eq!(
        lone("= exec([\"echo\", \"ok\"], panic = true).stdout").unwrap(),
        "ok\n"
    );
    // A program given a secret fails naming neither its status nor its
    // output, which are secrets; the failure is the call's, recovered and
    // retried as any.
    let secret = "let s = secret(\"pw\", \"id\")\n";
    let text = format!(
        "{secret}x = exec_secret([\"sh\", \"-c\", \"echo $T >&2; exit 3\"], secrets = {{T = \
         s.reveal()}})"
    );
    let error = lone_error(&text);
    assert_eq!(error.message, "a program given a secret failed");
    assert!(error.notes.is_empty());
    assert_eq!(
        lone(&format!(
            "{secret}= recover {{ exec_secret([\"false\"], secrets = {{T = s.reveal()}}).status }} \
             else (e) {{ secret(0, \"none\") }}"
        ))
        .unwrap(),
        "***"
    );
    // A program that cannot start is named: its name is no secret.
    assert_eq!(
        lone_error(&format!(
            "{secret}x = exec_secret([\"gcsl-no-such-program-12d\"], secrets = {{T = s.reveal()}})"
        ))
        .message,
        "cannot run `gcsl-no-such-program-12d`"
    );
}

#[test]
fn exec_until_ends_its_program_at_a_line_with_the_text() {
    // Its program, and what it started, killed at once: not 30 seconds.
    let started = std::time::Instant::now();
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"echo start; echo ready; sleep 30; echo late\"], until = \"ready\")")
            .unwrap(),
        "{status = 0; stdout = \"start\\nready\\n\"; stderr = \"\"}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    // In either stream, part of a line.
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"echo is ready now >&2; sleep 30\"], until = \"ready\").stderr")
            .unwrap(),
        "is ready now\n"
    );
    // Met, it is 0, whether the program then ends by itself or not, and so
    // never a failure.
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"echo ready; exit 4\"], until = \"ready\", panic = true).status")
            .unwrap(),
        "0"
    );
    // Never met, the program ends as it would.
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"echo x; exit 2\"], until = \"ready\", panic = false)")
            .unwrap(),
        "{status = 2; stdout = \"x\\n\"; stderr = \"\"}"
    );
    assert_eq!(
        lone("= exec_live([\"sh\", \"-c\", \"echo ready; sleep 30\"], until = \"ready\").stdout")
            .unwrap(),
        "ready\n"
    );
    // A prompt, its newline never written, written in pieces.
    let prompted = std::time::Instant::now();
    assert_eq!(
        lone(
            "= exec([\"sh\", \"-c\", \"printf 'Please press '; sleep 1; printf 'Enter: '; sleep 30\"], \
             until = \"press Enter\").stdout"
        )
        .unwrap(),
        "Please press Enter: "
    );
    assert!(prompted.elapsed() < std::time::Duration::from_secs(10));
    // An interactive program's output is not there to watch.
    let error = lone_error("r = exec([\"true\"], until = \"x\", interactive = true)");
    assert_eq!(
        error.message,
        "`until` watches a program's output, which an interactive one does not capture"
    );
}

#[test]
fn an_interactive_program_has_the_terminal_and_nothing_is_captured() {
    assert_eq!(
        lone("= exec([\"true\"], interactive = true)").unwrap(),
        "{status = 0; stdout = \"\"; stderr = \"\"}"
    );
    assert_eq!(
        lone("= exec([\"sh\", \"-c\", \"exit 2\"], interactive = true, panic = false).status")
            .unwrap(),
        "2"
    );
}

#[test]
fn a_timeout_kills_what_its_program_started() {
    // `sh` waits for the `sleep` it starts, which holds its output open:
    // both are killed at the deadline, not the shell alone.
    let text = "#[timeout(200ms)]\n\
                fn slow() -> Int { exec([\"sh\", \"-c\", \"sleep 30 & wait\"]).status }\n\
                r = recover { slow() } else (e) { 0 - 1 }";
    let started = std::time::Instant::now();
    assert_eq!(lone(text).unwrap(), "{r = -1}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn a_retried_call_runs_its_program_given_a_secret_again() {
    let counter = std::env::temp_dir().join(format!("gcsl-12d-retry-{}", std::process::id()));
    let counter = counter.to_str().unwrap();
    let _ = std::fs::remove_file(counter);
    let text = format!(
        "let s = secret(\"pw\", \"id\")\n\
         #[retry(2)]\n\
         fn flaky() -> Int {{\n\
             exec_secret([\"sh\", \"-c\", \"echo x >> {counter}; exit 1\"], secrets = {{T = s.reveal()}})\n\
             1\n\
         }}\n\
         x = flaky()"
    );
    assert_eq!(lone_error(&text).message, "a program given a secret failed");
    let runs = std::fs::read_to_string(counter).unwrap().lines().count();
    std::fs::remove_file(counter).unwrap();
    assert_eq!(runs, 3);
}
