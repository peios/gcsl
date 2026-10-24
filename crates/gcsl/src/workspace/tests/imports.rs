//! Tests of workspaces: imports, their targets, needs and clashes.

use super::*;

/// The files of the imports summary, in path order.
pub(super) const IMPORTS: [(&str, &str); 6] = [
    (
        "app/main.gcsl",
        "import lib\nimport tools/build as builder\nimport com as libs from lib\n\
             import lib/y\ncom.x.app.deps = [lib.com.x.lib, libs.y.lib]\n\
             com.x.app.tool = builder\ncom.x.app.all = y.com.y.lib.name",
    ),
    ("common.gcsl", "version = \"1.2\""),
    ("lib/x.gcsl", "com.x.lib.name = \"lib-{version}\""),
    ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
    ("tools/build.gcsl", "= \"build-{version}\""),
    ("workspace.gcsl", ""),
];

/// The scan of `files` told that `nested` are workspaces of their own.
pub(super) fn ws_nested(files: &[(&str, &str)], nested: &[&str]) -> Result<Scan, Vec<Diagnostic>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let nested: Vec<String> = nested.iter().map(|&n| n.to_owned()).collect();
    scan_with(&sources, &nested)
}

/// The message and note of the single error of scanning `files`.
pub(super) fn noted(files: &[(&str, &str)], nested: &[&str]) -> (String, Option<String>) {
    let errors = ws_nested(files, nested).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    (errors[0].message.clone(), errors[0].notes.first().cloned())
}

#[test]
fn imports_resolve_to_their_targets() {
    let scan = ws(&IMPORTS).unwrap();
    let main = &scan.files[1];
    // Its own imports; the implicit prelude's names follow them.
    let named: Vec<(&str, Target)> = main
        .aliases
        .iter()
        .filter(|a| !a.implicit)
        .map(|a| (scan.names.get(a.name), a.target))
        .collect();
    let lib = scan.dirs.iter().position(|d| d.path == "lib").unwrap();
    let com = scan.by_name[&(count(lib), ROOT, scan.names.lookup("com").unwrap())];
    assert_eq!(
        named,
        [
            ("lib", Target::Dir(count(lib))),
            ("builder", Target::File(5)),
            ("libs", Target::Node(com)),
            ("y", Target::File(4)),
        ]
    );
    // Needs per leaf: x, y and build, each through an import; `common`
    // only through them.
    assert_eq!(scan.needs[1], [3, 4, 5]);
    assert_eq!(scan.needed(1), [0, 2, 3, 4, 5, 1]);
    assert_eq!(scan.value_slot(5), Some(Slot::at(count(scan.nodes.len()))));
    assert_eq!(scan.value_slot(2), None);
}

#[test]
fn a_reference_through_an_import_needs_only_its_leaf() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib\nimport lib/y\nimport tools\na = lib.com.x.lib.name\nb = y";
    let scan = ws(&files).unwrap();
    // `lib.com.x.lib.name` is x's leaf; `y` whole is y; nothing reads
    // `tools`, so it needs nothing.
    assert_eq!(scan.needs[1], [3, 4]);
    files[0].1 = "import lib\na = lib";
    assert_eq!(ws(&files).unwrap().needs[1], [3, 4]);
    files[0].1 = "import lib\na = lib.com.y";
    assert_eq!(ws(&files).unwrap().needs[1], [4]);
}

#[test]
fn a_file_part_and_a_whole_node_are_told_apart() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib/y\na = y.com\nb = y.com.y";
    let scan = ws(&files).unwrap();
    let main = &scan.files[1];
    let scope = FileScope::new(&scan, main);
    let targets: Vec<Target> = main
        .mentions
        .foreign
        .iter()
        .flatten()
        .filter_map(|&(need, _)| match need {
            Need::Field(n) => scope.target(n),
            Need::Function(_) => None,
        })
        .collect();
    // `com` of `lib/` is shared with x: y's part of it. `com.y` is y's
    // alone: the merged node.
    assert!(
        matches!(targets[0], Target::Part { file: 4, .. }),
        "{targets:?}"
    );
    assert!(matches!(targets[1], Target::Node(_)), "{targets:?}");
}

#[test]
fn no_module_says_what_there_is() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import nope";
    let (message, note) = noted(&files, &["vendor"]);
    assert_eq!(message, "no module `nope`");
    assert_eq!(
        note.as_deref(),
        Some("the workspace has `app/`, `lib/`, `tools/`, `common`, `workspace`")
    );
    files[0].1 = "import lib/z";
    let (message, note) = noted(&files, &[]);
    assert_eq!(message, "no module `lib/z`");
    assert_eq!(note.as_deref(), Some("`lib/` has `x`, `y`"));
    files[0].1 = "import lib/x/z";
    assert_eq!(noted(&files, &[]).0, "no module `lib/x/z`");
    files[0].1 = "import vendor/pkg";
    let (_, note) = noted(&files, &["vendor"]);
    assert_eq!(note.as_deref(), Some("`vendor/` is a workspace of its own"));
}

/// A segment of a module path may have dots in it, as reverse-DNS names
/// do: it names a directory or a file whose name has them.
#[test]
fn a_module_path_segment_may_have_dots_in_it() {
    let files = [
        (
            "app.gcsl",
            "import pkgs/net.busybox.busybox as busybox
             import w from pkgs/dev.peios.live-root/root
= busybox.v + w",
        ),
        ("pkgs/net.busybox.busybox/pkg.gcsl", "v = 1"),
        ("pkgs/dev.peios.live-root/root.gcsl", "w = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(run_file(&files, "app.gcsl").unwrap(), "3");
}

/// A path in the old form, its segments separated by `.`, names no module;
/// when it would with each `.` a `/`, the error says so and suggests it.
#[test]
fn a_path_with_dots_for_slashes_is_suggested_the_slashes() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib.x as old";
    let errors = ws_nested(&files, &[]).unwrap_err();
    let [error] = errors.as_slice() else {
        panic!("expected one error: {errors:?}");
    };
    assert_eq!(error.message, "no module `lib.x`");
    assert_eq!(
        error.notes,
        ["module paths separate their segments with `/`: `lib/x` names a module"]
    );
    let suggestion = error.suggestion.as_deref().unwrap();
    assert_eq!(suggestion.replacement, "lib/x");
    assert_eq!(suggestion.span, Span::new(7, 12));
    // A name of dots that names nothing either way is no module, as any.
    files[0].1 = "import lib.q as old";
    let (message, note) = noted(&files, &[]);
    assert_eq!(message, "no module `lib.q`");
    assert!(note.unwrap().starts_with("the workspace has"));
}

/// An import that nothing mentions needs nothing, but is still resolved:
/// a broken one stops the run of every file, even one that does not
/// import anything.
#[test]
fn an_import_nothing_mentions_is_still_resolved() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib\nimport nope\na = 1";
    let errors = run_file(&files, "lib/x.gcsl").unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].0, "no module `nope`", "{errors:?}");
    files[0].1 = "import lib\na = 1";
    assert!(run_file(&files, "app/main.gcsl").is_ok());
}

#[test]
fn a_directory_and_a_file_of_one_name_are_reported_once() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib\nimport lib as again";
    files.push(("lib.gcsl", "z = 1"));
    let (message, _) = noted(&files, &[]);
    assert_eq!(message, "`lib` is both a directory and a file");
    files[0].1 = "a = 1";
    assert!(ws(&files).is_ok());
}

#[test]
fn imported_names_clash() {
    let mut files = IMPORTS.to_vec();
    let imported = "is imported in this file";
    for (text, message) in [
        ("import lib\nimport tools as lib", imported),
        ("import lib\nfn lib(): Int { 1 }", imported),
        ("import lib\nlet lib = 1", imported),
        ("import lib\nlib.a = 1", imported),
        ("import tools as version", "is a field of this directory"),
        ("import x from lib/x", "has no `x`"),
    ] {
        files[0].1 = text;
        assert!(noted(&files, &[]).0.ends_with(message), "{text}");
    }
    // A member or a `*` root merges with the file's own root of its
    // name.
    for text in [
        "import com from lib\ncom.a = 1",
        "import * from lib\ncom.a = 1",
    ] {
        files[0].1 = text;
        assert!(ws(&files).is_ok(), "{text}");
    }
    files[0].1 = "import * from lib\na = com.x.lib.name";
    assert!(ws(&files).is_ok());
}

#[test]
fn reaching_into_your_own_file_through_an_import_needs_itself() {
    let mut files = IMPORTS.to_vec();
    files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nall = lib";
    let (message, _) = noted(&files, &[]);
    assert_eq!(message, "`lib/x.gcsl` needs itself");
    files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nother = lib.com.y.lib.name";
    assert!(ws(&files).is_ok());
}

#[test]
fn files_importing_each_other_are_a_cycle() {
    let (message, _) = noted(
        &[
            ("a.gcsl", "import b\nx = b.y"),
            ("b.gcsl", "import a\ny = a.x"),
            ("workspace.gcsl", ""),
        ],
        &[],
    );
    assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
    // A whole read of a directory one of whose files needs this one is
    // the one-file form, at the read, and no cycle of two files.
    let files = [
        ("app.gcsl", "import lib\nall = lib"),
        ("lib/y.gcsl", "import app\ny = app.all"),
        ("workspace.gcsl", ""),
    ];
    let errors = match ws(&files) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    };
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["`app.gcsl` needs itself"]);
}

#[test]
fn the_imports_summary_runs() {
    assert_eq!(
        value(&IMPORTS, "app/main.gcsl"),
        "{com = {x = {app = {deps = [{name = \"lib-1.2\"}, {name = \"ylib\"}]; \
             tool = \"build-1.2\"; all = \"ylib\"}}}}"
    );
}

/// A workspace of a library directory with a `def`, a leaf and an
/// interior node in one file, a file failing at runtime if it runs, and
/// `main` as given.
pub(super) fn library(main: &str) -> Vec<(&str, &str)> {
    vec![
        ("app/main.gcsl", main),
        ("lib/bad.gcsl", "def bad(): Int { [1][5] }\nboom = bad()"),
        (
            "lib/f.gcsl",
            "def twice(n: Int): Int { n * 2 }\nleafy = 3\ncom.k.v = 1",
        ),
        ("lib/sub/s.gcsl", "import lib\nv = lib.leafy"),
        ("workspace.gcsl", ""),
    ]
}

#[test]
fn every_import_form_binds_its_names() {
    let files = library("import lib/f as lf\nimport lib/f\na = lf.leafy\nb = f.com.k.v");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3; b = 1}");
    let main = "import twice, leafy as l, com from lib\na = twice(l)\nb = com.k\nc = com.k.v";
    let files = library(main);
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{a = 6; b = {v = 1}; c = 1}"
    );
    let files = library("import * from lib/f\na = twice(leafy)\nb = com");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 6; b = {k = {v = 1}}}");
    // Each root of `*` is a member: `lib/bad.gcsl`'s `boom` is not read.
    let files = library("import * from lib\na = twice(leafy)");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 6}");
}

#[test]
fn a_def_is_read_from_its_functions_slot() {
    let files = library("import lib\nimport twice from lib/f\n= lib.twice(1) + twice(2)");
    assert_eq!(value(&files, "app/main.gcsl"), "6");
    let main = "import lib/f\nfn g(): Int { let r = f\nr.twice(5) }\nr = f.com\nn = g()";
    assert_eq!(
        value(&library(main), "app/main.gcsl"),
        "{r = {k = {v = 1}}; n = 10}"
    );
    let main = "import lib/f\nlet r = f\n= r.twice(4) + r.leafy";
    assert_eq!(value(&library(main), "app/main.gcsl"), "11");
}

/// A file mentioning a record-valued leaf through an import from every
/// kind of position, reading the directory whole and calling a `def`
/// out of the record, at the top level and inside functions, a nested
/// `def` and lazies.
pub(super) const POSITIONS: &str = r#"import lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := lib.com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy lib.com.x.lib.r
let lw = lazy lib
let g = fn(): {a: Int} { lib.com.x.lib.r }
let gw = fn(): Int { let w = lib; = w.twice(lib.com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = lib
    = lib.com.x.lib.r.a + w.com.x.lib.r.a
  }
}
let w = lib
p01 = "{lib.com.x.lib.r}"
p02 = xs[lib.com.x.lib.r.a]
p03 = m
p04 = for v in [lib.com.x.lib.r] { v.a }
p05 = for v in [1] if lib.com.x.lib.r.a == 42 { v }
p06 = for let i = lib.com.x.lib.r.a; i < lib.com.x.lib.r.a + 1; i := i + lib.com.x.lib.r.a { i }
p07 = for k < lib.com.x.lib.r.a { k := k + 50; lib.com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < lib.com.x.lib.r.a)
p09 = true and lib.com.x.lib.r.a == 42
p10 = lib.com.x.lib.r is {a: Int}
p11 = (lib.com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue lib.com.x.lib.r }; break lib.com.x.lib.r }
p13 = get(config = lib.com.x.lib.r)
p14 = wrap(lib.com.x.lib.r) { ...lib.com.x.lib.r }
p15 = { ...lib.com.x.lib.r }
p16 = len(0..lib.com.x.lib.r.a)
p17 = [lib.com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).com.x.lib.r.a
p20 = g()
p21 = { let q = lib.com.x.lib.r; = q }
p22 = w.twice(w.com.x.lib.r.a)
p23 = gw()
p24 = w.com"#;

#[test]
fn records_read_whole_have_the_types_of_their_roots() {
    let files = [
        ("app/main.gcsl", "import lib\nimport lib/x\nd = lib\nf = x"),
        (
            "lib/x.gcsl",
            "com.x.lib.n = 1\ndef twice(n: Int): Int { n * 2 }",
        ),
        ("lib/y.gcsl", "top = \"y\""),
        ("workspace.gcsl", ""),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    let index = workspace.file("app/main.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    // A directory's record has every file's roots, a `def`'s its
    // function's type; a file's, its own roots.
    assert!(
        dump.contains(
            "(name lib : {com: {x: {lib: {n: Int}}}, top: String, twice: fn(n: Int): Int}"
        )
    );
    assert!(dump.contains("(name x : {com: {x: {lib: {n: Int}}}, twice: fn(n: Int): Int}"));
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{d = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int; top = \"y\"}; \
             f = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int}}"
    );
}

#[test]
fn a_call_of_an_imported_def_is_of_a_known_callee() {
    // The call-site form of the implicit-await error, as for a `def` of
    // the directory, through a member and through a path.
    let files = [
        (
            "app/main.gcsl",
            "import inc from lib\nimport lib\n\
                 fn p(f: Future<Int>): Int { inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.inc(f) }",
        ),
        ("lib/x.gcsl", "def inc(n: Int): Int { n + 1 }"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        [
            "argument `f` would be awaited inside `inc`, which is pure",
            "argument `f` would be awaited inside `lib.inc`, which is pure",
        ]
    );
}

#[test]
fn an_import_is_read_from_every_position() {
    let files = [
        ("app/main.gcsl", POSITIONS),
        (
            "lib/x.gcsl",
            "let base = 40\ncom.x.lib.r = { a = base + 2 }\ndef twice(n: Int): Int { n * 2 }",
        ),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{outer = fn(): {inner: fn(): Int}; p01 = \"{{a = 42}}\"; p02 = 42; \
             p03 = {a = 42}; p04 = [42]; p05 = [1]; p06 = [42]; p07 = [{a = 42}]; \
             p08 = 42; p09 = true; p10 = true; p11 = 42; p12 = [{a = 42}, {a = 42}]; \
             p13 = 42; p14 = 84; p15 = {a = 42}; p16 = 42; p17 = [{a = 42}]; p18 = 84; \
             p19 = 84; p20 = {a = 42}; p21 = {a = 42}; p22 = 84; p23 = 84; \
             p24 = {x = {lib = {r = {a = 42}}}}}"
    );
}

#[test]
fn needs_through_an_import_are_per_leaf() {
    let files = library("import lib\na = lib.leafy");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
    let files = library("import lib\na = lib.com.k");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = {v = 1}}");
    let files = library("import lib\nfn g(): Int { lib.leafy }\na = g()");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
    let main = "import lib/f\ndef d(): Int { f.com.k.v + (fn(): Int { f.twice(1) })() }\na = d()";
    assert_eq!(
        value(&library(main), "app/main.gcsl"),
        "{d = fn(): Int; a = 3}"
    );
    // Read whole, every file of `lib/` runs, `lib/bad.gcsl` too.
    let messages = messages(
        &library("import lib\nlet r = lib\na = r.leafy"),
        "app/main.gcsl",
    );
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(value(&library(""), "lib/sub/s.gcsl"), "{v = 3}");
}

#[test]
fn a_value_file_is_imported_by_its_value() {
    let files = [
        ("a.gcsl", "import b\nimport c\nx = b + 1\ny = c.z"),
        ("b.gcsl", "= 41"),
        ("c.gcsl", "z = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a.gcsl"), "{x = 42; y = 2}");
    assert_eq!(value(&files, "b.gcsl"), "41");
}

#[test]
fn imported_names_are_the_files_own() {
    let main = "a = leafy + g(10)\nfn g(leafy: Int): Int { leafy + 1 }\nimport leafy from lib/f";
    assert_eq!(value(&library(main), "app/main.gcsl"), "{a = 14}");
    let mut files = library("import leafy from lib/f\na = leafy");
    files.push(("app/other.gcsl", "b = leafy"));
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        ["undefined variable `leafy`"]
    );
    files.pop();
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
}

#[test]
fn an_importing_files_items_are_in_any_order() {
    let lines = [
        "import lib/f",
        "a = f.leafy + b",
        "b = twice(2)",
        "import twice from lib/f",
    ];
    // Every rotation, and (as slice 23's helper in lowering, which runs
    // a file alone, compares) the reverse order.
    let reversed: Vec<&str> = lines.iter().rev().copied().collect();
    let orders = (0..lines.len())
        .map(|start| {
            let mut rotated = lines.to_vec();
            rotated.rotate_left(start);
            rotated
        })
        .chain([reversed]);
    for order in orders {
        let main = order.join("\n");
        let mut record: Vec<String> = value(&library(&main), "app/main.gcsl")
            .trim_matches(['{', '}'])
            .split("; ")
            .map(str::to_owned)
            .collect();
        record.sort();
        assert_eq!(record, ["a = 7", "b = 4"], "{main}");
    }
}
