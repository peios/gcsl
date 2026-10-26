//! Tests of workspaces: shadowable declarations and the prelude.

use super::*;

/// The files of the summary of slice 31, in path order.
pub(super) const DEFAULTS: [(&str, &str); 3] = [
    (
        "_lib/defaults.gcsl",
        "#[shadowable]\narch = \"x86_64\"\n#[shadowable] jobs = 4",
    ),
    (
        "pkg/build.gcsl",
        "import * from _lib/defaults\narch = \"aarch64\"          # shadows the default\n\
             j = jobs                   # 4",
    ),
    ("workspace.gcsl", ""),
];

#[test]
fn the_shadowable_summary_runs() {
    assert_eq!(
        value(&DEFAULTS, "pkg/build.gcsl"),
        "{arch = \"aarch64\"; j = 4}"
    );
    assert_eq!(
        value(&DEFAULTS, "_lib/defaults.gcsl"),
        "{arch = \"x86_64\"; jobs = 4}"
    );
    // The implicit prelude is the file's too, and a name of it is
    // shadowed by a declaration as before.
    let mut files = DEFAULTS.to_vec();
    files[1].1 = "import * from _lib/defaults\narch = \"aarch64\"\nn = len([jobs])\n\
                      trim = 3";
    assert_eq!(
        value(&files, "pkg/build.gcsl"),
        "{arch = \"aarch64\"; n = 1; trim = 3}"
    );
    // Without its mark, the import's `arch` conflicts, as it always has.
    files[0].1 = "arch = \"x86_64\"\n#[shadowable] jobs = 4";
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at pkg/build.gcsl (imported here); \
             _lib/defaults.gcsl: defined here; pkg/build.gcsl: defined here"
    );
}

#[test]
fn within_one_file_the_last_declaration_wins_for_the_whole_file() {
    for (text, expected) in [
        // A read above it sees it too.
        (
            "x = a\n#[shadowable] a = 1\na = 2\ny = a",
            "{x = 2; a = 2; y = 2}",
        ),
        // The last of several shadowable ones.
        (
            "#[shadowable] a = 1\n#[shadowable] a = 2\nx = a\n#[shadowable] a = 3",
            "{x = 3; a = 3}",
        ),
        // A `def` by a field, and a field by a `def`: the winner's type.
        (
            "#[shadowable] def f(): Int { 1 }\nf = 2\nx = f + 1",
            "{f = 2; x = 3}",
        ),
        (
            "#[shadowable] f = 2\ndef f(n: Int): Int { n + 1 }\nx = f(1)",
            "{f = fn(n: Int): Int; x = 2}",
        ),
        // A function reads the winner too.
        (
            "fn get(): Int { a }\n#[shadowable] a = 1\na = 5\nx = get()",
            "{a = 5; x = 5}",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // A shadowing declaration cannot read the value it shadows.
    assert_eq!(
        lone("#[shadowable] a = 1\na = a + 1").unwrap_err(),
        ["`a` depends on itself"]
    );
    // A plain one is shadowed by nothing, and a plain one followed by a
    // shadowable one is the conflict it always was.
    for text in ["a = 1\na = 2", "a = 1\n#[shadowable] a = 2"] {
        assert_eq!(
            lone(text).unwrap_err(),
            ["field `a` is defined twice"],
            "{text}"
        );
    }
}

#[test]
fn a_shadowed_declaration_is_checked_but_never_run() {
    // Its own errors are reported.
    assert_eq!(
        lone("#[shadowable] a = 1 + \"x\"\na = 2").unwrap_err(),
        ["cannot add Int and String"]
    );
    // It runs nothing.
    assert_eq!(
        lone("#[shadowable] a = panic(\"never\")\na = 2").unwrap(),
        "{a = 2}"
    );
    // It needs nothing: here it alone reads `lib/x.gcsl`, which is not
    // run, and whose cell it sees as erroneous, without an error.
    let files = [
        (
            "a.gcsl",
            "import * from lib/x\n#[shadowable] arch = thing + 1\narch = 2",
        ),
        ("lib/x.gcsl", "thing = panic(\"never run\")"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a.gcsl"), "{arch = 2}");
    let scan = ws(&files).unwrap();
    let a = scan.files.iter().position(|f| f.path == "a.gcsl").unwrap();
    assert!(scan.needs[a].is_empty(), "{:?}", scan.needs[a]);
    // Nor is it in a cycle.
    assert_eq!(
        lone("#[shadowable] a = b\nb = a\na = 1").unwrap(),
        "{b = 1; a = 1}"
    );
}

#[test]
fn across_the_files_of_a_directory_in_path_order() {
    let files = [
        ("a.gcsl", "#[shadowable] arch = \"x86_64\"\nmine = arch"),
        ("b.gcsl", "arch = \"aarch64\""),
        ("c.gcsl", "got = arch"),
        ("workspace.gcsl", ""),
    ];
    // Every file of the directory agrees, the shadowed one included,
    // whose record no longer has it.
    assert_eq!(value(&files, "c.gcsl"), "{got = \"aarch64\"}");
    assert_eq!(value(&files, "a.gcsl"), "{mine = \"aarch64\"}");
    assert_eq!(value(&files, "b.gcsl"), "{arch = \"aarch64\"}");
    // The reverse conflicts.
    let files = [
        ("a.gcsl", "arch = \"x86_64\""),
        ("b.gcsl", "#[shadowable] arch = \"aarch64\""),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at b.gcsl (defined here); \
             a.gcsl: defined here"
    );
}

#[test]
fn an_inner_directory_shadows_an_outer_one() {
    let files = [
        ("other.gcsl", "got = arch"),
        ("sub/s.gcsl", "arch = \"aarch64\"\nmine = arch"),
        ("sub/t.gcsl", "seen = arch"),
        ("workspace.gcsl", "#[shadowable] arch = \"x86_64\""),
    ];
    assert_eq!(value(&files, "other.gcsl"), "{got = \"x86_64\"}");
    assert_eq!(
        value(&files, "sub/s.gcsl"),
        "{arch = \"aarch64\"; mine = \"aarch64\"}"
    );
    assert_eq!(value(&files, "sub/t.gcsl"), "{seen = \"aarch64\"}");
    // The reverse conflicts, with its note, reported as ever at the
    // later of the two in the order of conflicts (the file's own
    // directory first, then upward).
    let files = [
        ("sub/s.gcsl", "#[shadowable] arch = \"aarch64\""),
        ("workspace.gcsl", "arch = \"x86_64\""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at workspace.gcsl (defined here); \
             sub/s.gcsl: defined here; note: one is in a directory above"
    );
}

#[test]
fn an_import_and_a_field_shadow_in_their_order() {
    // A field after an import of a shadowable leaf: the field wins.
    let mut files = DEFAULTS.to_vec();
    files[1].1 = "import arch from _lib/defaults\narch = \"own\"\ngot = arch";
    assert_eq!(
        value(&files, "pkg/build.gcsl"),
        "{arch = \"own\"; got = \"own\"}"
    );
    // An import after a shadowable field: the import wins in this
    // file's view, which is its own; the field is still its
    // directory's, which the other files of the directory see.
    let files = [
        ("lib/defs.gcsl", "arch = \"lib\""),
        (
            "pkg/a.gcsl",
            "#[shadowable] arch = \"own\"\nimport arch from lib/defs\ngot = arch",
        ),
        ("pkg/b.gcsl", "seen = arch"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "pkg/a.gcsl"),
        "{arch = \"own\"; got = \"lib\"}"
    );
    assert_eq!(value(&files, "pkg/b.gcsl"), "{seen = \"own\"}");
    // A plain field before the import conflicts, as it always has.
    let files = [
        ("lib/defs.gcsl", "arch = \"lib\""),
        ("pkg/a.gcsl", "arch = \"own\"\nimport arch from lib/defs"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at pkg/a.gcsl (imported here); \
             lib/defs.gcsl: defined here; pkg/a.gcsl: defined here"
    );
}

#[test]
fn a_marked_import_brings_shadowable_leaves() {
    let files = [
        (
            "a/a.gcsl",
            "#[shadowable] import * from lib/defs\narch = \"own\"\nj = jobs",
        ),
        (
            "b/b.gcsl",
            "#[shadowable] import arch, jobs as n from lib/defs\narch = 1\nn = 3",
        ),
        ("lib/defs.gcsl", "arch = \"lib\"\njobs = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a/a.gcsl"), "{arch = \"own\"; j = 2}");
    assert_eq!(value(&files, "b/b.gcsl"), "{arch = 1; n = 3}");
    // Unmarked, the same is the conflict it always was.
    let files = [
        ("a/a.gcsl", "import * from lib/defs\narch = \"own\""),
        ("lib/defs.gcsl", "arch = \"lib\""),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at a/a.gcsl (imported here); \
             lib/defs.gcsl: defined here; a/a.gcsl: defined here"
    );
    // A `let` or a `fn` of its name shadows it silently, from its item
    // on, or in the whole file.
    let files = [
        ("lib/defs.gcsl", "arch = \"lib\"\njobs = 2"),
        (
            "pkg/a.gcsl",
            "#[shadowable] import * from lib/defs\nbefore = arch\nlet arch = 1\n\
                 after = arch\nfn jobs(): Int { 9 }\nj = jobs()",
        ),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "pkg/a.gcsl"),
        "{before = \"lib\"; after = 1; j = 9}"
    );
}

#[test]
fn shadowability_travels_with_a_leaf() {
    // Through `*` from a file, a member, a directory's merged tree and
    // a directory above.
    let files = [
        ("_lib/defaults.gcsl", "#[shadowable] arch = \"x86_64\""),
        ("_lib/more.gcsl", "jobs = 4"),
        ("a/a.gcsl", "import * from _lib/defaults\narch = \"a\""),
        ("b/b.gcsl", "import arch from _lib/defaults\narch = \"b\""),
        ("c/c.gcsl", "import * from _lib\narch = \"c\"\nj = jobs"),
        ("d/d.gcsl", "import _lib\narch = \"d\"\nx = _lib.arch"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a/a.gcsl"), "{arch = \"a\"}");
    assert_eq!(value(&files, "b/b.gcsl"), "{arch = \"b\"}");
    assert_eq!(value(&files, "c/c.gcsl"), "{arch = \"c\"; j = 4}");
    assert_eq!(value(&files, "d/d.gcsl"), "{arch = \"d\"; x = \"x86_64\"}");
    // And to the directories below its own, which see it in the
    // directory's merged tree.
    let files = [
        ("e/e.gcsl", "#[shadowable] arch = \"e\"\ny = arch"),
        ("e/f/f.gcsl", "arch = \"f\"\nz = arch"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "e/f/f.gcsl"), "{arch = \"f\"; z = \"f\"}");
    assert_eq!(value(&files, "e/e.gcsl"), "{arch = \"e\"; y = \"e\"}");
}

#[test]
fn dotted_leaves_are_shadowed_leaf_by_leaf() {
    // Two files of one directory: the later leaf wins, the others of
    // the node merge as before.
    let files = [
        ("a.gcsl", "#[shadowable] com.x.port = 1\ncom.x.host = \"h\""),
        ("b.gcsl", "com.x.port = 2"),
        ("c.gcsl", "all = com"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "c.gcsl"),
        "{all = {x = {host = \"h\"; port = 2}}}"
    );
    // Through an import.
    let files = [
        (
            "lib/defs.gcsl",
            "#[shadowable] com.x.port = 1\ncom.x.host = \"h\"",
        ),
        (
            "pkg/a.gcsl",
            "import * from lib/defs\ncom.x.port = 2\nall = com",
        ),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "pkg/a.gcsl"),
        "{com = {x = {port = 2}}; all = {x = {port = 2; host = \"h\"}}}"
    );
}

#[test]
fn a_declaration_overlapping_shadowable_ones_shadows_them_all() {
    for (text, expected) in [
        // At, above and below its path.
        ("#[shadowable] a.x = 1\na = {z = 3}", "{a = {z = 3}}"),
        ("#[shadowable] d = {j = 4; k = 5}\nd.j = 8", "{d = {j = 8}}"),
        (
            "#[shadowable] a.x.y = 1\n#[shadowable] a.w = 2\na.x = 3",
            "{a = {w = 2; x = 3}}",
        ),
        // Not overlapping: they merge as before.
        ("#[shadowable] a.x = 1\na.y = 2", "{a = {x = 1; y = 2}}"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // If one it overlaps is not shadowable, the conflict as today.
    assert_eq!(
        lone("#[shadowable] a.x = 1\na.y = 2\na = {}").unwrap_err(),
        ["field `a` is defined twice"]
    );
    assert_eq!(
        lone("d = {j = 4}\nd.j = 8").unwrap_err(),
        ["field `d` is already defined"]
    );
    // Across files and directories, as within one.
    let files = [
        ("a.gcsl", "#[shadowable] d = {j = 4; k = 5}"),
        ("b.gcsl", "d.j = 8"),
        ("c.gcsl", "got = d"),
        ("sub/s.gcsl", "a = 1\nseen = d"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "c.gcsl"), "{got = {j = 8}}");
    assert_eq!(value(&files, "sub/s.gcsl"), "{a = 1; seen = {j = 8}}");
    let files = [
        ("sub/s.gcsl", "a = {z = 3}\ngot = a"),
        ("workspace.gcsl", "#[shadowable] a.x = 1\nseen = a"),
    ];
    assert_eq!(value(&files, "sub/s.gcsl"), "{a = {z = 3}; got = {z = 3}}");
    assert_eq!(
        value(&files, "workspace.gcsl"),
        "{a = {x = 1}; seen = {x = 1}}"
    );
}

#[test]
fn a_dotted_root_shadows_the_prelude_s_name_as_before() {
    // The five cases the old rule's drop by root name shadowed: each
    // is its result before this slice.
    assert_eq!(
        lone("len.x = 1\ny = len").unwrap(),
        "{len = {x = 1}; y = {x = 1}}"
    );
    let files = [
        ("app/main.gcsl", "import * from lib/a\ny = len"),
        ("lib/a.gcsl", "len.x = 1"),
        ("workspace.gcsl", "x = 1"),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{y = {x = 1}}");
    let files = [
        ("app/main.gcsl", "s = join\nn = len([1, 2])"),
        ("workspace.gcsl", "join.sep = \",\""),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{s = {sep = \",\"}; n = 2}");
    let files = [
        ("a.gcsl", "len.x = 1"),
        ("b.gcsl", "y = len\nparts = split(\"a,b\", \",\")"),
        ("workspace.gcsl", "x = 1"),
    ];
    assert_eq!(
        value(&files, "b.gcsl"),
        "{y = {x = 1}; parts = [\"a\", \"b\"]}"
    );
    // (The REPL's is `crate::session`'s.) An explicit import is plain,
    // and still conflicts.
    assert_eq!(
        lone("import * from prelude\nlen.x = 1").unwrap_err(),
        ["field `len` is already defined"]
    );
}

#[test]
fn no_prelude_turns_the_implicit_import_off() {
    assert_eq!(
        lone("#![no_prelude]\nn = len([1])").unwrap_err(),
        ["undefined variable `len`"]
    );
    assert_eq!(
        lone("# a comment\n#![no_prelude]\nimport len from builtins\nn = len([1])").unwrap(),
        "{n = 1}"
    );
    let files = [
        ("main.gcsl", "#![no_prelude]\nn = trim(\" a \")"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(messages(&files, "main.gcsl"), ["undefined variable `trim`"]);
    // The prelude has it, and so no import of its own; every other
    // file, the standard ones included, has the implicit one.
    let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "import path")]).unwrap();
    let prelude = scan.standard_file(PRELUDE).unwrap();
    let path = scan.standard_file("path").unwrap();
    let files = &scan.files;
    assert!(files[prelude as usize].ast.no_prelude().is_some());
    assert!(files[prelude as usize].ast.implicit_import().is_none());
    assert!(files[prelude as usize].aliases.iter().all(|a| !a.implicit));
    for file in [path, 2, 3] {
        let file = &files[file as usize];
        assert!(file.ast.implicit_import().is_some(), "{}", file.path);
        assert!(file.aliases.iter().any(|a| a.implicit), "{}", file.path);
    }
}

#[test]
fn the_clash_family_ignores_shadowable_leaves() {
    // A `let`, a `fn`, a `def`, a field or a whole import of the name
    // of a shadowable field of a directory above shadows it silently.
    let files = [
        ("a/a.gcsl", "let port = 5\nn = port"),
        ("b/b.gcsl", "fn port(): Int { 7 }\nn = port()"),
        ("c/c.gcsl", "def port(): Int { 8 }\nn = port()"),
        ("d/d.gcsl", "import lib/port\nn = port.x"),
        ("e/e.gcsl", "port = 6\nn = port"),
        ("lib/port.gcsl", "x = 9"),
        ("workspace.gcsl", "#[shadowable] port = 1"),
    ];
    assert_eq!(value(&files, "a/a.gcsl"), "{n = 5}");
    assert_eq!(value(&files, "b/b.gcsl"), "{n = 7}");
    assert_eq!(value(&files, "c/c.gcsl"), "{port = fn(): Int; n = 8}");
    assert_eq!(value(&files, "d/d.gcsl"), "{n = 9}");
    assert_eq!(value(&files, "e/e.gcsl"), "{port = 6; n = 6}");
    // Within one file, a `let` from its item on and a `fn` in the whole
    // file; the field is still the file's.
    assert_eq!(
        lone("#[shadowable] a = 1\nx = a\nlet a = 2\ny = a").unwrap(),
        "{a = 1; x = 1; y = 2}"
    );
    assert_eq!(
        lone("x = a()\n#[shadowable] a = 1\nfn a(): Int { 3 }").unwrap(),
        "{x = 3; a = 1}"
    );
    // Plain, they clash as they always have.
    let files = [
        ("sub/a.gcsl", "let port = 5"),
        ("workspace.gcsl", "port = 1"),
    ];
    assert_eq!(
        shown_checked_error(&files),
        "`port` is a field of this directory"
    );
    assert_eq!(
        lone("a = 1\nlet a = 2").unwrap_err(),
        ["`a` is a field of this file"]
    );
}

#[test]
fn a_declaration_a_later_file_shadows_is_still_checked_and_its_file_s() {
    // Its own error, against its own file.
    let sources: Vec<SourceFile> = [
        ("a.gcsl", "#[shadowable] arch = 1 + \"x\""),
        ("b.gcsl", "arch = 2"),
        ("workspace.gcsl", ""),
    ]
    .iter()
    .map(|&(path, text)| SourceFile::new(path, text).unwrap())
    .collect();
    let errors = check(&sources).unwrap_err();
    let scan = ws(&[("a.gcsl", ""), ("workspace.gcsl", "")]).unwrap();
    let a = scan.files.iter().position(|f| f.path == "a.gcsl").unwrap();
    let shown: Vec<(&str, Option<u32>)> = errors
        .iter()
        .map(|e| (e.message.as_str(), e.input))
        .collect();
    assert_eq!(shown, [("cannot add Int and String", Some(count(a)))]);
    // A `let` that shadows a file's own shadowable field is the file's
    // alone: the directory has the field.
    let files = [
        ("a.gcsl", "#[shadowable] a = 1\nlet a = 2\nmine = a"),
        ("b.gcsl", "x = a"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{x = 1}");
    assert_eq!(value(&files, "a.gcsl"), "{a = 1; mine = 2}");
    // A `fn` shadows a dotted shadowable root of its file in the whole
    // file; the root is still its field, and its directory's.
    let files = [
        (
            "a.gcsl",
            "#[shadowable] a.x = 1\nfn a(): Int { 3 }\nb = a()",
        ),
        ("c.gcsl", "y = a.x"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a.gcsl"), "{a = {x = 1}; b = 3}");
    assert_eq!(value(&files, "c.gcsl"), "{y = 1}");
}

/// The messages of the errors of checking `files`, joined.
pub(super) fn shown_checked_error(files: &[(&str, &str)]) -> String {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let errors = check(&sources).unwrap_err();
    let shown: Vec<String> = errors.into_iter().map(|e| e.message).collect();
    shown.join("; ")
}

#[test]
fn an_input_cannot_turn_the_prelude_off() {
    let started = start("/").unwrap();
    let (mut names, mut types) = (started.names, started.types);
    let source = SourceFile::new("input", "#![no_prelude]\nx = 1").unwrap();
    let earlier = Earlier {
        imports: &[],
        types: &[],
        store: &mut TypeStore::new(),
        def_types: &[],
        variants: &[],
        contexts: &[],
    };
    let errors = check_input(
        &started.standard,
        (&source, earlier),
        (&mut names, &mut types),
        Scope::of_slots(started.slot_names),
        2,
    )
    .unwrap_err();
    let shown: Vec<(String, Span, Option<u32>)> = errors
        .into_iter()
        .map(|e| (e.message, e.span, e.input))
        .collect();
    assert_eq!(
        shown,
        [(
            "`#![no_prelude]` applies only to a file".to_owned(),
            Span::new(0, 14),
            None
        )]
    );
}
