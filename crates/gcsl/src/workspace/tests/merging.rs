//! Tests of workspaces: views merged from several sources, their conflicts, and
//! benchmarks of merging directories.

use super::*;

/// The files of the merging summary (slice 27), in path order: `pkgs/`
/// is the root.
pub(super) const MERGE: [(&str, &str); 5] = [
    (
        "app/main.gcsl",
        "import * from lib\ncom.x.app.deps = [com.x.lib, com.y.lib]\n\
             com.x.app.base = com.base.name",
    ),
    ("app/vendor/v.gcsl", "com.vendor.thing = 1\nall = com"),
    ("lib/x.gcsl", "com.x.lib.name = \"lib\""),
    ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
    ("workspace.gcsl", "com.base.name = \"base\""),
];

/// The single error of scanning `files`, shown: its message and file,
/// its label, each other label's file and text, and its notes.
pub(super) fn shown_error(files: &[(&str, &str)]) -> String {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut loaded = Vec::new();
    let placed = (&sources[..], Placing::Workspace(&[]));
    // The scan recovers from its errors (see "Scan errors").
    let errors = match scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded) {
        Ok(scan) => scan.errors(),
        Err(errors) => errors,
    };
    assert_eq!(errors.len(), 1, "{errors:?}");
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    // The standard files the scan loaded take the first input numbers.
    let standard = Library::NEEDED.sources(&loaded);
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    all.extend(paths);
    let path = |input: Option<u32>| {
        input
            .and_then(|i| all.get(i as usize))
            .copied()
            .unwrap_or("?")
    };
    let error = &errors[0];
    let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
    if let Some(label) = &error.label {
        parts[0] = format!("{} ({label})", parts[0]);
    }
    for label in &error.labels {
        let file = path(label.input.or(error.input));
        parts.push(format!("{file}: {}", label.text));
    }
    for note in &error.notes {
        parts.push(format!("note: {note}"));
    }
    parts.join("; ")
}

/// The checked workspace of `files`.
pub(super) fn checked(files: &[(&str, &str)]) -> Workspace {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    check(&sources).unwrap()
}

/// Whether the first item of the top-level block of the file at `path`
/// of `workspace` stores a record: one assembled as the file starts.
pub(super) fn assembled_first(workspace: &Workspace, path: &str) -> bool {
    let index = workspace.file(path).unwrap();
    let FileRun::Pending(ir) = &workspace.runs[index as usize] else {
        panic!("expected pending IR for file {index}");
    };
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("expected a block")
    };
    let first = ir.items(items)[0];
    matches!(ir.get(first).kind, IrKind::Store(_, value)
            if matches!(ir.get(value).kind, IrKind::MakeRecord { .. }))
}

#[test]
fn the_merging_summary_runs_both_files() {
    assert_eq!(
        value(&MERGE, "app/main.gcsl"),
        "{com = {x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; \
             base = \"base\"}}}}"
    );
    // `all` has the root's part, `app/`'s and v's own, root directory
    // first, but not `lib/`'s, which v does not import.
    assert_eq!(
        value(&MERGE, "app/vendor/v.gcsl"),
        "{com = {vendor = {thing = 1}}; all = {base = {name = \"base\"}; \
             x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; base = \"base\"}}; \
             vendor = {thing = 1}}}"
    );
    // v owns a leaf of `com`: its view's `com` is a node of its own
    // schedule, after its own child and before `all`.
    let scan = ws(&MERGE).unwrap();
    let v = &scan.files[2];
    assert_eq!((v.view.own, v.view.end - v.view.start), (1, 1));
    let number = count(v.global.len());
    assert!(v.schedule.order.contains(&number));
    assert_eq!(v.schedule.point[number as usize], 1);
    // main reads no node of its view whole: nothing is assembled.
    let main = &scan.files[1];
    let own = count(main.global.len());
    assert!(main.schedule.order.iter().all(|&n| n < own));
    // Its `com` is the root's, `app/`'s and `lib/`'s, root first.
    let com = &scan.views[main.view.start as usize];
    let sources: Vec<u32> = com.parts.iter().map(|p| p.source).collect();
    assert_eq!(sources, [0, 1, 2]);
    // v's type of `all` is the record of the merged children.
    let mut workspace = checked(&MERGE);
    let dump = workspace.dump_types(2).unwrap();
    assert!(
        dump.contains(
            "(name com : {base: {name: String}, vendor: {thing: Int}, x: {app: {base: \
                 String, deps: List<{name: String}>}}}"
        ),
        "{dump}"
    );
}

/// [`POSITIONS`] through a merged node: `com` is the file's own and
/// `lib/`'s, imported with `*`, and `com.x.lib.r` is `lib/`'s leaf, read
/// from every kind of position; `com` is read whole too (a node of the
/// file's schedule, the file owning `com.x.app.name`), and a function
/// out of that record called. A `def` is a root, never a leaf under
/// `com`: `com.x.lib.f` is a field holding one, and `twice` itself is
/// called through the `*`.
pub(super) const MERGED_POSITIONS: &str = r#"import * from lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy com.x.lib.r
let lw = lazy com
let g = fn(): {a: Int} { com.x.lib.r }
let gw = fn(): Int { let w = com; = w.x.lib.f(com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = com
    = com.x.lib.r.a + w.x.lib.r.a
  }
}
let w = com
com.x.app.name = "app"
p01 = "{com.x.lib.r}"
p02 = xs[com.x.lib.r.a]
p03 = m
p04 = for v in [com.x.lib.r] { v.a }
p05 = for v in [1] if com.x.lib.r.a == 42 { v }
p06 = for let i = com.x.lib.r.a; i < com.x.lib.r.a + 1; i := i + com.x.lib.r.a { i }
p07 = for k < com.x.lib.r.a { k := k + 50; com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < com.x.lib.r.a)
p09 = true and com.x.lib.r.a == 42
p10 = com.x.lib.r is {a: Int}
p11 = (com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue com.x.lib.r }; break com.x.lib.r }
p13 = get(config = com.x.lib.r)
p14 = wrap(com.x.lib.r) { ...com.x.lib.r }
p15 = { ...com.x.lib.r }
p16 = len(0..com.x.lib.r.a)
p17 = [com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).x.lib.r.a
p20 = g()
p21 = { let q = com.x.lib.r; = q }
p22 = w.x.lib.f(w.x.lib.r.a)
p23 = gw()
p24 = twice(21)
p25 = com
p26 = com.x"#;

#[test]
fn a_merged_node_is_read_from_every_position() {
    let files = [
        ("app/main.gcsl", MERGED_POSITIONS),
        (
            "lib/x.gcsl",
            "let base = 40\ncom.x.lib.r = { a = base + 2 }\ncom.x.lib.f = twice\n\
                 def twice(n: Int): Int { n * 2 }",
        ),
        ("workspace.gcsl", ""),
    ];
    let lib = "lib = {r = {a = 42}; f = fn(n: Int): Int}";
    assert_eq!(
        value(&files, "app/main.gcsl"),
        format!(
            "{{outer = fn(): {{inner: fn(): Int}}; com = {{x = {{app = {{name = \"app\"}}}}}}; \
                 p01 = \"{{{{a = 42}}}}\"; p02 = 42; p03 = {{a = 42}}; p04 = [42]; p05 = [1]; \
                 p06 = [42]; p07 = [{{a = 42}}]; p08 = 42; p09 = true; p10 = true; p11 = 42; \
                 p12 = [{{a = 42}}, {{a = 42}}]; p13 = 42; p14 = 84; p15 = {{a = 42}}; \
                 p16 = 42; p17 = [{{a = 42}}]; p18 = 84; p19 = 84; p20 = {{a = 42}}; \
                 p21 = {{a = 42}}; p22 = 84; p23 = 84; p24 = 42; \
                 p25 = {{x = {{app = {{name = \"app\"}}; {lib}}}}}; \
                 p26 = {{app = {{name = \"app\"}}; {lib}}}}}"
        )
    );
}

#[test]
fn star_and_member_imports_merge_with_the_files_own_root() {
    let lib = [
        ("lib/x.gcsl", "com.x.lib.name = \"x\""),
        ("lib/y.gcsl", "com.y.lib.name = \"y\""),
        ("workspace.gcsl", ""),
    ];
    // A directory's tree, a file's own tree, and a member, each with an
    // own `com`: the import side's children after the file's own.
    for (main, all) in [
        (
            "import * from lib\ncom.app = 1\nall = com",
            "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
        ),
        (
            "import * from lib/y\ncom.app = 1\nall = com",
            "{app = 1; y = {lib = {name = \"y\"}}}",
        ),
        (
            "import com from lib\ncom.app = 1\nall = com",
            "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
        ),
    ] {
        let mut files = lib.to_vec();
        files.push(("app/main.gcsl", main));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            format!("{{com = {{app = 1}}; all = {all}}}"),
            "{main}"
        );
    }
    // A member that is a leaf, and one that is a `def`.
    let mut files = lib.to_vec();
    files.push(("lib/z.gcsl", "n = 1\ndef twice(k: Int): Int { k * 2 }"));
    files.push((
        "app/main.gcsl",
        "import n, twice from lib\nm = twice(n + 1)",
    ));
    assert_eq!(value(&files, "app/main.gcsl"), "{m = 4}");
}

#[test]
fn a_subdirectory_merges_under_a_root_above_two_levels_deep() {
    let files = [
        ("workspace.gcsl", "com.a = 1"),
        ("x/b.gcsl", "com.b = 2"),
        ("x/y/c.gcsl", "com.c = 3\nall = com\nb = com.b"),
    ];
    assert_eq!(
        value(&files, "x/y/c.gcsl"),
        "{com = {c = 3}; all = {a = 1; b = 2; c = 3}; b = 2}"
    );
    assert_eq!(value(&files, "x/b.gcsl"), "{com = {b = 2}}");
}

/// A whole read of each kind of node of the view: who assembles it, and
/// when, with its contents and their order.
#[test]
fn a_view_node_read_whole_of_each_kind() {
    // One directory's tree, without an import: slice 25's rule, the
    // last contributor's schedule; the view has no node of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "com.one = 1"),
        ("b.gcsl", "com.two = 2\nall = com"),
    ];
    let scan = ws(&files).unwrap();
    assert!(scan.views.is_empty());
    assert_eq!(
        value(&files, "b.gcsl"),
        "{com = {two = 2}; all = {one = 1; two = 2}}"
    );
    // Across directories, with no leaf of the reading file: assembled
    // as the file starts, the root directory's part first.
    let files = [
        ("workspace.gcsl", "com.a = 1"),
        ("sub/b.gcsl", "com.b = 2"),
        ("sub/c.gcsl", "x = 0\nall = com"),
    ];
    assert_eq!(value(&files, "sub/c.gcsl"), "{x = 0; all = {a = 1; b = 2}}");
    assert!(assembled_first(&checked(&files), "sub/c.gcsl"));
    // Across directories, with a leaf of the reading file: a node of
    // its schedule, after its own leaf, so not first.
    let files = [
        ("workspace.gcsl", "com.a = 1"),
        ("sub/b.gcsl", "com.b = 2\nall = com"),
    ];
    assert_eq!(
        value(&files, "sub/b.gcsl"),
        "{com = {b = 2}; all = {a = 1; b = 2}}"
    );
    assert!(!assembled_first(&checked(&files), "sub/b.gcsl"));
    // Across imports only, with no leaf of the reading file: as the
    // file starts, in the imports' source order.
    let files = [
        (
            "app/main.gcsl",
            "import * from tools\nimport * from lib\nall = com",
        ),
        ("lib/x.gcsl", "com.x = 1"),
        ("tools/t.gcsl", "com.t = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{all = {t = 2; x = 1}}");
    assert!(assembled_first(&checked(&files), "app/main.gcsl"));
    // Mixed: a directory above, the file's own directory, an import and
    // a leaf of the file, two levels of the view merged; `com.x` is
    // read whole too, and a part of it alone.
    let files = [
        (
            "app/main.gcsl",
            "import * from lib\ncom.x.app = 1\nall = com\nx = com.x",
        ),
        ("app/other.gcsl", "com.other = 2"),
        ("lib/x.gcsl", "com.x.lib = 3"),
        ("workspace.gcsl", "com.x.root = 4\ncom.base = 5"),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{com = {x = {app = 1}}; all = {x = {root = 4; app = 1; lib = 3}; base = 5; \
             other = 2}; x = {root = 4; app = 1; lib = 3}}"
    );
    let scan = ws(&files).unwrap();
    let main = &scan.files[1];
    // `com` and `com.x`, both with a leaf of main under them.
    assert_eq!((main.view.own, main.view.end - main.view.start), (2, 2));
}

#[test]
fn the_needs_of_a_view_node_are_per_leaf() {
    let base = [
        ("workspace.gcsl", ""),
        ("bad.gcsl", "def bad(): Int { [1][5] }\ncom.bad.v = bad()"),
        ("ok.gcsl", "com.ok.v = 1"),
    ];
    // A leaf read alone needs its own file only: `bad.gcsl` never runs.
    let mut files = base.to_vec();
    files.push(("app/main.gcsl", "com.app.v = 2\na = com.ok.v"));
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{com = {app = {v = 2}}; a = 1}"
    );
    // Read whole, every owner of every leaf runs, `bad.gcsl` too.
    files.pop();
    files.push(("app/main.gcsl", "com.app.v = 2\na = com"));
    assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
    files.pop();
    files.push(("app/main.gcsl", "a = com"));
    assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
}

#[test]
fn conflicts_across_directories() {
    let above = "one is in a directory above";
    // Two leaves, at the directory above, whichever path is first.
    assert_eq!(
        shown_error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
        format!(
            "field `version` is defined in two files at a.gcsl (defined here); \
                 sub/b.gcsl: defined here; note: {above}"
        )
    );
    assert_eq!(
        shown_error(&[("z.gcsl", "version = 1"), ("a/b.gcsl", "version = 2")]),
        format!(
            "field `version` is defined in two files at z.gcsl (defined here); \
                 a/b.gcsl: defined here; note: {above}"
        )
    );
    // A whole value extended by a path below, and the other way.
    let defined = format!(
        "field `com` is already defined at a.gcsl; sub/b.gcsl: defined here; note: {above}"
    );
    assert_eq!(
        shown_error(&[("a.gcsl", "com = 1"), ("sub/b.gcsl", "com.x = 2")]),
        defined
    );
    assert_eq!(
        shown_error(&[("a.gcsl", "com.x = 1"), ("sub/b.gcsl", "com = 2")]),
        defined
    );
    // A `def` is a leaf: with a field, either way, and with a `def`.
    for (a, b) in [
        ("def f(): Int { 1 }", "f = 2"),
        ("f = 2", "def f(): Int { 1 }"),
        ("def f(): Int { 1 }", "def f(): Int { 2 }"),
    ] {
        assert_eq!(
            shown_error(&[("a.gcsl", a), ("sub/b.gcsl", b)]),
            format!(
                "field `f` is defined in two files at a.gcsl (defined here); \
                     sub/b.gcsl: defined here; note: {above}"
            ),
            "{a} / {b}"
        );
    }
    // Once per pair, however many files' views have it.
    assert_eq!(
        ws(&[
            ("a.gcsl", "version = 1"),
            ("sub/b.gcsl", "version = 2"),
            ("sub/c.gcsl", "x = 1"),
            ("sub/d/e.gcsl", "y = 1"),
        ])
        .unwrap_err()
        .len(),
        1
    );
}

#[test]
fn conflicts_through_imports() {
    let lib = [
        ("lib/x.gcsl", "com.x.lib.name = \"x\""),
        ("workspace.gcsl", ""),
    ];
    let with = |main: &'static str, more: &[(&'static str, &'static str)]| {
        let mut files = lib.to_vec();
        files.push(("app/main.gcsl", main));
        files.extend_from_slice(more);
        shown_error(&files)
    };
    // Two leaves, an own leaf and an imported interior node, and an own
    // interior node and an imported leaf: at the import.
    assert_eq!(
        with("import * from lib\ncom.x.lib.name = \"own\"", &[]),
        "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
    );
    assert_eq!(
        with("import * from lib\ncom.x.lib = 1", &[]),
        "field `com.x.lib` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
    );
    assert_eq!(
        with("import * from lib\ncom.x.lib.name.deep = 1", &[]),
        "field `com.x.lib.name` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
    );
    // A member whose bound name is a plain root of the file, a leaf and
    // an interior node of it.
    let version = [("lib/v.gcsl", "version = 1")];
    assert_eq!(
        with("import version from lib\nversion = 2", &version),
        "field `version` is defined in two files at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
    );
    assert_eq!(
        with("import version from lib\nversion.minor = 2", &version),
        "field `version` is already defined at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
    );
    // Against a directory above the file (not above `lib/`), and
    // between two imports, at the later.
    let mut files = lib.to_vec();
    files.push(("app/sub/main.gcsl", "import * from lib"));
    files.push(("app/up.gcsl", "com.x.lib.name = \"up\""));
    assert_eq!(
        shown_error(&files),
        "field `com.x.lib.name` is defined in two files at app/sub/main.gcsl (imported \
             here); lib/x.gcsl: defined here; app/up.gcsl: defined here; note: one is in a \
             directory above"
    );
    assert_eq!(
        with(
            "import * from lib\nimport * from tools",
            &[("tools/t.gcsl", "com.x.lib.name = \"t\"")]
        ),
        "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             tools/t.gcsl: defined here; app/main.gcsl: imported here; lib/x.gcsl: defined here"
    );
}

#[test]
fn a_view_is_the_files_own() {
    // b does not see what a imported with `*`, nor its own directory's
    // tree `lib/`'s part.
    let mut files = vec![
        ("app/a.gcsl", "import * from lib\nx = com.lib.v"),
        ("lib/l.gcsl", "com.lib.v = 1"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "app/a.gcsl"), "{x = 1}");
    files.push(("app/b.gcsl", "y = com.lib.v"));
    assert_eq!(messages(&files, "app/b.gcsl"), ["undefined variable `com`"]);
    files.pop();
    files.push(("app/b.gcsl", "com.app.v = 2\ny = com.lib.v"));
    assert_eq!(
        messages(&files, "app/b.gcsl"),
        ["no field `lib` in `{app: {v: Int}}`"]
    );
}

#[test]
fn star_from_a_value_file_is_an_error() {
    let files = [
        ("app/main.gcsl", "import * from tools/build"),
        ("tools/build.gcsl", "= 1"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "a value file has no members to import at app/main.gcsl"
    );
}

#[test]
fn a_whole_import_still_clashes_with_a_root_of_the_view() {
    let files = |main: &'static str| {
        vec![
            ("app/main.gcsl", main),
            ("lib/x.gcsl", "com.x = 1"),
            ("tools/t.gcsl", "t = 1"),
            ("workspace.gcsl", ""),
        ]
    };
    for main in [
        "import * from lib\nimport tools as com",
        "import tools as com\nimport * from lib",
        "import tools as com\ncom.y = 1",
    ] {
        assert!(
            shown_error(&files(main)).starts_with("`com` is imported in this file"),
            "{main}"
        );
    }
}

#[test]
fn merging_the_same_cell_twice_is_idempotent() {
    let lib = [("lib/x.gcsl", "com.x.lib.name = 1"), ("workspace.gcsl", "")];
    // `*` from one directory twice.
    let mut files = lib.to_vec();
    files.push((
        "app/main.gcsl",
        "import * from lib\nimport * from lib\na = com.x.lib.name\nall = com",
    ));
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{a = 1; all = {x = {lib = {name = 1}}}}"
    );
    assert!(ws(&files).unwrap().views.is_empty());
    // `*` from the file's own directory (`app` inside `app/`), from a
    // file of it, and from a directory above, with its own leaves.
    for main in [
        "import * from app\ncom.main.v = 1\nall = com",
        "import * from app/other\ncom.main.v = 1\nall = com",
        "import * from app/main\ncom.main.v = 1\nall = com",
        "import com from app\ncom.main.v = 1\nall = com",
    ] {
        let mut files = lib.to_vec();
        files.push(("app/main.gcsl", main));
        files.push(("app/other.gcsl", "com.other.v = 2"));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{com = {main = {v = 1}}; all = {main = {v = 1}; other = {v = 2}}}",
            "{main}"
        );
        assert!(ws(&files).unwrap().views.is_empty(), "{main}");
    }
    let mut files = lib.to_vec();
    files.push((
        "app/sub/main.gcsl",
        "import * from app\ncom.sub = 1\nall = com",
    ));
    files.push(("app/other.gcsl", "com.other = 2"));
    assert_eq!(
        value(&files, "app/sub/main.gcsl"),
        "{com = {sub = 1}; all = {other = 2; sub = 1}}"
    );
}

/// A member of the file's own directory bound by another name is not
/// the same cell at the same path: reaching the file's own nodes through
/// it is still the one-file cycle, as through a whole import.
#[test]
fn a_renamed_member_reaching_the_files_own_nodes_needs_itself() {
    let files = [
        (
            "app/main.gcsl",
            "import com as c from app\ncom.x = 1\ny = c.x",
        ),
        ("workspace.gcsl", ""),
    ];
    assert!(shown_error(&files).starts_with(
        "`app/main.gcsl` needs itself at app/main.gcsl (`com.x` is computed when this \
             file runs)"
    ));
}

#[test]
fn an_importing_files_items_merge_in_any_order() {
    let lines = [
        "import * from lib",
        "com.x.app.a = com.x.lib.name + b",
        "b = 2",
        "all = com.x",
    ];
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
        let files = [
            ("app/main.gcsl", main.as_str()),
            ("lib/x.gcsl", "com.x.lib.name = 1"),
            ("workspace.gcsl", ""),
        ];
        let shown = value(&files, "app/main.gcsl");
        for field in [
            "com = {x = {app = {a = 3}}}",
            "b = 2",
            "all = {app = {a = 3}; lib = {name = 1}}",
        ] {
            assert!(shown.contains(field), "{main}: {shown}");
        }
    }
}

/// The time to check a workspace of `count` sibling directories of the
/// root, each with one file of ten leaves under the root's `com`, each
/// reading one leaf of the root's file through the merged `com`, and to
/// run the last: linear in `count`. Run with `cargo test --release --
/// --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_sibling_directories_merging() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [50_u32, 100, 200, 400, 800] {
        let mut sources = vec![
            SourceFile::new(
                "workspace.gcsl",
                (0..10).fold(String::new(), |mut text, leaf| {
                    writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                    text
                }),
            )
            .unwrap(),
        ];
        for dir in 0..count {
            let mut text = format!("com.d{dir}.l0 = com.base.l3 + 1\n");
            for leaf in 1..10 {
                writeln!(text, "com.d{dir}.l{leaf} = com.d{dir}.l{} + 1", leaf - 1).unwrap();
            }
            sources.push(SourceFile::new(format!("d{dir:04}/f.gcsl"), text).unwrap());
        }
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace
            .file(&format!("d{:04}/f.gcsl", count - 1))
            .unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        assert!(format!("{}", outcome.value).contains("l9 = 13"));
        writeln!(
            report,
            "{count} directories: scan {scanned:?}; check {checked:?}, check and run {total:?}"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

/// [`benchmark_sibling_directories_merging`] with every file in one
/// directory below the root: each file's view merges the root's `com`
/// with the directory's, which has every file's part, so a file's view
/// must not cost the size of its directory's tree. Linear in `count`.
/// Run with `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_one_directory_merging_with_the_root() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [50_u32, 100, 200, 400, 800] {
        let mut sources = vec![
            SourceFile::new(
                "workspace.gcsl",
                (0..10).fold(String::new(), |mut text, leaf| {
                    writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                    text
                }),
            )
            .unwrap(),
        ];
        for file in 0..count {
            let mut text = format!("com.f{file}.l0 = com.base.l3 + 1\n");
            for leaf in 1..10 {
                let before = leaf - 1;
                writeln!(text, "com.f{file}.l{leaf} = com.f{file}.l{before} + 1").unwrap();
            }
            sources.push(SourceFile::new(format!("app/f{file:04}.gcsl"), text).unwrap());
        }
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace
            .file(&format!("app/f{:04}.gcsl", count - 1))
            .unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        assert!(format!("{}", outcome.value).contains("l9 = 13"));
        writeln!(
            report,
            "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

/// The cost of a file's view in a chain of `depth` directories, each
/// below the one before and with one file of ten leaves under `com`,
/// each reading a leaf of the directory above through the merged `com`:
/// a file's view merges every directory above it, so the time per file
/// grows with its depth. Run with `cargo test --release -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_a_deep_chain_of_directories() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for depth in [10_u32, 25, 50, 100] {
        let mut path = String::new();
        let mut sources = Vec::new();
        for level in 0..depth {
            let mut text = if level == 0 {
                String::from("com.d0.l0 = 0\n")
            } else {
                format!("com.d{level}.l0 = com.d{}.l9 + 1\n", level - 1)
            };
            for leaf in 1..10 {
                let before = leaf - 1;
                writeln!(text, "com.d{level}.l{leaf} = com.d{level}.l{before} + 1").unwrap();
            }
            let name = if level == 0 {
                String::from("workspace.gcsl")
            } else {
                write!(path, "d{level}/").unwrap();
                format!("{path}f.gcsl")
            };
            sources.push(SourceFile::new(name, text).unwrap());
        }
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace.file(&format!("{path}f.gcsl")).unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        assert!(format!("{}", outcome.value).contains(&format!("l9 = {}", depth * 10 - 1)));
        let per_file = total / depth;
        writeln!(
            report,
            "{depth} deep: scan {scanned:?}; check {checked:?}, check and run {total:?}, \
                 {per_file:?} a file"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}
