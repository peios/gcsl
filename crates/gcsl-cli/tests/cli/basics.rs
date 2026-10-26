//! Tests of the `gcsl` command: values and errors, exit codes, and the dumps'
//! order.

use super::*;

#[test]
fn run_prints_the_value_without_its_type() {
    let output = gcsl(&["run", "tests/fixtures/ok.gcsl"]);
    assert_eq!(stdout(&output), "1.5\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn type_error_is_rendered_to_stderr_with_exit_code_1() {
    let output = gcsl(&["run", "tests/fixtures/type_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot floor-divide Float\n\
         \x20--> tests/fixtures/type_error.gcsl:2:8\n\
         \x20 |\n\
         2 |   (7.0 // 2)\n\
         \x20 |    --- ^^ - Int\n\
         \x20 |    |\n\
         \x20 |    Float\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn runtime_error_is_rendered_to_stderr_with_exit_code_1() {
    let output = gcsl(&["run", "tests/fixtures/runtime_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: division by zero\n\
         \x20--> tests/fixtures/runtime_error.gcsl:1:4\n\
         \x20 |\n\
         1 | (1 // 0) + (9223372036854775807 + 1)\n\
         \x20 |  - ^^ - 0\n\
         \x20 |  |\n\
         \x20 |  1\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn an_import_outside_a_workspace_is_an_error() {
    let output = gcsl(&["run", "tests/fixtures/import_outside.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `lib` is not a built-in or standard module, and there is no \
         `workspace.gcsl` above this file\n\
         \x20--> tests/fixtures/import_outside.gcsl:2:1\n\
         \x20 |\n\
         2 | import lib\n\
         \x20 | ^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn an_import_in_the_repl_is_an_error() {
    let output = repl("import lib\n1 + 1\n");
    assert_eq!(stdout(&output), "2 : Int\n");
    assert!(
        stderr(&output).starts_with(
            "error: `lib` is not a built-in or standard module, and there is no \
             `workspace.gcsl` above this file\n"
        ),
        "{}",
        stderr(&output)
    );
}

/// An input of the REPL with syntax errors shows every one, and runs and
/// keeps nothing of it.
#[test]
fn every_syntax_error_of_a_repl_input_is_shown() {
    let output = repl("let x = 1 2; let y = (3 3)\nx\n1 + 1\n");
    assert_eq!(stdout(&output), "2 : Int\n");
    let errors: Vec<&str> = stderr(&output)
        .lines()
        .filter(|line| line.starts_with("error"))
        .collect();
    assert_eq!(
        errors,
        [
            "error: expected end of item, found `2`",
            "error: expected `)`, found `3`",
            "error: undefined variable `x`",
        ],
        "{}",
        stderr(&output)
    );
}

/// A name that names nothing is shown with the name it was probably meant
/// to be, found among the earlier lines' too.
#[test]
fn a_misspelled_name_is_shown_with_what_it_was_probably_meant_to_be() {
    let output = repl("let version = 1\nversoin + 1\nlet r = {port = 80}\nr.prot\n");
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: undefined variable `versoin`\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | versoin + 1\n\
         \x20 | ^^^^^^^\n\
         \x20 |\n\
         \x20 = help: did you mean `version`?\n\
         error: no field `prot` in `{port: Int}`\n\
         \x20--> <repl>:1:2\n\
         \x20 |\n\
         1 | r.prot\n\
         \x20 |  ^^^^^\n\
         \x20 |\n\
         \x20 = note: the only field is `port`\n\
         \x20 = help: did you mean `port`?\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// Colour is only used on a terminal, and tests run through pipes, so this
/// is the plain path; `NO_COLOR` set is the other way to get it.
#[test]
fn piped_stderr_has_no_escape_codes() {
    let output = gcsl(&["run", "tests/fixtures/type_error.gcsl"]);
    assert!(!stderr(&output).contains("\x1b["));
    let output = Command::new(binary())
        .args(["run", "tests/fixtures/type_error.gcsl"])
        .current_dir(crate_dir())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!stderr(&output).contains("\x1b["));
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn missing_file_exits_with_code_2() {
    let output = gcsl(&["run", "tests/fixtures/does_not_exist.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).starts_with("error: cannot read tests/fixtures/does_not_exist.gcsl: "),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn usage_errors_exit_with_code_2() {
    assert_eq!(gcsl(&["run"]).status.code(), Some(2));
    assert_eq!(gcsl(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(
        gcsl(&["run", "tests/fixtures/ok.gcsl", "--dump-everything"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn dumps_print_in_pipeline_order_and_stop_before_evaluation() {
    // Flags deliberately out of order.
    let output = gcsl(&[
        "run",
        "--dump-ir",
        "tests/fixtures/ok.gcsl",
        "--dump-tokens",
    ]);
    assert_eq!(
        stdout(&output),
        "Newline @33..34\n\
         LParen @34..35\n\
         Int(7) @35..36\n\
         SlashSlash @37..39\n\
         Int(2) @40..41\n\
         RParen @41..42\n\
         Slash @43..44\n\
         Int(2) @45..46\n\
         Newline @46..47\n\
         Eof @46..46\n\
         (block @34..46\n\
         \x20 (float-div @34..46\n\
         \x20   (int-to-float @34..42\n\
         \x20     (int-floor-div @35..41\n\
         \x20       (int 7 @35..36)\n\
         \x20       (int 2 @40..41)))\n\
         \x20   (float 2.0 @45..46)))\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_ast_and_types() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/ok.gcsl",
        "--dump-types",
        "--dump-ast",
    ]);
    assert_eq!(
        stdout(&output),
        "(file\n\
         \x20 (expr @34..46\n\
         \x20   (/ @34..46\n\
         \x20     (paren @34..42\n\
         \x20       (// @35..41\n\
         \x20         (int 7 @35..36)\n\
         \x20         (int 2 @40..41)))\n\
         \x20     (int 2 @45..46))))\n\
         (file\n\
         \x20 (expr @34..46\n\
         \x20   (/ : Float @34..46\n\
         \x20     (paren : Int @34..42\n\
         \x20       (// : Int @35..41\n\
         \x20         (int 7 : Int @35..36)\n\
         \x20         (int 2 : Int @40..41)))\n\
         \x20     (int 2 : Float @45..46))))\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
