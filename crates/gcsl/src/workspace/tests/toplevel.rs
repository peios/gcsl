//! Tests of workspaces: top-level functions found by name, memoised results and
//! compositions.

use super::*;

/// A scratch directory of the test `name`'s own, empty, and the path of
/// the log file in it, which the [`environment`] workspaces append to.
pub(super) fn log_in(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("gcsl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log").display().to_string();
    (dir, log)
}

/// What the log `log` holds, empty if it is missing.
pub(super) fn logged(log: &str) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

/// A workspace with a root `pekit.gcsl` whose memoised `environment`
/// logs `boot` to `log` and returns a handle, and whose `teardown` logs
/// `down` and the handle's `pid`; a def-type `run`, whose definition
/// `uses` calls `environment` through an import of `pekit`, and `plain`
/// does not.
pub(super) fn environment(log: &str) -> [(&'static str, String); 3] {
    [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }".to_owned(),
        ),
        (
            "pekit.gcsl",
            format!(
                "def environment() ->> {{pid: Int, port: Int}} {{\n\
                     \x20   let out = sh(\"echo boot >> {log}\")\n\
                     \x20   {{ pid = 7; port = if out == \"\" {{ 22 }} else {{ 0 }} }}\n\
                     }}\n\
                     def teardown(env: {{pid: Int, port: Int}}) -> String {{\n\
                     \x20   sh(\"echo down {{env.pid}} >> {log}\")\n\
                     }}\n\
                     x = 1\n\
                     fn private() ->> Int {{ 1 }}"
            ),
        ),
        (
            "pkg/p.gcsl",
            "import pekit\n\
                 def run uses() { \"uses {pekit.environment().pid}\" }\n\
                 def run plain() { \"plain\" }"
                .to_owned(),
        ),
    ]
}

#[test]
fn a_top_level_function_is_found_by_name() {
    let (dir, log) = log_in("top-level");
    let files = environment(&log);
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    assert_eq!(
        workspace.top_level(pekit, "environment"),
        Some(TopLevel {
            ty: "fn() ->> {pid: Int, port: Int}".to_owned(),
            function: Some((ast::Effect::Memo, 0)),
        })
    );
    assert_eq!(
        workspace.top_level(pekit, "teardown"),
        Some(TopLevel {
            ty: "fn(env: {pid: Int, port: Int}) -> String".to_owned(),
            function: Some((ast::Effect::Impure, 1)),
        })
    );
    // A field that is no function; a private function, a name that is
    // none and a file that is none, nothing.
    assert_eq!(
        workspace.top_level(pekit, "x"),
        Some(TopLevel {
            ty: "Int".to_owned(),
            function: None,
        })
    );
    assert_eq!(workspace.top_level(pekit, "private"), None);
    assert_eq!(workspace.top_level(pekit, "nope"), None);
    assert_eq!(workspace.top_level(u32::MAX, "environment"), None);
    // An annotated field as its annotation is written, a function
    // field's as a diagnostic shows its type.
    let annotated = rooted(&[
        ("workspace.gcsl", "x = 1"),
        (
            "pekit.gcsl",
            "type Vm = {pid: Int}\n\
                 environment: fn() ->> Vm = fn() ->> Vm { { pid = 1 } }\n\
                 teardown = fn(env: Vm) -> {} { {} }",
        ),
    ])
    .unwrap();
    let file = annotated.file("pekit.gcsl").unwrap();
    assert_eq!(
        annotated.top_level(file, "environment"),
        Some(TopLevel {
            ty: "fn() ->> Vm".to_owned(),
            function: Some((ast::Effect::Memo, 0)),
        })
    );
    assert_eq!(
        annotated.top_level(file, "teardown"),
        Some(TopLevel {
            ty: "fn(env: Vm) -> {}".to_owned(),
            function: Some((ast::Effect::Impure, 1)),
        })
    );
    // Nothing has run.
    assert!(!workspace.memoised(pekit, "environment"));
    assert_eq!(
        workspace.check_composed(pekit, &["teardown", "environment"]),
        Ok(())
    );
    assert_eq!(logged(&log), "");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn memoised_says_whether_a_memoised_call_completed() {
    let (dir, log) = log_in("memoised");
    let files = environment(&log);
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    assert!(!workspace.memoised(pekit, "environment"));
    // `pekit.gcsl` runs, as `p.gcsl` needs it, but nothing calls
    // `environment`.
    let plain = workspace.invoke("run", "plain", &[]).unwrap();
    assert_eq!(plain.value.to_string(), "plain");
    assert!(!workspace.memoised(pekit, "environment"));
    // Called: it has returned, once, across invocations of the one
    // workspace.
    for _ in 0..2 {
        let uses = workspace.invoke("run", "uses", &[]).unwrap();
        assert_eq!(uses.value.to_string(), "uses 7");
        assert!(workspace.memoised(pekit, "environment"));
    }
    assert_eq!(logged(&log), "boot\n");
    // Not a memoised function, or no function, or none: never.
    for name in ["teardown", "x", "private", "nope"] {
        assert!(!workspace.memoised(pekit, name), "{name}");
    }
    // Another workspace has tables of its own.
    let fresh = rooted(&files).unwrap();
    assert!(!fresh.memoised(pekit, "environment"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_memoised_call_that_failed_or_read_another_caller_is_not_found() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }",
        ),
        (
            "pekit.gcsl",
            "import fileroot\n\
                 def environment() ->> String { panic(\"no vm\") }\n\
                 def called() ->> String { fileroot.caller }",
        ),
        (
            "pkg/p.gcsl",
            "import pekit\n\
                 def run boots() { pekit.environment() }\n\
                 def run reads() { pekit.called() }",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    assert!(workspace.invoke("run", "boots", &[]).is_err());
    assert!(!workspace.memoised(pekit, "environment"));
    // Its value depends on its caller, `pkg`, which a call written at
    // the root is not: such a call would run it again.
    let read = workspace.invoke("run", "reads", &[]).unwrap();
    assert_eq!(read.value.to_string(), format!("{SCRATCH}/pkg"));
    assert!(!workspace.memoised(pekit, "called"));
    // Called from the root, it is found.
    let mut rooted_call = rooted(&files).unwrap();
    let root = rooted_call.call_composed(pekit, &["called"]).unwrap();
    assert_eq!(root.value.to_string(), SCRATCH);
    assert!(rooted_call.memoised(pekit, "called"));
}

#[test]
fn memoised_calls_of_no_arguments_from_several_directories_are_told_apart() {
    // A memoised function of no parameters reading its caller, called
    // from two directories, and again from the first: one entry for
    // each, both with an empty key, each completing, and each found.
    let (dir, log) = log_in("empty-keys");
    let lib = format!(
        "import fileroot\nimport path\n\
             def called() ->> String {{\n\
             \x20   let out = sh(\"echo ran {{fileroot.caller}} >> {log}\")\n\
             \x20   \"{{fileroot.caller}}{{out}}\"\n\
             }}\n\
             def cycle() ->> String {{\n\
             \x20   let caller = fileroot.caller\n\
             \x20   let out = sh(\"echo cycle {{caller}} >> {log}\")\n\
             \x20   fileroot.helper()\n\
             \x20   if caller == path.join(fileroot.workspace, \"b\") {{ cycle() }} else {{ \"{{caller}}{{out}}\" }}\n\
             }}"
    );
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }",
        ),
        ("lib.gcsl", lib.as_str()),
        (
            "a/a.gcsl",
            "import lib\ndef run a() { lib.called() }\ndef run ca() { lib.cycle() }",
        ),
        (
            "b/b.gcsl",
            "import lib\ndef run b() { lib.called() }\ndef run cb() { lib.cycle() }",
        ),
        ("top.gcsl", "import lib\ndef run top() { lib.called() }"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let lib = workspace.file("lib.gcsl").unwrap();
    let invoked = |workspace: &mut Workspace, name: &str| {
        let outcome = workspace.invoke("run", name, &[]).unwrap();
        outcome.value.to_string().replace(SCRATCH, "<root>")
    };
    for _ in 0..2 {
        assert_eq!(invoked(&mut workspace, "a"), "<root>/a");
        assert_eq!(invoked(&mut workspace, "b"), "<root>/b");
    }
    assert_eq!(
        logged(&log).replace(SCRATCH, "<root>"),
        "ran <root>/a\nran <root>/b\n"
    );
    // Neither entry is a call from the root's.
    assert!(!workspace.memoised(lib, "called"));
    assert_eq!(invoked(&mut workspace, "top"), "<root>");
    assert!(workspace.memoised(lib, "called"));
    for name in ["a", "b", "top", "a"] {
        invoked(&mut workspace, name);
    }
    assert_eq!(
        logged(&log).replace(SCRATCH, "<root>"),
        "ran <root>/a\nran <root>/b\nran <root>\n"
    );
    // A call of no arguments depending on its own result is still one,
    // beside an entry of the same key that completed, which it leaves.
    assert_eq!(invoked(&mut workspace, "ca"), "<root>/a");
    let Err(InvokeError::Diagnostics(errors)) = workspace.invoke("run", "cb", &[]) else {
        panic!("expected `cb` to fail");
    };
    assert!(
        errors
            .iter()
            .any(|error| error.message == "this call depends on its own result"),
        "{errors:?}"
    );
    assert_eq!(invoked(&mut workspace, "ca"), "<root>/a");
    assert_eq!(
        logged(&log).replace(SCRATCH, "<root>"),
        "ran <root>/a\nran <root>/b\nran <root>\ncycle <root>/a\ncycle <root>/b\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_composition_of_top_level_functions_is_called_by_name() {
    let (dir, log) = log_in("composed");
    let files = environment(&log);
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    let uses = workspace.invoke("run", "uses", &[]).unwrap();
    assert_eq!(uses.value.to_string(), "uses 7");
    // The inner call takes what the first returned: no second boot.
    let down = workspace
        .call_composed(pekit, &["teardown", "environment"])
        .unwrap();
    assert_eq!(down.value.to_string(), "");
    assert_eq!(logged(&log), "boot\ndown 7\n");
    // Called alone, run again, as any call.
    workspace.call_composed(pekit, &["environment"]).unwrap();
    assert_eq!(logged(&log), "boot\ndown 7\n");
    // A composition of none is `{}`, and runs nothing.
    let none = workspace.call_composed(pekit, &[]).unwrap();
    assert_eq!(none.value, Value::Empty);
    // In a workspace where nothing has run, the call runs the file
    // first, and boots.
    let mut fresh = rooted(&files).unwrap();
    fresh
        .call_composed(pekit, &["teardown", "environment"])
        .unwrap();
    assert_eq!(logged(&log), "boot\ndown 7\nboot\ndown 7\n");
    // A name that is no top-level function of the file, before anything
    // runs.
    for name in ["nope", "x", "private"] {
        assert_eq!(
            fresh.call_composed(pekit, &["teardown", name]),
            Err(InvokeError::Request(RequestError::NoFunction {
                file: "pekit.gcsl".to_owned(),
                name: name.to_owned(),
            })),
            "{name}"
        );
    }
    assert_eq!(
        RequestError::NoFunction {
            file: "pekit.gcsl".to_owned(),
            name: "x".to_owned(),
        }
        .message(),
        "`pekit.gcsl` has no top-level function `x`"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_composition_is_checked_as_a_call_is() {
    // Each `teardown` with the `environment` it takes, or does not.
    let check = |environment: &str, teardown: &str| {
        let pekit = format!("type Vm = {{pid: Int}}\n{environment}\n{teardown}");
        let files = [("workspace.gcsl", "x = 1"), ("pekit.gcsl", pekit.as_str())];
        let mut workspace = rooted(&files).unwrap();
        let file = workspace.file("pekit.gcsl").unwrap();
        workspace
            .check_composed(file, &["teardown", "environment"])
            .map_err(|error| match error {
                InvokeError::Diagnostics(errors) => {
                    errors.into_iter().map(|e| e.message).collect::<Vec<_>>()
                }
                InvokeError::Request(error) => vec![error.message()],
            })
    };
    let memoised = "def environment() ->> Vm { { pid = 1 } }";
    for teardown in [
        "def teardown(env: Vm) -> {} { {} }",
        "def teardown(env: Vm) ->> {} { {} }",
        "def teardown(env: {pid: Int}) -> Int { env.pid }",
        "def teardown(env: Vm | String) -> {} { {} }",
    ] {
        assert_eq!(check(memoised, teardown), Ok(()), "{teardown}");
    }
    for teardown in [
        "def teardown(env: String) -> {} { {} }",
        "def teardown(env: {pid: Int, port: Int}) -> {} { {} }",
    ] {
        assert_eq!(
            check(memoised, teardown),
            Err(vec!["mismatched types".to_owned()]),
            "{teardown}"
        );
    }
    // An impure `environment`'s value too.
    assert_eq!(
        check(
            "def environment() -> Vm { { pid = 1 } }",
            "def teardown(env: Vm) ->> {} { {} }"
        ),
        Ok(())
    );
    // A `teardown` of no parameters takes nothing.
    let none = check(memoised, "def teardown() -> {} { {} }").unwrap_err();
    assert_eq!(none.len(), 1, "{none:?}");
}
