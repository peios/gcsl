//! Tests of workspaces: syntax and scan errors, conflicts, and failed files.

use super::*;

/// Every diagnostic of checking the workspace of `files`, as its
/// message, the path of its file and the source it is at.
pub(super) fn every_error(files: &[(&str, &str)]) -> Vec<(String, String, String)> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], "/");
    let mut shown = standard;
    let mut sorted = sources;
    sorted.sort_by(|a, b| a.name().cmp(b.name()));
    shown.extend(sorted);
    checked
        .unwrap_err()
        .into_iter()
        .map(|error| {
            let source = error.input.and_then(|input| shown.get(input as usize));
            (
                error.message,
                source.map_or_else(String::new, |s| s.name().to_owned()),
                source.map_or_else(String::new, |s| s.slice(error.span).to_owned()),
            )
        })
        .collect()
}

/// `triples` as [`every_error`] gives them.
pub(super) fn expected_errors(triples: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    triples
        .iter()
        .map(|&(message, path, at)| (message.to_owned(), path.to_owned(), at.to_owned()))
        .collect()
}

#[test]
fn a_syntax_error_hides_no_other_files_errors() {
    // `a` has a syntax error, and a type error where it parses; `b` an
    // unrelated type error; `c` reads `q`, which `a` declares where it
    // could not be parsed, and `d` reads `x`, whose value could not be.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "q . = 1\nx = (1 2)\ny: Int = \"s\""),
        ("b.gcsl", "z: Int = true"),
        ("c.gcsl", "w = q + 1"),
        ("d.gcsl", "v = x + 1\nu: Int = \"t\""),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("expected a field name after `.`", "a.gcsl", "="),
            ("expected `)`, found `2`", "a.gcsl", "2"),
            ("mismatched types", "a.gcsl", "\"s\""),
            ("mismatched types", "b.gcsl", "true"),
        ])
    );
}

#[test]
fn a_half_typed_item_leaves_the_next_line_to_be_checked() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "u.gcsl",
            "let a = 1 +\nlet b: Int = \"x\"\nlet c = f(1,, 2)\nlet e = 3\nx = \ny: Int = true",
        ),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("expected expression, found newline", "u.gcsl", "\n"),
            ("mismatched types", "u.gcsl", "\"x\""),
            ("expected expression, found `,`", "u.gcsl", ","),
            ("expected expression, found newline", "u.gcsl", "\n"),
            ("mismatched types", "u.gcsl", "true"),
        ])
    );
}

#[test]
fn a_name_that_names_nothing_is_an_error_where_no_broken_file_is_seen() {
    // `sub2/d` sees the root's files, not `sub1/a`.
    let files = [
        ("workspace.gcsl", ""),
        ("sub1/a.gcsl", "q . = 1"),
        ("sub2/d.gcsl", "w = nope + 1\nt: Nope = 1"),
        ("sub3/e.gcsl", "import sub1/a\nw = a.q + nope"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("expected a field name after `.`", "sub1/a.gcsl", "="),
            ("undefined variable `nope`", "sub2/d.gcsl", "nope"),
            ("unknown type `Nope`", "sub2/d.gcsl", "Nope"),
        ])
    );
}

#[test]
fn a_failed_check_leaves_the_recovered_trees_and_the_types_found() {
    let sources: Vec<SourceFile> = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = (1 2)\ny = 3"),
        ("b.gcsl", "z = 4"),
    ]
    .iter()
    .map(|&(path, text)| SourceFile::new(path, text).unwrap())
    .collect();
    let (standard, checked) = check_partial(&sources, &[], "/");
    let Err(Failed {
        diagnostics,
        partial: Some(partial),
    }) = checked
    else {
        panic!("{checked:?}");
    };
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let (a, b) = (standard.len(), standard.len() + 1);
    assert_eq!(partial.scan.files[a].path, "a.gcsl");
    assert_eq!(partial.scan.files[a].ast.syntax(), Syntax::Recovered);
    assert_eq!(partial.scan.files[b].ast.syntax(), Syntax::Clean);
    // `y` is in `a`'s tree; `a` failed, `b` was checked.
    assert_eq!(
        partial.scan.files[a]
            .ast
            .items(partial.scan.files[a].ast.root())
            .len(),
        2
    );
    assert!(partial.checked[a].is_none());
    assert!(partial.checked[b].is_some());
}

#[test]
fn an_unbalanced_file_is_not_checked() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x: Int = \"s\"\n}\ny = 1"),
        ("b.gcsl", "z: Int = true"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("unexpected `}`", "a.gcsl", "}"),
            ("mismatched types", "b.gcsl", "true"),
        ])
    );
}

#[test]
fn a_file_with_a_syntax_error_has_no_warnings() {
    // A list of an `Int` and a `String` is warned about, being of a
    // union; not in a file with a syntax error, whose warnings may be of
    // source that could not be parsed.
    let clean = [("workspace.gcsl", ""), ("a.gcsl", "x = 1\ny = [1, \"a\"]")];
    let sources: Vec<SourceFile> = clean
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    assert_eq!(check(&sources).unwrap().warnings().len(), 1);
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = 1 1\ny = [1, \"a\"]"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[("expected end of item, found `1`", "a.gcsl", "1")])
    );
}

#[test]
fn scan_errors_beside_a_syntax_error_are_reported_only_where_no_broken_file_is_seen() {
    // Two files of `sub2` define `k`; `sub1/a` has a syntax error and
    // two definitions of `j`, which may be the recovery's. The scan
    // recovers from the conflict, so `sub1/a` is checked as its parser
    // recovered it, and its check reports its own `j` twice.
    let files = [
        ("workspace.gcsl", ""),
        ("sub1/a.gcsl", "j = 1\nj = 2\nx = 1 1"),
        ("sub2/b.gcsl", "k = 1"),
        ("sub2/c.gcsl", "k = 2"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("field `j` is defined twice", "sub1/a.gcsl", "j"),
            ("expected end of item, found `1`", "sub1/a.gcsl", "1"),
            ("field `k` is defined in two files", "sub2/c.gcsl", "k"),
        ])
    );
}

/// A file with a syntax error, in a directory of its own that nothing
/// imports, which each case of the scan's errors is checked with too.
pub(super) const BROKEN: (&str, &str) = ("zz/broken.gcsl", "b = (1 2)");

/// Asserts that checking the workspace of `files` reports `expected`,
/// as [`expected_errors`] takes them, and with [`BROKEN`] added, those
/// and its syntax error: a scan error hides no other file's errors.
pub(super) fn scan_case(files: &[(&str, &str)], expected: &[(&str, &str, &str)]) {
    assert_eq!(every_error(files), expected_errors(expected), "{files:?}");
    let mut broken = files.to_vec();
    broken.push(BROKEN);
    let mut wanted = expected.to_vec();
    wanted.push(("expected `)`, found `2`", "zz/broken.gcsl", "2"));
    assert_eq!(every_error(&broken), expected_errors(&wanted), "{broken:?}");
}

#[test]
fn a_field_defined_in_two_files_hides_no_other_files_errors() {
    // clackatoo's minimal case: `r`'s error is reported beside the
    // conflict, and `p` is checked.
    let files = [
        ("workspace.gcsl", "# Root."),
        ("p/p.gcsl", "h = 1\nk = h"),
        ("q/a.gcsl", "x = 1"),
        ("q/b.gcsl", "x = 2"),
        ("r/r.gcsl", "y: Int = \"s\""),
    ];
    scan_case(
        &files,
        &[
            ("field `x` is defined in two files", "q/b.gcsl", "x"),
            ("mismatched types", "r/r.gcsl", "\"s\""),
        ],
    );
    // The scan alone reports the conflict, and recovers from it.
    let errors = ws(&files).unwrap_err();
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["field `x` is defined in two files"]);
    // The file asked for does not run.
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    assert!(check(&sources).is_err());
}

#[test]
fn a_conflicting_field_is_dropped_and_read_as_erroneous_everywhere() {
    // `q/b`'s `x` and `d` are dropped, `x` still checked as written;
    // every read of the conflicted `x` and `d`, in its directory, in a
    // directory below, or through an import, is erroneous, silently.
    let files = [
        ("workspace.gcsl", ""),
        ("q/a.gcsl", "x = 1\nd.e = 1"),
        ("q/b.gcsl", "x: Int = \"s\"\nd = 2\nown: Int = true"),
        ("q/c.gcsl", "u: String = x\nv: Bool = d"),
        ("q/sub/s.gcsl", "t: Bool = x + 1"),
        (
            "s/s.gcsl",
            "import x, d from q\nn: String = x\nm: Bool = d\nl: List<Int> = x\nz: Int = true",
        ),
    ];
    scan_case(
        &files,
        &[
            ("field `x` is defined in two files", "q/b.gcsl", "x"),
            ("mismatched types", "q/b.gcsl", "\"s\""),
            ("field `d` is already defined", "q/b.gcsl", "d"),
            ("mismatched types", "q/b.gcsl", "true"),
            ("mismatched types", "s/s.gcsl", "true"),
        ],
    );
}

#[test]
fn a_call_of_a_conflicting_def_types_nothing_of_its_arguments() {
    // The `def` would have typed `[]`: an erroneous callee leaves it
    // untyped, which is no error of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("q/a.gcsl", "def f(xs: List<Int>): Int { 1 }"),
        ("q/b.gcsl", "def f(xs: List<Int>): Int { 2 }"),
        (
            "s/s.gcsl",
            "import f from q\nr = f([])\nt: List<Int> = f(1)",
        ),
        ("q/c.gcsl", "u = f([], [])"),
    ];
    scan_case(
        &files,
        &[("field `f` is defined in two files", "q/b.gcsl", "f")],
    );
}

#[test]
fn a_three_way_conflict_keeps_the_first() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = 1"),
        ("b.gcsl", "x = \"b\""),
        ("c.gcsl", "x = true\ny: Int = x"),
    ];
    scan_case(
        &files,
        &[
            ("field `x` is defined in two files", "b.gcsl", "x"),
            ("field `x` is defined in two files", "c.gcsl", "x"),
        ],
    );
}

#[test]
fn an_import_of_no_module_binds_erroneous_names() {
    // What the imports would bind reads as `Error`; everything else of
    // the file is checked. A `*` of nothing leaves the names it would
    // bind unknown: a name that names nothing is no error there.
    let files = [
        ("workspace.gcsl", ""),
        (
            "m/m.gcsl",
            "import nope/thing\nimport a, b as c from nada\nv = thing.x + 1\n\
                 w: Bool = c + a\nt: thing.T = 1\nz = zork\ny: Int = \"s\"",
        ),
        (
            "m/n.gcsl",
            "import * from gone\nk = whatever + 1\nj: Int = \"s\"",
        ),
        ("o/o.gcsl", "import f from lib\nr = f + 1"),
        ("lib/l.gcsl", "g = 1"),
    ];
    scan_case(
        &files,
        &[
            ("no module `nope/thing`", "m/m.gcsl", "nope/thing"),
            ("no module `nada`", "m/m.gcsl", "nada"),
            ("undefined variable `zork`", "m/m.gcsl", "zork"),
            ("mismatched types", "m/m.gcsl", "\"s\""),
            ("no module `gone`", "m/n.gcsl", "gone"),
            ("mismatched types", "m/n.gcsl", "\"s\""),
            ("`lib` has no `f`", "o/o.gcsl", "f"),
        ],
    );
}

#[test]
fn files_that_need_each_other_are_not_checked_and_the_rest_is() {
    // `d` needs a file of the cycle, and is not checked either.
    let files = [
        ("workspace.gcsl", ""),
        ("cy/a.gcsl", "x = y + 1\nbad: Int = \"a\""),
        ("cy/b.gcsl", "y = x + 1"),
        ("d/d.gcsl", "import x from cy\nr = x\nq: Int = \"bad\""),
        ("e/e.gcsl", "z: Int = \"s\""),
    ];
    scan_case(
        &files,
        &[
            (
                "`cy/a.gcsl` and `cy/b.gcsl` need each other",
                "cy/a.gcsl",
                "y",
            ),
            ("mismatched types", "e/e.gcsl", "\"s\""),
        ],
    );
}

#[test]
fn a_type_imported_as_a_built_in_types_name_binds_nothing() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/t.gcsl", "type Foo = {a: Int}"),
        ("m.gcsl", "import Foo as Int from lib/t\nk: Int = 1"),
        ("o.gcsl", "z: Int = true"),
    ];
    scan_case(
        &files,
        &[
            ("`Int` is a built-in type", "m.gcsl", "Int"),
            ("mismatched types", "o.gcsl", "true"),
        ],
    );
}

#[test]
fn a_type_declared_in_two_files_is_erroneous_where_used() {
    let files = [
        ("workspace.gcsl", ""),
        ("t/a.gcsl", "type Foo = {a: Int}"),
        ("t/b.gcsl", "type Foo = String\nv: Foo = 1"),
        ("t/c.gcsl", "w: Foo = true"),
        ("o.gcsl", "z: Int = true"),
    ];
    scan_case(
        &files,
        &[
            ("mismatched types", "o.gcsl", "true"),
            ("type `Foo` is declared in two files", "t/b.gcsl", "Foo"),
        ],
    );
}

#[test]
fn a_conflict_of_a_view_is_erroneous_in_the_files_that_see_it() {
    // `a/b`'s `k` meets the `k` of the directory above, reported at
    // the directory above's: each file of `a/b` reads `k` as `Error`,
    // and `a`'s files read their own.
    let files = [
        ("workspace.gcsl", ""),
        ("a/x.gcsl", "k = 1\nm: String = k"),
        ("a/b/y.gcsl", "k = \"s\"\nu: Int = k"),
        ("a/b/z.gcsl", "w: Bool = k.f"),
    ];
    scan_case(
        &files,
        &[
            ("field `k` is defined in two files", "a/x.gcsl", "k"),
            ("mismatched types", "a/x.gcsl", "k"),
        ],
    );
}

#[test]
fn a_meta_declared_twice_is_dropped() {
    let files = [
        TAGGED,
        (
            "app/a.gcsl",
            "def tagged x() { \"x\" }\ndefmeta tagged x { tag = \"t\"; n = 1 }",
        ),
        ("app/b.gcsl", "defmeta tagged x { tag = \"u\"; n = 2 }"),
        ("o.gcsl", "z: Int = true"),
    ];
    let errors: Vec<String> = every_error(&files)
        .into_iter()
        .map(|(message, path, _)| format!("{message} at {path}"))
        .collect();
    assert_eq!(
        errors,
        [
            "`defmeta tagged x` is declared twice in this directory at app/b.gcsl",
            "mismatched types at o.gcsl",
        ]
    );
}

/// Each diagnostic of checking the workspace of `files`, as `MESSAGE at
/// FILE`, with its suggestion as `WRITTEN -> REPLACEMENT`, the text it
/// replaces read from its file at its span, if it has one.
pub(super) fn suggestions(files: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
    let mut sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    sources.sort_by(|a, b| a.name().cmp(b.name()));
    let (standard, checked) = check_with_standard(&sources, &[], "/");
    let Err(errors) = checked else {
        panic!("{files:?}: checked without an error");
    };
    let mut all = standard;
    all.extend(sources);
    errors
        .iter()
        .map(|error| {
            let file = error.input.and_then(|input| all.get(input as usize));
            let name = file.map_or("?", SourceFile::name);
            let shown = error.suggestion.as_ref().map(|found| {
                let written = file.map_or("?", |file| file.slice(found.span));
                format!("{written} -> {}", found.replacement)
            });
            (format!("{} at {name}", error.message), shown)
        })
        .collect()
}

/// The suggestion of the one diagnostic of checking `files`, as
/// [`suggestions`] shows it, its message starting with `message`.
pub(super) fn suggestion_of(files: &[(&str, &str)], message: &str) -> Option<String> {
    let found = suggestions(files);
    assert_eq!(found.len(), 1, "{found:?}");
    let (shown, suggestion) = found.into_iter().next().unwrap();
    assert!(shown.starts_with(message), "{shown}");
    suggestion
}

#[test]
fn an_import_of_no_module_suggests_one_at_its_level() {
    let lib = (
        "lib/util.gcsl",
        "version = 1\ntype Package = {name: String}",
    );
    let with = |text| [("workspace.gcsl", ""), lib, ("app.gcsl", text)];
    assert_eq!(
        suggestion_of(
            &with("import lib/utl\nx = 1"),
            "no module `lib/utl` at app.gcsl"
        ),
        Some("utl -> util".to_owned())
    );
    // A segment before the last: a directory.
    assert_eq!(
        suggestion_of(&with("import lbi/util\nx = 1"), "no module `lbi/util`"),
        Some("lbi -> lib".to_owned())
    );
    // One segment: a standard module too.
    assert_eq!(
        suggestion_of(&with("import pth\nx = 1"), "no module `pth`"),
        Some("pth -> path".to_owned())
    );
    // A member: a root or a type of the module.
    assert_eq!(
        suggestion_of(
            &with("import verison from lib/util\nx = 1"),
            "`lib.util` has no"
        ),
        Some("verison -> version".to_owned())
    );
    assert_eq!(
        suggestion_of(
            &with("import Pakage from lib/util\nx = 1"),
            "`lib.util` has no"
        ),
        Some("Pakage -> Package".to_owned())
    );
    // Too far: none.
    assert_eq!(
        suggestion_of(&with("import nothing\nx = 1"), "no module `nothing`"),
        None
    );
}

#[test]
fn a_name_of_the_view_is_suggested() {
    // A root of a directory above, a node's child, a type of the
    // directory.
    let files = [
        ("workspace.gcsl", "top = 1\ndev.peios.count = 2"),
        ("sub/a.gcsl", "x = tpo + 1"),
    ];
    assert_eq!(
        suggestion_of(&files, "undefined variable `tpo` at sub/a.gcsl"),
        Some("tpo -> top".to_owned())
    );
    let files = [
        ("workspace.gcsl", "top = 1\ndev.peios.count = 2"),
        ("sub/a.gcsl", "x = dev.peios.cuont + 1"),
    ];
    assert_eq!(
        suggestion_of(&files, "no field `cuont` in"),
        Some("cuont -> count".to_owned())
    );
    let files = [
        ("workspace.gcsl", ""),
        ("lib/types.gcsl", "type Package = {name: String}"),
        ("lib/a.gcsl", "p: Pakage = {name = \"n\"}"),
    ];
    assert_eq!(
        suggestion_of(&files, "unknown type `Pakage`"),
        Some("Pakage -> Package".to_owned())
    );
    // A meta's field.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev).pieos",
        ),
    ];
    assert_eq!(
        suggestion_of(&files, "the meta of `info dev` has no field `pieos`"),
        Some("pieos -> peios".to_owned())
    );
}

#[test]
fn a_failed_workspace_file_keeps_its_runtime_diagnostic() {
    let sources = [SourceFile::new("main.gcsl", "1 // 0").unwrap()];
    let mut workspace = check(&sources).unwrap();
    let main = workspace.file("main.gcsl").unwrap();
    let first = workspace.run(main).unwrap_err();
    assert_eq!(first[0].message, "division by zero");
    assert_eq!(first[0].input, Some(main));
    assert_eq!(workspace.run(main).unwrap_err(), first);
    assert_eq!(workspace.run(main).unwrap_err(), first);
}

#[test]
fn failed_files_do_not_repeat_effects_or_run_dependents() {
    let dir = fs_scratch("failed-workspace-file");
    let marker = format!("{dir}/attempts");
    let dependent_marker = format!("{dir}/dependent");
    let failed = format!(
        "import fs\n\
         let recorded = fs.write({marker:?}, if fs.exists({marker:?}) {{\n\
             fs.read({marker:?}) + \"x\"\n\
         }} else {{ \"x\" }})\n\
         answer = 1 // 0"
    );
    let dependent = format!(
        "import fs\n\
         let recorded = fs.write({dependent_marker:?}, \"ran\")\n\
         = answer"
    );
    let sources = [
        SourceFile::new("broken.gcsl", failed).unwrap(),
        SourceFile::new("dependent.gcsl", dependent).unwrap(),
        SourceFile::new("unrelated.gcsl", "= 42").unwrap(),
    ];
    let mut workspace = check(&sources).unwrap();
    let broken = workspace.file("broken.gcsl").unwrap();
    let dependent = workspace.file("dependent.gcsl").unwrap();
    let unrelated = workspace.file("unrelated.gcsl").unwrap();
    let first = workspace.run(broken).unwrap_err();
    assert_eq!(first[0].message, "division by zero");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    assert_eq!(workspace.run(broken).unwrap_err(), first);
    assert_eq!(workspace.run(dependent).unwrap_err(), first);
    assert_eq!(workspace.run(unrelated).unwrap().value, Value::Int(42));
    assert_eq!(workspace.run(dependent).unwrap_err(), first);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    assert!(!std::path::Path::new(&dependent_marker).exists());
    drop(workspace);
    std::fs::remove_dir_all(dir).unwrap();
}
