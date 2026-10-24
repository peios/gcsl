//! Tests of workspaces: the string built-ins, `path`, and versions.

use super::*;

/// The summary of slice 30, its `sh` a command every machine has.
pub(super) const STRINGS_SUMMARY: &str = "parts = split(\"a/b/c\", \"/\")\n\
                                   s = join(parts, \"-\")\n\
                                   yes = starts_with(\"/usr\", \"/\")\n\
                                   gz = ends_with(\"x.tar.gz\", \".gz\")\n\
                                   hash = trim(sh(\"echo ' abc '\"))\n\
                                   import path\n\
                                   p = path.join(\"/src/pkg\", \"../lib\")\n\
                                   q = path.join(\"/src/pkg\", \"/etc\")\n\
                                   d = path.dirname(\"/src/pkg/a.c\")\n\
                                   b = path.basename(\"/src/pkg/a.c\")\n\
                                   n = path.normalize(\"a/./b/../c/\")\n\
                                   abs = path.is_absolute(\"a\")";

/// The prelude's entry of the table of standard files, for a test's
/// own table.
pub(super) fn prelude_entry() -> (&'static str, &'static str) {
    crate::STANDARD[Library::NEEDED.find("prelude").unwrap()]
}

/// The workspace of `files` checked beside the standard library of
/// `table`, which it loads as a workspace does, with the names of the
/// standard files it loaded.
pub(super) fn checked_beside(
    table: &[(&str, &str)],
    files: &[(&str, &str)],
) -> (Vec<String>, Result<Workspace, Vec<Diagnostic>>) {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let library = Library {
        table,
        every: false,
    };
    let (standard, checked) = check_in((&sources, Placing::Workspace(&[])), library, "/");
    let names = standard.iter().map(|s| s.name().to_owned()).collect();
    (names, checked)
}

/// The value of the file at `path` of the workspace of `files`, checked
/// beside the standard library of `table`, with the names of the
/// standard files it loaded.
pub(super) fn value_beside(
    table: &[(&str, &str)],
    files: &[(&str, &str)],
    path: &str,
) -> (Vec<String>, String) {
    let (standard, checked) = checked_beside(table, files);
    let mut workspace = checked.unwrap();
    let index = workspace.file(path).unwrap();
    (standard, workspace.run(index).unwrap().value.to_string())
}

#[test]
fn the_strings_summary_runs_in_a_workspace_and_alone() {
    let expected = "{parts = [\"a\", \"b\", \"c\"]; s = \"a-b-c\"; yes = true; gz = true; \
                        hash = \"abc\"; p = \"/src/lib\"; q = \"/etc\"; d = \"/src/pkg\"; \
                        b = \"a.c\"; n = \"a/c\"; abs = false}";
    assert_eq!(lone(STRINGS_SUMMARY).unwrap(), expected);
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", STRINGS_SUMMARY)];
    assert_eq!(value(&files, "app/main.gcsl"), expected);
    // `path` is loaded, and takes the first input number, before the
    // prelude, in path order; each is a directory of its own.
    let scan = ws(&files).unwrap();
    assert_eq!(scan.standard(), 2);
    assert_eq!(scan.files[0].path, "<stdlib>/path.gcsl");
    assert_eq!(scan.files[1].path, "<stdlib>/prelude.gcsl");
    assert_ne!(scan.files[0].dir, scan.files[1].dir);
    for file in &scan.files[..2] {
        assert_eq!(scan.dirs[file.dir as usize].parent, None);
    }
    // It needs the prelude, whose `split` it calls: the prelude runs
    // first.
    assert_eq!(scan.needs[0], [1]);
    assert_eq!(&scan.order[..2], [1, 0]);
}

#[test]
fn split_gives_the_pieces_between_its_separator() {
    for (text, expected) in [
        ("= split(\"a/b/c\", \"/\")", "[\"a\", \"b\", \"c\"]"),
        ("= split(\"/a/b\", \"/\")", "[\"\", \"a\", \"b\"]"),
        ("= split(\"\", \"/\")", "[\"\"]"),
        ("= split(\"a//b\", \"/\")", "[\"a\", \"\", \"b\"]"),
        ("= split(\"a/\", \"/\")", "[\"a\", \"\"]"),
        ("= split(\"/\", \"/\")", "[\"\", \"\"]"),
        ("= split(\"abc\", \"/\")", "[\"abc\"]"),
        // A separator longer than one byte, and longer than the text.
        ("= split(\"a::b::\", \"::\")", "[\"a\", \"b\", \"\"]"),
        ("= split(\"a:::b\", \"::\")", "[\"a\", \":b\"]"),
        ("= split(\"ab\", \"abc\")", "[\"ab\"]"),
        // Text of several bytes a character, either side.
        (
            "= split(\"h\u{e9}llo w\u{f6}rld\", \"\u{f6}\")",
            "[\"h\u{e9}llo w\", \"rld\"]",
        ),
        (
            "= split(\"\u{65e5}\u{672c}\u{8a9e}\", \"\u{672c}\")",
            "[\"\u{65e5}\", \"\u{8a9e}\"]",
        ),
        ("= split(\"a\u{2192}b\", \"\u{2192}\")", "[\"a\", \"b\"]"),
        // Through a variable, by name.
        (
            "let f = split\n= f(sep = \",\", s = \"1,2\")",
            "[\"1\", \"2\"]",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // An empty separator is a runtime error at the call.
    let text = "x = 1\ny = split(\"abc\", \"\")";
    let error = lone_error(text);
    assert_eq!(error.message, "`split` needs a separator that is not empty");
    assert_eq!(error.span, Span::new(10, 26));
    assert_eq!(error.input, None);
    let error = lone_error("let sep = \"\"\nlet f = split\nx = f(\"a\", sep)");
    assert_eq!(error.message, "`split` needs a separator that is not empty");
}

#[test]
fn join_starts_with_ends_with_and_trim_work_on_strings() {
    for (text, expected) in [
        ("= join([\"a\", \"b\", \"c\"], \"-\")", "a-b-c"),
        ("let none: List<String> = []\n= join(none, \",\")", ""),
        ("= join([], \",\")", ""),
        ("= join([\"a\"], \",\")", "a"),
        ("= join([\"\", \"\"], \"/\")", "/"),
        ("= join([\"a\", \"b\"], \"\")", "ab"),
        ("= join([\"x\", \"y\"], \" \u{2192} \")", "x \u{2192} y"),
        ("= join(split(\"/a//b/\", \"/\"), \"/\")", "/a//b/"),
        ("= starts_with(\"/usr\", \"/\")", "true"),
        ("= starts_with(\"usr\", \"/\")", "false"),
        ("= starts_with(\"\", \"\")", "true"),
        ("= starts_with(\"a\", \"\")", "true"),
        ("= starts_with(\"\", \"a\")", "false"),
        ("= starts_with(\"/\", \"/usr\")", "false"),
        ("= ends_with(\"x.tar.gz\", \".gz\")", "true"),
        ("= ends_with(\"gz\", \".gz\")", "false"),
        ("= ends_with(\"a\", \"\")", "true"),
        // Byte-wise: a precomposed `é` is not an `e` with a combining
        // accent, though they look alike.
        ("= starts_with(\"\u{e9}t\u{e9}\", \"\u{e9}\")", "true"),
        ("= starts_with(\"e\\u{301}t\u{e9}\", \"\u{e9}\")", "false"),
        ("= ends_with(\"caf\u{e9}\", \"\u{e9}\")", "true"),
        ("= ends_with(\"cafe\\u{301}\", \"\u{e9}\")", "false"),
        ("= trim(\"  a b \\n\\t\")", "a b"),
        ("= trim(\"x\")", "x"),
        ("= trim(\"\")", ""),
        // Whitespace alone is nothing; Unicode's whitespace is trimmed
        // too, a no-break space and an ideographic space among it.
        ("= trim(\" \\n\\t\\r \")", ""),
        ("= trim(\"\\u{3000}x\\u{a0}\")", "x"),
        ("= trim(\"\\u{3000}\")", ""),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // Their values show as their types.
    for (text, expected) in [
        ("= split", "fn(s: String, sep: String): List<String>"),
        ("= join", "fn(parts: List<String>, sep: String): String"),
        ("= starts_with", "fn(s: String, prefix: String): Bool"),
        ("= ends_with", "fn(s: String, suffix: String): Bool"),
        ("= trim", "fn(s: String): String"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
    }
}

#[test]
fn path_handles_paths_lexically() {
    for (call, expected) in [
        // `normalize`, on the spec's examples and the edges.
        ("normalize(\"a/./b/../c/\")", "a/c"),
        ("normalize(\"\")", "."),
        ("normalize(\".\")", "."),
        ("normalize(\"./\")", "."),
        ("normalize(\"/\")", "/"),
        ("normalize(\"//\")", "/"),
        ("normalize(\"/../a\")", "/a"),
        ("normalize(\"../../a/..\")", "../.."),
        ("normalize(\"a/b/\")", "a/b"),
        ("normalize(\"a/b//\")", "a/b"),
        ("normalize(\"/a/b/\")", "/a/b"),
        ("normalize(\"a/../..\")", ".."),
        ("normalize(\"/a/../..\")", "/"),
        ("normalize(\"..\")", ".."),
        ("normalize(\"a//b\")", "a/b"),
        // `join`.
        ("join(\"/src/pkg\", \"../lib\")", "/src/lib"),
        ("join(\"/src/pkg\", \"/etc\")", "/etc"),
        ("join(\"/x\", \"\")", "/x"),
        ("join(\"\", \"a/b/\")", "a/b"),
        ("join(\"\", \"\")", "."),
        ("join(\"a\", \"b\")", "a/b"),
        ("join(\"a/\", \"../..\")", ".."),
        ("join(\"/\", \"..\")", "/"),
        ("join(\".\", \".\")", "."),
        // `dirname`.
        ("dirname(\"/src/pkg/a.c\")", "/src/pkg"),
        ("dirname(\"a\")", "."),
        ("dirname(\"/a\")", "/"),
        ("dirname(\"/\")", "/"),
        ("dirname(\"..\")", "."),
        ("dirname(\".\")", "."),
        ("dirname(\"\")", "."),
        ("dirname(\"//\")", "/"),
        ("dirname(\"a/../..\")", "."),
        ("dirname(\"/a/../..\")", "/"),
        ("dirname(\"a/b/\")", "a"),
        ("dirname(\"../../a\")", "../.."),
        // `basename`.
        ("basename(\"/src/pkg/a.c\")", "a.c"),
        ("basename(\"/\")", "/"),
        ("basename(\".\")", "."),
        ("basename(\"\")", "."),
        ("basename(\"//\")", "/"),
        ("basename(\"a/b/\")", "b"),
        ("basename(\"a/../..\")", ".."),
        ("basename(\"/a/../..\")", "/"),
        ("basename(\"a\")", "a"),
        // `is_absolute`.
        ("is_absolute(\"a\")", "false"),
        ("is_absolute(\"/\")", "true"),
        ("is_absolute(\"\")", "false"),
        ("is_absolute(\"//x\")", "true"),
    ] {
        let text = format!("import path\n= path.{call}");
        assert_eq!(lone(&text).unwrap(), expected, "{call}");
    }
    // In a workspace, as a member.
    let files = [
        ("workspace.gcsl", ""),
        (
            "main.gcsl",
            "import dirname from path\nd = dirname(\"a/b/c\")",
        ),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{d = \"a/b\"}");
}

#[test]
fn path_is_not_loaded_when_nobody_imports_it() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/a.gcsl", "x = [1]\ny = len(x)"),
        ("main.gcsl", "import lib/a\nz = a.y + 1"),
    ];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
    let mut workspace = checked.unwrap();
    assert!(
        workspace
            .scan
            .files
            .iter()
            .all(|f| f.path != "<stdlib>/path.gcsl")
    );
    // Its input numbers and slots are those of a run with no `path` at
    // all, beside the prelude alone.
    let (alone, bare) = checked_beside(&[prelude_entry()], &files);
    assert_eq!(alone, standard);
    let mut bare = bare.unwrap();
    let paths =
        |w: &Workspace| -> Vec<String> { w.scan.files.iter().map(|f| f.path.clone()).collect() };
    assert_eq!(paths(&workspace), paths(&bare));
    assert_eq!(workspace.scan.cells(), bare.scan.cells());
    assert_eq!(workspace.slot_names, bare.slot_names);
    let main = workspace.file("main.gcsl").unwrap();
    assert_eq!(main, 2);
    assert_eq!(workspace.dump_ir(main), bare.dump_ir(main));
    assert_eq!(
        workspace.run(main).unwrap().value,
        bare.run(main).unwrap().value
    );
    assert_eq!(workspace.frame.len(), bare.frame.len());
    // A lone file too: the prelude is input 0, the file the next.
    let source = SourceFile::new("lone.gcsl", "n = len([1])").unwrap();
    let (standard, checked) = check_lone_with_standard(&source, "/");
    assert_eq!(standard.len(), 1);
    assert_eq!(checked.unwrap().file("lone.gcsl"), Some(1));
}

#[test]
fn path_is_loaded_by_any_form_of_import() {
    for text in [
        "import path\nv = path.join(\"a\", \"b\")",
        "import path as p\nv = p.join(\"a\", \"b\")",
        "import join from path\nv = join(\"a\", \"b\")",
        "import join as pj from path\nv = pj(\"a\", \"b\")",
        "import normalize, join from path\nv = join(\"a\", \"b\")",
        // `*` binds its `join`, to which the prelude's yields.
        "import * from path\nv = join(\"a\", \"b\")",
    ] {
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
        assert_eq!(standard, ["<stdlib>/path.gcsl", "<stdlib>/prelude.gcsl"]);
        assert_eq!(value, "{v = \"a/b\"}", "{text}");
        assert_eq!(lone(text).unwrap(), "{v = \"a/b\"}", "{text}");
    }
    // By any scanned file, even one the file run does not need: every
    // file is checked, and the standard files loaded run first.
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "v = 1"),
        ("other.gcsl", "import path\nw = path.normalize(\"a/.\")"),
    ];
    let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
    assert_eq!(standard.len(), 2);
    assert_eq!(value, "{v = 1}");
    let workspace = checked(&files);
    assert_eq!(workspace.file("main.gcsl"), Some(2));
    assert_eq!(workspace.scan.needed(2), [1, 0, 2]);
    // An explicit `*` from both is the conflict of any two.
    let files = [
        ("workspace.gcsl", ""),
        (
            "main.gcsl",
            "import * from path\nimport * from prelude\nv = 1",
        ),
    ];
    assert!(
        shown_error(&files).starts_with("field `join` is defined in two files at main.gcsl"),
        "{}",
        shown_error(&files)
    );
}

#[test]
fn its_input_numbers_follow_the_standard_files_loaded() {
    // A syntax error is numbered after the standard files the files
    // that parsed need.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "import path"),
        ("b.gcsl", "y = ("),
    ];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard.len(), 2);
    assert_eq!(checked.unwrap_err()[0].input, Some(3));
    // A file with a syntax error imports what its recovered tree does.
    let files = [("workspace.gcsl", ""), ("a.gcsl", "import path\ny = (")];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard, ["<stdlib>/path.gcsl", "<stdlib>/prelude.gcsl"]);
    assert_eq!(checked.unwrap_err()[0].input, Some(2));
    // A runtime error raised in a standard file names its input: `a`,
    // loaded before the prelude, is input 0.
    let table = [
        ("a", "def fail() -> Int { panic(\"no\") }"),
        prelude_entry(),
    ];
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import a\nx = a.fail()"),
    ];
    let (standard, checked) = checked_beside(&table, &files);
    assert_eq!(standard, ["<stdlib>/a.gcsl", "<stdlib>/prelude.gcsl"]);
    let mut workspace = checked.unwrap();
    let main = workspace.file("main.gcsl").unwrap();
    let errors = workspace.run(main).unwrap_err();
    assert_eq!(errors[0].input, Some(0));
    // Run alone with `path`, the file's diagnostics name no input, and so
    // do those of `sh`, which reports them at its call.
    let error = lone_error("import path\nx = sh(\"exit 2\")");
    assert_eq!(error.input, None);
    let source = SourceFile::new("lone.gcsl", "import path\nx = 1 + true").unwrap();
    assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
}

#[test]
fn a_standard_module_only_another_imports_is_loaded() {
    // A table of two modules, `a` importing `b`, beside the prelude.
    let table = [
        ("a", "import b\nx = b.y + 1"),
        ("b", "y = len([1, 2])"),
        prelude_entry(),
    ];
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import a\nv = a.x")];
    let (standard, value) = value_beside(&table, &files, "main.gcsl");
    assert_eq!(
        standard,
        [
            "<stdlib>/a.gcsl",
            "<stdlib>/b.gcsl",
            "<stdlib>/prelude.gcsl"
        ]
    );
    assert_eq!(value, "{v = 3}");
    // `b` alone; and neither when nobody imports them.
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import y from b\nv = y"),
    ];
    let (standard, value) = value_beside(&table, &files, "main.gcsl");
    assert_eq!(standard, ["<stdlib>/b.gcsl", "<stdlib>/prelude.gcsl"]);
    assert_eq!(value, "{v = 2}");
    let files = [("workspace.gcsl", ""), ("main.gcsl", "v = 1")];
    let (standard, _) = value_beside(&table, &files, "main.gcsl");
    assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
    // Standard modules importing each other are the usual error.
    let table = [
        ("a", "import b\nx = b.y"),
        ("b", "import a\ny = a.x"),
        prelude_entry(),
    ];
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import a\nv = a.x")];
    let (_, checked) = checked_beside(&table, &files);
    let messages: Vec<String> = checked
        .unwrap_err()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(
        messages,
        ["`<stdlib>/a.gcsl` and `<stdlib>/b.gcsl` need each other"]
    );
}

#[test]
fn standard_modules_are_scopes_of_their_own() {
    // Two may declare one top-level name: the prelude's string `join`
    // and `path`'s, and two of a test's table.
    let text = "import path\nv = [join([\"a\", \"b\"], \"-\"), path.join(\"a\", \"b\")]";
    assert_eq!(lone(text).unwrap(), "{v = [\"a-b\", \"a/b\"]}");
    let table = [("a", "x = 1"), ("b", "x = 2"), prelude_entry()];
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import a\nimport b\nv = a.x * 10 + b.x"),
    ];
    assert_eq!(value_beside(&table, &files, "main.gcsl").1, "{v = 12}");
    // One sees nothing of another it does not import, though both are
    // loaded; it sees the prelude's names.
    let table = [("a", "x = 1"), ("b", "y = x"), prelude_entry()];
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import a\nimport b\nv = b.y"),
    ];
    let (standard, checked) = checked_beside(&table, &files);
    assert_eq!(standard.len(), 3);
    let errors = checked.unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message, "undefined variable `x`");
    assert_eq!(errors[0].input, Some(1));
    let table = [("a", "x = 1"), ("b", "y = half(2)"), prelude_entry()];
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import b\nv = b.y")];
    assert_eq!(value_beside(&table, &files, "main.gcsl").1, "{v = 1.0}");
    // Nor anything of the workspace.
    let table = [("a", "y = shared"), prelude_entry()];
    let files = [
        ("workspace.gcsl", "shared = 1"),
        ("main.gcsl", "import a\nv = a.y"),
    ];
    let (_, checked) = checked_beside(&table, &files);
    assert_eq!(
        checked.unwrap_err()[0].message,
        "undefined variable `shared`"
    );
}

#[test]
fn path_is_a_reserved_name() {
    for files in [
        &[("workspace.gcsl", ""), ("path.gcsl", "x = 1")][..],
        &[("path/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
    ] {
        // Reserved whether or not a file imports it.
        assert_eq!(
            error(files),
            (
                "`path` is the name of a standard module".to_owned(),
                Some(1),
                vec![]
            ),
            "{files:?}"
        );
    }
    // Below the root, a name of a path of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/path.gcsl", "x = 1"),
        ("main.gcsl", "import lib/path\nn = path.x"),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{n = 1}");
}

#[test]
fn a_file_s_own_join_shadows_the_prelude_s() {
    for (text, expected) in [
        (
            "join = \"mine\"\nv = join",
            "{join = \"mine\"; v = \"mine\"}",
        ),
        (
            "def join(a: Int, b: Int): Int { a + b }\nv = join(1, 2)",
            "{join = fn(a: Int, b: Int): Int; v = 3}",
        ),
        (
            "import path\njoin = 1\nv = path.join(\"a\", \"b\")",
            "{join = 1; v = \"a/b\"}",
        ),
        ("let trim = 2\nv = trim", "{v = 2}"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
}

#[test]
fn sh_s_message_has_its_standard_error_trimmed() {
    let error = lone_error("x = sh(\"printf ' a\\\\n\\\\n  ' >&2; exit 1\")");
    assert_eq!(
        error.message,
        "`printf ' a\\n\\n  ' >&2; exit 1` exited with status 1: a"
    );
    // Its result is still the raw standard output.
    assert_eq!(lone("= sh(\"printf ' a\\\\n'\")").unwrap(), " a\n");
}

/// The summary of slice 41.
pub(super) const MORE_STRINGS_SUMMARY: &str = "a = strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")  # \"6.6\"\n\
         b = strip_prefix(\"x\", \"y\")                                     # \"x\"\n\
         c = strip_suffix(\"a.tar.gz\", \".gz\")                            # \"a.tar\"\n\
         d = lines(\"a\\nb\\n\")                                            # [\"a\", \"b\"]\n\
         e = fields(\"  tcp\\t6  TCP \")                                   # [\"tcp\", \"6\", \"TCP\"]\n\
         f = compare_versions(\"6.10\", \"6.6\")                            # 1";

/// `text` run as a lone file and as a file of a workspace, each giving
/// `expected`.
pub(super) fn runs_to(text: &str, expected: &str) {
    assert_eq!(lone(text).unwrap(), expected, "{text}");
    let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
    assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
}

#[test]
fn the_more_strings_summary_runs_in_a_workspace_and_alone() {
    let expected = "{a = \"6.6\"; b = \"x\"; c = \"a.tar\"; d = [\"a\", \"b\"]; \
                        e = [\"tcp\", \"6\", \"TCP\"]; f = 1}";
    runs_to(MORE_STRINGS_SUMMARY, expected);
    // Nothing but the prelude is loaded for them.
    let files = [("workspace.gcsl", ""), ("main.gcsl", MORE_STRINGS_SUMMARY)];
    let scan = ws(&files).unwrap();
    assert_eq!(scan.standard(), 1);
    assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
}

#[test]
fn strip_prefix_and_strip_suffix_remove_what_is_there_once() {
    for (text, expected) in [
        (
            "= strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")",
            "6.6",
        ),
        ("= strip_prefix(\"x\", \"y\")", "x"),
        ("= strip_prefix(\"aab\", \"a\")", "ab"),
        ("= strip_prefix(\"abc\", \"abc\")", ""),
        ("= strip_prefix(\"ab\", \"abc\")", "ab"),
        ("= strip_prefix(\"abc\", \"\")", "abc"),
        ("= strip_prefix(\"\", \"\")", ""),
        ("= strip_prefix(\"\", \"a\")", ""),
        ("= strip_prefix(\"ba\", \"a\")", "ba"),
        ("= strip_suffix(\"a.tar.gz\", \".gz\")", "a.tar"),
        ("= strip_suffix(\"a.gz.gz\", \".gz\")", "a.gz"),
        ("= strip_suffix(\"a.tar\", \".gz\")", "a.tar"),
        ("= strip_suffix(\"abc\", \"\")", "abc"),
        ("= strip_suffix(\"\", \"\")", ""),
        ("= strip_suffix(\"gz\", \".gz\")", "gz"),
        ("= strip_suffix(\".gz\", \".gz\")", ""),
        // Byte-wise: a precomposed `é` is not an `e` with a combining
        // accent, and a prefix of `e` leaves the accent, a character of
        // its own, while it is not the start of a precomposed `é`.
        ("= strip_prefix(\"\u{e9}t\u{e9}\", \"\u{e9}\")", "t\u{e9}"),
        (
            "= strip_prefix(\"e\\u{301}t\u{e9}\", \"\u{e9}\")",
            "e\u{301}t\u{e9}",
        ),
        ("= strip_prefix(\"e\\u{301}x\", \"e\")", "\u{301}x"),
        ("= strip_prefix(\"\u{e9}t\u{e9}\", \"e\")", "\u{e9}t\u{e9}"),
        ("= strip_suffix(\"caf\u{e9}\", \"\u{e9}\")", "caf"),
        (
            "= strip_suffix(\"cafe\\u{301}\", \"\u{e9}\")",
            "cafe\u{301}",
        ),
        (
            "= strip_prefix(\"\u{65e5}\u{672c}\", \"\u{65e5}\")",
            "\u{672c}",
        ),
        // By name, and through a variable.
        ("= strip_prefix(prefix = \"v\", s = \"v1.2\")", "1.2"),
        ("let f = strip_suffix\n= f(\"x.c\", \".c\")", "x"),
    ] {
        runs_to(text, expected);
    }
}

#[test]
fn lines_and_fields_split_strings() {
    for (text, expected) in [
        ("= lines(\"a\\nb\\n\")", "[\"a\", \"b\"]"),
        ("= lines(\"\")", "[]"),
        ("= lines(\"\\n\")", "[\"\"]"),
        ("= lines(\"\\n\\n\")", "[\"\", \"\"]"),
        ("= lines(\"a\")", "[\"a\"]"),
        ("= lines(\"a\\n\\nb\")", "[\"a\", \"\", \"b\"]"),
        ("= lines(\"a\\r\\nb\\r\\n\")", "[\"a\", \"b\"]"),
        ("= lines(\"a\\r\\n\\r\\nb\")", "[\"a\", \"\", \"b\"]"),
        // A `\r` alone ends no line, and stays, as Rust's `lines` has
        // it.
        ("= lines(\"a\\rb\")", "[\"a\\rb\"]"),
        ("= lines(\"a\\r\")", "[\"a\\r\"]"),
        ("= lines(\" a \\n\\tb\")", "[\" a \", \"\\tb\"]"),
        (
            "= lines(\"h\u{e9}\\n\u{65e5}\")",
            "[\"h\u{e9}\", \"\u{65e5}\"]",
        ),
        ("= fields(\"  tcp\\t6  TCP \")", "[\"tcp\", \"6\", \"TCP\"]"),
        ("= fields(\"\")", "[]"),
        ("= fields(\"  \")", "[]"),
        ("= fields(\" \\n\\t\\r \")", "[]"),
        ("= fields(\"a\")", "[\"a\"]"),
        (
            "= fields(\"a b\\nc\\r\\nd\")",
            "[\"a\", \"b\", \"c\", \"d\"]",
        ),
        // Unicode's whitespace, as `trim`'s: a no-break space and an
        // ideographic space separate fields too.
        ("= fields(\"a\\u{a0}b\")", "[\"a\", \"b\"]"),
        (
            "= fields(\"\\u{3000}x\\u{3000}\\u{3000}y\")",
            "[\"x\", \"y\"]",
        ),
        (
            "= fields(\"h\u{e9} \u{65e5}\")",
            "[\"h\u{e9}\", \"\u{65e5}\"]",
        ),
        // Together, as a table is read.
        (
            "= for line in lines(\"tcp 6 TCP\\nudp\\t17 UDP\\n\") { fields(line)[1] }",
            "[\"6\", \"17\"]",
        ),
        ("let f = fields\n= f(s = \"x y\")", "[\"x\", \"y\"]"),
    ] {
        runs_to(text, expected);
    }
}

#[test]
fn compare_versions_orders_versions() {
    // Each pair in order, `a` before `b`: the other way round is `1`.
    let long = "1234567890123456789012345678901234567890";
    let longer = "12345678901234567890123456789012345678901";
    for (a, b) in [
        ("6.6", "6.10"),
        ("6.6", "6.6.1"),
        ("6.3", "6.3+nmu1"),
        ("6.3+nmu1", "6.4"),
        ("1.0~rc1", "1.0"),
        ("1.0~rc1", "1.0~rc2"),
        ("1.0~~", "1.0~"),
        ("1.0", "1.0a"),
        ("1.0", "1.0.0"),
        ("2", "10"),
        ("", "0"),
        ("", "a"),
        ("~", ""),
        ("1.01", "1.1"),
        ("01", "1"),
        ("a", "b"),
        ("Z", "a"),
        ("a", "."),
        ("a", "+"),
        ("1a", "1."),
        ("1.2.3", "1.2.10"),
        ("1:1.0", "1:1.1"),
        ("2.30-1", "2.30-2"),
        (long, longer),
        (
            "1.1234567890123456789012345678901234567890",
            "1.1234567890123456789012345678901234567891",
        ),
        // Bytes, not characters: a letter before any other byte, a
        // multi-byte character's bytes among them.
        ("1.a", "1.\u{e9}"),
        ("\u{e9}", "\u{f6}"),
    ] {
        let before = format!("= compare_versions(\"{a}\", \"{b}\")");
        runs_to(&before, "-1");
        let after = format!("= compare_versions(\"{b}\", \"{a}\")");
        runs_to(&after, "1");
    }
    // Equal strings, and only those, are `0`.
    for a in ["", "0", "6.6", "1.0~rc1", long, "\u{e9}"] {
        runs_to(&format!("= compare_versions(\"{a}\", \"{a}\")"), "0");
    }
    // By name, through `builtins`, and in a pure function.
    runs_to("= compare_versions(b = \"6.6\", a = \"6.10\")", "1");
    runs_to(
        "import builtins\n= builtins.compare_versions(\"1\", \"2\")",
        "-1",
    );
    runs_to(
        "fn newer(a: String, b: String): Bool { compare_versions(a, b) > 0 }\n\
             = [newer(\"6.10\", \"6.6\"), newer(\"1.0~rc1\", \"1.0\")]",
        "[true, false]",
    );
}

#[test]
fn the_more_string_builtins_are_values_of_their_types() {
    for (text, expected) in [
        ("= strip_prefix", "fn(s: String, prefix: String): String"),
        ("= strip_suffix", "fn(s: String, suffix: String): String"),
        ("= lines", "fn(s: String): List<String>"),
        ("= fields", "fn(s: String): List<String>"),
        ("= compare_versions", "fn(a: String, b: String): Int"),
        // Through `builtins`.
        (
            "import builtins\n= builtins.fields(\" a  b \")",
            "[\"a\", \"b\"]",
        ),
        (
            "import builtins\n= builtins.lines",
            "fn(s: String): List<String>",
        ),
        ("import lines from builtins\n= lines(\"x\\n\")", "[\"x\"]"),
    ] {
        runs_to(text, expected);
    }
    // A file's own declaration of one of their names shadows the
    // prelude's silently, and loses the built-in there.
    for (text, expected) in [
        (
            "fields = \"mine\"\nv = fields",
            "{fields = \"mine\"; v = \"mine\"}",
        ),
        ("let lines = 2\nv = lines", "{v = 2}"),
        (
            "def compare_versions(a: Int): Int { a }\nv = compare_versions(3)",
            "{compare_versions = fn(a: Int): Int; v = 3}",
        ),
        (
            "fields.x = 1\nv = trim(\" a \")",
            "{fields = {x = 1}; v = \"a\"}",
        ),
    ] {
        runs_to(text, expected);
    }
}
