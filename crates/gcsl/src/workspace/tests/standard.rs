//! Tests of workspaces: the standard library, the prelude and aliases of built-
//! in functions.

use super::*;

/// The summary of slice 28: the prelude's names, `builtins` and the
/// prelude imported by name.
pub(super) const STANDARD_SUMMARY: &str = "xs = [1, 2, 3]\n\
                                    n = len(xs)\n\
                                    h = half(n)\n\
                                    import builtins\n\
                                    same = builtins.len(xs) == n\n\
                                    import len as count from prelude\n\
                                    c = count(xs)";

/// The value of `text` run as a lone file, beside the standard library,
/// or the messages of its diagnostics.
pub(super) fn lone(text: &str) -> Result<String, Vec<String>> {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    crate::run(&source)
        .map(|outcome| outcome.value.to_string())
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect())
}

#[test]
fn the_standard_summary_runs_in_a_workspace_and_alone() {
    let expected = "{xs = [1, 2, 3]; n = 3; h = 1.5; same = true; c = 3}";
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", STANDARD_SUMMARY)];
    assert_eq!(value(&files, "app/main.gcsl"), expected);
    assert_eq!(lone(STANDARD_SUMMARY).unwrap(), expected);
    // The standard files are the first files, by input number, in a
    // directory of their own.
    let scan = ws(&files).unwrap();
    assert_eq!(scan.standard(), 1);
    assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
    assert_eq!(scan.dirs[scan.files[0].dir as usize].parent, None);
    assert_eq!(scan.order[0], 0);
    // The built-in functions' cells are the first.
    let builtins: Vec<Option<Builtin>> = (0..48).map(|node| scan.builtin(node)).collect();
    assert_eq!(
        builtins,
        [
            Some(Builtin::Len),
            Some(Builtin::Int),
            Some(Builtin::Float),
            Some(Builtin::Exec),
            Some(Builtin::Panic),
            Some(Builtin::Split),
            Some(Builtin::Join),
            Some(Builtin::StartsWith),
            Some(Builtin::EndsWith),
            Some(Builtin::Trim),
            Some(Builtin::StripPrefix),
            Some(Builtin::StripSuffix),
            Some(Builtin::Lines),
            Some(Builtin::Fields),
            Some(Builtin::CompareVersions),
            Some(Builtin::FsExists),
            Some(Builtin::FsRead),
            Some(Builtin::FsMkdir),
            Some(Builtin::FsCopy),
            Some(Builtin::FsTempDir),
            Some(Builtin::FsWrite),
            Some(Builtin::SortVersions),
            Some(Builtin::HostArch),
            Some(Builtin::HostOs),
            Some(Builtin::Show),
            Some(Builtin::ExecLive),
            Some(Builtin::VariantMatches),
            Some(Builtin::VariantPayload),
            Some(Builtin::PartialMatches),
            Some(Builtin::Secret),
            Some(Builtin::SecretValue),
            Some(Builtin::SecretReveal),
            Some(Builtin::File),
            Some(Builtin::FileRead),
            Some(Builtin::ExecSecret),
            Some(Builtin::Dir),
            Some(Builtin::DirFiles),
            Some(Builtin::DirFile),
            Some(Builtin::DirDir),
            Some(Builtin::FsComposeDir),
            Some(Builtin::DirSandbox),
            Some(Builtin::SandboxExec),
            Some(Builtin::SandboxFile),
            Some(Builtin::SandboxDir),
            Some(Builtin::Contains),
            Some(Builtin::ListUnique),
            Some(Builtin::ListContains),
            None
        ]
    );
}

#[test]
fn len_is_reached_by_every_road() {
    for text in [
        "n = len([1, 2])",
        "import len from builtins\nn = len([1, 2])",
        "import builtins\nn = builtins.len([1, 2])",
        "import * from builtins\nn = len([1, 2])",
        "import prelude\nn = prelude.len([1, 2])",
        "let l = len\nn = l([1, 2])",
        "let l = len\nlet m = l\nn = m([1, 2])",
        "l = len\nn = l([1, 2])",
        "import len as count from prelude\nlet l = count\nn = l([1, 2])",
        "import builtins\nl = builtins.len\nn = l([1, 2])",
    ] {
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        let shown = value(&files, "main.gcsl");
        assert!(shown.contains("n = 2"), "{text}: {shown}");
        assert!(lone(text).unwrap().contains("n = 2"), "{text}");
    }
    // An alias of `len` is of its opaque type; the callee of a call of
    // it has the function type of the call.
    let mut workspace = checked(&[("workspace.gcsl", ""), ("main.gcsl", "l = len\nn = l([1])")]);
    let index = workspace.file("main.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(name len : builtin len @4..7)"), "{dump}");
    assert!(
        dump.contains("(name l : fn(List<Int>): Int @12..13)"),
        "{dump}"
    );
}

#[test]
fn an_alias_of_len_crosses_files_directories_and_imports() {
    // A field of the root's, a member import's alias of an alias in
    // another directory, and a `let` of that in a third file.
    let files = [
        ("workspace.gcsl", "measure = len"),
        (
            "lib/a.gcsl",
            "import len as count from prelude\nsize = count",
        ),
        (
            "app/main.gcsl",
            "import size from lib\nlet l = size\nn = l([1, 2]) + measure([1])\n\
                 def twice(v: Int[]): Int { l(v) + size(v) }",
        ),
        ("app/other.gcsl", "m = measure([1, 2, 3]) + twice([1])"),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{n = 3; twice = fn(v: List<Int>): Int}"
    );
    assert_eq!(value(&files, "app/other.gcsl"), "{m = 5}");
    assert_eq!(value(&files, "lib/a.gcsl"), "{size = builtin len}");
    // Read whole, a record holding one is a use of it as a value.
    let mut whole = files.to_vec();
    whole.push(("app/whole.gcsl", "import lib\nall = lib"));
    assert_eq!(
        messages(&whole, "app/whole.gcsl"),
        ["`len` is a built-in function and cannot be used as a value yet"]
    );
}

#[test]
fn int_and_float_are_values_of_their_function_types() {
    for (text, expected) in [
        ("= int(2.7)", "2"),
        ("= int(-2.7)", "-2"),
        ("= int(x = 9.99)", "9"),
        ("= float(3)", "3.0"),
        ("= float(-3)", "-3.0"),
        ("= int", "fn(x: Float): Int"),
        ("import builtins\n= builtins.float", "fn(x: Int): Float"),
        ("import int from builtins\n= int(1.5)", "1"),
        // Passed, stored and called through a variable.
        ("let i = int\n= i(-0.5)", "0"),
        (
            "fn apply(f: fn(x: Float): Int, v: Float): Int { f(v) }\n= apply(int, 7.5)",
            "7",
        ),
        ("let fs = [float, float]\n= fs[1](2)", "2.0"),
        ("let r = { f = float }\n= r.f(4)", "4.0"),
        ("let mut f = int\nf := int\n= f(3.5)", "3"),
        ("= half(3)", "1.5"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // Truncation toward zero fails when the result is no `Int`.
    for (text, message) in [
        ("= int(1e300)", "integer overflow"),
        ("= int(-1e300)", "integer overflow"),
        ("= int(1.0 / 0.0)", "integer overflow"),
        ("= int(0.0 / 0.0)", "cannot truncate NaN to an Int"),
        ("let i = int\n= i(1e300)", "integer overflow"),
    ] {
        assert_eq!(lone(text).unwrap_err(), [message], "{text}");
    }
    assert_eq!(
        lone("= int(9223372036854774784.0)").unwrap(),
        "9223372036854774784"
    );
    assert_eq!(
        lone("= int(-9223372036854775808.0)").unwrap(),
        "-9223372036854775808"
    );
    // Functions have no equality, built-in ones included.
    assert_eq!(
        lone("= int === int").unwrap_err(),
        ["cannot compare functions"]
    );
}

#[test]
fn a_file_s_own_names_shadow_the_prelude_s_silently() {
    // A field, a `def`, a `let` and a `fn`: no clash, and the built-in
    // is gone there.
    for (text, expected) in [
        ("len = 1\nn = len + 1", "{len = 1; n = 2}"),
        (
            "def len(v: Int[]): Int { v[0] + 41 }\nn = len([1])",
            "{len = fn(v: List<Int>): Int; n = 42}",
        ),
        ("let len = 5\nn = len", "{n = 5}"),
        ("fn len(): Int { 7 }\nn = len()", "{n = 7}"),
        ("let int = 2\nn = int * 3", "{n = 6}"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // A `let` shadows from its item on, as it always has.
    assert_eq!(
        lone("n = len([1])\nlet len = 5\nm = len").unwrap(),
        "{n = 1; m = 5}"
    );
    // A field of the directory, or of a directory above, shadows it in
    // every file of the directory, and an explicit import does too.
    let files = [
        ("workspace.gcsl", "len = 3"),
        ("sub/a.gcsl", "n = len + 1"),
        ("sub/b.gcsl", "import int from builtins\nm = len + int(1.5)"),
    ];
    assert_eq!(value(&files, "sub/a.gcsl"), "{n = 4}");
    assert_eq!(value(&files, "sub/b.gcsl"), "{m = 4}");
    // Nothing of the implicit import is an alias the scan knows of.
    let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "len = 1")]).unwrap();
    let names: Vec<&str> = scan.files[2]
        .aliases
        .iter()
        .map(|alias| scan.names.get(alias.name))
        .collect();
    assert_eq!(
        names,
        [
            "show",
            "int",
            "float",
            "half",
            "exec",
            "exec_live",
            "panic",
            "split",
            "join",
            "starts_with",
            "ends_with",
            "trim",
            "strip_prefix",
            "strip_suffix",
            "lines",
            "fields",
            "compare_versions",
            "sort_versions",
            "sh",
            "sh_live",
            "secret",
            "exec_secret",
            "file",
            "dir",
            "build"
        ]
    );
    assert!(scan.files[2].aliases.iter().all(|alias| alias.implicit));
    assert!(scan.files[0].aliases.iter().all(|alias| !alias.implicit));
}

#[test]
fn an_explicit_star_from_the_prelude_conflicts_as_any() {
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import * from prelude\nlen = 3"),
    ];
    assert_eq!(
        shown_error(&files),
        "field `len` is defined in two files at main.gcsl (imported here); \
             <stdlib>/prelude.gcsl: defined here; main.gcsl: defined here"
    );
    // The same for the `builtins` module, whose side has no definition.
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import * from builtins\nlen = 3"),
    ];
    assert_eq!(
        shown_error(&files),
        "field `len` is defined in two files at main.gcsl (imported here); \
             main.gcsl: defined here; note: `len` is a built-in function"
    );
    // An explicit import of the prelude's name and the implicit one are
    // one cell: no conflict.
    assert_eq!(
        lone("import * from prelude\nn = len([1, 2])").unwrap(),
        "{n = 2}"
    );
    // A `let` of an imported name is the clash it always was.
    assert_eq!(
        lone("import len from prelude\nlet len = 1").unwrap_err(),
        ["`len` is imported in this file"]
    );
}

#[test]
fn the_standard_modules_names_are_reserved() {
    for (files, message, input) in [
        (
            &[("workspace.gcsl", ""), ("prelude.gcsl", "x = 1")][..],
            "`prelude` is the name of a standard module",
            Some(1),
        ),
        (
            &[("builtins/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
            "`builtins` is the name of a built-in module",
            Some(1),
        ),
        (
            &[("builtins.gcsl", ""), ("workspace.gcsl", "")][..],
            "`builtins` is the name of a built-in module",
            Some(1),
        ),
    ] {
        assert_eq!(
            error(files),
            (message.to_owned(), input, vec![]),
            "{files:?}"
        );
    }
    // Below the root, a name of a path of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/prelude.gcsl", "x = 1"),
        ("main.gcsl", "import lib/prelude\nn = prelude.x + len([1])"),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{n = 2}");
    // A path of more than one segment never names a standard module.
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import prelude/len")];
    assert_eq!(error(&files).0, "no module `prelude/len`");
}

#[test]
fn a_lone_file_imports_only_the_standard_library() {
    assert_eq!(
        lone("import nothing").unwrap_err(),
        [
            "`nothing` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
        ]
    );
    assert_eq!(
        lone("x = 1 + true\nimport a from c/d").unwrap_err(),
        [
            "cannot add Int and Bool",
            "`c/d` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
        ]
    );
    assert_eq!(lone("import builtins\n= builtins.len([1])").unwrap(), "1");
    assert_eq!(lone("import prelude\n= prelude.half(1)").unwrap(), "0.5");
    // Its diagnostics name no input; a standard file's names its own.
    let source = SourceFile::new("lone.gcsl", "x = 1 + true").unwrap();
    assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
    let source = SourceFile::new("lone.gcsl", "import * from prelude\nlen = 1").unwrap();
    let error = &crate::run(&source).unwrap_err()[0];
    assert_eq!(error.input, None);
    let inputs: Vec<Option<u32>> = error.labels.iter().map(|l| l.input).collect();
    assert_eq!(inputs, [Some(0), None]);
}

#[test]
fn the_table_of_standard_files_is_the_stdlib_directory() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib");
    let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "gcsl"))
        .map(|path| {
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            (stem, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort();
    let table: Vec<(String, String)> = crate::STANDARD
        .iter()
        .map(|&(stem, text)| (stem.to_owned(), text.to_owned()))
        .collect();
    assert_eq!(table, found);
    // Each is a standard file, in path order, named under `<stdlib>/`.
    let names: Vec<String> = standard_sources()
        .iter()
        .map(|source| source.name().to_owned())
        .collect();
    let expected: Vec<String> = found
        .iter()
        .map(|(stem, _)| format!("<stdlib>/{stem}.gcsl"))
        .collect();
    assert_eq!(names, expected);
}
