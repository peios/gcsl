//! Tests of the `gcsl` command: the REPL, defaults, top-level functions and
//! fields, and returns.

use super::*;

/// Runs the REPL with `input` on stdin until it ends.
pub(super) fn repl(input: &str) -> Output {
    let mut child = Command::new(binary())
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
         \x20 (store describe#123 @156..225\n\
         \x20   (make-closure #0 @156..225))\n\
         \x20 (store even#124 @226..290\n",
        // One that copies `rate` just after `rate` is bound.
        "\n  (store rate#129 @494..506\n\
         \x20   (int 3 @505..506))\n\
         \x20 (store scale#127 @525..559\n\
         \x20   (make-closure #4 @525..559\n\
         \x20     (load rate#129 @525..559)))\n\
         \x20 (store scaled#78 @507..524\n",
        // One that copies a mutable variable where it is written.
        "\n  (store counted#128 @643..670\n\
         \x20   (make-closure #5 @643..670\n",
        // A function reads another of the top level from the top level's
        // frame, not by a capture.
        "\n        (fn top even#124 @190..194)\n",
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
        "\n  (store name#81 @273..295\n",
        "\n  (store lib#80 @273..295\n\
         \x20   (make-record [name] @273..295\n\
         \x20     (load name#81 @273..295)))\n\
         \x20 (store deps#78 @211..239\n\
         \x20   (make-list @228..239\n\
         \x20     (load lib#80 @229..238)))\n\
         \x20 (store version#82 @296..321\n",
        // An interior node is assembled from its children's cells once all
        // of them are computed.
        "\n  (store app#77 @211..239\n\
         \x20   (make-record [deps name] @211..239\n\
         \x20     (load deps#78 @211..239)\n\
         \x20     (load name#79 @240..272)))\n",
        // A root read between its items is computed whole, before the read.
        "\n  (store part#83 @526..536\n\
         \x20   (make-record [a b] @526..536\n\
         \x20     (load a#84 @526..536)\n\
         \x20     (load b#86 @550..560)))\n\
         \x20 (store whole#85 @537..549\n",
        // The record of the input is made from the roots' cells.
        "\n  (make-record [com version part whole label] @59..574\n\
         \x20   (load com#75 @211..214)\n",
        // A function reads a cell from the top level's frame, not by a
        // capture.
        "\n    (top top name#81 @451..465))",
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
         \x20     (for hit#123 @532..582\n",
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

#[test]
fn repl_calls_the_prelude_s_methods_unless_a_line_shadows_one() {
    let output = repl(
        "\" a \".trim()\n\
         [1, 1, 2].unique()\n\
         fn (s: String) trim(): String { \"mine\" }\n\
         \" a \".trim()\n",
    );
    assert_eq!(
        stdout(&output),
        "\"a\" : String\n\
         [1, 2] : List<Int>\n\
         \"mine\" : String\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn repl_checks_what_an_earlier_line_s_function_needs() {
    let output = repl(
        "context arch: String\n\
         fn f(): String { ^arch }\n\
         f\n\
         f() with { arch = \"x86\" }\n\
         f()\n",
    );
    assert_eq!(
        stdout(&output),
        "fn(^arch): String : fn(^arch): String\n\
         \"x86\" : String\n"
    );
    assert!(
        stderr(&output).contains("error: context `arch` has no value here"),
        "{}",
        stderr(&output)
    );
}
