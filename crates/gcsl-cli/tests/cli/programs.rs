//! Tests of the `gcsl` command: running programs, and the string built-ins.

use super::*;

/// `exec` and `sh` in a lone file and in a workspace: the program's output
/// is captured into the value, never shown.
#[test]
fn run_runs_programs_with_exec_and_sh() {
    let output = gcsl(&["run", "tests/fixtures/exec.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{r = {status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}; out = \"hi\\n\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // In a workspace: another file's pure function returns a step that
    // runs `sh` when awaited.
    let output = gcsl(&["run", "tests/fixtures/ws_exec/app.gcsl"]);
    assert_eq!(stdout(&output), "{built = \"make app\\n\"; status = 1}\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// A `panic` is a runtime error of the program's own wording, at the call,
/// with exit code 1; an embedded newline continues the message on the next
/// line, indented under its first.
#[test]
fn a_panic_is_rendered_with_exit_code_1() {
    let output = gcsl(&["run", "tests/fixtures/panic.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: negative size -2\n\
         \x20--> tests/fixtures/panic.gcsl:2:16\n\
         \x20 |\n\
         2 |     if n < 0 { panic(\"negative size {n}\") }\n\
         \x20 |                ^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/panic_lines.gcsl"]);
    assert_eq!(
        stderr(&output),
        "error: one\n\
         \x20      two\n\
         \x20--> tests/fixtures/panic_lines.gcsl:1:1\n\
         \x20 |\n\
         1 | panic(\"one\\ntwo\")\n\
         \x20 | ^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// A failing `sh` is the prelude's panic, located at the call of `sh`,
/// which is `#[track_caller]`; its standard error is trimmed, so its
/// trailing newline leaves no empty line.
#[test]
fn a_failing_sh_is_rendered_at_its_call() {
    let output = gcsl(&["run", "tests/fixtures/sh_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `echo nope >&2; exit 2` exited with status 2: nope\n\
         \x20--> tests/fixtures/sh_error.gcsl:1:7\n\
         \x20 |\n\
         1 | out = sh(\"echo nope >&2; exit 2\")\n\
         \x20 |       ^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The string built-ins and `path`: the summary of slice 30, but `sh`.
#[test]
fn run_has_the_string_built_ins_and_path() {
    let output = gcsl(&["run", "tests/fixtures/string_builtins.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{parts = [\"a\", \"b\", \"c\"]; s = \"a-b-c\"; yes = true; gz = true; hash = \"abc\"; \
         p = \"/src/lib\"; q = \"/etc\"; d = \"/src/pkg\"; b = \"a.c\"; n = \"a/c\"; abs = false}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // The dump is the file's own: a call of a string built-in is a call of
    // the prelude's cell, and `path.join` reads `path`'s `join`, another
    // cell of another scope.
    let output = gcsl(&["run", "tests/fixtures/string_builtins.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    assert!(
        ir.contains("  (store parts#72 @0..27\n    (call @8..27\n      (load split#55 @8..13)\n"),
        "{ir}"
    );
    assert!(ir.contains("(load join#56 @32..36)"), "{ir}");
    assert!(ir.contains("(load join#108 @153..162)"), "{ir}");
    // Nothing of `path`'s own code.
    assert!(!ir.contains("segments"), "{ir}");
    assert_eq!(output.status.code(), Some(0));
}

/// The more string built-ins and `compare_versions`: the summary of slice
/// 41.
#[test]
fn run_has_the_more_string_built_ins_and_compare_versions() {
    let output = gcsl(&["run", "tests/fixtures/more_strings.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{a = \"6.6\"; b = \"x\"; c = \"a.tar\"; d = [\"a\", \"b\"]; \
         e = [\"tcp\", \"6\", \"TCP\"]; f = 1}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// A workspace one of whose files imports `path`: the standard files it
/// loads take the first input numbers, and a diagnostic of a workspace
/// file is still rendered against that file, whichever file is run.
#[test]
fn a_workspace_that_loads_path_renders_its_files_diagnostics() {
    let expected = "error: mismatched types\n\
                    \x20--> tests/fixtures/ws_path/app.gcsl:2:12\n\
                    \x20 |\n\
                    2 | bad = trim(1)\n\
                    \x20 |       ---- ^ expected String, found Int\n\
                    \x20 |       |\n\
                    \x20 |       the function is fn(s: String): String\n";
    for file in [
        "tests/fixtures/ws_path/app.gcsl",
        "tests/fixtures/ws_path/lib.gcsl",
    ] {
        let output = gcsl(&["run", file]);
        assert_eq!(stdout(&output), "", "{file}");
        assert_eq!(stderr(&output), expected, "{file}");
        assert_eq!(output.status.code(), Some(1), "{file}");
    }
}

#[test]
fn repl_has_the_string_built_ins_and_path() {
    let output = repl(
        "split(\"a/b\", \"/\")\n\
         import path\n\
         path.join(\"a\", \"b\")\n\
         split(\"a\", \"\")\n\
         join([\"x\", \"y\"], \"-\")\n",
    );
    assert_eq!(
        stdout(&output),
        "[\"a\", \"b\"] : List<String>\n\"a/b\" : String\n\"x-y\" : String\n"
    );
    assert_eq!(
        stderr(&output),
        "error: `split` needs a separator that is not empty\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | split(\"a\", \"\")\n\
         \x20 | ^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn exec_with_no_program_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/exec_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `exec` needs a program to run\n\
         \x20--> tests/fixtures/exec_error.gcsl:2:5\n\
         \x20 |\n\
         2 | r = exec(none)\n\
         \x20 |     ^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// A program a line runs reads the null device, not the REPL's input: `cat`
/// ends at once, and the next line is still the REPL's.
#[test]
fn repl_runs_programs_and_reads_its_next_line() {
    let output = repl(
        "exec([\"echo\", \"hi\"])\n\
         1 + 1\n\
         exec([\"cat\"]).stdout\n\
         2 + 2\n\
         sh(\"echo s\")\n\
         panic(\"no\")\n\
         3 + 3\n",
    );
    assert_eq!(
        stdout(&output),
        "{status = 0; stdout = \"hi\\n\"; stderr = \"\"} : \
         {status: Int, stderr: String, stdout: String}\n\
         2 : Int\n\
         \"\" : String\n\
         4 : Int\n\
         \"s\\n\" : String\n\
         6 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "error: no\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | panic(\"no\")\n\
         \x20 | ^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// `exec_live` copies the program's standard output and standard error to
/// `gcsl`'s stderr as they arrive, and captures them too: stdout has only
/// the value. The two streams are read by two threads, so the order of
/// their lines on stderr is not fixed.
#[test]
fn run_copies_what_exec_live_runs_to_stderr() {
    let output = gcsl(&["run", "tests/fixtures/exec_live.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{status = 0; stdout = \"out\\n\"; stderr = \"err\\n\"}\n"
    );
    let mut copied: Vec<&str> = stderr(&output).split_inclusive('\n').collect();
    copied.sort_unstable();
    assert_eq!(copied, ["err\n", "out\n"], "{}", stderr(&output));
    assert_eq!(output.status.code(), Some(0));
    // A last line without a newline is copied as it is, no newline added.
    let output = gcsl(&["run", "tests/fixtures/exec_live_partial.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{status = 0; stdout = \"a\\nb\"; stderr = \"\"}\n"
    );
    assert_eq!(stderr(&output), "a\nb");
    assert_eq!(output.status.code(), Some(0));
}

/// More output than a pipe holds, on both streams, each filled while the
/// other waits: the run ends, every line is copied whole, never mixed with
/// another, and the capture is whole.
#[test]
fn exec_live_copies_and_captures_more_than_a_pipe_holds() {
    let output = gcsl(&["run", "tests/fixtures/exec_live_large.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{status = 0; out = 200000; err = 100000}\n"
    );
    let copied = stderr(&output);
    assert_eq!(copied.len(), 600_000);
    assert_eq!(copied.lines().filter(|&line| line == "o").count(), 200_000);
    assert_eq!(copied.lines().filter(|&line| line == "e").count(), 100_000);
    assert_eq!(output.status.code(), Some(0));
}

/// A failing `sh_live` is the prelude's panic, located at its call as
/// `sh`'s is, its message without the standard error, which was copied as
/// it ran.
#[test]
fn a_failing_sh_live_is_rendered_without_its_standard_error() {
    let output = gcsl(&["run", "tests/fixtures/sh_live_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "nope\n\
         error: `echo nope >&2; exit 2` exited with status 2\n\
         \x20--> tests/fixtures/sh_live_error.gcsl:1:5\n\
         \x20 |\n\
         1 | x = sh_live(\"echo nope >&2; exit 2\")\n\
         \x20 |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// A line of the REPL runs `exec_live` and `sh_live` as a file does: the
/// copy on stderr, the value on stdout, and the next line still the REPL's.
#[test]
fn repl_runs_programs_live() {
    let output = repl(
        "exec_live([\"echo\", \"hi\"])\n\
         1 + 1\n\
         exec_live([\"cat\"]).stdout\n\
         sh_live(\"echo s\")\n\
         sh_live(\"exit 3\")\n\
         2 + 2\n",
    );
    assert_eq!(
        stdout(&output),
        "{status = 0; stdout = \"hi\\n\"; stderr = \"\"} : \
         {status: Int, stderr: String, stdout: String}\n\
         2 : Int\n\
         \"\" : String\n\
         \"s\\n\" : String\n\
         4 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "hi\n\
         s\n\
         error: `exit 3` exited with status 3\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | sh_live(\"exit 3\")\n\
         \x20 | ^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
