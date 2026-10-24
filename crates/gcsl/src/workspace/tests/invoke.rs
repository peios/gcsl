//! Tests of workspaces: invoking definitions by name, listing them, and request
//! errors.

use super::*;

/// Jack's `/home/jack/projects/peios/newpkgs` as it is written, but for
/// `&` written as a spread and the commas of the package's block as
/// newlines, as in [`JACKS_PACKAGE`], with no line invoking the package:
/// `ppk.commit` stubbed by a module of the test's own, which returns its
/// arguments, the content joined to its caller's directory as the real
/// one joins it.
pub(super) const NEWPKGS: [(&str, &str); 3] = [
    JACKS_PACKAGE[0],
    (
        "_lib/ppk.gcsl",
        "import path\n\
             import fileroot\n\
             def commit(name: String, version: String, arch: String, content: String) -> String {\n\
             \x20   let content = path.join(fileroot.caller, content)\n\
             \x20   \"{name} {version} {arch} {content}\"\n\
             }",
    ),
    (
        "dev.peios.testpkg/test.gcsl",
        "def package dev.peios.testpkg() {\n\
             \x20   version = \"0.1.0\"\n\
             \x20   arch = \"x86_64\"\n\
             \x20   content = \"usr\"\n\
             }\n",
    ),
];

/// Checks the workspace of `files`, located at [`SCRATCH`], and invokes
/// the definition `path` of the def-type `def_type` in it, with the
/// arguments `args`, each a parameter's name and the text of its value,
/// read by [`Workspace::arguments`].
pub(super) fn invoke_with(
    files: &[(&str, &str)],
    (def_type, path): (&str, &str),
    args: &[(&str, &str)],
) -> Result<Value, InvokeError> {
    let mut workspace = rooted(files).unwrap_or_else(|errors| panic!("{errors:?}"));
    let texts: Vec<(&str, Option<&str>)> = args
        .iter()
        .map(|&(name, text)| (name, Some(text)))
        .collect();
    let args = workspace.arguments(def_type, path, &texts)?;
    workspace
        .invoke(def_type, path, &args)
        .map(|outcome| outcome.value)
}

/// The argument named `name` of the string `text`, as
/// [`Workspace::invoke`] takes it.
pub(super) fn string(name: &str, text: &str) -> (String, Given) {
    (name.to_owned(), Given::Str(text.to_owned()))
}

/// [`invoke_with`]'s value with no arguments shown, [`SCRATCH`] shown as
/// `<root>`.
pub(super) fn invoked_text(files: &[(&str, &str)], def_type: &str, path: &str) -> String {
    invoked_text_with(files, (def_type, path), &[])
}

/// [`invoke_with`]'s value shown, [`SCRATCH`] shown as `<root>`.
pub(super) fn invoked_text_with(
    files: &[(&str, &str)],
    (def_type, path): (&str, &str),
    args: &[(&str, &str)],
) -> String {
    match invoke_with(files, (def_type, path), args) {
        Ok(value) => value.to_string().replace(SCRATCH, "<root>"),
        Err(error) => panic!("{def_type} {path} {args:?}: {error:?}"),
    }
}

/// The request error of invoking `path` of `def_type` in the workspace
/// of `files`, with no arguments: its message and its notes, joined by
/// `; `.
pub(super) fn request_error(files: &[(&str, &str)], def_type: &str, path: &str) -> String {
    request_error_with(files, (def_type, path), &[])
}

/// [`request_error`] with the arguments `args`, each a parameter's name
/// and the text of its value.
pub(super) fn request_error_with(
    files: &[(&str, &str)],
    (def_type, path): (&str, &str),
    args: &[(&str, &str)],
) -> String {
    match invoke_with(files, (def_type, path), args) {
        Err(InvokeError::Request(error)) => {
            let mut parts = vec![error.message()];
            parts.extend(
                error
                    .notes()
                    .into_iter()
                    .map(|note| format!("note: {note}")),
            );
            assert_eq!(error.to_string(), error.message());
            parts.join("; ")
        }
        other => panic!("{def_type} {path}: {other:?}"),
    }
}

/// The diagnostics invoking `path` of `def_type` in the workspace of
/// `files` fails with, each shown as [`check_errors`] shows them.
pub(super) fn invoke_errors(files: &[(&str, &str)], def_type: &str, path: &str) -> Vec<String> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], SCRATCH);
    let errors = match checked.unwrap().invoke(def_type, path, &[]) {
        Err(InvokeError::Diagnostics(errors)) => errors,
        other => panic!("{def_type} {path}: {other:?}"),
    };
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    all.extend(paths);
    shown_errors(&errors, &all)
}

/// `errors`, each shown with its labels and notes, joined by `; `, its
/// input and each label's by its path in `all`, by input number.
pub(super) fn shown_errors(errors: &[Diagnostic], all: &[&str]) -> Vec<String> {
    let path = |input: Option<u32>| {
        input
            .and_then(|i| all.get(i as usize))
            .copied()
            .unwrap_or("?")
    };
    errors
        .iter()
        .map(|error| {
            let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
            if let Some(label) = &error.label {
                parts[0] = format!("{} ({label})", parts[0]);
            }
            for label in &error.labels {
                let at = path(label.input.or(error.input));
                parts.push(format!("{at}: {}", label.text));
            }
            parts.extend(error.notes.iter().map(|note| format!("note: {note}")));
            parts.join("; ")
        })
        .collect()
}

#[test]
fn jacks_package_is_invoked_by_name() {
    // The record the def-type returns; the stub given the package's
    // directory as the content's base, through `fileroot.helper()`.
    assert_eq!(
        invoked_text(&NEWPKGS, "package", "dev.peios.testpkg"),
        "{pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
             <root>/dev.peios.testpkg/usr\"}"
    );
    // Listed: the def-type, its file and the span of its name, and the
    // definition, its file and the span of its path.
    let workspace = rooted(&NEWPKGS).unwrap();
    let listed = workspace.def_types().unwrap();
    let root = workspace.file("workspace.gcsl").unwrap();
    let package = workspace.file("dev.peios.testpkg/test.gcsl").unwrap();
    let name = count(NEWPKGS[0].1.find("def package(").unwrap() + 4);
    assert_eq!(
        listed,
        [DefType {
            path: "package".to_owned(),
            file: root,
            span: Span::new(name, name + 7),
            parameters: Vec::new(),
            optional: Vec::new(),
            definitions: vec![Definition {
                path: "dev.peios.testpkg".to_owned(),
                dir: "dev.peios.testpkg".to_owned(),
                file: package,
                span: Span::new(12, 29),
            }],
        }]
    );
}

/// The paths of the def-types `workspace` lists, each with the paths
/// and files of its definitions: `package: a.one in a/one.gcsl`.
pub(super) fn listed(workspace: &Workspace, files: &[(&str, &str)]) -> Vec<String> {
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    let standard = workspace.scan.standard as usize;
    let path = |file: u32| paths[file as usize - standard];
    workspace
        .def_types()
        .unwrap()
        .iter()
        .map(|def_type| {
            let definitions: Vec<String> = def_type
                .definitions
                .iter()
                .map(|definition| format!("{} in {}", definition.path, path(definition.file)))
                .collect();
            format!(
                "{} in {}: {}",
                def_type.path,
                path(def_type.file),
                definitions.join(", ")
            )
        })
        .collect()
}

#[test]
fn every_definition_is_listed_under_the_def_type_it_names() {
    // Definitions in two sibling directories, which the root sees
    // neither of; one through an alias of its def-type, listed by its
    // path as written; a def-type with none; two def-types of the root
    // with definitions; and a dotted def-type.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def build(name: String, f: fn(): String): String { \"built {f()}\" }\n\
                 #[def_type]\n\
                 def test(name: String, f: fn(): String): String { \"tested {f()}\" }",
        ),
        (
            "lib/types.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String) -> String { \"{name}: {f()}\" }\n\
                 #[def_type]\n\
                 def unused(name: String, f: fn(): Int): Int { f() }\n\
                 #[def_type]\n\
                 def kinds.tool(name: String, f: fn(): String): String { f() }",
        ),
        (
            "a/one.gcsl",
            "import * from lib/types\n\
                 def package a.one() { \"one\" }\n\
                 def build a.one() { \"a\" }\n\
                 def kinds.tool hammer() { \"hit\" }",
        ),
        (
            "b/two.gcsl",
            "import package as pkg from lib/types\n\
                 def pkg b.two() { \"two\" }\n\
                 def test b.two() { \"b\" }",
        ),
    ];
    let workspace = rooted(&files).unwrap();
    assert_eq!(
        listed(&workspace, &files),
        [
            "package in lib/types.gcsl: a.one in a/one.gcsl, b.two in b/two.gcsl",
            "unused in lib/types.gcsl: ",
            "kinds.tool in lib/types.gcsl: hammer in a/one.gcsl",
            "build in workspace.gcsl: a.one in a/one.gcsl",
            "test in workspace.gcsl: b.two in b/two.gcsl",
        ]
    );
    // The dotted def-type's name is its whole path.
    let tool = &workspace.def_types().unwrap()[2];
    let text = files[1].1;
    let at = count(text.find("kinds.tool").unwrap());
    assert_eq!(tool.span, Span::new(at, at + 10));
    // Each is invoked by its def-type's path and its own.
    assert_eq!(invoked_text(&files, "package", "a.one"), "a.one: one");
    assert_eq!(invoked_text(&files, "package", "b.two"), "b.two: two");
    assert_eq!(invoked_text(&files, "build", "a.one"), "built a");
    assert_eq!(invoked_text(&files, "test", "b.two"), "tested b");
    assert_eq!(invoked_text(&files, "kinds.tool", "hammer"), "hit");
}

/// The paths of the files of the workspace of `files`, by input number,
/// the standard files `standard` first.
pub(super) fn all_paths<'a>(standard: &'a [SourceFile], files: &[(&'a str, &str)]) -> Vec<&'a str> {
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    all.extend(paths);
    all
}

/// The errors [`Workspace::def_types`] fails with in the workspace of
/// `files`, each shown as [`shown_errors`] shows it.
pub(super) fn listing_errors(files: &[(&str, &str)]) -> Vec<String> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], SCRATCH);
    let errors = checked.unwrap().def_types().unwrap_err();
    shown_errors(&errors, &all_paths(&standard, files))
}

/// A def-type of the root, for the tests of duplicates.
pub(super) const PACKAGE: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\ndef package(name: String, f: fn(): String): String { \"{name} {f()}\" }",
);

/// The note on a duplicate definition.
pub(super) const DEFINED: &str = "note: a definition invoked by name must be the only one of its path in \
                       its directory, the directories above it and those below it";

/// The note on a definition under another.
pub(super) const UNDER: &str = "note: a definition invoked by name must not lie under another in its \
                     directory, the directories above it or those below it";

#[test]
fn one_path_in_directories_apart_is_named_with_its_directory() {
    // In two sibling directories, which see neither the other: no error.
    // Another definition, unique, is invoked by its path alone.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package dup() { \"a\" }"),
        ("b/y.gcsl", "def package dup() { \"b\" }"),
        (
            "c/z.gcsl",
            "def package other() { \"c\" }\nz = package other()",
        ),
    ];
    assert_eq!(invoked_text(&siblings, "package", "other"), "other c");
    // The shared path alone names neither; each is named with its
    // directory.
    assert_eq!(
        request_error(&siblings, "package", "dup"),
        "`package dup` is defined in 2 directories, so it is named with its directory; \
         note: name one of `a/dup`, `b/dup`"
    );
    assert_eq!(invoked_text(&siblings, "package", "a/dup"), "dup a");
    assert_eq!(invoked_text(&siblings, "package", "b/dup"), "dup b");
    // A directory without the definition names none, and the listing
    // names the shared one qualified.
    assert_eq!(
        request_error(&siblings, "package", "c/dup"),
        "`package` has no definition `c/dup`; note: its definitions are `a/dup`, `b/dup`, \
         `other`"
    );
    // The listing gives each its directory.
    let workspace = checked(&siblings);
    let listed = workspace.def_types().unwrap();
    let dirs: Vec<(&str, &str, String)> = listed[0]
        .definitions
        .iter()
        .map(|d| (d.path.as_str(), d.dir.as_str(), d.qualified()))
        .collect();
    assert_eq!(
        dirs,
        [
            ("dup", "a", "a/dup".to_owned()),
            ("dup", "b", "b/dup".to_owned()),
            ("other", "c", "c/other".to_owned()),
        ]
    );
    // One of the root is qualified as `/PATH`.
    let rooted = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 def package top() { \"root\" }",
        ),
        ("a/x.gcsl", "def package top2() { \"a\" }"),
    ];
    assert_eq!(invoked_text(&rooted, "package", "/top"), "top root");
    assert_eq!(invoked_text(&rooted, "package", "top"), "top root");
}

#[test]
fn one_path_in_directories_that_see_each_other_fails_every_invocation() {
    // A `#[shadowable]` one of the root, shadowed in a directory below:
    // both are found, the first in path order first.
    let shadowed = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package dup() { \"root\" }",
        ),
        (
            "sub/s.gcsl",
            "def package dup() { \"sub\" }\ny = package dup()",
        ),
    ];
    let twice = format!(
        "`package dup` is defined 2 times in directories that see each other at sub/s.gcsl; \
             workspace.gcsl: also defined here; {DEFINED}"
    );
    assert_eq!(invoke_errors(&shadowed, "package", "dup"), [twice.as_str()]);
    assert_eq!(listing_errors(&shadowed), [twice.as_str()]);
    // An ordinary run allows them, and runs.
    assert_eq!(rooted_text(&shadowed, "sub/s.gcsl"), "{y = \"dup sub\"}");
    // And one shadowed in its own file, which is in no tree.
    let own = [
        PACKAGE,
        (
            "a.gcsl",
            "#[shadowable] def package dup() { \"one\" }\n\
                 def package dup() { \"two\" }\n\
                 y = package dup()",
        ),
    ];
    assert_eq!(
        invoke_errors(&own, "package", "dup"),
        [format!(
            "`package dup` is defined 2 times in directories that see each other at a.gcsl; \
                 a.gcsl: also defined here; {DEFINED}"
        )]
    );
    assert_eq!(rooted_text(&own, "a.gcsl"), "{y = \"dup two\"}");
    // Two chains of one path: the root's with one below it, and a pair
    // apart from it in siblings, each chain one error.
    let chains = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[shadowable] def package dup() { \"root\" }",
        ),
        ("a/x.gcsl", "def package dup() { \"a\" }"),
        ("b/y.gcsl", "def package dup() { \"b\" }"),
    ];
    assert_eq!(
        listing_errors(&chains),
        [format!(
            "`package dup` is defined 3 times in directories that see each other at \
                 a/x.gcsl; b/y.gcsl: also defined here; workspace.gcsl: also defined here; \
                 {DEFINED}"
        )]
    );
}

#[test]
fn two_def_types_of_one_path_fail_every_invocation() {
    let declared = "note: a def-type invoked by name must be the only one of its name \
                        in the workspace";
    let def_type = "#[def_type]\n\
                        def package(name: String, f: fn(): String): String { f() }";
    // In two sibling directories.
    let siblings = [
        ("workspace.gcsl", ""),
        ("a/x.gcsl", def_type),
        ("b/y.gcsl", def_type),
        (
            "c/z.gcsl",
            "#[def_type]\ndef other(name: String, f: fn(): Int): Int { f() }",
        ),
        ("c/w.gcsl", "def other one() { 1 }\nw = other one()"),
    ];
    let twice = format!(
        "def-type `package` is declared 2 times in the workspace at a/x.gcsl; \
             b/y.gcsl: also declared here; {declared}"
    );
    assert_eq!(invoke_errors(&siblings, "other", "one"), [twice.as_str()]);
    assert_eq!(listing_errors(&siblings), [twice.as_str()]);
    assert_eq!(rooted_text(&siblings, "c/w.gcsl"), "{w = 1}");
    // A `#[shadowable]` one and the one that shadows it.
    let shadowed = [(
        "workspace.gcsl",
        "#[shadowable]\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { \"first\" }\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { f() }\n\
             def package x() { \"x\" }\n\
             y = package x()",
    )];
    assert_eq!(
        invoke_errors(&shadowed, "package", "x"),
        [format!(
            "def-type `package` is declared 2 times in the workspace at workspace.gcsl; \
                 workspace.gcsl: also declared here; {declared}"
        )]
    );
    assert_eq!(
        rooted_text(&shadowed, "workspace.gcsl"),
        "{package = fn(name: String | Future<String>, f: fn(): String): String; y = \"x\"}"
    );
}

#[test]
fn a_definition_under_another_fails_every_invocation() {
    // In two sibling directories, which see neither the other: no error.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package app() { \"a\" }"),
        (
            "b/y.gcsl",
            "def package app.core() { \"b\" }\ny = package app.core()",
        ),
        ("c/z.gcsl", "def package apple() { \"c\" }"),
    ];
    assert!(checked(&siblings).def_types().is_ok());
    assert_eq!(invoked_text(&siblings, "package", "app.core"), "app.core b");
    // Under one in a directory above it, which it shadows: invoking
    // another definition, `apple`, which lies under nothing, fails, and so
    // does the listing.
    let below = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package app() { \"root\" }",
        ),
        (
            "b/y.gcsl",
            "def package app.core() { \"b\" }\ny = package app.core()",
        ),
        ("c/z.gcsl", "def package apple() { \"c\" }"),
    ];
    let under = format!(
        "`package app.core` lies under another `package` definition at b/y.gcsl; \
             workspace.gcsl: `app` is defined here; {UNDER}"
    );
    assert_eq!(invoke_errors(&below, "package", "apple"), [under.as_str()]);
    assert_eq!(listing_errors(&below), [under.as_str()]);
    // An ordinary run allows them, and runs.
    assert_eq!(rooted_text(&below, "b/y.gcsl"), "{y = \"app.core b\"}");
    // In one directory, through `#[shadowable]`.
    let shadowed = [
        PACKAGE,
        (
            "a.gcsl",
            "#[shadowable] def package app.core() { \"core\" }\n\
                 def package app() { \"app\" }\n\
                 y = package app()",
        ),
    ];
    assert_eq!(
        listing_errors(&shadowed),
        [format!(
            "`package app.core` lies under another `package` definition at a.gcsl; \
                 a.gcsl: `app` is defined here; {UNDER}"
        )]
    );
    assert_eq!(rooted_text(&shadowed, "a.gcsl"), "{y = \"app app\"}");
    // Three levels down one chain of directories: `a.b` lies under one,
    // `a.b.c` under two, labelled in path then source order of their files.
    let levels = [
        PACKAGE,
        ("x/y/one.gcsl", "def package a.b.c() { \"c\" }"),
        ("x/two.gcsl", "#[shadowable] def package a.b() { \"b\" }"),
        ("three.gcsl", "#[shadowable] def package a() { \"a\" }"),
    ];
    assert_eq!(
        listing_errors(&levels),
        [
            format!(
                "`package a.b` lies under another `package` definition at x/two.gcsl; \
                     three.gcsl: `a` is defined here; {UNDER}"
            ),
            format!(
                "`package a.b.c` lies under another `package` definition at x/y/one.gcsl; \
                     three.gcsl: `a` is defined here; x/two.gcsl: `a.b` is defined here; \
                     {UNDER}"
            ),
        ]
    );
    // Definitions of other def-types do not overlap: `build a.b.c` lies
    // under no `package` definition.
    let other = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def build(name: String, f: fn(): String): String { f() }",
        ),
        ("x/y/one.gcsl", "def build a.b.c() { \"c\" }"),
        levels[2],
        levels[3],
    ];
    assert_eq!(
        listing_errors(&other),
        [format!(
            "`package a.b` lies under another `package` definition at x/two.gcsl; \
                 three.gcsl: `a` is defined here; {UNDER}"
        )]
    );
}

#[test]
fn a_definition_both_duplicated_and_under_another_has_both_errors() {
    // `app` twice in one directory chain, and `app.core` twice under it in
    // directories below them: each `app.core` lies under both `app`s.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[shadowable] def package app() { \"root\" }",
        ),
        ("a/x.gcsl", "#[shadowable] def package app() { \"a\" }"),
        ("a/b/y.gcsl", "def package app.core() { \"b\" }"),
        ("a/c/z.gcsl", "def package app.core() { \"c\" }"),
    ];
    let under = |at: &str| {
        format!(
            "`package app.core` lies under another `package` definition at {at}; \
                 a/x.gcsl: `app` is defined here; workspace.gcsl: `app` is defined here; \
                 {UNDER}"
        )
    };
    // In path order of their files: `a/b/y.gcsl` first.
    let expected = [
        under("a/b/y.gcsl"),
        under("a/c/z.gcsl"),
        format!(
            "`package app` is defined 2 times in directories that see each other at \
                 a/x.gcsl; workspace.gcsl: also defined here; {DEFINED}"
        ),
    ];
    assert_eq!(listing_errors(&files), expected);
    assert_eq!(invoke_errors(&files, "package", "app"), expected);
}

#[test]
fn what_cannot_be_invoked_is_a_request_error() {
    let files = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }"),
        (
            "lib/defs.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def empty(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def deploy(name: String, f: fn(): String, target: String, mode: String): String { f() }\n\
                 def package dev.peios.b() { \"b\" }\n\
                 def package dev.peios.a() { \"a\" }\n\
                 def deploy site() { \"s\" }",
        ),
    ];
    assert_eq!(
        request_error(&files, "pkg", "x"),
        "no def-type `pkg`; note: the def-types are `build`, `deploy`, `empty`, `package`"
    );
    assert_eq!(
        request_error(&[("workspace.gcsl", "x = 1")], "pkg", "x"),
        "no def-type `pkg`; note: this workspace declares no def-types"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios.nope"),
        "`package` has no definition `dev.peios.nope`; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    assert_eq!(
        request_error(&files, "empty", "x"),
        "`empty` has no definition `x`; note: it has no definitions"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios"),
        "`dev.peios` is not a `package` definition; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    assert_eq!(
        request_error(&files, "package", "dev"),
        "`dev` is not a `package` definition; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    // A prefix of a name is no path above it.
    assert_eq!(
        request_error(&files, "package", "dev.pei"),
        "`package` has no definition `dev.pei`; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    assert_eq!(
        request_error(&files, "build", "x"),
        "`build` needs `mode`, which was not given"
    );
    assert_eq!(
        request_error(&files, "deploy", "site"),
        "`deploy` needs `target` and `mode`, which were not given"
    );
    // Before parameters, the path.
    assert_eq!(
        request_error(&files, "build", "y"),
        "`build` has no definition `y`; note: its only definition is `x`"
    );
    // A path that is a definition's, and above another's, in a sibling
    // directory, which sees nothing of it: each is invoked.
    let under = "`package app.core` lies under another `package` definition";
    let note = UNDER;
    let files = [
        PACKAGE,
        ("a/x.gcsl", "def package app() { \"a\" }"),
        ("b/y.gcsl", "def package app.core() { \"b\" }"),
    ];
    assert_eq!(invoked_text(&files, "package", "app"), "app a");
    assert_eq!(invoked_text(&files, "package", "app.core"), "app.core b");
    // A `#[shadowable]` definition that one of another path, above it,
    // shadows in its own directory, which is in no tree: found, and the
    // one below.
    let files = [
        PACKAGE,
        (
            "a.gcsl",
            "#[shadowable] def package app.core() { \"core\" }\n\
                 def package app() { \"app\" }",
        ),
    ];
    let shadowed = format!("{under} at a.gcsl; a.gcsl: `app` is defined here; {note}");
    assert_eq!(
        invoke_errors(&files, "package", "app.core"),
        [shadowed.as_str()]
    );
    assert_eq!(invoke_errors(&files, "package", "app"), [shadowed.as_str()]);
}

#[test]
fn a_request_error_names_the_only_one_there_is() {
    let files = [
        PACKAGE,
        ("lib/defs.gcsl", "def package dev.peios.a() { \"a\" }"),
    ];
    assert_eq!(
        request_error(&files, "pkg", "x"),
        "no def-type `pkg`; note: the only def-type is `package`"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios.nope"),
        "`package` has no definition `dev.peios.nope`; \
             note: its only definition is `dev.peios.a`"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios"),
        "`dev.peios` is not a `package` definition; \
             note: its only definition is `dev.peios.a`"
    );
    // Two or more are listed, as `what_cannot_be_invoked_is_a_request_error`
    // shows with more.
    let files = [
        BUILD,
        ("lib/defs.gcsl", PACKAGE.1),
        (
            "lib/more.gcsl",
            "def package a() { \"a\" }\ndef package b() { \"b\" }",
        ),
    ];
    assert_eq!(
        request_error(&files, "pkg", "x"),
        "no def-type `pkg`; note: the def-types are `build`, `package`"
    );
    assert_eq!(
        request_error(&files, "package", "c"),
        "`package` has no definition `c`; note: its definitions are `a`, `b`"
    );
}

#[test]
fn the_prelude_s_build_is_listed_once_the_workspace_builds_with_it() {
    let listed = |files: &[(&str, &str)]| -> Vec<(String, Vec<String>)> {
        rooted(files)
            .unwrap()
            .def_types()
            .unwrap()
            .into_iter()
            .map(|def| {
                let paths = def.definitions.into_iter().map(|d| d.path).collect();
                (def.path, paths)
            })
            .collect()
    };
    // Not until a definition is made with it.
    assert!(listed(&[("workspace.gcsl", ""), ("app.gcsl", "x = 1")]).is_empty());
    assert_eq!(
        listed(&[
            ("workspace.gcsl", ""),
            ("app/build.gcsl", "def build hello() { 1 }")
        ]),
        [("build".to_owned(), vec!["hello".to_owned()])]
    );
    // A workspace's own `build` is the only one.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type] def build(name: String, f: fn(): Int): Int { f() }",
        ),
        ("app/build.gcsl", "def build hello() { 1 }"),
    ];
    let def_types = rooted(&files).unwrap().def_types().unwrap();
    assert_eq!(def_types.len(), 1);
    assert_eq!(def_types[0].definitions[0].path, "hello");
    assert!(def_types[0].file >= 1);
}
