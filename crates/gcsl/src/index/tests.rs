use super::*;
use crate::workspace::index_with_standard;

/// The workspace of `files`, each a path and a text, indexed, with a
/// `workspace.gcsl` if they have none.
fn indexed(files: &[(&str, &str)]) -> Indexed {
    let mut sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    if !files.iter().any(|&(path, _)| path == "workspace.gcsl") {
        sources.push(SourceFile::new("workspace.gcsl", "").unwrap());
    }
    sources.sort_by(|a, b| a.name().cmp(b.name()));
    index_with_standard(&sources, &[], "/")
}

/// The byte offset in `text` of the `|` in `needle`, found with it
/// taken out: `"let |x"` is the `x` after the first `let `.
fn offset(text: &str, needle: &str) -> u32 {
    let bar = needle.find('|').unwrap_or(0);
    let plain = needle.replacen('|', "", 1);
    let at = text
        .find(&plain)
        .unwrap_or_else(|| panic!("{plain:?} is not in {text:?}"));
    u32::try_from(at + bar).unwrap()
}

/// A place to ask about, and what the answers must be.
struct Case {
    /// What it shows.
    name: &'static str,
    /// The files.
    files: Vec<(&'static str, &'static str)>,
    /// The file asked about, and where in it, as [`offset`] finds it.
    at: (&'static str, &'static str),
    /// What the hover must hold, each in turn, if it must have one.
    hover: Option<Vec<&'static str>>,
    /// The declarations the definition must be: each a file and where in
    /// it the name starts, as [`offset`] finds it, and the name's length.
    definition: Vec<(&'static str, &'static str, u32)>,
}

/// Asks `case` of its files with `extra` added to the end of the file
/// asked about, and checks the answers, `how` naming the variant.
fn ask(case: &Case, extra: &str, how: &str) {
    let files: Vec<(&str, String)> = case
        .files
        .iter()
        .map(|&(path, text)| {
            let text = if path == case.at.0 {
                format!("{text}{extra}")
            } else {
                text.to_owned()
            };
            (path, text)
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let Indexed {
        index,
        diagnostics,
        succeeded,
        ..
    } = indexed(&borrowed);
    let what = format!("{} ({how})", case.name);
    // The error added is one: the check fails, and is indexed anyway.
    assert_eq!(succeeded, extra.is_empty(), "{what}: {diagnostics:?}");
    let mut index = index.unwrap_or_else(|| panic!("{what}: no index: {diagnostics:?}"));
    if extra.is_empty() {
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity != crate::Severity::Error),
            "{what}: {diagnostics:?}"
        );
    }
    let text_of = |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, t)| t.clone())
            .unwrap()
    };
    let file = index.file(case.at.0).unwrap();
    assert!(index.checked(file), "{what}: not checked");
    let at = offset(&text_of(case.at.0), case.at.1);
    let hover = index.hover(file, at);
    match (&case.hover, &hover) {
        (Some(wanted), Some(found)) => {
            for part in wanted {
                assert!(found.text.contains(part), "{what}: {part:?} in {found:?}");
            }
            assert!(found.span.start <= at && at < found.span.end, "{what}");
        }
        (None, None) => {}
        _ => panic!("{what}: hover {hover:?}, wanted {:?}", case.hover),
    }
    let found: Vec<(String, u32, u32)> = index
        .definition(file, at)
        .into_iter()
        .map(|l| {
            let path = index.path(l.file).unwrap().to_owned();
            (path, l.span.start, l.span.end)
        })
        .collect();
    let wanted: Vec<(String, u32, u32)> = case
        .definition
        .iter()
        .map(|&(path, needle, length)| {
            let start = offset(&text_of(path), needle);
            (path.to_owned(), start, start + length)
        })
        .collect();
    assert_eq!(found, wanted, "{what}");
}

/// A recursive function.
const FACT: &str = "fn fact(n: Int): Int {\n    if n < 2 { 1 } else { n * fact(n - 1) }\n}\nr \
                        = fact(5)\n";

/// A function with a parameter, `let`s and a `for`.
const LOCALS: &str = "fn f(a: Int): Int {\n    let b = a + 1\n    let xs = for i in [1, 2] { \
                          i * b }\n    b + len(xs)\n}\nr = f(1)\n";

/// A def-type.
const PACKAGE: &str = "#[def_type]\ndef package(name: String, func: fn(): String): String { \
                           func() }\n";

/// The cases of the index's answers.
fn cases() -> Vec<Case> {
    let mut cases = local_cases();
    cases.extend(field_cases());
    cases.extend(other_cases());
    cases.extend(standard_cases());
    cases
}

/// The cases of locals.
fn local_cases() -> Vec<Case> {
    let (fact, locals) = (FACT, LOCALS);
    vec![
        Case {
            name: "a parameter",
            files: vec![("app.gcsl", locals)],
            at: ("app.gcsl", "= |a + 1"),
            hover: Some(vec!["a: Int", "parameter"]),
            definition: vec![("app.gcsl", "f(|a: Int)", 1)],
        },
        Case {
            name: "a let",
            files: vec![("app.gcsl", locals)],
            at: ("app.gcsl", "    |b + len"),
            hover: Some(vec!["b: Int"]),
            definition: vec![("app.gcsl", "let |b", 1)],
        },
        Case {
            name: "a for binding",
            files: vec![("app.gcsl", locals)],
            at: ("app.gcsl", "{ |i * b"),
            hover: Some(vec!["i: Int", "`for` variable"]),
            definition: vec![("app.gcsl", "for |i in", 1)],
        },
        Case {
            name: "a declaration itself",
            files: vec![("app.gcsl", locals)],
            at: ("app.gcsl", "let |b"),
            hover: Some(vec!["b: Int"]),
            definition: vec![("app.gcsl", "let |b", 1)],
        },
        Case {
            name: "shadowing",
            files: vec![(
                "app.gcsl",
                "fn g(): String {\n    let x = 1\n    let x = \"s\"\n    x\n}\nr = g()\n",
            )],
            at: ("app.gcsl", "    |x\n}"),
            hover: Some(vec!["x: String"]),
            definition: vec![("app.gcsl", "let |x = \"s\"", 1)],
        },
        Case {
            name: "a named fn",
            files: vec![("app.gcsl", fact)],
            at: ("app.gcsl", "r = |fact(5)"),
            hover: Some(vec!["fn fact(n: Int): Int"]),
            definition: vec![("app.gcsl", "fn |fact", 4)],
        },
        Case {
            name: "recursion",
            files: vec![("app.gcsl", fact)],
            at: ("app.gcsl", "n * |fact"),
            hover: Some(vec!["fn fact(n: Int): Int"]),
            definition: vec![("app.gcsl", "fn |fact", 4)],
        },
    ]
}

/// The cases of fields and imports.
fn field_cases() -> Vec<Case> {
    vec![
        Case {
            name: "a field of the file",
            files: vec![("app.gcsl", "a = 1\nb = a + 1\n")],
            at: ("app.gcsl", "= |a + 1"),
            hover: Some(vec!["a: Int", "field of the root directory"]),
            definition: vec![("app.gcsl", "|a = 1", 1)],
        },
        Case {
            name: "a field of another file of the directory",
            files: vec![
                ("sub/app.gcsl", "x = base + 1\n"),
                ("sub/lib.gcsl", "base = 1\n"),
            ],
            at: ("sub/app.gcsl", "|base"),
            hover: Some(vec!["base: Int", "field of directory `sub/`"]),
            definition: vec![("sub/lib.gcsl", "|base", 4)],
        },
        Case {
            name: "a field of a directory above",
            files: vec![("sub/app.gcsl", "x = top + 1\n"), ("top.gcsl", "top = 2\n")],
            at: ("sub/app.gcsl", "|top"),
            hover: Some(vec!["top: Int", "field of the root directory"]),
            definition: vec![("top.gcsl", "|top", 3)],
        },
        Case {
            name: "an open field",
            files: vec![("app.gcsl", "cfg.a = 1\ncfg.b = \"s\"\nr = cfg\n")],
            at: ("app.gcsl", "r = |cfg"),
            hover: Some(vec!["cfg: {a: Int, b: String}"]),
            definition: vec![("app.gcsl", "|cfg.a", 3), ("app.gcsl", "|cfg.b", 3)],
        },
        Case {
            name: "a segment of a dotted path",
            files: vec![("app.gcsl", "cfg.a = 1\ncfg.b = \"s\"\nr = cfg.b\n")],
            at: ("app.gcsl", "r = cfg.|b"),
            hover: Some(vec!["cfg.b: String"]),
            definition: vec![("app.gcsl", "cfg.|b", 1)],
        },
        Case {
            name: "an imported name",
            files: vec![
                ("app.gcsl", "import v from lib/x\nr = v + 1\n"),
                ("lib/x.gcsl", "v = 1\n"),
            ],
            at: ("app.gcsl", "r = |v"),
            hover: Some(vec!["v: Int", "imported from `lib/x`"]),
            definition: vec![("lib/x.gcsl", "|v", 1)],
        },
        Case {
            name: "an import's path",
            files: vec![
                ("app.gcsl", "import v from lib/x\nr = v + 1\n"),
                ("lib/x.gcsl", "v = 1\n"),
            ],
            at: ("app.gcsl", "lib/|x"),
            hover: Some(vec!["lib.x: {v: Int}", "file `lib/x.gcsl`"]),
            definition: vec![("lib/x.gcsl", "|", 0)],
        },
    ]
}

/// The cases of def-types, types and records' fields.
fn other_cases() -> Vec<Case> {
    let package = PACKAGE;
    vec![
        Case {
            name: "a def-type reference",
            files: vec![
                ("app.gcsl", "r = package dev.x\n"),
                ("pkg.gcsl", "def package dev.x() { \"hi\" }\n"),
                ("workspace.gcsl", package),
            ],
            at: ("app.gcsl", "dev.|x"),
            hover: Some(vec!["package dev.x", "definition of def-type `package`"]),
            definition: vec![("pkg.gcsl", "dev.|x", 1)],
        },
        Case {
            name: "a definition's path",
            files: vec![
                ("app.gcsl", "r = package dev.x\n"),
                ("pkg.gcsl", "def package dev.x() { \"hi\" }\n"),
                ("workspace.gcsl", package),
            ],
            at: ("pkg.gcsl", "dev.|x"),
            hover: Some(vec!["def package dev.x(): String"]),
            definition: vec![("workspace.gcsl", "def |package", 7)],
        },
        Case {
            name: "a type's name",
            files: vec![(
                "app.gcsl",
                "type V = {version: String}\nfn f(v: V): String { v.version }\nr = f({version = \"1\"})\n",
            )],
            at: ("app.gcsl", "v: |V"),
            hover: Some(vec!["type V = {version: String}"]),
            definition: vec![("app.gcsl", "type |V", 1)],
        },
        Case {
            name: "a field of a record value",
            files: vec![(
                "app.gcsl",
                "type V = {version: String}\nfn f(v: V): String { v.version }\nr = f({version = \"1\"})\n",
            )],
            at: ("app.gcsl", "v.|version"),
            hover: Some(vec!["version: String"]),
            definition: vec![],
        },
        Case {
            name: "a field of a record literal",
            files: vec![(
                "app.gcsl",
                "fn g(): Int {\n    let c = {port = 80}\n    c.port\n}\nr = g()\n",
            )],
            at: ("app.gcsl", "c.|port"),
            hover: Some(vec!["port: Int"]),
            definition: vec![("app.gcsl", "{|port = 80", 4)],
        },
    ]
}

/// The cases of the standard library, of built-ins and of nothing.
fn standard_cases() -> Vec<Case> {
    let (package, locals) = (PACKAGE, LOCALS);
    vec![
        Case {
            name: "a standard function",
            files: vec![("app.gcsl", "r = len([1, 2])\n")],
            at: ("app.gcsl", "|len"),
            hover: Some(vec![
                "fn<T>(list: List<T>): Int",
                "standard library `prelude`",
            ]),
            definition: vec![],
        },
        Case {
            name: "a standard module's function",
            files: vec![("app.gcsl", "import path\nr = path.join(\"a\", \"b\")\n")],
            at: ("app.gcsl", "path.|join"),
            hover: Some(vec![
                "def join(base: String, rel: String): String",
                "standard library `path`",
            ]),
            definition: vec![],
        },
        Case {
            name: "a built-in function",
            files: vec![("app.gcsl", "import builtins\nr = builtins.int(1.5)\n")],
            at: ("app.gcsl", "builtins.|int"),
            hover: Some(vec!["int: fn(x: Float): Int", "built-in"]),
            definition: vec![],
        },
        Case {
            name: "a definition's def-type",
            files: vec![
                ("pkg.gcsl", "def package dev.x() { \"hi\" }\n"),
                ("workspace.gcsl", package),
            ],
            at: ("pkg.gcsl", "def |package"),
            hover: Some(vec!["def package(name: ", "func: fn(): String): String"]),
            definition: vec![("workspace.gcsl", "def |package", 7)],
        },
        Case {
            name: "a built-in type",
            files: vec![("app.gcsl", "r: Int = 1\n")],
            at: ("app.gcsl", "|Int"),
            hover: Some(vec!["Int", "built-in type"]),
            definition: vec![],
        },
        Case {
            name: "a keyword",
            files: vec![("app.gcsl", locals)],
            at: ("app.gcsl", "|let b"),
            hover: None,
            definition: vec![],
        },
        Case {
            name: "whitespace",
            files: vec![("app.gcsl", "a = 1\nb = a  + 1\n")],
            at: ("app.gcsl", "a | + 1"),
            hover: None,
            definition: vec![],
        },
        Case {
            name: "a literal",
            files: vec![("app.gcsl", "a = 1\nb = a + 12\n")],
            at: ("app.gcsl", "1|2"),
            hover: None,
            definition: vec![],
        },
    ]
}

#[test]
fn each_case_is_answered_in_a_file_clean_with_a_type_error_and_with_a_syntax_error() {
    for case in cases() {
        ask(&case, "", "clean");
        ask(&case, "\nbroken_type = 1 + \"a\"\n", "type error elsewhere");
        ask(&case, "\nbroken_syntax = (1 2)\n", "syntax error elsewhere");
    }
}

#[test]
fn an_erroneous_expression_has_no_hover() {
    let Indexed { index, .. } = indexed(&[("app.gcsl", "x = 1 + \"a\"\ny = x\n")]);
    let mut index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    let text = "x = 1 + \"a\"\ny = x\n";
    assert_eq!(index.hover(file, offset(text, "y = |x")), None);
    // But its declaration is still found.
    assert_eq!(index.definition(file, offset(text, "y = |x")).len(), 1);
}

#[test]
fn an_unbalanced_file_is_not_indexed() {
    let Indexed {
        index, succeeded, ..
    } = indexed(&[("app.gcsl", "x = 1\n}\ny = x\n")]);
    assert!(!succeeded);
    let index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    assert!(!index.checked(file));
    assert!(index.occurrences(file).is_empty());
}

#[test]
fn a_long_record_type_is_cut_short() {
    let mut text = String::from("big = {\n");
    for at in 0..40 {
        let _ = writeln!(text, "    field{at} = {at}");
    }
    text.push_str("}\nr = big\n");
    let Indexed { index, .. } = indexed(&[("app.gcsl", &text)]);
    let mut index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    let hover = index.hover(file, offset(&text, "r = |big")).unwrap();
    let lines = hover.text.lines().count();
    // The fence, MAX_LINES lines of code, the fence, a blank and the
    // origin.
    assert_eq!(lines, MAX_LINES + 4, "{}", hover.text);
    assert!(hover.text.contains("\n…\n```"), "{}", hover.text);
}

#[test]
fn a_quantity_is_shown_by_its_type() {
    let text = "t = 30s\nm = 2GiB\nr = t\ns = m\n";
    let Indexed { index, .. } = indexed(&[("app.gcsl", text)]);
    let mut index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    for (at, ty) in [("r = |t", "Duration"), ("s = |m", "Size")] {
        let hover = index.hover(file, offset(text, at)).unwrap();
        assert!(hover.text.contains(ty), "{at}: {}", hover.text);
    }
}

#[test]
fn a_wide_record_type_shows_its_rest_field_after_its_named_fields() {
    // Too wide for one line, a field per line, the rest-field last; the
    // named fields are never lost to it.
    let text = "type File = {destination_path: String, source_path_in_build: String, \
                *: {mode: String, owner_principal: String}}\n\
                f: File = { destination_path = \"a\"; source_path_in_build = \"b\" }\n\
                r = f\n";
    let Indexed { index, .. } = indexed(&[("workspace.gcsl", text)]);
    let mut index = index.unwrap();
    let file = index.file("workspace.gcsl").unwrap();
    let hover = index.hover(file, offset(text, "r = |f")).unwrap();
    assert!(
        hover.text.contains(
            "{\n  destination_path: String,\n  source_path_in_build: String,\n  \
             *: {mode: String, owner_principal: String},\n}"
        ),
        "{}",
        hover.text
    );
    // Narrow enough, on one line.
    let text = "f: {to: String, *: Int} = { to = \"a\" }\nr = f\n";
    let Indexed { index, .. } = indexed(&[("app.gcsl", text)]);
    let mut index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    let hover = index.hover(file, offset(text, "r = |f")).unwrap();
    assert!(
        hover.text.contains("{to: String, *: Int}"),
        "{}",
        hover.text
    );
}

/// The files' index, and the input number of the file at `path`.
fn index_of(files: &[(&str, &str)], path: &str) -> (Index, u32) {
    let Indexed {
        index, diagnostics, ..
    } = indexed(files);
    let index = index.unwrap_or_else(|| panic!("no index: {diagnostics:?}"));
    let file = index.file(path).unwrap();
    (index, file)
}

/// `locations` as each file's path and the text at each place.
fn shown(index: &Index, files: &[(&str, &str)], locations: &[Location]) -> Vec<(String, String)> {
    locations
        .iter()
        .map(|location| {
            let path = index.path(location.file).unwrap();
            let text = files.iter().find(|(p, _)| *p == path).unwrap().1;
            let at = &text[location.span.start as usize..location.span.end as usize];
            (path.to_owned(), format!("{at}@{}", location.span.start))
        })
        .collect()
}

/// The places of `locations`, each a file and where in it the name
/// starts, as [`offset`] finds it.
fn places(files: &[(&str, &str)], wanted: &[(&str, &str)]) -> Vec<(String, u32)> {
    wanted
        .iter()
        .map(|&(path, needle)| {
            let text = files.iter().find(|(p, _)| *p == path).unwrap().1;
            (path.to_owned(), offset(text, needle))
        })
        .collect()
}

/// The references at `at` of `files`, as files and starts.
fn references(files: &[(&str, &str)], at: (&str, &str), declarations: bool) -> Vec<(String, u32)> {
    let (index, file) = index_of(files, at.0);
    let text = files.iter().find(|(p, _)| *p == at.0).unwrap().1;
    let found = index.references(file, offset(text, at.1), declarations);
    // Every place found is the symbol's name or a segment of it.
    assert!(!shown(&index, files, &found).is_empty() || found.is_empty());
    found
        .iter()
        .map(|l| (index.path(l.file).unwrap().to_owned(), l.span.start))
        .collect()
}

#[test]
fn references_of_a_local_with_and_without_its_declaration() {
    let files = [("app.gcsl", LOCALS)];
    let all = places(
        &files,
        &[
            ("app.gcsl", "let |b"),
            ("app.gcsl", "i * |b"),
            ("app.gcsl", "    |b + len"),
        ],
    );
    assert_eq!(references(&files, ("app.gcsl", "i * |b"), true), all);
    assert_eq!(references(&files, ("app.gcsl", "i * |b"), false), all[1..]);
    // Nothing for a keyword.
    assert_eq!(references(&files, ("app.gcsl", "|let b"), true), []);
}

#[test]
fn references_of_a_node_across_files() {
    let files = [
        ("sub/app.gcsl", "x = base + 1\ny = base\n"),
        ("sub/lib.gcsl", "base = 1\n"),
    ];
    let all = places(
        &files,
        &[
            ("sub/app.gcsl", "x = |base"),
            ("sub/app.gcsl", "y = |base"),
            ("sub/lib.gcsl", "|base"),
        ],
    );
    assert_eq!(references(&files, ("sub/lib.gcsl", "|base"), true), all);
    assert_eq!(references(&files, ("sub/app.gcsl", "y = |base"), true), all);
}

#[test]
fn references_of_a_definition_include_its_path_and_its_references() {
    let files = [
        ("app.gcsl", "r = package dev.x\n"),
        ("pkg.gcsl", "def package dev.x() { \"hi\" }\n"),
        ("workspace.gcsl", PACKAGE),
    ];
    let all = places(&files, &[("app.gcsl", "dev.|x"), ("pkg.gcsl", "dev.|x")]);
    assert_eq!(references(&files, ("pkg.gcsl", "dev.|x"), true), all);
    assert_eq!(references(&files, ("pkg.gcsl", "dev.|x"), false), all[..1]);
}

#[test]
fn references_of_an_imported_name() {
    let files = [
        ("app.gcsl", "import v from lib/x\nr = v + 1\n"),
        ("lib/x.gcsl", "v = 1\n"),
    ];
    let all = places(
        &files,
        &[
            ("app.gcsl", "import |v"),
            ("app.gcsl", "r = |v"),
            ("lib/x.gcsl", "|v"),
        ],
    );
    assert_eq!(references(&files, ("app.gcsl", "r = |v"), true), all);
    assert_eq!(references(&files, ("lib/x.gcsl", "|v"), true), all);
    // A standard function's, in the workspace only.
    let files = [("app.gcsl", "a = len([1])\nb = len([2])\n")];
    let all = places(
        &files,
        &[("app.gcsl", "a = |len"), ("app.gcsl", "b = |len")],
    );
    assert_eq!(references(&files, ("app.gcsl", "a = |len"), true), all);
}

#[test]
fn a_rebind_is_the_variable_it_rebinds_and_writes_it() {
    for text in [
        "fn g(): Int {\n    let n = 1\n    n := n + 1\n    n\n}\nr = g()\n",
        "fn g(): Int {\n    let mut n = 1\n    n := n + 1\n    n\n}\nr = g()\n",
    ] {
        let files = [("app.gcsl", text)];
        let (index, file) = index_of(&files, "app.gcsl");
        let found: Vec<(u32, bool)> = index
            .highlights(file, offset(text, "    |n\n}"))
            .iter()
            .map(|h| (h.span.start, h.write))
            .collect();
        let wanted = vec![
            (offset(text, "n = 1"), true),
            (offset(text, "|n :="), true),
            (offset(text, ":= |n"), false),
            (offset(text, "    |n\n}"), false),
        ];
        assert_eq!(found, wanted, "{text}");
        // The declaration is the `let`'s.
        assert_eq!(index.definition(file, offset(text, "    |n\n}")).len(), 1);
        assert_eq!(
            index.definition(file, offset(text, "    |n\n}"))[0]
                .span
                .start,
            offset(text, "n = 1")
        );
    }
}

/// Renames what is at `at` of `files` to `new`: the files' texts as
/// renamed, each changed one with its path, or why not.
fn renamed(
    files: &[(&str, &str)],
    at: (&str, &str),
    new: &str,
) -> Result<Vec<(String, String)>, String> {
    let (index, file) = index_of(files, at.0);
    let text = files.iter().find(|(p, _)| *p == at.0).unwrap().1;
    let edits = index.rename((file, offset(text, at.1)), new, |sources| {
        index_with_standard(sources, &[], "/")
    })?;
    let mut changed: Vec<(String, String)> = Vec::new();
    for (path, text) in files {
        let input = index.file(path).unwrap();
        let mut edited = (*text).to_owned();
        let mut mine: Vec<&Edit> = edits.iter().filter(|e| e.file == input).collect();
        mine.sort_by_key(|e| std::cmp::Reverse(e.span.start));
        if mine.is_empty() {
            continue;
        }
        for edit in mine {
            edited.replace_range(edit.span.start as usize..edit.span.end as usize, &edit.text);
        }
        changed.push(((*path).to_owned(), edited));
    }
    Ok(changed)
}

#[test]
fn a_rename_renames_every_file_and_keeps_an_import_s_alias() {
    let files = [
        ("sub/app.gcsl", "x = base + 1\ny = base\n"),
        ("sub/lib.gcsl", "base = 1\n"),
        ("other.gcsl", "import base as b from sub/lib\nz = b\n"),
    ];
    let at = index_of(&files, "sub/lib.gcsl");
    assert_eq!(
        at.0.prepare_rename(at.1, 0),
        Ok(Prepared {
            span: Span::new(0, 4),
            placeholder: "base".to_owned()
        })
    );
    assert_eq!(
        renamed(&files, ("sub/app.gcsl", "y = |base"), "root"),
        Ok(vec![
            (
                "sub/app.gcsl".to_owned(),
                "x = root + 1\ny = root\n".to_owned()
            ),
            ("sub/lib.gcsl".to_owned(), "root = 1\n".to_owned()),
            (
                "other.gcsl".to_owned(),
                "import root as b from sub/lib\nz = b\n".to_owned()
            ),
        ])
    );
    // The alias itself is not renamed from here.
    let refused = renamed(&files, ("other.gcsl", "z = |b"), "c").unwrap_err();
    assert!(refused.contains("alias"), "{refused}");
}

#[test]
fn a_rename_of_a_local_and_of_a_type() {
    let files = [("app.gcsl", LOCALS)];
    assert_eq!(
        renamed(&files, ("app.gcsl", "i * |b"), "next"),
        Ok(vec![(
            "app.gcsl".to_owned(),
            LOCALS
                .replace("let b", "let next")
                .replace("i * b", "i * next")
                .replace("    b + len", "    next + len")
        )])
    );
    let text =
        "type V = {version: String}\nfn f(v: V): String { v.version }\nr = f({version = \"1\"})\n";
    let files = [("app.gcsl", text)];
    assert_eq!(
        renamed(&files, ("app.gcsl", "v: |V"), "Version"),
        Ok(vec![(
            "app.gcsl".to_owned(),
            text.replace("type V", "type Version")
                .replace("v: V", "v: Version")
        )])
    );
}

#[test]
fn a_rename_of_a_definition_s_segment_renames_its_meta_and_its_references() {
    let workspace = "#[def_type]\ndef package(name: String, f: fn(v: String): String META-> {versions: \
                         List<String>}, version: String) -> String { f(version) }\n";
    let files = [
        ("workspace.gcsl", workspace),
        (
            "app/a.gcsl",
            "defmeta package dev.x { versions = [\"0.1.0\"] }\ndef package dev.x(v) { v }\n",
        ),
        ("app/b.gcsl", "r = package dev.x\n"),
    ];
    let renamed = renamed(&files, ("app/a.gcsl", "def package dev.|x"), "go-1+").unwrap();
    assert_eq!(
            renamed,
            vec![
                (
                    "app/a.gcsl".to_owned(),
                    "defmeta package dev.go-1+ { versions = [\"0.1.0\"] }\ndef package dev.go-1+(v) { v }\n"
                        .to_owned()
                ),
                ("app/b.gcsl".to_owned(), "r = package dev.go-1+\n".to_owned()),
            ]
        );
    let refused = renamed_err(&files, ("app/a.gcsl", "def package dev.|x"), "a.b");
    assert!(refused.contains("segment"), "{refused}");
}

/// Why renaming what is at `at` of `files` to `new` is refused.
#[test]
fn renaming_a_variable_keeps_a_shorthand_argument_naming_its_parameter() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "app.gcsl",
            "fn f(out: Int, compiler: Int): Int { out + compiler }\n\
             let compiler = 2\n\
             r = f(out = 1, compiler)\n",
        ),
    ];
    let changed = renamed(&files, ("app.gcsl", "let |compiler"), "cc").unwrap();
    assert_eq!(
        changed,
        [(
            "app.gcsl".to_owned(),
            "fn f(out: Int, compiler: Int): Int { out + compiler }\n\
             let cc = 2\n\
             r = f(out = 1, compiler = cc)\n"
                .to_owned()
        )]
    );
}

#[test]
fn a_parameter_read_in_a_later_default_is_one_symbol() {
    // A default that is no constant is its own function in the checker,
    // whose parameters copy the earlier ones: reading one there is still
    // the parameter, as is a name it captures from where `f` is written.
    let text = "let base = 1\nfn f(a: Int, b: Int = a + base): Int { a * b }\nr = f(2)\n";
    let files = [("app.gcsl", text)];
    let all = places(
        &files,
        &[
            ("app.gcsl", "fn f(|a"),
            ("app.gcsl", "= |a + base"),
            ("app.gcsl", "{ |a * b"),
        ],
    );
    assert_eq!(references(&files, ("app.gcsl", "= |a + base"), true), all);
    assert_eq!(references(&files, ("app.gcsl", "{ |a * b"), true), all);
    assert_eq!(
        renamed(&files, ("app.gcsl", "= |a + base"), "first"),
        Ok(vec![(
            "app.gcsl".to_owned(),
            text.replace("f(a: Int", "f(first: Int")
                .replace("= a + base", "= first + base")
                .replace("{ a * b", "{ first * b")
        )])
    );
    let base = places(
        &files,
        &[("app.gcsl", "let |base"), ("app.gcsl", "a + |base")],
    );
    assert_eq!(references(&files, ("app.gcsl", "a + |base"), true), base);
    let Indexed { index, .. } = indexed(&files);
    let mut index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    let hover = index.hover(file, offset(text, "= |a + base")).unwrap();
    assert!(hover.text.contains("Int"), "{}", hover.text);
}

fn renamed_err(files: &[(&str, &str)], at: (&str, &str), new: &str) -> String {
    renamed(files, at, new).unwrap_err()
}

#[test]
fn a_lone_file_is_documented_and_renamed_and_an_open_field_renamed_whole() {
    let text = "w = 0\n# Doc.\nx = 1\ny = x\n";
    let source = SourceFile::new("/x/lone.gcsl", text).unwrap();
    let Indexed { index, .. } = crate::workspace::index_lone_with_standard(&source, "/x");
    let index = index.unwrap();
    // The scan names a lone file by its file name.
    let file = index.file("lone.gcsl").unwrap();
    let at = offset(text, "y = |x");
    assert_eq!(index.documentation(file, at).as_deref(), Some("Doc."));
    assert_eq!(index.prepare_rename(file, at).unwrap().placeholder, "x");
    let edits = index
        .rename((file, at), "z", |sources| {
            crate::workspace::index_lone_with_standard(&sources[0], "/x")
        })
        .unwrap();
    assert_eq!(edits.len(), 2);
    // The standard files' sources are known too, for their doc comments.
    let (index, _) = index_of(&[("app.gcsl", "r = len([1])\n")], "app.gcsl");
    assert!((0..index.standard()).all(|file| !index.text(file).is_empty()));
    // Every item that defines an open field.
    let files = [
        ("app.gcsl", "cfg.a = 1\ncfg.b = 2\nr = cfg\n"),
        ("b.gcsl", "s = cfg.a\n"),
    ];
    assert_eq!(
        renamed(&files, ("app.gcsl", "r = |cfg"), "conf"),
        Ok(vec![
            (
                "app.gcsl".to_owned(),
                "conf.a = 1\nconf.b = 2\nr = conf\n".to_owned()
            ),
            ("b.gcsl".to_owned(), "s = conf.a\n".to_owned()),
        ])
    );
}

#[test]
fn a_rename_that_would_capture_a_name_is_refused() {
    let text = "fn g(): Int {\n    let a = 1\n    let b = 2\n    a + b\n}\nr = g()\n";
    let refused = renamed(&[("app.gcsl", text)], ("app.gcsl", "let |a"), "b").unwrap_err();
    assert!(refused.contains("refer to something else"), "{refused}");
    assert!(refused.contains("app.gcsl:4:5"), "{refused}");
    // A field of the file captured by a local of a function.
    let text = "top = 1\nfn g(): Int {\n    let inner = 2\n    top + inner\n}\nr = g()\n";
    let refused = renamed(&[("app.gcsl", text)], ("app.gcsl", "|top = 1"), "inner").unwrap_err();
    assert!(refused.contains("refer to something else"), "{refused}");
}

#[test]
fn a_rename_that_collides_with_a_directory_s_field_is_refused() {
    let files = [("sub/a.gcsl", "x = 1\n"), ("sub/b.gcsl", "y = 2\n")];
    let refused = renamed(&files, ("sub/a.gcsl", "|x"), "y").unwrap_err();
    assert!(refused.contains("be an error"), "{refused}");
    assert!(refused.contains("`y`"), "{refused}");
}

#[test]
fn a_rename_to_what_is_no_name_is_refused() {
    let files = [("app.gcsl", "a = 1\nb = a\n")];
    for new in ["1a", "a b", "let", "", "x.y", "\"q\""] {
        let refused = renamed(&files, ("app.gcsl", "b = |a"), new).unwrap_err();
        assert!(refused.contains("is not a name"), "{new}: {refused}");
    }
    // The same name: nothing to do.
    assert_eq!(renamed(&files, ("app.gcsl", "b = |a"), "a"), Ok(Vec::new()));
}

#[test]
fn what_the_standard_library_a_directory_or_a_string_declares_is_not_renamed() {
    /// The files, where to rename, and what the refusal says.
    type Refused<'a> = (&'a [(&'a str, &'a str)], (&'a str, &'a str), &'a str);
    let cases: [Refused<'_>; 6] = [
        (
            &[("app.gcsl", "r = len([1])\n")],
            ("app.gcsl", "|len"),
            "standard library",
        ),
        (
            &[("app.gcsl", "r: Int = 1\n")],
            ("app.gcsl", "|Int"),
            "built-in type",
        ),
        (
            &[
                ("app.gcsl", "import lib/x\nr = x.v\n"),
                ("lib/x.gcsl", "v = 1\n"),
            ],
            ("app.gcsl", "import |lib"),
            "directories or files",
        ),
        (
            &[
                ("app.gcsl", "import lib/x\nr = x.v\n"),
                ("lib/x.gcsl", "v = 1\n"),
            ],
            ("app.gcsl", "r = |x"),
            "directories or files",
        ),
        (
            &[("app.gcsl", "\"a-b\" = 1\nr = a-b\n")],
            ("app.gcsl", "r = |a-b"),
            "as a string",
        ),
        (
            &[("app.gcsl", "a = 1 + 2\n")],
            ("app.gcsl", "1 |+"),
            "no name here",
        ),
    ];
    for (files, at, why) in cases {
        let (index, file) = index_of(files, at.0);
        let text = files.iter().find(|(p, _)| *p == at.0).unwrap().1;
        let refused = index.prepare_rename(file, offset(text, at.1)).unwrap_err();
        assert!(refused.contains(why), "{at:?}: {refused}");
    }
}

#[test]
fn a_file_s_symbols_nest_record_blocks_three_deep() {
    let text = "import lib/x\n\
                    type V = {version: String}\n\
                    let helper = 1\n\
                    fn twice(n: Int): Int { n * 2 }\n\
                    def public(): Int { 1 }\n\
                    cfg = {\n    a = 1\n    b = {\n        c = {\n            d = {\n                e = 1\n            }\n        }\n    }\n}\n\
                    path.to = \"s\"\n";
    let files = [("app.gcsl", text), ("lib/x.gcsl", "v = 1\n")];
    let (index, file) = index_of(&files, "app.gcsl");
    let symbols = index.document_symbols(file);
    let top: Vec<(&str, SymbolKind, Option<&str>)> = symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind, s.detail.as_deref()))
        .collect();
    assert_eq!(
        top,
        [
            ("imports", SymbolKind::Module, None),
            ("V", SymbolKind::Type, Some("{version: String}")),
            ("helper", SymbolKind::Variable, Some("Int")),
            ("twice", SymbolKind::Function, Some("fn(n: Int): Int")),
            ("public", SymbolKind::Function, Some("fn(): Int")),
            (
                "cfg",
                SymbolKind::Field,
                Some("{a: Int, b: {c: {d: {e: Int}}}}")
            ),
            ("path.to", SymbolKind::Field, Some("String")),
        ]
    );
    assert_eq!(symbols[0].children[0].name, "lib.x");
    let cfg = &symbols[5];
    assert_eq!(
        &text[cfg.selection.start as usize..cfg.selection.end as usize],
        "cfg"
    );
    let names: Vec<&str> = cfg.children.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    // Three levels: `cfg`, `b`, `c`; `d` is not listed.
    let c = &cfg.children[1].children[0];
    assert_eq!(c.name, "c");
    assert!(c.children.is_empty());
    // Across the workspace, by a subsequence of the name, any case.
    let found: Vec<String> = index
        .workspace_symbols("TWC", 200)
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(found, ["twice"]);
    let found = index.workspace_symbols("", 200);
    assert!(
        found.iter().any(|s| s.name == "v" && s.file != file),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|s| s.name == "c" && s.container.as_deref() == Some("b"))
    );
    assert_eq!(index.workspace_symbols("", 3).len(), 3);
}

#[test]
fn a_definition_s_symbol_is_named_by_its_path() {
    let files = [
        ("pkg.gcsl", "def package dev.x() { \"hi\" }\n"),
        ("workspace.gcsl", PACKAGE),
    ];
    let (index, file) = index_of(&files, "pkg.gcsl");
    let symbols = index.document_symbols(file);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "package dev.x");
    assert_eq!(symbols[0].kind, SymbolKind::Definition);
    assert_eq!(symbols[0].detail.as_deref(), Some("fn(): String"));
}

#[test]
fn doc_comments_are_the_comment_lines_directly_above() {
    let text = "w = 0\n# The answer.\n#   Indented.\n#\nanswer = 42\nx = answer # trailing\n# Not x's: a blank line.\n\ny = x\n\
                    # Through an attribute.\n#[shadowable]\nq = 1\n\
                    fn f(): Int {\n    # A local.\n    let l = 1\n    l\n}\nr = f() + q + y\n";
    let files = [("app.gcsl", text)];
    let (mut index, file) = index_of(&files, "app.gcsl");
    let doc = |index: &Index, needle: &str| index.documentation(file, offset(text, needle));
    assert_eq!(
        doc(&index, "x = |answer"),
        Some("The answer.\n  Indented.".to_owned())
    );
    assert_eq!(doc(&index, "y = |x"), None);
    assert_eq!(doc(&index, "r = f() + q + |y"), None);
    assert_eq!(
        doc(&index, "+ |q"),
        Some("Through an attribute.".to_owned())
    );
    assert_eq!(doc(&index, "    |l\n"), Some("A local.".to_owned()));
    let hover = index.hover(file, offset(text, "x = |answer")).unwrap();
    assert_eq!(
        hover.text,
        "```gcsl\nanswer: Int\n```\n\nThe answer.\n  Indented.\n\nfield of the root directory"
    );
    // A definition with none has its meta's.
    let workspace = "#[def_type]\ndef package(name: String, f: fn(v: String): String META-> \
                         {versions: List<String>}, version: String) -> String { f(version) }\n";
    let files = [
        ("workspace.gcsl", workspace),
        (
            "app/a.gcsl",
            "import fileroot\n# The x package.\ndefmeta package dev.x { versions = [] }\n\n\
                 def package dev.x(v) { v }\n",
        ),
        ("app/b.gcsl", "r = package dev.x\n"),
    ];
    let (index, file) = index_of(&files, "app/b.gcsl");
    let found = index.documentation(file, offset(files[2].1, "dev.|x"));
    assert_eq!(found.as_deref(), Some("The x package."));
}

#[test]
fn a_file_s_header_is_no_doc_comment() {
    // A run of comments from the file's first line, or a shebang's, is
    // the file's header: a lock file's, directly above its one field;
    // one after a shebang; one above a `def`. Anywhere else, the same
    // run is the doc comment of the item below it.
    for (text, needle, wanted) in [
        (
            "# Written by `pekit lock`. Do not edit.\nlocked = [1]\nr = locked\n",
            "r = |locked",
            None,
        ),
        (
            "#!/usr/bin/env gcsl\n# A script.\nx = 1\nr = x\n",
            "r = |x",
            None,
        ),
        ("# Root.\ndef f(): Int { 1 }\nr = f()\n", "r = |f", None),
        ("v = 0\n# Doc.\nx = 1\nr = x\n", "r = |x", Some("Doc.")),
        (
            "#!/usr/bin/env gcsl\n\n# A script.\nx = 1\nr = x\n",
            "r = |x",
            Some("A script."),
        ),
    ] {
        let (index, file) = index_of(&[("app.gcsl", text)], "app.gcsl");
        let found = index.documentation(file, offset(text, needle));
        assert_eq!(found.as_deref(), wanted, "{text}");
    }
    // A package's header, directly above its meta, describes the file,
    // not the definition: newpkgs writes its packages' imports between
    // (see "Documentation").
    let workspace = "#[def_type]\ndef package(name: String, f: fn(v: String): String META-> \
                         {versions: List<String>}, version: String) -> String { f(version) }\n";
    let files = [
        ("workspace.gcsl", workspace),
        (
            "app/a.gcsl",
            "# The x package.\ndefmeta package dev.x { versions = [] }\n\n\
                 def package dev.x(v) { v }\n",
        ),
        ("app/b.gcsl", "r = package dev.x\n"),
    ];
    let (index, file) = index_of(&files, "app/b.gcsl");
    assert_eq!(
        index.documentation(file, offset(files[2].1, "dev.|x")),
        None
    );
}

#[test]
fn a_file_that_needs_one_that_failed_is_indexed_and_reports_nothing() {
    let files = [
        ("a.gcsl", "x = 1\nbad = 1 + \"a\"\n"),
        (
            "b.gcsl",
            "y = x\nz = y + 1\nw = undefined_name\nfn f(n: Int): Int { n }\n",
        ),
        ("c.gcsl", "import v from lib/l\nu = v + 1\n"),
        ("lib/l.gcsl", "v = 1\nworse = 1 + \"a\"\n"),
    ];
    let Indexed {
        index, diagnostics, ..
    } = indexed(&files);
    let mut index = index.unwrap();
    let b = index.file("b.gcsl").unwrap();
    assert!(index.checked(b));
    // What `b` declares itself has its type; what `a` does, whose
    // declarations were never committed, is found but erroneous.
    let hover = index.hover(b, offset(files[1].1, "{ |n }")).unwrap();
    assert!(hover.text.contains("n: Int"), "{}", hover.text);
    assert_eq!(index.definition(b, offset(files[1].1, "y = |x")).len(), 1);
    // A file that imports from one that failed has the types its check
    // found.
    let c = index.file("c.gcsl").unwrap();
    let hover = index.hover(c, offset(files[2].1, "u = |v")).unwrap();
    assert!(hover.text.contains("v: Int"), "{}", hover.text);
    // No occurrence for the name that names nothing.
    assert!(index.at(b, offset(files[1].1, "|undefined_name")).is_none());
    // The diagnostics are the check's, nothing of `b` among them.
    let mut sources: Vec<SourceFile> = files
        .iter()
        .chain(&[("workspace.gcsl", "")])
        .map(|&(p, t)| SourceFile::new(p, t).unwrap())
        .collect();
    sources.sort_by(|a, b| a.name().cmp(b.name()));
    let (_, checked) = crate::workspace::check_with_standard(&sources, &[], "/");
    let Err(failed) = checked else { panic!() };
    assert_eq!(format!("{diagnostics:?}"), format!("{failed:?}"));
    assert_eq!(diagnostics.len(), 2);
}

#[test]
fn a_long_signature_breaks_its_parameters_at_80_columns() {
    let text = "def configure(first_parameter: String, second_parameter: String, third: Int): Int { third }\n\
                    r = configure(\"a\", \"b\", 1)\n";
    let files = [("app.gcsl", text)];
    let (mut index, file) = index_of(&files, "app.gcsl");
    let hover = index.hover(file, offset(text, "r = |configure")).unwrap();
    assert!(
        hover
            .text
            .starts_with("```gcsl\ndef configure(\n  first_parameter: String")
            && hover.text.contains(",\n  second_parameter: String")
            && hover.text.contains(",\n  third: Int,\n): Int\n```"),
        "{}",
        hover.text
    );
    // One that fits stays on its line.
    let text = "def short(a: Int): Int { a }\nr = short(1)\n";
    let (mut index, file) = index_of(&[("app.gcsl", text)], "app.gcsl");
    let hover = index.hover(file, offset(text, "r = |short")).unwrap();
    assert!(
        hover
            .text
            .starts_with("```gcsl\ndef short(a: Int): Int\n```"),
        "{}",
        hover.text
    );
}

#[test]
fn a_half_typed_access_or_call_keeps_what_it_has_checked() {
    // A field access with no name, at the end of a line and of the
    // file: its record is checked, and has its hover.
    for text in [
        "c = {port = 80}\nx = c.\ny = 1\n",
        "c = {port = 80}\nx = c.",
    ] {
        let (mut index, file) = index_of(&[("app.gcsl", text)], "app.gcsl");
        let hover = index.hover(file, offset(text, "x = |c")).unwrap();
        assert!(hover.text.contains("c: {port: Int}"), "{}", hover.text);
    }
    // An unclosed call's callee and arguments.
    let text = "fn f(n: Int): Int { n }\nlet v = 2\nx = f(v, ";
    let (mut index, file) = index_of(&[("app.gcsl", text)], "app.gcsl");
    let hover = index.hover(file, offset(text, "x = |f")).unwrap();
    assert!(hover.text.contains("fn f(n: Int): Int"), "{}", hover.text);
    let hover = index.hover(file, offset(text, "f(|v")).unwrap();
    assert!(hover.text.contains("v: Int"), "{}", hover.text);
}

#[test]
fn a_mutable_variable_shows_mut() {
    let text = "fn g(): Int {\n    let mut n = 1\n    n := n + 1\n    n\n}\nr = g()\n";
    let Indexed { index, .. } = indexed(&[("app.gcsl", text)]);
    let mut index = index.unwrap();
    let file = index.file("app.gcsl").unwrap();
    let hover = index.hover(file, offset(text, "    |n\n}")).unwrap();
    assert!(hover.text.contains("mut n: Int"), "{}", hover.text);
}

#[test]
fn a_scan_error_leaves_the_rest_of_the_workspace_indexed() {
    // clackatoo's minimal case: the conflict and `r`'s error are both
    // reported, and `p` answers a hover.
    let files = [
        ("workspace.gcsl", "# Root.\n"),
        ("p/p.gcsl", "h = 1\nk = h\n"),
        ("q/a.gcsl", "x = 1\n"),
        ("q/b.gcsl", "x = 2\n"),
        ("r/r.gcsl", "y: Int = \"s\"\n"),
    ];
    let Indexed {
        index,
        diagnostics,
        succeeded,
        ..
    } = indexed(&files);
    assert!(!succeeded);
    let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        ["field `x` is defined in two files", "mismatched types"]
    );
    let mut index = index.unwrap();
    let file = index.file("p/p.gcsl").unwrap();
    let hover = index.hover(file, offset(files[1].1, "k = |h")).unwrap();
    assert!(hover.text.contains("h: Int"), "{}", hover.text);
    assert_eq!(
        places(&files, &[("p/p.gcsl", "|h = 1")]),
        index
            .definition(file, offset(files[1].1, "k = |h"))
            .iter()
            .map(|l| (index.path(l.file).unwrap().to_owned(), l.span.start))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_read_of_a_conflicted_field_goes_to_every_definition() {
    // Through an import, from a file of the directory and from one
    // below it: each read goes to both definitions, and has no hover.
    let files = [
        ("q/a.gcsl", "x = 1\n"),
        ("q/b.gcsl", "x = 2\n"),
        ("q/c.gcsl", "w = x\n"),
        ("q/sub/d.gcsl", "v = x\n"),
        ("s/s.gcsl", "import x from q\nn = x + 1\n"),
    ];
    let (mut index, _) = index_of(&files, "s/s.gcsl");
    let both = places(&files, &[("q/a.gcsl", "|x = 1"), ("q/b.gcsl", "|x = 2")]);
    for (path, needle) in [
        ("s/s.gcsl", "n = |x"),
        ("q/c.gcsl", "w = |x"),
        ("q/sub/d.gcsl", "v = |x"),
        ("q/a.gcsl", "|x = 1"),
    ] {
        let file = index.file(path).unwrap();
        let at = offset(files.iter().find(|(p, _)| *p == path).unwrap().1, needle);
        let found: Vec<(String, u32)> = index
            .definition(file, at)
            .iter()
            .map(|l| (index.path(l.file).unwrap().to_owned(), l.span.start))
            .collect();
        assert_eq!(found, both, "{path}");
        if path != "q/a.gcsl" {
            assert_eq!(index.hover(file, at), None, "{path}");
        }
    }
}

#[test]
fn a_rename_that_would_make_a_scan_error_is_refused() {
    let files = [("a.gcsl", "x = 1\n"), ("b.gcsl", "y = 2\nz = y\n")];
    let refused = renamed_err(&files, ("b.gcsl", "z = |y"), "x");
    assert!(
        refused.contains("field `x` is defined in two files"),
        "{refused}"
    );
}
