//! Tests of the `gcsl` command: strings, records, functions, futures and
//! memoised functions, run and dumped.

use super::*;

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
         \x20 (store who#137 @0..17\n\
         \x20   (str \"world\" @10..17))\n\
         \x20 (interp @18..36\n\
         \x20   (str \"hello,\\t\" @18..36)\n\
         \x20   (load who#137 @28..31)\n\
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
         \x20 (store host#92 @39..64\n\
         \x20   (str \"localhost\" @53..64))\n\
         \x20 (store port#93 @65..83\n\
         \x20   (int 8080 @79..83))\n\
         \x20 (store name#94 @84..96\n\
         \x20   (str \"api\" @91..96))\n\
         \x20 (store enabled#96 @97..122\n\
         \x20   (bool true @118..122))\n\
         \x20 (store tls#95 @97..122\n\
         \x20   (make-record [enabled] @97..122\n\
         \x20     (load enabled#96 @97..122)))\n\
         \x20 (store server#91 @39..64\n\
         \x20   (make-record [host port tls] @39..64\n\
         \x20     (load host#92 @39..64)\n\
         \x20     (load port#93 @65..83)\n\
         \x20     (load tls#95 @97..122)))\n\
         \x20 (make-record [server name] @39..122\n\
         \x20   (load server#91 @39..45)\n\
         \x20   (load name#94 @84..88)))\n"
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
            "  (store at#142 @254..301\n\
             \x20   (make-closure #2 @263..301\n\
             \x20     (load base#141 @263..301)))\n"
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
