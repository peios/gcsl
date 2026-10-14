//! Tests that run the built `gcsl` binary.

use std::io::Write;
use std::process::{Command, Output, Stdio};

/// Runs `gcsl` with `args` from this crate's directory, so fixture paths
/// (and therefore the file names in diagnostics) are stable.
fn gcsl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

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

/// Colour is only used on a terminal, and tests run through pipes, so this
/// is the plain path; `NO_COLOR` set is the other way to get it.
#[test]
fn piped_stderr_has_no_escape_codes() {
    let output = gcsl(&["run", "tests/fixtures/type_error.gcsl"]);
    assert!(!stderr(&output).contains("\x1b["));
    let output = Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .args(["run", "tests/fixtures/type_error.gcsl"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
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

#[test]
fn run_evaluates_a_file_of_items() {
    let output = gcsl(&["run", "tests/fixtures/items.gcsl"]);
    assert_eq!(stdout(&output), "37.5\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_prints_a_string_as_its_text() {
    let output = gcsl(&["run", "tests/fixtures/strings.gcsl"]);
    assert_eq!(
        stdout(&output),
        "GCSL has 3 items\nready: true\nC:\\{done}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // The string's own escapes are its text: a tab, and a final newline
    // before the one `run` adds.
    let output = gcsl(&["run", "tests/fixtures/greeting.gcsl"]);
    assert_eq!(stdout(&output), "hello,\tworld!\n\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_tokens_shows_string_pieces_quoted() {
    let output = gcsl(&["run", "tests/fixtures/greeting.gcsl", "--dump-tokens"]);
    assert_eq!(
        stdout(&output),
        "Let @0..3\n\
         Ident(who) @4..7\n\
         Eq @8..9\n\
         Str(\"world\") @10..17\n\
         Newline @17..18\n\
         StrStart(\"hello,\\t\") @18..28\n\
         Ident(who) @28..31\n\
         StrEnd(\"!\\n\") @31..36\n\
         Newline @36..37\n\
         Eof @36..36\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_ir_shows_strings_and_ifs() {
    let output = gcsl(&["run", "tests/fixtures/greeting.gcsl", "--dump-ir"]);
    assert_eq!(
        stdout(&output),
        "(block @0..36\n\
         \x20 (store who#48 @0..17\n\
         \x20   (str \"world\" @10..17))\n\
         \x20 (interp @18..36\n\
         \x20   (str \"hello,\\t\" @18..36)\n\
         \x20   (load who#48 @28..31)\n\
         \x20   (str \"!\\n\" @18..36)))\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_prints_nothing_for_an_empty_record_result() {
    let output = gcsl(&["run", "tests/fixtures/empty.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_prints_a_record_as_it_would_be_written() {
    let output = gcsl(&["run", "tests/fixtures/record.gcsl"]);
    assert_eq!(stdout(&output), "{x = 1; y = \"a\"}\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_prints_merged_dotted_fields_as_one_record() {
    let output = gcsl(&["run", "tests/fixtures/dotted.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{server = {host = \"localhost\"; port = 8080; tls = {enabled = true}}; name = \"api\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_ir_shows_updates() {
    let output = gcsl(&["run", "tests/fixtures/dotted.gcsl", "--dump-ir"]);
    assert_eq!(
        stdout(&output),
        "(block @39..122\n\
         \x20 (store host#46 @39..64\n\
         \x20   (str \"localhost\" @53..64))\n\
         \x20 (store port#47 @65..83\n\
         \x20   (int 8080 @79..83))\n\
         \x20 (store name#48 @84..96\n\
         \x20   (str \"api\" @91..96))\n\
         \x20 (store enabled#50 @97..122\n\
         \x20   (bool true @118..122))\n\
         \x20 (store tls#49 @97..122\n\
         \x20   (make-record [enabled] @97..122\n\
         \x20     (load enabled#50 @97..122)))\n\
         \x20 (store server#45 @39..64\n\
         \x20   (make-record [host port tls] @39..64\n\
         \x20     (load host#46 @39..64)\n\
         \x20     (load port#47 @65..83)\n\
         \x20     (load tls#49 @97..122)))\n\
         \x20 (make-record [server name] @39..122\n\
         \x20   (load server#45 @39..45)\n\
         \x20   (load name#48 @84..88)))\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_calls_functions() {
    let output = gcsl(&["run", "tests/fixtures/functions.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{web = \"web runs nginx on 8080\"; api = \"api runs api on 8001!\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_functions_and_calls() {
    let output = gcsl(&["run", "tests/fixtures/functions.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    assert!(tokens.starts_with("Newline @68..69\nFn @69..71\nIdent(service) @72..79\n"));
    assert!(tokens.contains("\nArrow @213..215\n"), "{tokens}");
    let output = gcsl(&["run", "tests/fixtures/functions.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    assert!(
        ast.starts_with(
            "(file\n\
             \x20 (fn service @69..189\n\
             \x20   (fn @69..189\n\
             \x20     (params (name String) (config {port: Int, image: String}))\n\
             \x20     (ret pure String)\n"
        ),
        "{ast}"
    );
    assert!(ast.contains("\n        (named config\n"), "{ast}");
    assert!(ast.contains("\n      (trailing\n        (block @"), "{ast}");
    let output = gcsl(&["run", "tests/fixtures/functions.gcsl", "--dump-types"]);
    let types = stdout(&output);
    assert!(
        types.contains(
            "(let at @254..301\n\
             \x20   (fn : fn(offset: Int): Int @263..301\n"
        ),
        "{types}"
    );
    let output = gcsl(&["run", "tests/fixtures/functions.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    assert!(
        ir.contains(
            "  (store at#53 @254..301\n\
             \x20   (make-closure #2 @263..301\n\
             \x20     (load base#52 @263..301)))\n"
        ),
        "{ir}"
    );
    assert!(
        ir.contains(
            "fn #2 at (frame 1, params 1, captures 1, height 3)\n\
             \x20 (block @284..301\n\
             \x20   (int-add @286..299\n\
             \x20     (load-capture 0 @286..290)\n\
             \x20     (load offset#0 @293..299)))"
        ),
        "{ir}"
    );
    assert!(
        ir.contains("\nfn #0 service (frame 2, params 2, captures 0, height 4)\n"),
        "{ir}"
    );
    // An impure function's `String` parameter may be a future, which the
    // concatenation awaits.
    assert!(
        ir.contains(
            "\nfn #1 shout (frame 1, params 1, captures 0, height 4)\n\
             \x20 (block @223..237\n\
             \x20   (str-concat @225..235\n\
             \x20     (await @225..229\n\
             \x20       (load text#0 @225..229))\n"
        ),
        "{ir}"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_awaits_a_planned_build_step() {
    let output = gcsl(&["run", "tests/fixtures/futures.gcsl"]);
    // A future shows as its type, once run as before.
    assert_eq!(
        stdout(&output),
        "{name = \"gcsl\"; step = Future<String>}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_futures() {
    let output = gcsl(&["run", "tests/fixtures/futures.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nLazy @221..225\n",
        "\nAwait @397..402\n",
        "\nPipe @537..538\n",
        "\nIdent(Future) @179..185\nLt @185..186\nIdent(String) @186..192\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/futures.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n      (ret pure {name: String, build: Future<String>})\n",
        "\n        (field build @213..239\n\
         \x20         (lazy @221..239\n\
         \x20           (call @226..239\n",
        "\n        (expr @397..413\n\
         \x20         (await @397..413\n\
         \x20           (. build @403..413\n",
        "\n      (params (step String))\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/futures.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        // A future of the call's result is a future of a string.
        "(lazy : Future<String> @221..239\n",
        "(name compile : fn(target: String | Future<String>) -> String @",
        "(await : String @397..413\n",
        // Written bare, passed through unmeasured: inferred to be either.
        "(name keep : fn(step: String | Future<String>): {step: String | Future<String>} @",
        "(name step : String | Future<String> @564..568)",
        // An impure function's bare record result takes its laziness from
        // its body, which returns a present record of a present name.
        "(name built : {build: Future<String>, name: String} @610..615)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/futures.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        "\n    (store build#2 @213..239\n\
         \x20     (make-future #1 @221..239\n\
         \x20       (load name#1 @221..239)))\n",
        "\nfuture #1 (frame 0, params 0, captures 1, height 3)\n",
        "\n    (await @397..413\n\
         \x20     (field build @403..413\n\
         \x20       (load conf#2 @403..407)))\n",
        "\n        (arg 0 @389..393\n\
         \x20         (await @389..393\n\
         \x20           (load name#1 @389..393)))))\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_memoises_fetches() {
    let output = gcsl(&["run", "tests/fixtures/memo.gcsl"]);
    // `weight(80)` finishes only because each argument runs once.
    assert_eq!(
        stdout(&output),
        "{core = \"core\"; deps = 1; fib-80 = 23416728348467685; step = Future<Int>; \
         doubled = 42}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_memoised_functions() {
    let output = gcsl(&["run", "tests/fixtures/memo.gcsl", "--dump-tokens"]);
    assert!(stdout(&output).contains("\nMemoArrow @104..107\n"));
    let output = gcsl(&["run", "tests/fixtures/memo.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n      (ret memo {name: String, deps: Int})\n",
        "\n      (params (n Int))\n      (ret memo Int)\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/memo.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        // Its parameter is present; its result is inferred from its body.
        "(name fetch : fn(name: String) ->> {deps: Int, name: String} @473..478)",
        "(name weight : fn(n: Int) ->> Int @579..585)",
        "(name later : fn(n: Int) ->> Future<Int> @597..602)",
        // Inside its body, its own name has the declared type.
        "(name weight : fn(n: Int) ->> Int | Future<Int> @314..320)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/memo.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        "\nmemo fn #0 fetch (frame 3, params 1, captures 0, height 6)\n",
        "\nmemo fn #1 weight (frame 1, params 1, captures 0, height 9)\n",
        "\nfuture #2 (frame 0, params 0, captures 1, height 2)\n",
        "\nmemo fn #3 later (frame 1, params 1, captures 0, height 3)\n",
        "\n          (await @314..327\n\
         \x20           (call @314..327\n\
         \x20             (load-callee @314..320)\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_memoised_call_depending_on_its_own_result_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/memo_cycle.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: this call depends on its own result\n\
         \x20--> tests/fixtures/memo_cycle.gcsl:1:33\n\
         \x20 |\n\
         1 | fn size(name: String) ->> Int { size(name) + 1 }\n\
         \x20 |                                 ^^^^^^^^^^\n\
         2 | size(\"core\")\n\
         \x20 | ------------ called here with the same arguments\n\
         \x20 |\n\
         \x20 = note: a `->>` function called again with the same arguments while it is running \
         would never finish\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_builds_a_deployment_of_lists() {
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{count = 3; first = 22; report = \"[\\\"web\\\", \\\"db\\\"]: [8080, 5432]\"; \
         same = true; services = [{name = \"web\"; replicas = 2}, {name = \"db\"; replicas = \
         1}]}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_lists() {
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nIdent(String) @230..236\nLBracket @236..237\nRBracket @237..238\n",
        "\nDotDotDot @576..579\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n      (params (names String[]))\n      (ret impure Int[])\n",
        "\n    (list @445..469\n\
         \x20     (int 22 @446..448)\n\
         \x20     (index @450..458\n\
         \x20       (name found @450..455)\n\
         \x20       (int 0 @456..457))\n",
        "\n    (list @575..643\n\
         \x20     (spread\n\
         \x20       (list @579..611\n",
        "(let open : List<Present<Int>> @414..469\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        // A bare list parameter of an impure function is the either shape,
        // element and all; its result is inferred from its body's list.
        "(name ports : fn(names: List<String | Future<String>> | Future<List<String | \
         Future<String>>>) -> List<Future<Int>> @334..339)",
        "(list : List<String> @308..321\n",
        // An element is read as it is.
        "(index : Future<Int> @450..458\n",
        "(name len : fn(List<Int>): Int @478..481)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/lists.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        "\n    (make-list @445..469\n\
         \x20     (int 22 @446..448)\n\
         \x20     (await @450..458\n\
         \x20       (index @450..458\n\
         \x20         (load found#56 @450..455)\n\
         \x20         (int 0 @456..457)))\n",
        "\n    (list-len @478..487\n\
         \x20     (load open#57 @482..486)))\n",
        "\n      (await *! @524..529\n\
         \x20       (load found#56 @524..529))\n",
        "\n    (list-eq == * @539..563\n",
        "\n    (make-list @575..643\n\
         \x20     (spread @576..611\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn an_index_out_of_range_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/index_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: index out of range\n\
         \x20--> tests/fixtures/index_error.gcsl:2:1\n\
         \x20 |\n\
         2 | ports[2]\n\
         \x20 | ^^^^^^^^ the index is 2, and the list has 2 elements\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_rolls_out_a_fleet_with_fors() {
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{count = 3; plan = \"[\\\"web-1\\\", \\\"web-2\\\"] on [8000, 8001, 8002]\"; squares = \
         [1, 4, 9]; same = true}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_fors_and_ranges() {
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nFor @213..216\nIdent(i) @217..218\nIn @219..221\nInt(0) @222..223\nDotDot @223..225\n",
        "\nInt(1) @687..688\nDotDotEq @688..691\nInt(3) @691..692\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n    (for host @310..354\n\
         \x20     (name hosts @322..327)\n\
         \x20     (guard\n\
         \x20       (!= @331..345\n\
         \x20         (name host @331..335)\n\
         \x20         (str \"db-1\" @339..345)))\n\
         \x20     (block @346..354\n",
        "\n    (for i @678..702\n\
         \x20     (..= @687..692\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(for i : List<Int> @213..248\n\
         \x20     (.. : List<Int> @222..235\n",
        "(for host : List<String> @310..354\n",
        // A statement of `{}`s, and one of futures.
        "(for host : List<{}> @426..459\n",
        "(for host : List<Future<{}>> @524..562\n",
        // The body is awaited to fit the expected element type, so the
        // list holds what it is once awaited.
        "(for p : List<Int> @596..621\n",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/fors.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A `for` shows its variable's slot, named, as every slot is, by
        // the last variable stored in it.
        "\n    (for ports#54 @213..248\n\
         \x20     (range @222..235\n\
         \x20       (int 0 @222..223)\n\
         \x20       (list-len @225..235\n",
        "\n    (for webs#55 if @310..354\n\
         \x20     (load hosts#53 @322..327)\n\
         \x20     (str-cmp != @331..345\n",
        // The statement of `{}`s discards its body's values; the statement
        // of futures builds its list, and awaits it deeply.
        "\n  (for-each checks#56 @426..459\n\
         \x20   (load webs#55 @438..442)\n",
        "\n  (await *! @524..562\n\
         \x20   (for checks#56 @524..562\n",
        "\n    (for checks#56 @596..621\n\
         \x20     (load ports#54 @605..610)\n\
         \x20     (await @611..621\n",
        "\n    (for i#57 @678..702\n\
         \x20     (range-inclusive @687..692\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_range_too_large_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/range_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: range too large\n\
         \x20--> tests/fixtures/range_error.gcsl:1:11\n\
         \x20 |\n\
         1 | let ids = 0..=9223372036854775807\n\
         \x20 |           ^^^^^^^^^^^^^^^^^^^^^^^ this range has 9223372036854775808 elements\n\
         \x20 |\n\
         \x20 = note: a range used as a list holds every element, and a list cannot hold more \
         than 384307168202282325. A `for` over a range directly builds no list\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn iterating_over_what_is_not_a_list_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/iterate_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot iterate over a value of type Int: it is not a list\n\
         \x20--> tests/fixtures/iterate_error.gcsl:2:10\n\
         \x20 |\n\
         2 | for r in replicas { r }\n\
         \x20 |          ^^^^^^^^ this is Int\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_shows_fors_and_ranges() {
    let output = repl(
        "let xs = for i in 0..3 { i * 2 }\n\
         xs\n\
         for x in xs if x > 0 { lazy x }\n\
         0..=2\n",
    );
    assert_eq!(
        stdout(&output),
        "[0, 2, 4] : List<Int>\n\
         [Future<Int>, Future<Int>] : List<Future<Int>>\n\
         [0, 1, 2] : List<Int>\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_tallies_checks_in_mutable_variables() {
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{checked = 550; slow = \"web-2\"; port = 443; hosts = [\"web-1\", \"db-1\"]; average = \
         183}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_mutable_variables() {
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    let token = "\nLet @159..162\nMut @163..166\nIdent(total) @167..172\nEq @173..174\n";
    assert!(tokens.contains(token), "{token}: {tokens}");
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n  (let mut total @159..176\n\
         \x20   (int 0 @175..176))\n",
        "\n      (block @299..366\n\
         \x20       (rebind total @303..324\n",
        "\n        (rebind conf.tls @491..507\n",
        "\n        (let mut sum @793..808\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(for c : List<{}> @283..366\n",
        "(name slowest : {host: String, ms: Int} @337..344)",
        // A dotted rebind leaves the variable's type as it was declared.
        "(name conf : {port: Int, tls: Bool} @907..911)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/mut.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A rebind in a `for`'s body, and in an `if` in it, stores into the
        // variable's one slot.
        "\n  (store total#55 @159..176\n\
         \x20   (int 0 @175..176))\n",
        "\n    (block @299..366\n\
         \x20     (store total#55 @303..324\n\
         \x20       (int-add @312..324\n\
         \x20         (load total#55 @312..317)\n",
        "\n        (block @348..364\n\
         \x20         (store slowest#56 @350..362\n\
         \x20           (load conf#57 @361..362))\n",
        // The guard reads the slot the body writes.
        "\n      (int-cmp >= @621..635\n\
         \x20       (load budget#58 @621..627)\n",
        "\n        (store budget#58 @638..661\n",
        // The closure copies the variable when it is made.
        "\n  (store report#60 @721..753\n\
         \x20   (make-closure #0 @734..753\n\
         \x20     (load total#55 @734..753)))\n\
         \x20 (store total#55 @754..764\n",
        // A function's own variable, in its own frame.
        "\n      (block @823..841\n\
         \x20       (store sum#1 @825..839\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn rebinding_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/rebind_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot rebind `total` here: it is declared in an enclosing block\n\
         \x20--> tests/fixtures/rebind_error.gcsl:2:19\n\
         \x20 |\n\
         1 | let total = 0\n\
         \x20 | ------------- declared here\n\
         2 | for x in [1, 2] { total := total + x }\n\
         \x20 |                   ^^^^^\n\
         \x20 |\n\
         \x20 = note: shadow it with `let total = ...` instead; only a variable declared with \
         `let mut` can be rebound from an inner block\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/mut_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot rebind `total` here: it is declared outside this function\n\
         \x20--> tests/fixtures/mut_error.gcsl:2:29\n\
         \x20 |\n\
         1 | let mut total = 0\n\
         \x20 | ----------------- declared here\n\
         2 | let add = fn(x: Int): Int { total := total + x; total }\n\
         \x20 |                             ^^^^^\n\
         \x20 |\n\
         \x20 = note: a function holds its own copy of each variable it uses, made when the \
         function is created; rebinding the copy would not change `total`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_works_through_a_queue_with_every_kind_of_loop() {
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{done = 2; costs = [3, 8, 5]; loose = [{}, \"build\"]; first = [\"build\"]; spent = [6, \
         2]; attempts = [1, 2]; rows = [[10, 20], [50]]}\n"
    );
    // Find-first without a final `continue` collects `{}` too: a warning.
    // With one, none.
    assert_eq!(
        stderr(&output),
        "warning: the values this loop collects have different types\n\
         \x20--> tests/fixtures/loops.gcsl:9:13\n\
         \x20 |\n\
         9 | let loose = for j in jobs { if j.cost > 5 { break j.name } }\n\
         \x20 |             ^^^             ------------------------------\n\
         \x20 |                             |                     |\n\
         \x20 |                             |                     this is String\n\
         \x20 |                             this is {}\n\
         \x20 |\n\
         \x20 = note: the loop is `List<String | {}>`; annotate it with that type if all are \
         meant, or tell them apart with `is`\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_loops_and_their_stops() {
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nDo @825..827\n",
        "\nBreak @473..478\n",
        "\nContinue @1017..1025\nLabel(rows) @1025..1030\n",
        "\nFor @972..975\nLabel(rows) @975..980\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n    (for @306..363\n\
         \x20     (let i @310..319\n\
         \x20       (int 0 @318..319))\n\
         \x20     (cond\n",
        "\n      (rebind i @336..346\n",
        "\n    (do @825..871\n\
         \x20     (block @828..857\n",
        "\n      (cond\n\
         \x20       (< @862..871\n",
        "(for'rows r @972..1044\n",
        "(continue'rows @1017..1030)",
        "(break @473..485\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(for : List<{}> @205..238\n",
        "(for : List<Int> @306..363\n",
        "(for j : List<String | {}> @441..489\n",
        "(for j : List<String> @558..616\n",
        "(do : List<Int> @825..871\n",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/loops.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A statement loop collects nothing.
        "\n  (loop-each @205..238\n",
        // A header's items before the condition run once, in a block
        // around the loop; those after it are the loop's last parts.
        "\n    (block @306..363\n\
         \x20     (store costs#50 @310..319\n",
        "\n      (loop @306..363\n",
        // `for { }` tests `true`.
        "\n    (loop @692..753\n\
         \x20     (bool true @692..695)\n",
        "\n    (do @825..871\n",
        // A stop counts the loops it leaves.
        "(break 0 @473..485\n",
        "(continue 0 @606..614)",
        "(continue 1 @1017..1030)",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn loop_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/loop_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot rebind `i` here: it is declared in the header of a `for`\n\
         \x20--> tests/fixtures/loop_error.gcsl:1:50\n\
         \x20 |\n\
         1 | let squares = for let i = 0; i < 3; i := i + 1 { i := i * i }\n\
         \x20 |                   --------- declared here        ^\n\
         \x20 |\n\
         \x20 = note: a variable of a `for` header is rebound in the header, after the \
         condition; declare it with `let mut i = ...` in the header to rebind it from the body \
         too\n\
         \n\
         error: `break` cannot leave a function\n\
         \x20--> tests/fixtures/loop_error.gcsl:2:56\n\
         \x20 |\n\
         2 | let found = for x in [1, 2] { let check = fn(): Bool { break x }; x }\n\
         \x20 |                                           --           ^^^^^\n\
         \x20 |                                           |\n\
         \x20 |                                           inside a function here\n\
         \x20 |\n\
         \x20 = note: the body of a function runs when it is called, which may be after the \
         loop has ended\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_tells_the_members_of_unions_apart() {
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{ports = [8080, 5432]; label = 2; first = 8081; same = false}\n"
    );
    // The two services' ports differ: a union, and a warning.
    assert_eq!(
        stderr(&output),
        "warning: the elements of this list have different types\n\
         \x20--> tests/fixtures/unions.gcsl:2:48\n\
         \x20 |\n\
         2 | let services = [{ name = \"web\"; port = 8080 }, { name = \"db\"; port = \
         \"postgres\" }]\n\
         \x20 |                 -----------------------------  \
         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ this is {name: String, port: String}\n\
         \x20 |                 |\n\
         \x20 |                 the elements so far are {name: String, port: Int}\n\
         \x20 |\n\
         \x20 = note: the list is `List<{name: String, port: Int | String}>`; annotate it with \
         that type if both are meant, or tell them apart with `is`\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_unions_tests_and_narrowing() {
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in ["\nIs @326..328\n", "\nAs @629..631\n"] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    let node = "(is Int @324..332\n\
                \x20             (name p @324..325))\n";
    assert!(ast.contains(node), "{node}: {ast}");
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        // The field the elements disagree on is the union of both.
        "(name s : {name: String, port: Int | String} @312..313)",
        // `p` is an Int in one branch and a String in the other.
        "(name p : Int | String @324..325))",
        "(name p : Int @335..336)",
        "(name p : String @347..348)",
        "(as Int : Int @612..635",
        "(=== : Bool @708..745",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/unions.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // The test looks at the kind alone, and writes the slot the
        // branches read, which a later variable takes once it is free.
        "(is int first#52 @324..332\n\
         \x20           (load label#51 @324..325))\n\
         \x20         (block @333..338\n\
         \x20           (load first#52 @335..336))\n",
        "(as int @612..635\n",
        "(value-same === @708..745\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn union_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/union_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "warning: the branches of this `if` have different types\n\
         \x20--> tests/fixtures/union_error.gcsl:1:36\n\
         \x20 |\n\
         1 | let port = if true { 8080 } else { \"http\" }\n\
         \x20 |                      ----          ^^^^^^ this is String\n\
         \x20 |                      |\n\
         \x20 |                      this is Int\n\
         \x20 |\n\
         \x20 = note: the `if` is `Int | String`; annotate it with that type if both are meant, \
         or tell them apart with `is`\n\
         \n\
         error: cannot add `Int | String` and Int\n\
         \x20--> tests/fixtures/union_error.gcsl:2:6\n\
         \x20 |\n\
         2 | port + 1\n\
         \x20 | ---- ^ - Int\n\
         \x20 | |\n\
         \x20 | Int | String\n\
         \x20 |\n\
         \x20 = note: tell them apart first, as in `if port is Int { ... }`\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/as_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `port` is not an Int\n\
         \x20--> tests/fixtures/as_error.gcsl:2:1\n\
         \x20 |\n\
         2 | port as Int\n\
         \x20 | ^^^^^^^^^^^ it is a String\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_updates_elements_of_lists() {
    let output = gcsl(&["run", "tests/fixtures/update.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{up = true; ports = [81, 5433]; cells = [[0, 1], [2, 3]]; counts = [0, 5, 0]; before = \
         [0, 0, 0]; r = {a = 8; b = 7}}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_element_updates() {
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    let token = "\nIdent(hosts) @247..252\nLBracket @252..253\nInt(0) @253..254\nRBracket \
                 @254..255\nDot @255..256\nIdent(up) @256..258\nColonEq @259..261\n";
    assert!(tokens.contains(token), "{token}: {tokens}");
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        // A path shows as it is written, and each position before the
        // value.
        "\n  (rebind hosts[0].up @247..266\n\
         \x20   (at\n\
         \x20     (int 0 @253..254))\n\
         \x20   (bool true @262..266))\n",
        "\n        (rebind hosts[i].ports[0] @364..406\n\
         \x20         (at\n\
         \x20           (name i @370..371))\n\
         \x20         (at\n\
         \x20           (int 0 @379..380))\n",
        "\n              (rebind grid[y][x] @497..520\n\
         \x20               (at\n\
         \x20                 (name y @502..503))\n\
         \x20               (at\n\
         \x20                 (name x @505..506))\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "\n  (rebind counts[1] @628..642\n\
         \x20   (at\n\
         \x20     (int 1 : Int @635..636))\n",
        // The list another variable holds keeps its type, and so does the
        // one updated.
        "(name before : List<Int> @884..890)",
        "(name counts : List<Int> @866..872)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/update.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // One update for the whole path: its value, its list and its
        // positions, and its steps.
        "\n  (store hosts#48 @247..266\n\
         \x20   (update [].up @247..258\n\
         \x20     (bool true @262..266)\n\
         \x20     (load hosts#48 @247..252)\n\
         \x20     (int 0 @253..254)))\n",
        "\n      (store hosts#48 @364..406\n\
         \x20       (update [].ports[] @364..381\n\
         \x20         (int-add @385..406\n",
        "\n            (update [][] @497..507\n",
        // The value, which rebinds `r`, comes before the read of `r`.
        "\n  (store r#52 @735..778\n\
         \x20   (update .a @735..738\n\
         \x20     (block @742..778\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn update_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/update_range_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: index out of range\n\
         \x20--> tests/fixtures/update_range_error.gcsl:2:1\n\
         \x20 |\n\
         2 | ports[2] := 8080\n\
         \x20 | ^^^^^^^^ the index is 2, and the list has 2 elements\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/update_list_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot update `replicas[0]`: `replicas` is Int, not a list\n\
         \x20--> tests/fixtures/update_list_error.gcsl:2:1\n\
         \x20 |\n\
         2 | replicas[0] := 4\n\
         \x20 | --------^^^\n\
         \x20 | |\n\
         \x20 | this is Int\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_rebinds_mutable_variables_across_lines() {
    let output = repl(
        "let mut n = 0\n\
         for i in 0..4 { n := n + i }\n\
         n\n\
         let f = fn(): Int { n := 1; n }\n\
         { n := n * 2 }\n\
         n\n",
    );
    // The `for` is its input's result, a list of its body's `{}`s.
    assert_eq!(
        stdout(&output),
        "[{}, {}, {}, {}] : List<{}>\n6 : Int\n12 : Int\n"
    );
    assert!(
        stderr(&output)
            .starts_with("error: cannot rebind `n` here: it is declared outside this function\n"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_spreads_defaults_into_a_configuration() {
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{host = \"localhost\"; port = 8080; url = \"localhost:8080\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_spreads() {
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    assert!(
        tokens.contains("\nDotDotDot @176..179\nIdent(defaults) @179..187\n"),
        "{tokens}"
    );
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    let node = "\n    (block @172..227\n\
                \x20     (spread @176..187\n\
                \x20       (name defaults @179..187))\n\
                \x20     (field port @190..201\n";
    assert!(ast.contains(node), "{node}: {ast}");
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "\n    (block : {host: String, port: Int, url: String} @172..227\n\
         \x20     (spread @176..187\n\
         \x20       (name defaults : {host: String, port: Int} @179..187))\n",
        // The names the spread defines, one of them overridden.
        "(name host : String @212..216)",
        "(name port : Int @219..223)",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/spread.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    // The operand is stored once, in a slot of its own, and each field is
    // read from it by name into the field's slot; the override stores into
    // the same slot, and the field keeps its place.
    let node = "\n      (store url#51 @176..187\n\
                \x20       (load defaults#48 @179..187))\n\
                \x20     (store conf#49 @176..187\n\
                \x20       (field host @179..187\n\
                \x20         (load url#51 @179..187)))\n\
                \x20     (store port#50 @176..187\n\
                \x20       (field port @179..187\n\
                \x20         (load url#51 @179..187)))\n\
                \x20     (store port#50 @190..201\n\
                \x20       (int 8080 @197..201))\n";
    assert!(ir.contains(node), "{node}: {ir}");
    let node = "\n      (make-record [host port url] @172..227\n\
                \x20       (load conf#49 @179..187)\n\
                \x20       (load port#50 @190..194)\n\
                \x20       (load url#51 @204..207))))\n";
    assert!(ir.contains(node), "{node}: {ir}");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_spread_of_a_field_defined_before_it_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/spread_twice.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: field `port` is defined twice\n\
         \x20--> tests/fixtures/spread_twice.gcsl:4:6\n\
         \x20 |\n\
         3 |   port = 8080\n\
         \x20 |   ---- first defined here\n\
         4 |   ...base\n\
         \x20 |      ^^^^ `port` is a field of this, of type Int\n\
         \x20 |\n\
         \x20 = note: a spread cannot replace a field defined before it; put the spread first\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_shows_lists_and_their_types() {
    let output = repl(
        "let xs = [1, lazy 2]\n\
         xs\n\
         \"{xs}\"\n\
         xs[1] + len(xs)\n\
         [...xs, 3][0]\n",
    );
    assert_eq!(
        stdout(&output),
        "[1, Future<Int>] : List<Int | Future<Int>>\n\
         \"[1, 2]\" : String\n\
         4 : Int\n\
         1 : Int | Future<Int>\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_shows_futures_as_their_types() {
    let output = repl(
        "let x = lazy 20 + 1\n\
         x\n\
         x * 2\n\
         await x\n\
         { f = x }\n",
    );
    assert_eq!(
        stdout(&output),
        "Future<Int> : Future<Int>\n\
         42 : Int\n\
         21 : Int\n\
         {f = Future<Int>} : {f: Future<Int>}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// The warning `tests/fixtures/warning.gcsl` produces, as rendered.
const FIXTURE_WARNING: &str = "warning: field `z` is not in `{x: Int}`\n\
     \x20--> tests/fixtures/warning.gcsl:1:32\n\
     \x20 |\n\
     1 | let point: {x: Int} = { x = 1; z = 2 }\n\
     \x20 |            --------            ^\n\
     \x20 |            |\n\
     \x20 |            expected because of this annotation\n\
     \x20 |\n\
     \x20 = note: it is allowed, but nothing with this type can see it\n";

#[test]
fn warnings_go_to_stderr_and_the_value_still_prints() {
    let output = gcsl(&["run", "tests/fixtures/warning.gcsl"]);
    assert_eq!(stdout(&output), "1\n");
    assert_eq!(stderr(&output), FIXTURE_WARNING);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn with_errors_warnings_go_to_stderr_with_them_in_source_order() {
    let output = gcsl(&["run", "tests/fixtures/warning_and_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "warning: field `z` is not in `{x: Int}`\n\
         \x20--> tests/fixtures/warning_and_error.gcsl:1:32\n\
         \x20 |\n\
         1 | let point: {x: Int} = { x = 1; z = 2 }\n\
         \x20 |            --------            ^\n\
         \x20 |            |\n\
         \x20 |            expected because of this annotation\n\
         \x20 |\n\
         \x20 = note: it is allowed, but nothing with this type can see it\n\
         \n\
         error: no field `z` in `{x: Int}`\n\
         \x20--> tests/fixtures/warning_and_error.gcsl:2:8\n\
         \x20 |\n\
         2 | = point.z\n\
         \x20 |        ^^\n\
         \x20 |\n\
         \x20 = note: the only field is `x`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn dumps_show_record_types_and_record_ir() {
    let output = gcsl(&["run", "tests/fixtures/warning.gcsl", "--dump-types"]);
    assert_eq!(
        stdout(&output),
        "(file\n\
         \x20 (let point : {x: Int} @0..38\n\
         \x20   (block : {x: Int, z: Int} @22..38\n\
         \x20     (field x @24..29\n\
         \x20       (int 1 : Int @28..29))\n\
         \x20     (field z @31..36\n\
         \x20       (int 2 : Int @35..36))))\n\
         \x20 (result @39..48\n\
         \x20   (. x : Int @41..48\n\
         \x20     (name point : {x: Int} @41..46))))\n"
    );
    // A run that stops after a dump still reports its warnings.
    assert_eq!(stderr(&output), FIXTURE_WARNING);
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/record.gcsl", "--dump-ir"]);
    assert_eq!(
        stdout(&output),
        "(block @32..45\n\
         \x20 (store x#45 @32..37\n\
         \x20   (int 1 @36..37))\n\
         \x20 (store y#46 @38..45\n\
         \x20   (str \"a\" @42..45))\n\
         \x20 (make-record [x y] @32..45\n\
         \x20   (load x#45 @32..33)\n\
         \x20   (load y#46 @38..39)))\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_names_and_slots() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/empty.gcsl",
        "--dump-tokens",
        "--dump-ir",
    ]);
    assert_eq!(
        stdout(&output),
        "Let @0..3\n\
         Ident(count) @4..9\n\
         Eq @10..11\n\
         Int(1) @12..13\n\
         Newline @13..14\n\
         Ident(count) @14..19\n\
         ColonEq @20..22\n\
         Ident(count) @23..28\n\
         Plus @29..30\n\
         Int(1) @31..32\n\
         Newline @32..33\n\
         Eof @32..32\n\
         (block @0..32\n\
         \x20 (store count#48 @0..13\n\
         \x20   (int 1 @12..13))\n\
         \x20 (store count#49 @14..32\n\
         \x20   (int-add @23..32\n\
         \x20     (load count#48 @23..28)\n\
         \x20     (int 1 @31..32)))\n\
         \x20 (empty @0..32))\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_stops_before_a_runtime_error() {
    let output = gcsl(&["run", "tests/fixtures/runtime_error.gcsl", "--dump-ir"]);
    assert!(stdout(&output).starts_with("(block @0..36\n  (int-add @0..36\n"));
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_then_error_prints_dumped_stages_and_the_error() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/type_error.gcsl",
        "--dump-ast",
        "--dump-ir",
    ]);
    assert!(stdout(&output).starts_with("(file\n  (expr @0..16\n    (+ @0..16\n"));
    assert!(!stdout(&output).contains("float-"));
    assert!(stderr(&output).starts_with("error: cannot floor-divide Float\n"));
    assert_eq!(output.status.code(), Some(1));
}

/// The binary runs the pipeline on a thread with enough stack for the
/// deepest accepted expression, whatever the main thread's stack is.
#[test]
fn deepest_accepted_expression_runs() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("deep.gcsl");
    std::fs::write(&path, format!("1{}", " + 1".repeat(gcsl::MAX_DEPTH - 1))).unwrap();
    let output = gcsl(&["run", path.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(stderr(&output), "");
    assert_eq!(stdout(&output), format!("{}\n", gcsl::MAX_DEPTH));
    assert_eq!(output.status.code(), Some(0));
}

/// A syntax error in one file of a workspace, or in the file itself, runs
/// nothing: neither a file's top-level `sh` nor the file asked for. Every
/// syntax error of the workspace is rendered, and a type error of a file
/// that parses with them.
#[test]
fn a_syntax_error_anywhere_runs_nothing() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-syntax-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let marker = |name: &str| dir.join(name).to_str().unwrap().to_owned();
    let files = [
        ("workspace.gcsl", String::new()),
        (
            "top.gcsl",
            format!("z = sh(\"touch {}\")\nw: Int = \"s\"", marker("TOP")),
        ),
        (
            "bad.gcsl",
            format!("x = sh(\"touch {}\")\ny = (1 2)\nv = [3 3]", marker("BAD")),
        ),
    ];
    for (path, text) in &files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    let outputs: Vec<Output> = ["top.gcsl", "bad.gcsl"]
        .iter()
        .map(|file| gcsl(&["run", dir.join(file).to_str().unwrap()]))
        .collect();
    let markers = [dir.join("TOP").exists(), dir.join("BAD").exists()];
    // Without the errors, `top.gcsl` runs.
    std::fs::write(dir.join("bad.gcsl"), "x = 1").unwrap();
    std::fs::write(
        dir.join("top.gcsl"),
        &files[1].1[..files[1].1.find('\n').unwrap()],
    )
    .unwrap();
    let fixed = gcsl(&["run", dir.join("top.gcsl").to_str().unwrap()]);
    let ran = dir.join("TOP").exists();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(markers, [false, false]);
    assert_eq!(stderr(&fixed), "");
    assert!(ran);
    for output in &outputs {
        assert_eq!(stdout(output), "");
        let errors: Vec<&str> = stderr(output)
            .lines()
            .filter(|line| line.starts_with("error"))
            .collect();
        assert_eq!(
            errors,
            [
                "error: expected `)`, found `2`",
                "error: expected `,` or `]`, found `3`",
                "error: mismatched types",
            ],
            "{}",
            stderr(output)
        );
        assert_eq!(output.status.code(), Some(1));
    }
}

/// An error of the scan of a workspace (a field defined in two files, an
/// import of no module, files that need each other) runs nothing: neither a
/// file's top-level `sh`, in a file with the error or in another, nor the
/// file asked for. Every error of the scan is rendered, and a type error of
/// another file.
#[test]
fn a_scan_error_anywhere_runs_nothing() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("q")).unwrap();
    std::fs::create_dir_all(dir.join("cy")).unwrap();
    let marker = |name: &str| dir.join(name).to_str().unwrap().to_owned();
    let files = [
        ("workspace.gcsl", String::new()),
        (
            "top.gcsl",
            format!("z = sh(\"touch {}\")\nw: Int = \"s\"", marker("TOP")),
        ),
        ("q/a.gcsl", format!("x = sh(\"touch {}\")", marker("A"))),
        (
            "q/b.gcsl",
            format!("import nope\nx = sh(\"touch {}\")", marker("B")),
        ),
        (
            "cy/c.gcsl",
            format!("c = d\ne = sh(\"touch {}\")", marker("C")),
        ),
        ("cy/d.gcsl", "d = c".to_owned()),
    ];
    for (path, text) in &files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    let outputs: Vec<Output> = ["top.gcsl", "q/a.gcsl", "q/b.gcsl", "cy/c.gcsl"]
        .iter()
        .map(|file| gcsl(&["run", dir.join(file).to_str().unwrap()]))
        .collect();
    let markers = ["TOP", "A", "B", "C"].map(|name| dir.join(name).exists());
    // Without the errors, `top.gcsl` runs.
    std::fs::write(dir.join("q/b.gcsl"), "y = 1").unwrap();
    std::fs::write(dir.join("cy/d.gcsl"), "d = 1").unwrap();
    std::fs::write(
        dir.join("top.gcsl"),
        &files[1].1[..files[1].1.find('\n').unwrap()],
    )
    .unwrap();
    let fixed = gcsl(&["run", dir.join("top.gcsl").to_str().unwrap()]);
    let ran = dir.join("TOP").exists();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(markers, [false; 4]);
    assert_eq!(stderr(&fixed), "");
    assert!(ran);
    for output in &outputs {
        assert_eq!(stdout(output), "");
        let errors: Vec<&str> = stderr(output)
            .lines()
            .filter(|line| line.starts_with("error"))
            .collect();
        assert_eq!(
            errors,
            [
                "error: `cy/c.gcsl` and `cy/d.gcsl` need each other",
                "error: no module `nope`",
                "error: field `x` is defined in two files",
                "error: mismatched types",
            ],
            "{}",
            stderr(output)
        );
        assert_eq!(output.status.code(), Some(1));
    }
}

/// `--dump-ast` of a file with syntax errors prints nothing and shows them
/// all.
#[test]
fn dump_ast_shows_every_syntax_error() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-dump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("two.gcsl");
    std::fs::write(&path, "x = (1 2)\ny = 3\nz = [4 4]").unwrap();
    let output = gcsl(&["run", path.to_str().unwrap(), "--dump-ast"]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(stdout(&output), "");
    let errors: Vec<&str> = stderr(&output)
        .lines()
        .filter(|line| line.starts_with("error"))
        .collect();
    assert_eq!(
        errors,
        [
            "error: expected `)`, found `2`",
            "error: expected `,` or `]`, found `4`"
        ]
    );
    assert_eq!(output.status.code(), Some(1));
}

/// Runs the REPL with `input` on stdin until it ends.
fn repl(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn repl_keeps_variables_across_lines() {
    let output = repl(
        "let x = 1\n\
         let y = x + 1\n\
         x + y\n\
         x := x * 10; x + y\n\
         {}\n\
         let z = 1.5 + x\n\
         x\n",
    );
    // Nothing for `let`, `:=` or a `{}` value; the failed line changes
    // nothing.
    assert_eq!(stdout(&output), "3 : Int\n12 : Int\n10 : Int\n");
    assert_eq!(
        stderr(&output),
        "error: cannot add Float and Int\n\
         \x20--> <repl>:1:13\n\
         \x20 |\n\
         1 | let z = 1.5 + x\n\
         \x20 |         --- ^ - Int\n\
         \x20 |         |\n\
         \x20 |         Float\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_prints_strings_quoted() {
    let output = repl(
        "let s = \"a\\tb\"\n\
         s\n\
         s + \"\\\"c\\\"\"\n\
         \"{s} {1 < 2}\"\n\
         1 < 2\n\
         if true { \"yes\" } else { \"no\" }\n",
    );
    assert_eq!(
        stdout(&output),
        "\"a\\tb\" : String\n\
         \"a\\tb\\\"c\\\"\" : String\n\
         \"a\\tb true\" : String\n\
         true : Bool\n\
         \"yes\" : String\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_shows_records_with_their_types() {
    let output = repl(
        "let p = { x = 1; y = \"a\" }\n\
         p\n\
         p.y\n\
         let q: {x: Int} = p\n\
         q\n\
         { inner = { n = 1.5 }; \"a b\" = true }\n",
    );
    assert_eq!(
        stdout(&output),
        "{x = 1; y = \"a\"} : {x: Int, y: String}\n\
         \"a\" : String\n\
         {x = 1; y = \"a\"} : {x: Int}\n\
         {inner = {n = 1.5}; \"a b\" = true} : {\"a b\": Bool, inner: {n: Float}}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_prints_warnings_before_the_value() {
    let output = repl(
        "let p: {x: Int} = { x = 1; z = 2 }\n\
         { x = 1; z = 2 } : {x: Int}\n",
    );
    // The first line's value is `{}`, so only its warning is shown; the
    // second shows its warning and then its value.
    assert_eq!(stdout(&output), "{x = 1; z = 2} : {x: Int}\n");
    assert_eq!(
        stderr(&output),
        "warning: field `z` is not in `{x: Int}`\n\
         \x20--> <repl>:1:28\n\
         \x20 |\n\
         1 | let p: {x: Int} = { x = 1; z = 2 }\n\
         \x20 |        --------            ^\n\
         \x20 |        |\n\
         \x20 |        expected because of this annotation\n\
         \x20 |\n\
         \x20 = note: it is allowed, but nothing with this type can see it\n\
         warning: field `z` is not in `{x: Int}`\n\
         \x20--> <repl>:1:10\n\
         \x20 |\n\
         1 | { x = 1; z = 2 } : {x: Int}\n\
         \x20 |          ^         -------- ascribed here\n\
         \x20 |\n\
         \x20 = note: it is allowed, but nothing with this type can see it\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_gives_defaults_across_lines() {
    // A function of an earlier line gives its defaults; its type does not
    // show them, and a copy of it gives none.
    let output = repl(
        "def greet(name: String, greeting: String = \"hello\"): String { \"{greeting}, {name}\" }\n\
         greet(\"jack\")\n\
         greet(\"jack\", \"hi\")\n\
         let g = greet\n\
         g(\"jack\")\n",
    );
    assert_eq!(
        stdout(&output),
        "{greet = fn(name: String, greeting: String): String} : \
         {greet: fn(name: String, greeting: String): String}\n\
         \"hello, jack\" : String\n\
         \"hi, jack\" : String\n"
    );
    assert_eq!(
        stderr(&output),
        "error: expected 2 arguments, found 1\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | g(\"jack\")\n\
         \x20 | -^^^^^^^^\n\
         \x20 | |\n\
         \x20 | the function is fn(name: String, greeting: String): String\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The summary of slice 49: a record literal written where a type with a
/// defaulted field is expected is given the field.
#[test]
fn run_gives_record_literals_their_fields_defaults() {
    let output = gcsl(&["run", "tests/fixtures/record_defaults.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{entries = [{from = \"a\"; to = \"b\"; exclude = []}, \
         {from = \"c\"; to = \"d\"; exclude = [\"x\"]}]; \
         f = fn(e: {exclude: List<String> = [], from: String, to: String}): Int; n = 0}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // A value that is no such literal is not given it.
    let output = gcsl(&["run", "tests/fixtures/record_defaults_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: mismatched types\n\
         \x20--> tests/fixtures/record_defaults_error.gcsl:4:7\n\
         \x20 |\n\
         4 | m = f(plain)\n\
         \x20 |     - ^^^^^ expected {exclude: List<String> = [], from: String, to: String}, \
         found {from: String, to: String}\n\
         \x20 |     |\n\
         \x20 |     the function is fn(e: {exclude: List<String> = [], from: String, to: String}): Int\n\
         \x20 |\n\
         \x20 = note: a default fills only a record literal written where \
         `{exclude: List<String> = [], from: String, to: String}` is expected\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_gives_record_literals_defaults_across_lines() {
    // A type of an earlier line gives its defaults; a type shows them.
    let output = repl(
        "type Install = { from: String, exclude: List<String> = [] }\n\
         let e: Install = { from = \"a\" }\n\
         e\n\
         [{ from = \"b\" }] : List<Install>\n",
    );
    assert_eq!(
        stdout(&output),
        "{from = \"a\"; exclude = []} : {exclude: List<String> = [], from: String}\n\
         [{from = \"b\"; exclude = []}] : List<{exclude: List<String> = [], from: String}>\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// The three forms of slice 52: a record literal that a spread `for`'s
/// body, a `for` that is a function's result, or a body block results in,
/// where a type with a defaulted field is expected, is given the field.
#[test]
fn run_gives_defaults_to_literals_in_blocks_and_fors() {
    let output = gcsl(&["run", "tests/fixtures/record_defaults_reach.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{content = [{from = \"dest/usr/bin/zstd\"; to = \"usr/bin/zstd\"; exclude = []}, \
         {from = \"dest/usr/bin/unzstd\"; to = \"usr/bin/unzstd\"; exclude = []}]; \
         all = [{from = \"src/COPYING\"; to = \"x\"; exclude = []}, \
         {from = \"src/LGPL-2.1\"; to = \"x\"; exclude = []}]; \
         one = {from = \"src/LICENSE\"; to = \"y\"; exclude = []}}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// The form of slice 54: a record literal in the list literal that a
/// spread `if`'s `else` branch results in, where a list of a type with a
/// defaulted field is expected, is given the field.
#[test]
fn run_gives_defaults_to_literals_in_spread_ifs() {
    let output = gcsl(&["run", "tests/fixtures/record_defaults_spread.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{none_installed = []; content = [{from = \"a\"; to = \"b\"; exclude = []}, \
         {from = \"src/src/tools/cargo/LICENSE-MIT\"; to = \"c\"; exclude = []}]}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_gives_defaults_to_literals_in_blocks_and_ifs() {
    // A function of an earlier line whose body results in the literal; an
    // `if` whose branches do.
    let output = repl(
        "type Install = { from: String, exclude: List<String> = [] }\n\
         fn one(): Install { { from = \"a\" } }\n\
         one()\n\
         (if true { { from = \"b\" } } else { { from = \"c\" } }) : Install\n",
    );
    assert_eq!(
        stdout(&output),
        "{from = \"a\"; exclude = []} : {exclude: List<String> = [], from: String}\n\
         {from = \"b\"; exclude = []} : {exclude: List<String> = [], from: String}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_shows_functions_and_their_errors_against_their_own_line() {
    let output = repl(
        "fn div(a: Int, b: Int): Int { a // b }\n\
         let x = 10\n\
         let get = fn(): Int { x }\n\
         x := 20\n\
         get()\n\
         get\n\
         div(1, 0)\n\
         div(4, 2)\n",
    );
    assert_eq!(
        stdout(&output),
        "10 : Int\n\
         fn(): Int : fn(): Int\n\
         2 : Int\n"
    );
    // The error inside `div` is shown against the line that defined it.
    assert_eq!(
        stderr(&output),
        "error: division by zero\n\
         \x20--> <repl>:1:33\n\
         \x20 |\n\
         1 | fn div(a: Int, b: Int): Int { a // b }\n\
         \x20 |                               - ^^ - 0\n\
         \x20 |                               |\n\
         \x20 |                               1\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_leaves_functions_blocks_and_loops_early() {
    let output = gcsl(&["run", "tests/fixtures/return.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{signs = [-1, 0, 1]; job = 7; pick = \"cached\"; hit = 8; polled = 3; capped = [3, 5, \
         5]}\n"
    );
    // A loop that can end as its list, or be returned from: a union, and a
    // warning. One that cannot end: neither.
    assert_eq!(
        stderr(&output),
        "warning: this loop is its list or a value returned to it, of different types\n\
         \x20 --> tests/fixtures/return.gcsl:11:11\n\
         \x20  |\n\
         11 | let hit = for'scan c in costs { if c > 5 { return'scan c } }\n\
         \x20  |           ^^^ its list is List<{}>                     - this is Int\n\
         \x20  |\n\
         \x20  = note: the loop is `Int | List<{}>`; annotate it with that type if all are \
         meant, or tell them apart with `is`\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_uses_top_level_functions_in_any_order() {
    let output = gcsl(&["run", "tests/fixtures/toplevel_functions.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{limit = 10; total = 55; describe = fn(n: Int): String; scaled = 12; seen = [1, 2]; \
         kind = \"odd\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_top_level_functions_and_where_they_are_made() {
    let path = "tests/fixtures/toplevel_functions.gcsl";
    let output = gcsl(&["run", path, "--dump-tokens"]);
    let tokens = stdout(&output);
    assert!(tokens.contains("\nDef @156..159\n"), "{tokens}");
    let output = gcsl(&["run", path, "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "\n  (def describe @156..225\n",
        "\n  (fn even @226..290\n",
        "\n  (fn scale @525..559\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", path, "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // Those that copy nothing are made as the input starts, in their
        // own slots, the first of the input's.
        "(block @123..726\n\
         \x20 (store describe#54 @156..225\n\
         \x20   (make-closure #0 @156..225))\n\
         \x20 (store even#55 @226..290\n",
        // One that copies `rate` just after `rate` is bound.
        "\n  (store rate#60 @494..506\n\
         \x20   (int 3 @505..506))\n\
         \x20 (store scale#58 @525..559\n\
         \x20   (make-closure #4 @525..559\n\
         \x20     (load rate#60 @525..559)))\n\
         \x20 (store scaled#48 @507..524\n",
        // One that copies a mutable variable where it is written.
        "\n  (store counted#59 @643..670\n\
         \x20   (make-closure #5 @643..670\n",
        // A function reads another of the top level from the top level's
        // frame, not by a capture.
        "\n        (fn top even#55 @190..194)\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_reports_a_function_used_before_it_exists() {
    let output = gcsl(&["run", "tests/fixtures/functions_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `total` cannot be used here yet\n\
         \x20--> tests/fixtures/functions_error.gcsl:1:9\n\
         \x20 |\n\
         1 | let x = total()\n\
         \x20 |         ^^^^^\n\
         2 | let k = 10\n\
         \x20 | ---------- `total` reads `k`, which is bound here\n\
         \x20 |\n\
         \x20 = note: a function can be used once every variable it reads is bound\n\
         \n\
         error: `first` cannot be used here yet\n\
         \x20--> tests/fixtures/functions_error.gcsl:5:9\n\
         \x20 |\n\
         5 | let y = first()\n\
         \x20 |         ^^^^^\n\
         6 | fn first(): Int { second() }\n\
         7 | fn second(): Int { n }\n\
         \x20 | ---------------------- `first` uses `second`, and `second` reads the mutable \
         variable `n`, so it exists from here\n\
         \x20 |\n\
         \x20 = note: a function copies a mutable variable where the function is written\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_uses_top_level_fields_in_any_order() {
    let output = gcsl(&["run", "tests/fixtures/fields.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{com = {x = {app = {deps = [{name = \"lib\"}]; name = \"app-v2\"}; lib = {name = \
         \"lib\"}}}; version = \"v2\"; part = {a = 1; b = 2}; whole = {a = 1; b = 2}; label = \
         \"lib\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_the_cells_of_top_level_fields_and_their_points() {
    let output = gcsl(&["run", "tests/fixtures/fields.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // `com.x.lib.name` and the interior node `com.x.lib` are computed
        // before `com.x.app.deps`, the first item that needs them.
        "\n  (store name#51 @273..295\n",
        "\n  (store lib#50 @273..295\n\
         \x20   (make-record [name] @273..295\n\
         \x20     (load name#51 @273..295)))\n\
         \x20 (store deps#48 @211..239\n\
         \x20   (make-list @228..239\n\
         \x20     (load lib#50 @229..238)))\n\
         \x20 (store version#52 @296..321\n",
        // An interior node is assembled from its children's cells once all
        // of them are computed.
        "\n  (store app#47 @211..239\n\
         \x20   (make-record [deps name] @211..239\n\
         \x20     (load deps#48 @211..239)\n\
         \x20     (load name#49 @240..272)))\n",
        // A root read between its items is computed whole, before the read.
        "\n  (store part#53 @526..536\n\
         \x20   (make-record [a b] @526..536\n\
         \x20     (load a#54 @526..536)\n\
         \x20     (load b#56 @550..560)))\n\
         \x20 (store whole#55 @537..549\n",
        // The record of the input is made from the roots' cells.
        "\n  (make-record [com version part whole label] @59..574\n\
         \x20   (load com#45 @211..214)\n",
        // A function reads a cell from the top level's frame, not by a
        // capture.
        "\n    (top top name#51 @451..465))",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    // No dotted field rebuilds its root item by item.
    assert!(!ir.contains("update-field"), "{ir}");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn run_reports_a_field_used_before_it_can_be_computed_and_a_cycle() {
    let output = gcsl(&["run", "tests/fixtures/fields_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `com` cannot be used here yet\n\
         \x20--> tests/fixtures/fields_error.gcsl:1:5\n\
         \x20 |\n\
         1 | x = com\n\
         \x20 |     ^^^\n\
         2 | com.x.app.name = \"app-{version}\"\n\
         3 | let major = 2\n\
         \x20 | ------------- `com` has `com.x.app.name`, `com.x.app.name` uses `version`, and \
         `version` reads `major`, which is bound here\n\
         \x20 |\n\
         \x20 = note: a field can be computed once every variable it reads is bound\n\
         \n\
         error: `a` depends on itself\n\
         \x20--> tests/fixtures/fields_error.gcsl:5:1\n\
         \x20 |\n\
         5 | a = b + 1\n\
         \x20 | ^   - `a` uses `b` here\n\
         6 | b = a + 1\n\
         \x20 |     - `b` uses `a` here\n\
         \x20 |\n\
         \x20 = note: a field cannot be computed from itself, even through a function\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn dumps_show_returns_and_their_catches() {
    let output = gcsl(&["run", "tests/fixtures/return.gcsl", "--dump-tokens"]);
    let tokens = stdout(&output);
    for token in [
        "\nReturn @102..108\nNeg @109..110\n",
        "\nLBrace @384..385\nLabel(pick) @385..390\n",
        "\nFor @532..535\nLabel(scan) @535..540\n",
    ] {
        assert!(tokens.contains(token), "{token}: {tokens}");
    }
    let output = gcsl(&["run", "tests/fixtures/return.gcsl", "--dump-ast"]);
    let ast = stdout(&output);
    for node in [
        "(return @102..111\n",
        "(block'pick @384..436\n",
        "(return'pick @403..423\n",
        "(for'scan c @532..582\n",
        "(block'it @858..892\n",
    ] {
        assert!(ast.contains(node), "{node}: {ast}");
    }
    let output = gcsl(&["run", "tests/fixtures/return.gcsl", "--dump-types"]);
    let types = stdout(&output);
    for node in [
        "(lazy : Future<Int> @282..316\n",
        "(block'pick : String @384..436\n",
        "(for'scan c : Int | List<{}> @532..582\n",
        // A loop that cannot end is only what is returned to it.
        "(for'spin : Int @692..759\n",
        "(for c : List<Int> @843..892\n",
    ] {
        assert!(types.contains(node), "{node}: {types}");
    }
    let output = gcsl(&["run", "tests/fixtures/return.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    for node in [
        // A function's `return` names it by `fn`.
        "(return fn @102..111\n",
        // A labelled block's and a loop's by their catches' numbers.
        "(catch 0 @384..436\n",
        "(return 0 @403..423\n",
        "\n    (catch 1 @532..582\n\
         \x20     (for hit#54 @532..582\n",
        // A loop that cannot end collects nothing.
        "\n    (catch 2 @692..759\n\
         \x20     (loop-each @692..759\n",
    ] {
        assert!(ir.contains(node), "{node}: {ir}");
    }
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn return_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/return_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `return` outside a function\n\
         \x20--> tests/fixtures/return_error.gcsl:2:1\n\
         \x20 |\n\
         2 | return limit\n\
         \x20 | ^^^^^^\n\
         \x20 |\n\
         \x20 = note: only a function or a `lazy` can be returned from without a label; a block \
         or a loop labelled `'L` is left with `return'L`\n\
         \n\
         error: `return` cannot leave a function\n\
         \x20--> tests/fixtures/return_error.gcsl:3:56\n\
         \x20 |\n\
         3 | let found = for'scan x in [1, 2] { let f = fn(): Int { return'scan x }; x }\n\
         \x20 |                -----                       --          ^^^^^^^^^^^\n\
         \x20 |                |                           |\n\
         \x20 |                |                           inside a function here\n\
         \x20 |                the loop it names is outside it\n\
         \x20 |\n\
         \x20 = note: the body of a function runs when it is called, which may be after the \
         loop has ended\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// Each file of a workspace prints its own value: its own fields, with the
/// directory's declarations and those of the directories above in scope.
/// The nested workspace and the names starting with `.` are not files of
/// `ws`: `.hidden/bad.gcsl` does not parse, and both they and the nested
/// workspace declare `version` again, which would be errors.
#[test]
fn run_runs_a_file_of_a_workspace() {
    for (file, value) in [
        (
            "app",
            "{com = {x = {app = {name = \"app-1.2\"; deps = [{name = \"lib-1.2\"; src = \
             \"lib-1.2.tar\"}]}}}}\n",
        ),
        (
            "common",
            "{version = \"1.2\"; tagged = fn(n: String): String}\n",
        ),
        (
            "lib",
            "{com = {x = {lib = {name = \"lib-1.2\"; src = \"lib-1.2.tar\"}}}}\n",
        ),
        ("workspace", ""),
        (
            "sub/tool",
            "{tool = \"tool-1.2\"; uses = \"lib-1.2.tar\"}\n",
        ),
        ("nested/inner", "{inner = \"nested\"}\n"),
        ("nested/workspace", "{version = \"nested\"}\n"),
    ] {
        let output = gcsl(&["run", &format!("tests/fixtures/ws/{file}.gcsl")]);
        assert_eq!(stdout(&output), value, "{file}");
        assert_eq!(stderr(&output), "", "{file}");
        assert_eq!(output.status.code(), Some(0), "{file}");
    }
}

#[test]
fn run_renders_a_workspace_conflict_with_both_files() {
    let output = gcsl(&["run", "tests/fixtures/ws_conflict/app.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: field `com.x.lib.name` is defined in two files\n\
         \x20--> tests/fixtures/ws_conflict/lib.gcsl:1:1\n\
         \x20 |\n\
         1 | com.x.lib.name = \"lib\"\n\
         \x20 | ^^^^^^^^^^^^^^ defined here\n\
         \x20 |\n\
         \x20::: tests/fixtures/ws_conflict/app.gcsl:1:1\n\
         \x20 |\n\
         1 | com.x.lib.name = \"app\"\n\
         \x20 | -------------- defined here\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_renders_files_that_need_each_other() {
    let output = gcsl(&["run", "tests/fixtures/ws_cycle/lib.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `app.gcsl` and `lib.gcsl` need each other\n\
         \x20--> tests/fixtures/ws_cycle/app.gcsl:2:19\n\
         \x20 |\n\
         2 | com.x.app.deps = [com.x.lib]\n\
         \x20 |                   ^^^^^^^^^ `app.gcsl` uses `com.x.lib` here\n\
         \x20 |\n\
         \x20::: tests/fixtures/ws_cycle/lib.gcsl:2:20\n\
         \x20 |\n\
         2 | com.x.lib.users = [com.x.app.name]\n\
         \x20 |                    -------------- `lib.gcsl` uses `com.x.app.name` here\n\
         \x20 |\n\
         \x20 = note: a file runs as a whole before a file that needs it, so two files cannot \
         need each other\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// A file does not see the declarations of a directory below it, and its
/// error stops the run of every file of the workspace, the file below
/// included.
#[test]
fn run_hides_a_subdirectorys_declarations_from_the_directory_above() {
    for path in [
        "tests/fixtures/ws_below/top.gcsl",
        "tests/fixtures/ws_below/sub/inner.gcsl",
    ] {
        let output = gcsl(&["run", path]);
        assert_eq!(stdout(&output), "", "{path}");
        assert_eq!(
            stderr(&output),
            "error: undefined variable `count`\n\
             \x20--> tests/fixtures/ws_below/top.gcsl:1:9\n\
             \x20 |\n\
             1 | total = count + 1\n\
             \x20 |         ^^^^^\n",
            "{path}"
        );
        assert_eq!(output.status.code(), Some(1), "{path}");
    }
}

/// A runtime error in a function of another file is rendered against that
/// file.
#[test]
fn run_renders_a_runtime_error_in_another_files_function() {
    let output = gcsl(&["run", "tests/fixtures/ws_closure/app.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: index out of range\n\
         \x20--> tests/fixtures/ws_closure/lib.gcsl:1:43\n\
         \x20 |\n\
         1 | def pick(list: List<Int>, at: Int): Int { list[at] }\n\
         \x20 |                                           ^^^^^^^^ the index is 2, and the list \
         has 2 elements\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The dumps are the file's alone: its tokens and tree, and its types and
/// IR as checked in the workspace, where `tagged` and `com.x.lib` are
/// another file's.
#[test]
fn dumps_in_a_workspace_are_the_files_alone() {
    let path = "tests/fixtures/ws/app.gcsl";
    let output = gcsl(&["run", path, "--dump-tokens"]);
    assert!(stdout(&output).starts_with("Ident(com) @0..3\n"));
    assert!(!stdout(&output).contains("version"));
    let output = gcsl(&["run", path, "--dump-ast"]);
    assert!(stdout(&output).starts_with("(file\n  (field com.x.app.name @0..30\n"));
    let output = gcsl(&["run", path, "--dump-types"]);
    assert!(stdout(&output).contains("(name tagged : fn(n: String): String @17..23)"));
    assert!(!stdout(&output).contains("version"));
    let output = gcsl(&["run", path, "--dump-ir"]);
    let ir = stdout(&output);
    assert!(ir.contains("(load lib#52 @49..58)"), "{ir}");
    assert!(!ir.contains("store version"), "{ir}");
    assert_eq!(output.status.code(), Some(0));
}

/// The imports summary: `app/main.gcsl` reads `lib/`'s leaves, a member of
/// it, a file of it and a value file of `tools/`, each through its alias;
/// the imported files run alone as they would without the importer.
#[test]
fn run_follows_imports_across_directories() {
    for (file, value) in [
        (
            "app/main",
            "{com = {x = {app = {deps = [{name = \"lib-1.2\"}, {name = \"ylib\"}]; tool = \
             \"build-1.2\"; all = \"ylib\"}}}}\n",
        ),
        ("lib/x", "{com = {x = {lib = {name = \"lib-1.2\"}}}}\n"),
        ("tools/build", "build-1.2\n"),
    ] {
        let output = gcsl(&["run", &format!("tests/fixtures/ws_imports/{file}.gcsl")]);
        assert_eq!(stdout(&output), value, "{file}");
        assert_eq!(stderr(&output), "", "{file}");
        assert_eq!(output.status.code(), Some(0), "{file}");
    }
}

/// A path into a nested workspace is no module of this one, and the note
/// says why: the CLI tells the scan which directories it left out.
#[test]
fn run_renders_no_module_with_its_note() {
    let output = gcsl(&["run", "tests/fixtures/ws_no_module/app.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: no module `vendor.pkg`\n\
         \x20--> tests/fixtures/ws_no_module/app.gcsl:1:8\n\
         \x20 |\n\
         1 | import vendor.pkg\n\
         \x20 |        ^^^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: `vendor/` is a workspace of its own\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// Two files of two directories that import each other need each other.
#[test]
fn run_renders_an_import_cycle() {
    let output = gcsl(&["run", "tests/fixtures/ws_import_cycle/a/one.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `a/one.gcsl` and `b/two.gcsl` need each other\n\
         \x20--> tests/fixtures/ws_import_cycle/a/one.gcsl:3:8\n\
         \x20 |\n\
         3 | uses = two.second\n\
         \x20 |        ^^^^^^^^^^ `a/one.gcsl` uses `second` here\n\
         \x20 |\n\
         \x20::: tests/fixtures/ws_import_cycle/b/two.gcsl:2:10\n\
         \x20 |\n\
         2 | second = one.first\n\
         \x20 |          --------- `b/two.gcsl` uses `first` here\n\
         \x20 |\n\
         \x20 = note: a file runs as a whole before a file that needs it, so two files cannot \
         need each other\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The dumps of an importing file print its imports as written, type each
/// reference through an alias as the node it reaches, and load the cells
/// of the files that own them.
#[test]
fn dumps_of_an_importing_file() {
    let path = "tests/fixtures/ws_imports/app/main.gcsl";
    let output = gcsl(&["run", path, "--dump-ast"]);
    assert!(stdout(&output).starts_with(
        "(file\n  (import lib @0..10)\n  (import tools.build as builder @11..40)\n  \
         (import com as libs from lib @41..68)\n  (import lib.y @69..81)\n"
    ));
    let output = gcsl(&["run", path, "--dump-types"]);
    let types = stdout(&output);
    assert!(
        types.contains("(name builder : String @144..151)"),
        "{types}"
    );
    assert!(
        types.contains("(name libs : {y: {lib: {name: String}}} @115..119)"),
        "{types}"
    );
    let output = gcsl(&["run", path, "--dump-ir"]);
    let ir = stdout(&output);
    assert!(ir.contains("(load build#59 @144..151)"), "{ir}");
    assert!(!ir.contains("store version"), "{ir}");
    assert!(!ir.contains("store name"), "{ir}");
    assert_eq!(output.status.code(), Some(0));
}

/// The merging summary: dotted paths merge across a directory boundary and
/// an import, each file reading its own view; `all` in `v.gcsl` has the
/// root's part, `app/`'s and its own, root directory first, not `lib/`'s.
#[test]
fn run_merges_dotted_paths_across_directories_and_imports() {
    for (file, value) in [
        (
            "app/main",
            "{com = {x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; base = \
             \"base\"}}}}\n",
        ),
        (
            "app/vendor/v",
            "{com = {vendor = {thing = 1}}; all = {base = {name = \"base\"}; x = {app = {deps = \
             [{name = \"lib\"}, {name = \"ylib\"}]; base = \"base\"}}; vendor = {thing = 1}}}\n",
        ),
    ] {
        let output = gcsl(&["run", &format!("tests/fixtures/ws_merge/{file}.gcsl")]);
        assert_eq!(stdout(&output), value, "{file}");
        assert_eq!(stderr(&output), "", "{file}");
        assert_eq!(output.status.code(), Some(0), "{file}");
    }
}

/// One leaf in a directory and in a directory above: a conflict, reported
/// at the directory above, with the note.
#[test]
fn run_renders_a_conflict_across_a_directory_boundary() {
    let output = gcsl(&["run", "tests/fixtures/ws_merge/conflict_above/sub/b.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: field `version` is defined in two files\n\
         \x20--> tests/fixtures/ws_merge/conflict_above/workspace.gcsl:1:1\n\
         \x20 |\n\
         1 | version = \"1\"\n\
         \x20 | ^^^^^^^ defined here\n\
         \x20 |\n\
         \x20::: tests/fixtures/ws_merge/conflict_above/sub/b.gcsl:1:1\n\
         \x20 |\n\
         1 | version = \"2\"\n\
         \x20 | ------- defined here\n\
         \x20 |\n\
         \x20 = note: one is in a directory above\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// One leaf of the file and of a file it imports with `*`: a conflict at
/// the import, with the imported definition's excerpt.
#[test]
fn run_renders_a_conflict_through_an_import() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_merge/conflict_import/app/main.gcsl",
    ]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: field `com.x.lib.name` is defined in two files\n\
         \x20--> tests/fixtures/ws_merge/conflict_import/app/main.gcsl:1:8\n\
         \x20 |\n\
         1 | import * from lib\n\
         \x20 |        ^ imported here\n\
         2 | com.x.lib.name = \"app\"\n\
         \x20 | -------------- defined here\n\
         \x20 |\n\
         \x20::: tests/fixtures/ws_merge/conflict_import/lib/x.gcsl:1:1\n\
         \x20 |\n\
         1 | com.x.lib.name = \"lib\"\n\
         \x20 | -------------- defined here\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// A file outside any workspace runs alone, as before workspaces existed,
/// even with workspaces in the directories beside it.
#[test]
fn run_outside_a_workspace_is_unchanged() {
    let output = gcsl(&["run", "tests/fixtures/ok.gcsl"]);
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/type_error.gcsl"]);
    assert!(stderr(&output).contains(" --> tests/fixtures/type_error.gcsl:2:8\n"));
    assert!(!stderr(&output).contains(":::"));
}

/// The standard library's summary: the prelude's names, and `builtins` and
/// the prelude imported by name, in a lone file and in a workspace, whose
/// root's `half` shadows the prelude's.
#[test]
fn run_has_the_standard_library_alone_and_in_a_workspace() {
    let output = gcsl(&["run", "tests/fixtures/standard.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{xs = [1, 2, 3]; n = 3; h = 1.5; same = true; c = 3}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/ws_standard/app/main.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{xs = [1, 2, 3]; n = 3; h = 1.5; same = true; c = 3; i = 2}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // The dumps are the file's own: a call of `len` is the length of its
    // list; the prelude's `half` is read from its cell.
    let output = gcsl(&["run", "tests/fixtures/standard.gcsl", "--dump-ir"]);
    let ir = stdout(&output);
    assert!(
        ir.contains("  (store n#46 @87..98\n    (list-len @91..98\n      (load xs#45 @95..97)))\n"),
        "{ir}"
    );
    assert!(ir.contains("(load half#50 @103..107)"), "{ir}");
    assert!(!ir.contains("store len"), "{ir}");
    let output = gcsl(&["run", "tests/fixtures/standard.gcsl", "--dump-types"]);
    let types = stdout(&output);
    assert!(
        types.contains("(name count : fn(List<Int>): Int @193..198)"),
        "{types}"
    );
}

/// A special built-in used as a value, and a conflict with the prelude,
/// whose definition is shown in `<stdlib>/prelude.gcsl`.
#[test]
fn run_renders_the_alias_rule_and_the_prelude() {
    let output = gcsl(&["run", "tests/fixtures/builtin_value.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `len` is a built-in function and cannot be used as a value yet\n\
         \x20--> tests/fixtures/builtin_value.gcsl:1:10\n\
         \x20 |\n\
         1 | sizes = [len]\n\
         \x20 |          ^^^\n\
         \x20 |\n\
         \x20 = note: a built-in function with a type for any list cannot be written as a type \
         yet; call it, or bind it to a name\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl(&["run", "tests/fixtures/prelude_conflict.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: field `len` is defined in two files\n\
         \x20 --> tests/fixtures/prelude_conflict.gcsl:1:8\n\
         \x20  |\n\
         \x201 | import * from prelude\n\
         \x20  |        ^ imported here\n\
         \x202 | len = 3\n\
         \x20  | --- defined here\n\
         \x20  |\n\
         \x20 ::: <stdlib>/prelude.gcsl:22:1\n\
         \x20  |\n\
         22 | len = builtins.len\n\
         \x20  | --- defined here\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn repl_has_the_standard_library() {
    let output = repl(
        "len([1, 2])\n\
         let l = len\n\
         l([1])\n\
         import builtins; builtins.int(2.5)\n\
         len\n\
         let len = 3\n\
         len\n",
    );
    assert_eq!(stdout(&output), "2 : Int\n1 : Int\n2 : Int\n3 : Int\n");
    assert_eq!(
        stderr(&output),
        "error: `len` is a built-in function and cannot be used as a value yet\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | len\n\
         \x20 | ^^^\n\
         \x20 |\n\
         \x20 = note: a built-in function with a type for any list cannot be written as a type \
         yet; call it, or bind it to a name\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_keeps_imports_across_lines() {
    let output = repl(
        "import builtins\n\
         builtins.float(1)\n\
         import len as count from prelude\n\
         count([1, 2])\n\
         import * from builtins\n\
         int(2.5)\n\
         let count = 5\n\
         count\n\
         import zzz; import half as h from prelude\n\
         h(1)\n",
    );
    assert_eq!(stdout(&output), "1.0 : Float\n2 : Int\n2 : Int\n5 : Int\n");
    assert!(
        stderr(&output).starts_with(
            "error: `zzz` is not a built-in or standard module, and there is no \
             `workspace.gcsl` above this file\n"
        ),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("error: undefined variable `h`\n"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(0));
}

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

/// A failing `sh` is the prelude's panic, located where it happens, in
/// `<stdlib>/prelude.gcsl`, with no trace to the call of `sh`; its standard
/// error is trimmed, so its trailing newline leaves no empty line.
#[test]
fn a_failing_sh_is_rendered_in_the_prelude() {
    let output = gcsl(&["run", "tests/fixtures/sh_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `echo nope >&2; exit 2` exited with status 2: nope\n\
         \x20 --> <stdlib>/prelude.gcsl:69:9\n\
         \x20  |\n\
         69 |         panic(\"`{script}` exited with status {result.status}: {trim(result.stderr)}\")\n\
         \x20  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
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
        ir.contains("  (store parts#50 @0..27\n    (call @8..27\n      (load split#37 @8..13)\n"),
        "{ir}"
    );
    assert!(ir.contains("(load join#38 @32..36)"), "{ir}");
    assert!(ir.contains("(load join#66 @153..162)"), "{ir}");
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

/// A failing `sh_live` is the prelude's panic, located in it as `sh`'s is,
/// its message without the standard error, which was copied as it ran.
#[test]
fn a_failing_sh_live_is_rendered_without_its_standard_error() {
    let output = gcsl(&["run", "tests/fixtures/sh_live_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "nope\n\
         error: `echo nope >&2; exit 2` exited with status 2\n\
         \x20 --> <stdlib>/prelude.gcsl:81:9\n\
         \x20  |\n\
         81 |         panic(\"`{script}` exited with status {result.status}\")\n\
         \x20  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
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
         \x20 --> <stdlib>/prelude.gcsl:81:9\n\
         \x20  |\n\
         81 |         panic(\"`{script}` exited with status {result.status}\")\n\
         \x20  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The summary of slice 31: a default marked `#[shadowable]`, shadowed by a
/// file importing it, which the file it is declared in still has.
#[test]
fn run_shadows_a_shadowable_default() {
    let output = gcsl(&["run", "tests/fixtures/ws_shadowable/pkg/build.gcsl"]);
    assert_eq!(stdout(&output), "{arch = \"aarch64\"; j = 4}\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/ws_shadowable/_lib/defaults.gcsl"]);
    assert_eq!(stdout(&output), "{arch = \"x86_64\"; jobs = 4}\n");
    assert_eq!(output.status.code(), Some(0));
}

/// An attribute error, after a shebang and a comment that looks like an
/// attribute but is not one, rendered at the attribute.
#[test]
fn run_renders_an_attribute_error() {
    let output = gcsl(&["run", "tests/fixtures/attribute_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `#[shadowable]` applies only to a file's top-level fields, `def`s and imports\n\
         \x20--> tests/fixtures/attribute_error.gcsl:3:1\n\
         \x20 |\n\
         3 | #[shadowable] let jobs = 4\n\
         \x20 | ^^^^^^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: a `let` or `fn` already shadows\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// `#![no_prelude]` and `#[shadowable]` run, and the dumps show them.
#[test]
fn run_dumps_attributes() {
    let output = gcsl(&["run", "tests/fixtures/attributes.gcsl"]);
    assert_eq!(stdout(&output), "{jobs = 8; n = 1}\n");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/attributes.gcsl", "--dump-ast"]);
    assert_eq!(
        stdout(&output),
        "(file #![no_prelude]\n\
         \x20 (import len from builtins @15..39)\n\
         \x20 #[shadowable] (field jobs @54..62\n\
         \x20   (int 4 @61..62))\n\
         \x20 (field jobs @63..71\n\
         \x20   (int 8 @70..71))\n\
         \x20 (field n @72..87\n\
         \x20   (call @76..87\n\
         \x20     (name len @76..79)\n\
         \x20     (list @80..86\n\
         \x20       (name jobs @81..85)))))\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/attributes.gcsl", "--dump-tokens"]);
    assert!(
        stdout(&output).starts_with("InnerAttribute(no_prelude) @0..14\nNewline @14..15\n"),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("\nAttribute(shadowable) @40..53\n"),
        "{}",
        stdout(&output)
    );
}

/// The REPL: an input cannot turn the prelude off, and `#[shadowable]`
/// changes nothing between inputs.
#[test]
fn repl_has_attributes_of_its_own() {
    let output = repl("#![no_prelude]\n#[shadowable] a = 1\na = 2\na\n");
    assert_eq!(
        stdout(&output),
        "{a = 1} : {a: Int}\n{a = 2} : {a: Int}\n2 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "error: `#![no_prelude]` applies only to a file\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | #![no_prelude]\n\
         \x20 | ^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The types summary: named record types, one built from another by
/// spread, read qualified, imported by member, and seen through the
/// directory chain, with an annotated field.
#[test]
fn run_uses_declared_types() {
    let output = gcsl(&["run", "tests/fixtures/ws_types/pkg/app.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{p = {version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\"}; steps = []; \
         n = \"hello\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/ws_types/workspace.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{make = fn(name: String, f: fn(): {arch: String, content: String, version: String}): \
         {pkg: {arch: String, content: String, name: String, version: String}}; \
         built = {pkg = {arch = \"x\"; content = \"usr\"; version = \"1\"; name = \"hello\"}}}\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// A cycle of types across two files, rendered at the first in path order,
/// labelled at each step in its own file.
#[test]
fn run_renders_a_cycle_of_types() {
    let output = gcsl(&["run", "tests/fixtures/ws_types_error/sub/node.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: type `Node` refers to itself\n\
         \x20--> tests/fixtures/ws_types_error/node.gcsl:1:6\n\
         \x20 |\n\
         1 | type Node = { ...Tree, depth: Int }\n\
         \x20 |      ^^^^        ---- `Node` uses `Tree` here\n\
         \x20 |\n\
         \x20::: tests/fixtures/ws_types_error/workspace.gcsl:1:40\n\
         \x20 |\n\
         1 | type Tree = { name: String, kids: List<Node> }\n\
         \x20 |                                        ---- `Tree` uses `Node` here\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// `--dump-ast` shows `type` items as written; `--dump-types` shows each
/// with its expansion, and every use of a type as written.
#[test]
fn dumps_show_types() {
    let types = "tests/fixtures/ws_types/_lib/types.gcsl";
    let output = gcsl(&["run", types, "--dump-ast"]);
    assert_eq!(
        stdout(&output),
        "(file\n\
         \x20 (type PackageDef {version: String, arch: String, content: String} @0..81)\n\
         \x20 (type Package {...PackageDef, name: String} @82..128))\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", types, "--dump-types"]);
    assert_eq!(
        stdout(&output),
        "(file\n\
         \x20 (type PackageDef {version: String, arch: String, content: String} = \
         {arch: String, content: String, version: String} @0..81)\n\
         \x20 (type Package {...PackageDef, name: String} = \
         {arch: String, content: String, name: String, version: String} @82..128))\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let app = "tests/fixtures/ws_types/pkg/app.gcsl";
    let output = gcsl(&["run", app, "--dump-types"]);
    assert!(
        stdout(&output).starts_with(
            "(file\n\
             \x20 (import Package from _lib.types @0..30)\n\
             \x20 (field p : Package @31..102\n\
             \x20   (block : {arch: String, content: String, name: String, version: String} \
             @44..102\n"
        ),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains(
            "\n  (field steps : Steps @103..120\n    (list : List<fn() -> String | \
             Future<String>> @118..120))\n"
        ),
        "{}",
        stdout(&output)
    );
}

/// The dotted `def`s summary: packages named by dotted paths, recursion by
/// path, a prefix read whole, a directory's files and imports.
#[test]
fn run_uses_dotted_defs() {
    let output = gcsl(&["run", "tests/fixtures/ws_dotted/pkgs/b.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{dev = {peios = {version = \"1\"}}; greeting = \"hello world\"; \
         all = {hello = fn(): String; count = fn(n: Int): Int; name = fn(): String; \
         version = \"1\"}; counted = 3}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/ws_dotted/pkgs/a.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{dev = {peios = {hello = fn(): String; count = fn(n: Int): Int; \
         name = fn(): String}}}\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/ws_dotted/app/main.gcsl"]);
    assert_eq!(stdout(&output), "{hello = \"hello world\"; count = 4}\n");
    assert_eq!(output.status.code(), Some(0));
}

/// `--dump-ast` shows a dotted `def` by its path, and `--dump-ir` its code.
#[test]
fn dumps_show_dotted_defs() {
    let file = "tests/fixtures/ws_dotted/pkgs/a.gcsl";
    let output = gcsl(&["run", file, "--dump-ast"]);
    assert!(
        stdout(&output).starts_with(
            "(file\n\
             \x20 (def dev.peios.hello @0..60\n\
             \x20   (fn @0..60\n\
             \x20     (params)\n\
             \x20     (ret pure String)\n"
        ),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("\n  (def dev.peios.count @61..153\n"),
        "{}",
        stdout(&output)
    );
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", file, "--dump-ir"]);
    let heads: Vec<&str> = stdout(&output)
        .lines()
        .filter(|line| line.starts_with("fn #"))
        .collect();
    assert_eq!(
        heads,
        [
            "fn #0 dev.peios.hello (frame 0, params 0, captures 0, height 4)",
            "fn #1 dev.peios.count (frame 1, params 1, captures 0, height 8)",
            "fn #2 dev.peios.name (frame 0, params 0, captures 0, height 2)",
        ]
    );
    assert_eq!(output.status.code(), Some(0));
}

/// A dotted `def` that cannot be used yet, rendered with its path.
#[test]
fn run_renders_a_dotted_def_not_ready() {
    let output = gcsl(&["run", "tests/fixtures/dotted_def_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `dev.peios.get` cannot be used here yet\n\
         \x20--> tests/fixtures/dotted_def_error.gcsl:1:5\n\
         \x20 |\n\
         1 | x = dev.peios.get()\n\
         \x20 |     ^^^^^^^^^^^^^\n\
         2 | let k = 1\n\
         \x20 | --------- `dev.peios.get` reads `k`, which is bound here\n\
         \x20 |\n\
         \x20 = note: a function can be used once every variable it reads is bound\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The REPL: a line may declare a type, which later lines see, and a later
/// line's declaration of its name shadows it.
#[test]
fn repl_keeps_types_across_lines() {
    let output = repl(
        "type Point = {x: Int, y: Int}\n\
         let p: Point = { x = 1; y = 2 }\n\
         p\n\
         type Point = {x: Int}\n\
         p2: Point = { x = 3 }\n\
         type Box<T> = {v: T}\n",
    );
    assert_eq!(
        stdout(&output),
        "{x = 1; y = 2} : {x: Int, y: Int}\n{p2 = {x = 3}} : {p2: {x: Int}}\n"
    );
    assert_eq!(
        stderr(&output),
        "error: a `type` takes no type parameters\n\
         \x20--> <repl>:1:9\n\
         \x20 |\n\
         1 | type Box<T> = {v: T}\n\
         \x20 |         ^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The REPL: a line may declare a dotted `def`, which later lines call by
/// its path; a quoted segment of its path is an error.
#[test]
fn repl_keeps_dotted_defs_across_lines() {
    let output = repl(
        "def dev.peios.count(n: Int): Int { if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) } }\n\
         dev.peios.count(3)\n\
         def dev.\"x y\"(): Int { 1 }\n",
    );
    assert_eq!(
        stdout(&output),
        "{dev = {peios = {count = fn(n: Int): Int}}} : {dev: {peios: {count: fn(n: Int): Int}}}\n\
         3 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "error: a `def`'s path is made of identifiers\n\
         \x20--> <repl>:1:9\n\
         \x20 |\n\
         1 | def dev.\"x y\"(): Int { 1 }\n\
         \x20 |         ^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The def-types summary: Jack's packages, defined with the def-type
/// `package` and invoked by its path, the stub of `ppk` given the package's
/// directory, run from another working directory than the workspace's.
#[test]
fn run_defines_and_invokes_with_a_def_type() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_deftypes/dev.peios.testpkg/test.gcsl",
    ]);
    assert_eq!(
        stdout(&output).replace(&crate_dir(), "<dir>"),
        "{result = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
         name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
         <dir>/tests/fixtures/ws_deftypes/dev.peios.testpkg/usr -> \
         <dir>/tests/fixtures/ws_deftypes/_dist\"}}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// A reference to a definition the def-type does not have, rendered with
/// the definitions it has.
#[test]
fn run_renders_a_missing_definition() {
    let output = gcsl(&["run", "tests/fixtures/def_type_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `build` has no definition `lib.cor`\n\
         \x20--> tests/fixtures/def_type_error.gcsl:4:11\n\
         \x20 |\n\
         4 | x = build lib.cor()\n\
         \x20 |           ^^^^^^^\n\
         \x20 |\n\
         \x20 = note: the only definition of `build` here is `lib.core`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// `--dump-ast` shows a definition and a reference as they are written,
/// and `--dump-ir` the definition's code by its path and the reference as
/// the closure it makes.
#[test]
fn dumps_show_def_types() {
    let file = "tests/fixtures/ws_deftypes/dev.peios.testpkg/test.gcsl";
    let output = gcsl(&["run", file, "--dump-ast"]);
    assert_eq!(
        stdout(&output),
        "(file\n\
         \x20 (def-type-def package dev.peios.testpkg @0..97\n\
         \x20   (fn @12..97\n\
         \x20     (params)\n\
         \x20     (block @32..97\n\
         \x20       (field version @38..55\n\
         \x20         (str \"0.1.0\" @48..55))\n\
         \x20       (field arch @60..75\n\
         \x20         (str \"x86_64\" @67..75))\n\
         \x20       (field content @80..95\n\
         \x20         (str \"usr\" @90..95)))))\n\
         \x20 (field result @100..136\n\
         \x20   (call @109..136\n\
         \x20     (def-type-ref package dev.peios.testpkg @109..134))))\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", file, "--dump-ir"]);
    let heads: Vec<&str> = stdout(&output)
        .lines()
        .filter(|line| line.starts_with("fn #"))
        .collect();
    assert_eq!(
        heads,
        [
            "fn #0 package dev.peios.testpkg (frame 3, params 0, captures 0, height 3)",
            "fn #1 (frame 0, params 0, captures 2, height 4)",
        ]
    );
    assert!(
        stdout(&output).contains("(call @109..136\n      (make-closure #1 @109..134\n"),
        "{}",
        stdout(&output)
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The REPL: a line may declare a def-type, define with it and invoke it,
/// and later lines see its definitions; two names in a row that name no
/// def-type are reported with a note.
#[test]
fn repl_keeps_def_types_across_lines() {
    let output = repl(
        "#[def_type] def p(n: String, f: fn(): Int) -> Int { f() }\n\
         def p y() { 2 }\n\
         p y()\n\
         let g = p y\n\
         g()\n\
         x = a b\n",
    );
    assert_eq!(
        stdout(&output),
        "{p = fn(n: String | Future<String>, f: fn(): Int) -> Int} : \
         {p: fn(n: String | Future<String>, f: fn(): Int) -> Int}\n\
         2 : Int\n\
         2 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "error: undefined variable `a`\n\
         \x20--> <repl>:1:5\n\
         \x20 |\n\
         1 | x = a b\n\
         \x20 |     ^\n\
         \x20 |\n\
         \x20 = note: two names in a row are read as a def-type reference, as in \
         `package dev.x()`; is an operator or a line break missing?\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The canonical path of this crate's directory, which the `fileroot`
/// tests' paths start with, whatever directory they run from.
fn crate_dir() -> String {
    std::fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// A workspace whose library works on its callers' paths, run from another
/// working directory than the workspace's, and from inside it: the paths are
/// the same, absolute, the caller's directory and the workspace's root.
#[test]
fn fileroot_paths_do_not_depend_on_the_working_directory() {
    let expected = "{a = \"<dir>/tests/fixtures/ws_fileroot/pkg/usr -> \
                    <dir>/tests/fixtures/ws_fileroot/dist\"; \
                    b = \"<dir>/tests/fixtures/ws_fileroot/pkg/usr -> \
                    <dir>/tests/fixtures/ws_fileroot/dist\"; \
                    here = \"<dir>/tests/fixtures/ws_fileroot/pkg\"; \
                    root = \"<dir>/tests/fixtures/ws_fileroot\"}\n";
    let dir = crate_dir();
    for (cwd, file) in [
        ("tests", "fixtures/ws_fileroot/pkg/test.gcsl"),
        ("tests/fixtures/ws_fileroot/pkg", "test.gcsl"),
        ("tests/fixtures/ws_fileroot/_lib", "../pkg/test.gcsl"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_gcsl"))
            .args(["run", file])
            .current_dir(format!("{}/{cwd}", env!("CARGO_MANIFEST_DIR")))
            .output()
            .unwrap();
        assert_eq!(stderr(&output), "", "{cwd}");
        assert_eq!(stdout(&output).replace(&dir, "<dir>"), expected, "{cwd}");
        assert_eq!(output.status.code(), Some(0));
    }
    // A lone file is the workspace of its own directory.
    let output = Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .args(["run", "fixtures/lone_fileroot.gcsl"])
        .current_dir(format!("{}/tests", env!("CARGO_MANIFEST_DIR")))
        .output()
        .unwrap();
    assert_eq!(
        stdout(&output).replace(&dir, "<dir>"),
        "{here = \"<dir>/tests/fixtures\"; root = \"<dir>/tests/fixtures\"}\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The REPL's inputs are in its working directory.
#[test]
fn repl_inputs_are_in_the_working_directory() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .current_dir(format!("{}/tests", env!("CARGO_MANIFEST_DIR")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"import fileroot\nfileroot.workspace\nfileroot.caller\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        stdout(&output).replace(&crate_dir(), "<dir>"),
        "\"<dir>/tests\" : String\n\"<dir>/tests\" : String\n"
    );
    assert_eq!(stderr(&output), "");
}

/// A use of `fileroot` but a read of its members, rendered.
#[test]
fn fileroot_errors_are_rendered() {
    let output = gcsl(&["run", "tests/fixtures/fileroot_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `fileroot`'s members are read as `fileroot.this`\n\
         \x20--> tests/fixtures/fileroot_error.gcsl:2:8\n\
         \x20 |\n\
         2 | import this from fileroot\n\
         \x20 |        ^^^^\n\
         \x20 |\n\
         \x20 = note: import the module whole, `import fileroot`, and read each member through it\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// Jack's newpkgs workspace, every file as he wrote it, its `ppk` wrapper
/// the real one: each file checks, nothing running (the dumps stop before
/// the run), the definition's type carrying its def-type's meta, which
/// prints as written in the def-type's parameters.
#[test]
fn jacks_newpkgs_check_as_written() {
    for file in [
        "workspace.gcsl",
        "_lib/ppk.gcsl",
        "dev.peios.testpkg/dev.peios.testpkg.gcsl",
        "dev.peios.testpkg/build.gcsl",
    ] {
        let output = gcsl(&[
            "run",
            &format!("tests/fixtures/ws_newpkgs/{file}"),
            "--dump-types",
        ]);
        assert_eq!(stderr(&output), "", "{file}");
        assert_eq!(output.status.code(), Some(0), "{file}");
    }
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_newpkgs/workspace.gcsl",
        "--dump-types",
    ]);
    assert!(
        stdout(&output).contains(
            "(params (name String) (func fn(v: Version): PackageDef META-> PackageMeta) \
             (version String))"
        ),
        "{}",
        stdout(&output)
    );
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_newpkgs/dev.peios.testpkg/dev.peios.testpkg.gcsl",
        "--dump-types",
    ]);
    // The meta's function returns the meta's type, either-shaped under
    // `META->`; the definition's type is the def-type's parameter's.
    let either = "{versions: List<{version: String | Future<String>} | Future<{version: String \
                  | Future<String>}>> | Future<List<{version: String | Future<String>} | \
                  Future<{version: String | Future<String>}>>>}";
    let meta = format!("{either} | Future<{either}>");
    assert!(
        stdout(&output).contains(&format!(
            "(defmeta package dev.peios.testpkg @0..122\n    (fn : fn() -> {meta} @16..122\n"
        )),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains(&format!(
            "(def-type-def package dev.peios.testpkg @124..224\n    (fn : fn(v: {{version: \
             String}}): {{arch: String, content: String, version: String}} META-> {meta} \
             @136..224\n"
        )),
        "{}",
        stdout(&output)
    );
}

/// Jack's newpkgs with `ppk.commit` stubbed, invoked from source: the
/// def-type reads its definition's meta for the version asked for, and
/// panics, as Jack's code says, for one the meta does not list.
#[test]
fn run_reads_a_definition_s_meta() {
    let output = gcsl(&["run", "tests/fixtures/ws_metas/dev.peios.testpkg/run.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{result = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
         name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_metas/dev.peios.testpkg/missing.gcsl",
    ]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: dev.peios.testpkg has no version 0.3.0\n\
         \x20 --> tests/fixtures/ws_metas/workspace.gcsl:86:9\n\
         \x20  |\n\
         86 |         panic(\"{name} has no version {version}\")\n\
         \x20  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// `--dump-ast` shows a definition's meta as written, its body the
/// function's; `--dump-ir` the meta's code by its def-type and path, the
/// definition's naming the cell of its meta, and a read of the meta as the
/// call of the `meta` of the function.
#[test]
fn dumps_show_metas() {
    let file = "tests/fixtures/ws_metas/dev.peios.testpkg/dev.peios.testpkg.gcsl";
    let output = gcsl(&["run", file, "--dump-ast"]);
    assert!(
        stdout(&output).starts_with(
            "(file\n\
             \x20 (defmeta package dev.peios.testpkg @0..122\n\
             \x20   (fn @16..122\n\
             \x20     (params)\n\
             \x20     (block @34..122\n\
             \x20       (field versions @40..120\n"
        ),
        "{}",
        stdout(&output)
    );
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", file, "--dump-ir"]);
    let heads: Vec<&str> = stdout(&output)
        .lines()
        .filter(|line| line.starts_with("fn #"))
        .collect();
    assert_eq!(heads.len(), 2, "{}", stdout(&output));
    assert_eq!(
        heads[0],
        "fn #0 defmeta package dev.peios.testpkg (frame 1, params 0, captures 0, height 6)"
    );
    assert!(
        heads[1].starts_with("fn #1 package dev.peios.testpkg meta #")
            && heads[1].ends_with(" (frame 4, params 1, captures 0, height 4)"),
        "{}",
        heads[1]
    );
    let output = gcsl(&["run", "tests/fixtures/ws_metas/workspace.gcsl", "--dump-ir"]);
    assert!(
        stdout(&output).contains(
            "(field versions @3871..3884\n            (await @3871..3884\n              \
             (call @3871..3884\n                (meta @3871..3884\n                  \
             (load func#1 @3871..3875))))))"
        ),
        "{}",
        stdout(&output)
    );
}

/// A definition of a def-type that declares a meta, with none in its
/// directory, rendered with the meta as the def-type writes it.
#[test]
fn run_renders_a_missing_defmeta() {
    let output = gcsl(&["run", "tests/fixtures/meta_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `build x` has no `defmeta` in its directory\n\
         \x20--> tests/fixtures/meta_error.gcsl:4:5\n\
         \x20 |\n\
         4 | def build x() { 1 }\n\
         \x20 |     ^^^^^^^\n\
         \x20 |\n\
         \x20 = note: `build` declares a meta, `META: {tag: String}`, for each of its \
         definitions\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// Jack's newpkgs with `ppk.commit` stubbed, and the examples of the comment
/// of his package's file: the meta read through a reference, in parentheses
/// and without, and a field of the call's result; and the error of reading
/// the meta's field from the call's result (a workspace of its own, since a
/// workspace is checked whole).
#[test]
fn run_reads_metas_through_references() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_metarefs/dev.peios.testpkg/reads.gcsl",
    ]);
    assert_eq!(
        stdout(&output),
        "{parenthesised = [{version = \"0.1.0\"}, {version = \"0.2.0\"}]; dotted = [{version = \
         \"0.1.0\"}, {version = \"0.2.0\"}]; version = \"0.1.0\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&[
        "run",
        "tests/fixtures/ws_metarefs_error/dev.peios.testpkg/error.gcsl",
    ]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: no field `versions` in `{manifest: String, pkg: {arch: String, content: String, \
         name: String, version: String}}`\n\
         \x20--> tests/fixtures/ws_metarefs_error/dev.peios.testpkg/error.gcsl:2:48\n\
         \x20 |\n\
         2 | versions = (package dev.peios.testpkg(\"0.1.0\")).versions\n\
         \x20 |                                                ^^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: the fields are `manifest` and `pkg`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// `--dump-ast` shows a reference by a path past its definition as written,
/// `--dump-types` as the reference to the definition, of a type with its
/// meta, in a field access, and `--dump-ir` a reference's function by the
/// capture whose meta it carries.
#[test]
fn dumps_show_references_past_definitions() {
    let file = "tests/fixtures/ws_metarefs/dev.peios.testpkg/reads.gcsl";
    let output = gcsl(&["run", file, "--dump-ast"]);
    assert!(
        stdout(&output).contains(
            "(field dotted @120..163\n    (def-type-ref package dev.peios.testpkg.versions \
             @129..163))\n"
        ),
        "{}",
        stdout(&output)
    );
    let output = gcsl(&["run", file, "--dump-types"]);
    let either = "{version: String | Future<String>}";
    let versions =
        format!("List<{either} | Future<{either}>> | Future<List<{either} | Future<{either}>>>");
    let reference = format!(
        "fn(version: String | Future<String>) -> {{manifest: String, pkg: {{arch: String, \
         content: String, name: String, version: String}}}} META-> {{versions: {versions}}} | \
         Future<{{versions: {versions}}}>"
    );
    assert!(
        stdout(&output).contains(&format!(
            "(field dotted @120..163\n    (. versions : {versions} @129..163\n      \
             (def-type-ref package dev.peios.testpkg : {reference} @129..154)))\n"
        )),
        "{}",
        stdout(&output)
    );
    let output = gcsl(&["run", file, "--dump-ir"]);
    assert!(
        stdout(&output)
            .contains("fn #0 meta capture 1 (frame 1, params 1, captures 2, height 4)\n"),
        "{}",
        stdout(&output)
    );
}

/// The REPL: a meta read through a reference to an earlier line's
/// definition.
#[test]
fn repl_reads_metas_through_references() {
    let output = repl(
        "#[def_type] def p(n: String, f: fn(): Int META: {v: Int}): Int { f() + f.v }\n\
         defmeta p y { v = 40 }; def p y() { 2 }\n\
         p y.v\n\
         (p y).v + 1\n\
         p y.w\n",
    );
    assert_eq!(
        stdout(&output),
        "{p = fn(n: String | Future<String>, f: fn(): Int META: {v: Int}): Int} : \
         {p: fn(n: String | Future<String>, f: fn(): Int META: {v: Int}): Int}\n\
         40 : Int\n\
         41 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "error: the meta of `p y` has no field `w`\n\
         \x20--> <repl>:1:4\n\
         \x20 |\n\
         1 | p y.w\n\
         \x20 |    ^^\n\
         \x20 |\n\
         \x20 = note: the only field is `v`\n\
         \x20 = note: `p y.w` is read as the field `w` of the meta of `p y`, its longest prefix \
         that is a definition\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// The REPL: a definition and its meta, declared in one line, are read by
/// later lines; a definition or a meta alone in a line is the error, since
/// each line is checked on its own.
#[test]
fn repl_keeps_metas_with_their_definitions() {
    let output = repl(
        "#[def_type] def p(n: String, f: fn(): Int META: {v: Int}): Int { f() + f.v }\n\
         defmeta p y { v = 40 }; def p y() { 2 }\n\
         p y()\n\
         def p z() { 3 }\n\
         defmeta p w { v = 0 }\n",
    );
    assert_eq!(
        stdout(&output),
        "{p = fn(n: String | Future<String>, f: fn(): Int META: {v: Int}): Int} : \
         {p: fn(n: String | Future<String>, f: fn(): Int META: {v: Int}): Int}\n\
         42 : Int\n"
    );
    assert_eq!(
        stderr(&output),
        "error: `p z` has no `defmeta` in its directory\n\
         \x20--> <repl>:1:5\n\
         \x20 |\n\
         1 | def p z() { 3 }\n\
         \x20 |     ^^^\n\
         \x20 |\n\
         \x20 = note: `p` declares a meta, `META: {v: Int}`, for each of its definitions\n\
         error: `defmeta p w` has no definition `w` in its directory\n\
         \x20--> <repl>:1:1\n\
         \x20 |\n\
         1 | defmeta p w { v = 0 }\n\
         \x20 | ^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_evaluates_each_line() {
    let output = repl("1 + 2\n\n   \n7 / 2\n1 +\n-0.0\n");
    assert_eq!(stdout(&output), "3 : Int\n3.5 : Float\n-0.0 : Float\n");
    assert_eq!(
        stderr(&output),
        "error: expected expression, found end of input\n\
         \x20--> <repl>:1:4\n\
         \x20 |\n\
         1 | 1 +\n\
         \x20 |    ^\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// Asserts that `dir`, a temporary directory `fs.temp_dir` made in a run of
/// `gcsl` that has exited, was absolute, the first of its parent, in the
/// system's temporary directory, and is gone with its parent.
fn assert_temp_dir_gone(dir: &str) {
    let path = std::path::Path::new(dir);
    assert!(path.is_absolute(), "{dir}");
    assert_eq!(path.file_name().unwrap(), "1", "{dir}");
    let parent = path.parent().unwrap();
    assert!(
        parent
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("gcsl-"),
        "{dir}"
    );
    assert_eq!(parent.parent().unwrap(), std::env::temp_dir());
    assert!(!path.exists(), "{dir}");
    assert!(!parent.exists(), "{dir}");
}

/// The summary of slice 42, run from other working directories than the
/// file's: its file is found from its own directory, and the temporary
/// directory it made, which it prints, is gone once `gcsl` has exited.
#[test]
fn fs_works_from_any_directory_and_its_temporary_directory_goes_with_gcsl() {
    let file = format!("{}/tests/fixtures/files.gcsl", env!("CARGO_MANIFEST_DIR"));
    for (cwd, file) in [
        (
            format!("{}/tests", env!("CARGO_MANIFEST_DIR")),
            "fixtures/files.gcsl",
        ),
        (
            format!("{}/tests/fixtures/ws", env!("CARGO_MANIFEST_DIR")),
            "../files.gcsl",
        ),
        ("/".to_owned(), file.as_str()),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_gcsl"))
            .args(["run", file])
            .current_dir(&cwd)
            .output()
            .unwrap();
        assert_eq!(stderr(&output), "", "{cwd}");
        let shown = stdout(&output);
        let Some((_, rest)) = shown.split_once("temp = \"") else {
            panic!("{cwd}: {shown}");
        };
        let (temp, _) = rest.split_once('"').unwrap();
        assert_eq!(
            shown,
            format!("{{there = true; text = \"copied by fs\\n\"; temp = \"{temp}\"}}\n"),
            "{cwd}"
        );
        assert_temp_dir_gone(temp);
        assert_eq!(output.status.code(), Some(0));
    }
}

/// A run that fails after making a temporary directory, and putting a
/// directory in it, removes it as well.
#[test]
fn a_temporary_directory_goes_with_a_run_that_failed() {
    let output = gcsl(&["run", "tests/fixtures/files_panic.gcsl"]);
    assert_eq!(stdout(&output), "");
    let shown = stderr(&output);
    let Some((temp, _)) = shown
        .strip_prefix("error: ")
        .and_then(|rest| rest.split_once('\n'))
    else {
        panic!("{shown}");
    };
    assert_eq!(
        shown,
        format!(
            "error: {temp}\n\
             \x20--> tests/fixtures/files_panic.gcsl:4:1\n\
             \x20 |\n\
             4 | panic(dir)\n\
             \x20 | ^^^^^^^^^^\n"
        )
    );
    assert_temp_dir_gone(temp);
    assert_eq!(output.status.code(), Some(1));
}

/// A relative path given to `fs` is an error at the call, naming the member
/// as `fs.read` whatever it was imported as.
#[test]
fn a_relative_path_given_to_fs_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/files_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `fs.read` needs an absolute path, found `fs_input.txt`\n\
         \x20--> tests/fixtures/files_error.gcsl:2:8\n\
         \x20 |\n\
         2 | text = slurp(\"fs_input.txt\")\n\
         \x20 |        ^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The REPL: `import fs`, then a read on a later line; and a temporary
/// directory made on a line, gone once the REPL has ended.
#[test]
fn repl_imports_fs_and_reads_a_file() {
    let input = format!(
        "import fs\nfs.read(\"{}/tests/fixtures/fs_input.txt\")\nfs.temp_dir()\n",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = repl(&input);
    assert_eq!(stderr(&output), "");
    let shown = stdout(&output);
    let (read, temp) = shown.split_once('\n').unwrap();
    assert_eq!(read, "\"copied by fs\\n\" : String");
    let temp = temp
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix("\" : String\n"))
        .unwrap_or_else(|| panic!("{shown}"));
    assert_temp_dir_gone(temp);
    assert_eq!(output.status.code(), Some(0));
}

/// The summary of slice 43, its lock written into a temporary directory
/// and read back: `sort_versions`, `show` and `fs.write`.
#[test]
fn run_sorts_versions_shows_values_and_writes_files() {
    let output = gcsl(&["run", "tests/fixtures/locks.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{a = [\"1.0~rc1\", \"1.0\", \"6.6\", \"6.6.1\", \"6.10\"]; \
         b = \"{{version = \\\"6.6\\\"; commit = \\\"3a8f\\\"}}\"; \
         c = \"\\\"say \\\\\\\"hi\\\\\\\" {{{{x}}}}\\\\n\\\"\"; d = \"[1, 2]\"; \
         written = \"locked = [{{version = \\\"6.6\\\"; commit = \\\"3a8f\\\"}}]\\n\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// A function given to `show` is an error of the check, at the argument,
/// its type in a note.
#[test]
fn a_function_given_to_show_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/show_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot show a function: `show` renders data as source\n\
         \x20--> tests/fixtures/show_error.gcsl:2:13\n\
         \x20 |\n\
         2 | text = show(handlers)\n\
         \x20 |             ^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: the argument is of type `{run: fn(): Int}`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The REPL: `show` on a line, through an alias a later line makes, and
/// `sort_versions`.
#[test]
fn repl_shows_values_and_sorts_versions() {
    let output = repl(
        "show({a = 1})\n\
         let s = show\n\
         s(\"x\")\n\
         sort_versions([\"10\", \"2\"])\n",
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(
        stdout(&output),
        "\"{{a = 1}}\" : String\n\
         \"\\\"x\\\"\" : String\n\
         [\"2\", \"10\"] : List<String>\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

/// Runs `gcsl lsp` with `messages`, each framed as LSP frames a message,
/// on its stdin until it ends.
fn lsp(messages: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        write!(stdin, "Content-Length: {}\r\n\r\n{message}", message.len()).unwrap();
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

/// The messages of `output`, LSP frames on stdout, unframed: nothing else
/// may be there.
fn frames(output: &Output) -> Vec<&str> {
    let mut rest = stdout(output);
    let mut messages = Vec::new();
    while !rest.is_empty() {
        let header = rest.strip_prefix("Content-Length: ").unwrap();
        let (length, body) = header.split_once("\r\n\r\n").unwrap();
        let (message, after) = body.split_at(length.parse().unwrap());
        messages.push(message);
        rest = after;
    }
    messages
}

/// LSP's `initialize`, from a client offering utf-8 and utf-16.
const LSP_INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]}}}}"#;
/// LSP's `initialized`.
const LSP_INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
/// LSP's `shutdown`.
const LSP_SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;
/// LSP's `exit`.
const LSP_EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit"}"#;

/// `gcsl lsp` speaks LSP on stdin and stdout: it answers `initialize` with
/// its capabilities (diagnostics, hover, go to definition, references,
/// highlights, rename, symbols, formatting, completion, signature help and
/// inlay hints)
/// and `shutdown` with null, and exits with code 0 on
/// `exit` after `shutdown`, and 1 on `exit` alone.
#[test]
fn lsp_initializes_and_shuts_down() {
    let output = lsp(&[LSP_INITIALIZE, LSP_INITIALIZED, LSP_SHUTDOWN, LSP_EXIT]);
    let messages = frames(&output);
    let [initialized, shut_down] = messages.as_slice() else {
        panic!("{messages:?}");
    };
    assert_eq!(
        *initialized,
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"capabilities":{{"completionProvider":{{"resolveProvider":false,"triggerCharacters":["."]}},"definitionProvider":true,"documentFormattingProvider":true,"documentHighlightProvider":true,"documentRangeFormattingProvider":true,"documentSymbolProvider":true,"hoverProvider":true,"inlayHintProvider":true,"positionEncoding":"utf-8","referencesProvider":true,"renameProvider":{{"prepareProvider":true}},"signatureHelpProvider":{{"retriggerCharacters":[","],"triggerCharacters":["(",","]}},"textDocumentSync":{{"change":2,"openClose":true,"save":true}},"workspaceSymbolProvider":true}},"serverInfo":{{"name":"gcsl","version":"{}"}}}}}}"#,
            env!("CARGO_PKG_VERSION")
        )
    );
    assert_eq!(*shut_down, r#"{"jsonrpc":"2.0","id":2,"result":null}"#);
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // `exit` without `shutdown`.
    let output = lsp(&[LSP_INITIALIZE, LSP_INITIALIZED, LSP_EXIT]);
    assert_eq!(frames(&output).len(), 1);
    assert_eq!(output.status.code(), Some(1));
}

/// A scratch directory for a `gcsl fmt` test, `name`'s own, empty and
/// canonical, under the system's temporary directory, removed when dropped.
struct FmtScratch(std::path::PathBuf);

impl FmtScratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gcsl-cli-fmt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(std::fs::canonicalize(dir).unwrap())
    }

    /// Writes `text` to `path` below the directory, making the directories
    /// on the way, its modification time set an hour back; returns its
    /// path.
    fn write(&self, path: &str, text: &str) -> std::path::PathBuf {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        let back = std::time::SystemTime::now() - std::time::Duration::from_hours(1);
        file.set_modified(back).unwrap();
        path
    }

    /// The text of the file at `path` below the directory.
    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.0.join(path)).unwrap()
    }

    /// The modification time of the file at `path` below the directory.
    fn modified(&self, path: &str) -> std::time::SystemTime {
        std::fs::metadata(self.0.join(path))
            .unwrap()
            .modified()
            .unwrap()
    }
}

impl Drop for FmtScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `gcsl` with `args` in `dir`, with `input` as its standard input.
fn gcsl_in(dir: &std::path::Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gcsl"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// A file that `gcsl fmt` changes, and what it changes it to.
const MESSY: &str = "x = {\n  a=1;\n\n\n  b = [\n    1\n  ]   # one\n}\n\n";
const FORMATTED: &str = "x = {\n    a = 1\n\n    b = [\n        1,\n    ] # one\n}\n";

/// `gcsl fmt FILE` formats a file in place, and leaves a formatted one
/// alone: not written, its modification time kept.
#[test]
fn fmt_formats_a_file_in_place_and_writes_nothing_formatted() {
    let scratch = FmtScratch::new("in-place");
    scratch.write("messy.gcsl", MESSY);
    scratch.write("tidy.gcsl", FORMATTED);
    let before = scratch.modified("tidy.gcsl");
    let output = gcsl_in(&scratch.0, &["fmt", "messy.gcsl", "tidy.gcsl"], "");
    assert_eq!(stderr(&output), "");
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("messy.gcsl"), FORMATTED);
    assert_eq!(scratch.read("tidy.gcsl"), FORMATTED);
    assert_eq!(scratch.modified("tidy.gcsl"), before);
    // Formatting it again writes nothing either.
    let before = scratch.modified("messy.gcsl");
    let output = gcsl_in(&scratch.0, &["fmt", "messy.gcsl"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.modified("messy.gcsl"), before);
    // No temporary file is left beside them.
    let names: Vec<String> = std::fs::read_dir(&scratch.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

/// `gcsl fmt` of a symbolic link formats the file it leads to, and the link
/// stays a link.
#[test]
fn fmt_writes_through_a_symbolic_link() {
    let scratch = FmtScratch::new("link");
    scratch.write("target.gcsl", MESSY);
    std::os::unix::fs::symlink(scratch.0.join("target.gcsl"), scratch.0.join("link.gcsl")).unwrap();
    let output = gcsl_in(&scratch.0, &["fmt", "link.gcsl"], "");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("target.gcsl"), FORMATTED);
    let link = std::fs::symlink_metadata(scratch.0.join("link.gcsl")).unwrap();
    assert!(link.file_type().is_symlink());
}

/// `gcsl fmt --check` writes nothing, lists the files formatting would
/// change and exits with code 1 if there are any.
#[test]
fn fmt_check_lists_what_would_change_and_writes_nothing() {
    let scratch = FmtScratch::new("check");
    scratch.write("messy.gcsl", MESSY);
    scratch.write("tidy.gcsl", FORMATTED);
    let before = scratch.modified("messy.gcsl");
    let output = gcsl_in(
        &scratch.0,
        &["fmt", "--check", "tidy.gcsl", "messy.gcsl"],
        "",
    );
    assert_eq!(stdout(&output), "messy.gcsl\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(scratch.read("messy.gcsl"), MESSY);
    assert_eq!(scratch.modified("messy.gcsl"), before);
    let output = gcsl_in(&scratch.0, &["fmt", "--check", "tidy.gcsl"], "");
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// `gcsl fmt -` formats standard input to standard output; with `--check`
/// it writes `<stdin>` if that would change, and nothing else.
#[test]
fn fmt_formats_standard_input_to_standard_output() {
    let scratch = FmtScratch::new("stdin");
    let output = gcsl_in(&scratch.0, &["fmt", "-"], MESSY);
    assert_eq!(stdout(&output), FORMATTED);
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl_in(&scratch.0, &["fmt", "--check", "-"], MESSY);
    assert_eq!(stdout(&output), "<stdin>\n");
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl_in(&scratch.0, &["fmt", "--check", "-"], FORMATTED);
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // With a syntax error: nothing on standard output, the error shown.
    let output = gcsl_in(&scratch.0, &["fmt", "-"], "x = (1 2)\n");
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("error: expected `)`, found `2`"));
    assert!(
        stderr(&output).contains("<stdin>:1:8"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1));
    // `-` with other paths is a usage error.
    let output = gcsl_in(&scratch.0, &["fmt", "-", "x.gcsl"], "");
    assert_eq!(output.status.code(), Some(2));
}

/// A file with a syntax error is left alone, its error shown, and the
/// other files are formatted; the exit code is 1. A path that is not there
/// exits with code 2.
#[test]
fn fmt_leaves_a_file_with_an_error_alone() {
    let scratch = FmtScratch::new("error");
    let broken = "x  =  (1 2)\n";
    scratch.write("broken.gcsl", broken);
    scratch.write("messy.gcsl", MESSY);
    let before = scratch.modified("broken.gcsl");
    let output = gcsl_in(&scratch.0, &["fmt", "broken.gcsl", "messy.gcsl"], "");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("error: expected `)`, found `2`"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("broken.gcsl:1:10"),
        "{}",
        stderr(&output)
    );
    assert_eq!(scratch.read("broken.gcsl"), broken);
    assert_eq!(scratch.modified("broken.gcsl"), before);
    assert_eq!(scratch.read("messy.gcsl"), FORMATTED);
    let output = gcsl_in(&scratch.0, &["fmt", "missing.gcsl"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("error: cannot read missing.gcsl"));
}

/// With no paths, `gcsl fmt` formats every file of the workspace the
/// working directory is in, but those of a nested workspace and those under
/// a name starting with `.`; a directory given has every `.gcsl` file under
/// it formatted, those of the workspaces nested in it too. Outside any
/// workspace, no paths is an error.
#[test]
fn fmt_formats_a_workspace_or_a_directory() {
    let scratch = FmtScratch::new("workspace");
    for path in [
        "workspace.gcsl",
        "sub/a.gcsl",
        ".hidden/b.gcsl",
        "nested/workspace.gcsl",
        "nested/c.gcsl",
    ] {
        scratch.write(path, MESSY);
    }
    scratch.write("sub/notes.txt", MESSY);
    // Each named relative to the working directory when under it, in the
    // order of their paths in the workspace.
    let output = gcsl_in(&scratch.0.join("sub"), &["fmt", "--check"], "");
    let root = scratch.0.display();
    assert_eq!(stdout(&output), format!("a.gcsl\n{root}/workspace.gcsl\n"));
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl_in(&scratch.0.join("sub"), &["fmt"], "");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("workspace.gcsl"), FORMATTED);
    assert_eq!(scratch.read("sub/a.gcsl"), FORMATTED);
    for path in [
        ".hidden/b.gcsl",
        "nested/workspace.gcsl",
        "nested/c.gcsl",
        "sub/notes.txt",
    ] {
        assert_eq!(scratch.read(path), MESSY, "{path}");
    }
    // A directory: every `.gcsl` file under it, nested workspaces' too.
    let output = gcsl_in(&scratch.0, &["fmt", "."], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("nested/workspace.gcsl"), FORMATTED);
    assert_eq!(scratch.read("nested/c.gcsl"), FORMATTED);
    assert_eq!(scratch.read(".hidden/b.gcsl"), MESSY);
    assert_eq!(scratch.read("sub/notes.txt"), MESSY);
    // No workspace above.
    let outside = FmtScratch::new("outside");
    let output = gcsl_in(&outside.0, &["fmt"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("no `workspace.gcsl`"),
        "{}",
        stderr(&output)
    );
}

/// A file with a warning and nothing wrong, whose top-level field runs `sh`
/// when the file is run.
const CHECKED: &str = "z = sh(\"touch MARKER\")\np: {x: Int} = {x = 1; y = 2}\n";

/// `gcsl check FILE` checks the file's workspace and runs nothing: no `sh`
/// of a file that would run, so no marker is made. What it shows is what
/// `gcsl run` shows of the workspace's diagnostics, then a summary; the
/// exit code is 0 with warnings alone.
#[test]
fn check_runs_nothing_and_shows_what_run_shows() {
    let scratch = FmtScratch::new("check-runs-nothing");
    scratch.write("workspace.gcsl", "");
    scratch.write("app.gcsl", CHECKED);
    let output = gcsl_in(&scratch.0, &["check", "app.gcsl"], "");
    let made = scratch.0.join("MARKER").exists();
    assert!(!made, "`gcsl check` ran `sh`");
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    // `gcsl run` runs it, and shows the same warning.
    let run = gcsl_in(&scratch.0, &["run", "app.gcsl"], "");
    assert!(scratch.0.join("MARKER").exists());
    assert!(stderr(&run).starts_with("warning: field `y` is not in"));
    assert_eq!(
        stderr(&output),
        format!("{}0 errors, 1 warning in 2 files checked\n", stderr(&run))
    );
}

/// Each form of path `gcsl check` takes: none (the working directory's
/// workspace), a directory (every workspace at or under it, nested ones
/// included, and the one it is in), a file (its workspace, or itself
/// alone); each workspace once. An error exits with code 1; a path not
/// there, or no workspace for no paths, with code 2.
#[test]
fn check_takes_files_directories_and_nothing() {
    let scratch = FmtScratch::new("check-paths");
    scratch.write("workspace.gcsl", "");
    scratch.write("sub/a.gcsl", "a = 1 + \"s\"\n");
    scratch.write("nested/workspace.gcsl", "");
    scratch.write("nested/deeper/workspace.gcsl", "");
    scratch.write("nested/b.gcsl", CHECKED);
    scratch.write(".hidden/workspace.gcsl", "");
    scratch.write(".hidden/h.gcsl", "h = 1 + \"s\"\n");
    let check = |dir: &str, args: &[&str]| {
        let mut all = vec!["check"];
        all.extend_from_slice(args);
        let output = gcsl_in(&scratch.0.join(dir), &all, "");
        let summary = stderr(&output).lines().last().unwrap_or("").to_owned();
        (summary, output)
    };
    // No paths: the workspace of the working directory, alone.
    let (summary, output) = check("sub", &[]);
    assert_eq!(summary, "1 error, 0 warnings in 2 files checked");
    assert!(
        stderr(&output).contains("--> a.gcsl:1:7"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1));
    // A directory: the workspace there and those nested in it, but those
    // under a name starting with `.`.
    let (summary, output) = check("", &["."]);
    assert_eq!(summary, "1 error, 1 warning in 5 files checked");
    assert!(
        stderr(&output).contains("--> sub/a.gcsl:1:7"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("--> nested/b.gcsl:2:"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1));
    // A nested workspace, alone: warnings only, code 0. Given twice, as a
    // directory and by a file of it, it is checked once.
    let (summary, output) = check("", &["nested", "nested/b.gcsl"]);
    assert_eq!(summary, "0 errors, 1 warning in 3 files checked");
    assert_eq!(output.status.code(), Some(0));
    // A directory inside a workspace: that workspace.
    let (summary, _) = check("", &["sub"]);
    assert_eq!(summary, "1 error, 0 warnings in 2 files checked");
    // A file of the hidden directory, which is a workspace of its own.
    let (summary, _) = check("", &[".hidden/h.gcsl"]);
    assert_eq!(summary, "1 error, 0 warnings in 2 files checked");
    // A file in no workspace, alone; a path that is not there.
    let outside = FmtScratch::new("check-outside");
    outside.write("lone.gcsl", "x = 1\n");
    let output = gcsl_in(&outside.0, &["check", "lone.gcsl"], "");
    assert_eq!(stderr(&output), "no problems in 1 file checked\n");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl_in(&outside.0, &["check", "lone.gcsl", "missing.gcsl"], "");
    assert_eq!(
        stderr(&output),
        "error: cannot read missing.gcsl: no such file or directory\n"
    );
    assert_eq!(output.status.code(), Some(2));
    // No paths, and no workspace above.
    let output = gcsl_in(&outside.0, &["check"], "");
    assert!(
        stderr(&output).contains("no `workspace.gcsl`"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(2));
}

/// `gcsl check --message-format json` prints each diagnostic as a JSON
/// object on a line of standard output, columns counted in characters, and
/// the summary to standard error, rendering nothing.
#[test]
fn check_prints_json_lines() {
    let scratch = FmtScratch::new("check-json");
    scratch.write("workspace.gcsl", "");
    scratch.write("app.gcsl", "y = \"é\" + 1\n");
    scratch.write("b.gcsl", CHECKED);
    let output = gcsl_in(&scratch.0, &["check", "--message-format", "json"], "");
    assert_eq!(stderr(&output), "1 error, 1 warning in 3 files checked\n");
    assert_eq!(output.status.code(), Some(1));
    let lines: Vec<serde_json::Value> = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let [error, warning] = lines.as_slice() else {
        panic!("{}", stdout(&output));
    };
    assert_eq!(
        *error,
        serde_json::json!({
            "path": "app.gcsl",
            "severity": "error",
            "message": "cannot add String and Int",
            "line": 1,
            "column": 9,
            "end_line": 1,
            "end_column": 10,
            "labels": [
                {"path": "app.gcsl", "line": 1, "column": 5, "end_line": 1, "end_column": 8, "text": "String"},
                {"path": "app.gcsl", "line": 1, "column": 11, "end_line": 1, "end_column": 12, "text": "Int"},
            ],
            "notes": [],
        })
    );
    assert_eq!(warning["path"], "b.gcsl");
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["line"], 2);
    assert!(
        !warning["notes"].as_array().unwrap().is_empty(),
        "{warning}"
    );
    // Nothing on standard output for a clean workspace.
    scratch.write("app.gcsl", "y = 1\n");
    let output = gcsl_in(
        &scratch.0,
        &["check", "--message-format", "json", "app.gcsl"],
        "",
    );
    assert_eq!(stdout(&output).lines().count(), 1);
    assert_eq!(output.status.code(), Some(0));
}
