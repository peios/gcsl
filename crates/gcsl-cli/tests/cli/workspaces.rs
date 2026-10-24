//! Tests of the `gcsl` command: workspaces, imports, conflicts and the standard
//! library.

use super::*;

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
    assert!(ir.contains("(load lib#62 @49..58)"), "{ir}");
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
        "error: no module `vendor/pkg`\n\
         \x20--> tests/fixtures/ws_no_module/app.gcsl:1:8\n\
         \x20 |\n\
         1 | import vendor/pkg\n\
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
        "(file\n  (import lib @0..10)\n  (import tools/build as builder @11..40)\n  \
         (import com as libs from lib @41..68)\n  (import lib/y @69..81)\n"
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
    assert!(ir.contains("(load build#69 @144..151)"), "{ir}");
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
        ir.contains("  (store n#56 @87..98\n    (list-len @91..98\n      (load xs#55 @95..97)))\n"),
        "{ir}"
    );
    assert!(ir.contains("(load half#60 @103..107)"), "{ir}");
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
         \x20 ::: <stdlib>/prelude.gcsl:21:1\n\
         \x20  |\n\
         21 | len = builtins.len\n\
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
