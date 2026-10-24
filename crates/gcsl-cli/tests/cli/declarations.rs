//! Tests of the `gcsl` command: shadowing, attributes, types, dotted defs and
//! def-types.

use super::*;

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
             \x20 (import Package from _lib/types @0..30)\n\
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

/// The REPL: a line may declare a type, a generic one too, which later
/// lines see, and a later line's declaration of its name shadows it.
#[test]
fn repl_keeps_types_across_lines() {
    let output = repl(
        "type Point = {x: Int, y: Int}\n\
         let p: Point = { x = 1; y = 2 }\n\
         p\n\
         type Point = {x: Int}\n\
         p2: Point = { x = 3 }\n\
         type Box<T> = {v: T}\n\
         b: Box<String> = { v = \"s\" }\n\
         c: Box = { v = 1 }\n",
    );
    assert_eq!(
        stdout(&output),
        "{x = 1; y = 2} : {x: Int, y: Int}\n{p2 = {x = 3}} : {p2: {x: Int}}\n\
         {b = {v = \"s\"}} : {b: {v: String}}\n"
    );
    assert_eq!(
        stderr(&output),
        "error: type `Box` needs its type arguments\n\
         \x20--> <repl>:1:4\n\
         \x20 |\n\
         1 | c: Box = { v = 1 }\n\
         \x20 |    ^^^\n\
         \x20 |\n\
         \x20 = note: `Box` takes 1 type argument\n"
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
            "fn #0 testpkg package dev.peios.testpkg (frame 3, params 0, captures 0, height 3)",
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
