//! Tests of the `gcsl` command: file roots, metas and references.

use super::*;

/// The canonical path of this crate's directory, which the `fileroot`
/// tests' paths start with, whatever directory they run from.
pub(super) fn crate_dir() -> String {
    static DIRECTORY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let Some(temporary) = std::env::var_os("TEST_TMPDIR") else {
                return std::fs::canonicalize(source).unwrap();
            };
            // Bazel's per-file runfiles symlinks otherwise canonicalize outside
            // the working directory and change the diagnostic path spelling.
            // TEST_TMPDIR is disposable and owned by this test invocation.
            let directory = PathBuf::from(temporary).join("cli-fixtures");
            copy_fixtures(
                &source.join("tests/fixtures"),
                &directory.join("tests/fixtures"),
            );
            std::fs::canonicalize(directory).unwrap()
        })
        .to_str()
        .unwrap()
        .to_owned()
}

/// Copy declared fixture files rather than their runfiles transport symlinks.
pub(super) fn copy_fixtures(source: &std::path::Path, destination: &std::path::Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let destination = destination.join(entry.file_name());
        if source.is_dir() {
            copy_fixtures(&source, &destination);
        } else {
            std::fs::copy(source, destination).unwrap();
        }
    }
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
        let output = Command::new(binary())
            .args(["run", file])
            .current_dir(format!("{}/{cwd}", crate_dir()))
            .output()
            .unwrap();
        assert_eq!(stderr(&output), "", "{cwd}");
        assert_eq!(stdout(&output).replace(&dir, "<dir>"), expected, "{cwd}");
        assert_eq!(output.status.code(), Some(0));
    }
    // A lone file is the workspace of its own directory.
    let output = Command::new(binary())
        .args(["run", "fixtures/lone_fileroot.gcsl"])
        .current_dir(format!("{}/tests", crate_dir()))
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
    let mut child = Command::new(binary())
        .current_dir(format!("{}/tests", crate_dir()))
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
        heads[1].starts_with("fn #1 testpkg package dev.peios.testpkg meta #")
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
