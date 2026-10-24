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
        (
            "= exec([\"false\"])",
            "{status = 1; stdout = \"\"; stderr = \"\"}",
        ),
        (
            "= exec([\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"])",
            "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
        ),
        // A non-zero status is data, never an error.
        ("= exec([\"false\"]).status != 0", "true"),
        // Killed by a signal: 128 plus its number.
        ("= exec([\"sh\", \"-c\", \"kill -9 $$\"]).status", "137"),
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
        // Named, through a variable, a field, a `->` function.
        ("= exec(argv = [\"echo\", \"n\"]).stdout", "n\n"),
        ("let e = exec\n= e([\"echo\", \"v\"]).stdout", "v\n"),
        (
            "let r = { run = exec }\n= r.run([\"echo\", \"f\"]).stdout",
            "f\n",
        ),
        (
            "fn run(a: String) -> String { exec([\"echo\", a]).stdout }\n= run(\"i\")",
            "i\n",
        ),
        (
            "fn pick(f: fn(argv: Present<List<String>>) -> {status: Int}): \
                 fn(Present<List<String>>) -> {status: Int} { f }\n\
                 = pick(exec)([\"false\"]).status",
            "1",
        ),
        (
            "import exec as run from builtins\n= run([\"true\"]).status",
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
        (
            "= exec",
            "fn(argv: List<Revealed<String>>) -> {status: Int, stderr: String, stdout: String}",
        ),
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
fn sh_returns_its_output_or_panics_in_the_prelude() {
    assert_eq!(lone("= sh(\"echo hi\")").unwrap(), "hi\n");
    assert_eq!(lone("= sh(\"printf hi\")").unwrap(), "hi");
    // The panic is the prelude's, located in it: input 0.
    let error = lone_error("x = sh(\"echo nope >&2; exit 2\")");
    assert_eq!(
        error.message,
        "`echo nope >&2; exit 2` exited with status 2: nope"
    );
    assert_eq!(error.input, Some(0));
    let prelude = Library::NEEDED.find("prelude").unwrap();
    let prelude = crate::STANDARD[prelude].1;
    let at = u32::try_from(prelude.find("panic(\"`").unwrap()).unwrap();
    assert_eq!(error.span.start, at);
    // With no standard error, the message ends in ": ".
    assert_eq!(
        lone_error("x = sh(\"exit 2\")").message,
        "`exit 2` exited with status 2: "
    );
    let files = [("workspace.gcsl", ""), ("main.gcsl", "x = sh(\"exit 2\")")];
    assert_eq!(
        run_file(&files, "main.gcsl").unwrap_err(),
        [("`exit 2` exited with status 2: ".to_owned(), Some(0))]
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
            "rb = exec([\"false\"])\nlet e = exec\ns = e([\"true\"])\nt = run()",
        ),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    // The driver wrote the thirty built-in functions' cells.
    assert_eq!(assert_builtins_written(&workspace).len(), 30);
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
            let text = format!("= {builtin}({argv})");
            assert_eq!(lone(&text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text.as_str())];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }
    // Named, through a variable, a field, an impure function, an import,
    // and a future argument, as `exec`.
    for (text, expected) in [
        ("= exec_live(argv = [\"echo\", \"n\"]).stdout", "n\n"),
        ("let e = exec_live\n= e([\"echo\", \"v\"]).stdout", "v\n"),
        (
            "fn run(a: String) -> String { exec_live([\"echo\", a]).stdout }\n= run(\"i\")",
            "i\n",
        ),
        (
            "import exec_live as run from builtins\n= run([\"true\"]).status",
            "0",
        ),
        (
            "let xs = lazy [\"echo\", \"later\"]\n= exec_live(xs).stdout",
            "later\n",
        ),
        // The value shows as its type, `exec`'s.
        (
            "= exec_live",
            "fn(argv: List<Revealed<String>>) -> {status: Int, stderr: String, stdout: String}",
        ),
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
            "ra = exec_live([\"true\"])\nlet e = exec_live\ns = e([\"false\"])\nt = exec([\"true\"])",
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
    assert_eq!(error.input, Some(0));
    let prelude = Library::NEEDED.find("prelude").unwrap();
    let prelude = crate::STANDARD[prelude].1;
    let def = prelude.find("def sh_live(").unwrap();
    let at = def + prelude[def..].find("panic(\"`").unwrap();
    assert_eq!(error.span.start, u32::try_from(at).unwrap());
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "x = sh_live(\"exit 2\")"),
    ];
    assert_eq!(
        run_file(&files, "main.gcsl").unwrap_err(),
        [("`exit 2` exited with status 2".to_owned(), Some(0))]
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
