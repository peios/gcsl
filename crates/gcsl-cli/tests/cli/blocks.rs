//! Tests of the `gcsl` command: functions that take a block, and the
//! trailing blocks that fill it.

use super::*;

#[test]
fn run_calls_blocks_in_their_caller_s_scope() {
    let output = gcsl(&["run", "tests/fixtures/blocks.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{done = \"booted;measured;\"; got = \"w\"; n = 42}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["check", "tests/fixtures/blocks.gcsl"]);
    assert_eq!(stderr(&output), "no problems in 1 file checked\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn block_errors_are_rendered_with_exit_code_1() {
    let output = gcsl(&["run", "tests/fixtures/blocks_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `return` cannot leave a block\n\
         \x20--> tests/fixtures/blocks_error.gcsl:3:43\n\
         \x20 |\n\
         3 |   for x in xs { step(\"look\") { if x > 1 { return x } } }\n\
         \x20 |                 ----                      ^^^^^^\n\
         \x20 |                 |\n\
         \x20 |                 inside the block given to this call\n\
         \x20 |\n\
         \x20 = note: the block runs when the function it is given to calls it, which the \
         function goes on from; end the block with a value the function can act on instead\n\
         \n\
         error: a block can only be called\n\
         \x20--> tests/fixtures/blocks_error.gcsl:7:14\n\
         \x20 |\n\
         6 | fn keep(body: block -> Int) -> Int {\n\
         \x20 |         ------------------ `body` is a block\n\
         7 |   let held = body\n\
         \x20 |              ^^^^\n\
         \x20 |\n\
         \x20 = note: a block is no value: call it, `body()`, or pass it on by calling it in \
         another call's block, `f(x) { body() }`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn dumps_show_block_types_block_arguments_and_calls_of_blocks() {
    let path = "tests/fixtures/blocks.gcsl";
    let output = gcsl(&["run", path, "--dump-ast"]);
    let ast = stdout(&output);
    for params in [
        "(params (name String) (body block -> {}))",
        "(params (name String) (body block -> T))",
        "(params (body block: Int))",
    ] {
        assert!(ast.contains(params), "{ast}");
    }
    let output = gcsl(&["run", path, "--dump-types"]);
    let types = stdout(&output);
    assert!(
        types.contains(
            "(fn : fn(name: String | Future<String>, body: block -> {} | Future<{}>) -> {} @"
        ),
        "{types}"
    );
    assert!(
        types.contains("(name body : block -> {} | Future<{}> @"),
        "{types}"
    );
    assert!(
        types.contains("(fn : fn(body: block: Int): Int @"),
        "{types}"
    );
    let output = gcsl(&["run", path, "--dump-ir"]);
    let ir = stdout(&output);
    assert!(ir.contains("    (arg 1 block @324..350\n"), "{ir}");
    assert!(ir.contains("      (arg 0 block @444..450\n"), "{ir}");
    assert_eq!(ir.matches("(call-block @").count(), 4, "{ir}");
}

#[test]
fn repl_blocks_rebind_the_session_s_variables() {
    let output = repl(
        "fn twice(body: block -> {}) -> {} { body(); body(); = {} }\n\
         let mut n = 1\n\
         twice { n := n * 3 }\n\
         n\n\
         fn keep(body: block: Int): Int { let b = body; = 1 }\n\
         twice { n := n + 1 }; n\n\
         twice\n",
    );
    assert_eq!(
        stdout(&output),
        "9 : Int\n\
         11 : Int\n\
         fn(body: block -> {} | Future<{}>) -> {} : fn(body: block -> {} | Future<{}>) -> {}\n"
    );
    assert_eq!(
        stderr(&output),
        "error: a block can only be called\n\
         \x20--> <repl>:1:42\n\
         \x20 |\n\
         1 | fn keep(body: block: Int): Int { let b = body; = 1 }\n\
         \x20 |         ----------------                 ^^^^\n\
         \x20 |         |\n\
         \x20 |         `body` is a block\n\
         \x20 |\n\
         \x20 = note: a block is no value: call it, `body()`, or pass it on by calling it in \
         another call's block, `f(x) { body() }`\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
