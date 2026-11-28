//! Tests of workspaces: a library a host gives a file as its prelude (see
//! [Given preludes](crate::workspace#given-preludes)).

use super::fileroot::SCRATCH;
use super::graph::every;
use super::*;

/// A harness's library, `kit`: its own `test`, which shadows the
/// prelude's, a `step` that takes a block, and a constant.
const KIT: &str = "#[def_type]\n\
     def test(name: String, f: fn() -> {}) -> String { f(); = \"kit {name}\" }\n\
     def step(name: String, body: block -> {}) -> {} { body() }\n\
     limit = 3\n";

/// The workspace of `files`, each a path, a text and the library given it
/// as a prelude, if one is, beside the library `kit` ([`KIT`]), checked.
fn given(files: &[(&str, &str, Option<&str>)]) -> Result<Workspace, Vec<Diagnostic>> {
    let mut sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text, prelude)| {
            let source = SourceFile::new(path, text).unwrap();
            match prelude {
                Some(library) => source.with_prelude([format!("import * from \"{library}\"")]),
                None => source,
            }
        })
        .collect();
    sources.push(SourceFile::new("<lib>/kit/kit.gcsl", KIT).unwrap());
    check_with_standard(&sources, &[], SCRATCH).1
}

/// The messages of the errors of `files`, as [`given`] checks them.
fn messages(files: &[(&str, &str, Option<&str>)]) -> Vec<String> {
    match given(files) {
        Ok(_) => Vec::new(),
        Err(errors) => errors
            .iter()
            .filter(|error| error.severity == crate::diagnostic::Severity::Error)
            .map(|error| error.message.clone())
            .collect(),
    }
}

#[test]
fn a_file_given_a_prelude_sees_its_names_with_no_import() {
    let mut workspace = given(&[
        ("workspace.gcsl", "", None),
        (
            "t.prvm",
            "def test one() { let mut n = 0; step(\"s\") { n := limit } }\nx = test one()",
            Some("kit"),
        ),
    ])
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    let file = workspace.file("t.prvm").unwrap();
    let value = workspace.run(file).unwrap().value;
    assert_eq!(value.to_string(), "{x = \"kit one\"}");
}

#[test]
fn a_given_prelude_shadows_the_prelude_and_is_shadowed_by_the_file() {
    // The library's `test` is the one a `def test` names, not the
    // prelude's; a declaration of the file's own, and of a directory above,
    // shadows the library's silently.
    let mut workspace = given(&[
        ("workspace.gcsl", "", None),
        ("top.gcsl", "limit = 10", None),
        ("a/t.prvm", "y = limit\nlen = 2\nz = len", Some("kit")),
        (
            "b/t.prvm",
            "def test one() { }\nx = test one()",
            Some("kit"),
        ),
    ])
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    let a = workspace.file("a/t.prvm").unwrap();
    assert_eq!(
        workspace.run(a).unwrap().value.to_string(),
        "{y = 10; len = 2; z = 2}"
    );
    let b = workspace.file("b/t.prvm").unwrap();
    assert_eq!(
        workspace.run(b).unwrap().value.to_string(),
        "{x = \"kit one\"}"
    );
}

#[test]
fn a_file_not_given_the_prelude_does_not_see_it() {
    assert_eq!(
        messages(&[
            ("workspace.gcsl", "", None),
            ("t.prvm", "x = limit", Some("kit")),
            ("u.gcsl", "y = step", None),
        ]),
        ["undefined variable `step`"]
    );
}

#[test]
fn a_given_prelude_not_found_is_one_error_at_the_file_s_start() {
    let errors = given(&[
        ("workspace.gcsl", "", None),
        (
            "t.prvm",
            "def test one() { step(\"s\") { } }\nx = limit",
            Some("absent"),
        ),
    ])
    .unwrap_err();
    let errors: Vec<&Diagnostic> = errors
        .iter()
        .filter(|error| error.severity == crate::diagnostic::Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(
        errors[0].message,
        prelude_not_given_message("absent"),
        "{errors:?}"
    );
    assert_eq!(
        errors[0].message,
        "the prelude `absent` given for this file is not found"
    );
    assert_eq!(errors[0].span, Span::new(0, 0));
}

#[test]
fn no_prelude_turns_a_given_prelude_off() {
    assert_eq!(
        messages(&[
            ("workspace.gcsl", "", None),
            ("t.prvm", "#![no_prelude]\nx = limit", Some("kit")),
        ]),
        ["undefined variable `limit`"]
    );
    let source = SourceFile::new("t.prvm", "#![no_prelude]\nx = 1")
        .unwrap()
        .with_prelude(["import * from \"kit\""]);
    assert!(library_imports(&source).is_empty());
}

#[test]
fn the_given_prelude_is_the_first_library_a_file_imports() {
    let source = SourceFile::new("t.prvm", "import \"other\"\nx = 1")
        .unwrap()
        .with_prelude(["import * from \"kit\""]);
    assert_eq!(library_imports(&source), ["kit", "other"]);
}

#[test]
fn a_host_invokes_the_given_prelude_s_def_type_by_its_path() {
    // The library's `test` replaces the prelude's of its path, so a host
    // asking for `test` gets it, and lists it alone.
    let mut workspace = given(&[
        ("workspace.gcsl", "", None),
        (
            "t.prvm",
            "def test one() { }\ndef test two() { }",
            Some("kit"),
        ),
    ])
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    let listed = workspace.def_types().unwrap();
    let tests: Vec<&DefType> = listed.iter().filter(|found| found.path == "test").collect();
    assert_eq!(tests.len(), 1, "{listed:?}");
    let names: Vec<&str> = tests[0]
        .definitions
        .iter()
        .map(|definition| definition.path.as_str())
        .collect();
    assert_eq!(names, ["one", "two"]);
    let call = workspace.prepare_call(("test", "two"), &[], &[]).unwrap();
    let mut graph = crate::eval::graph::Graph::new();
    assert_eq!(
        workspace.run_prepared(&call, &mut graph, every()).unwrap(),
        Progress::Done(Value::Str(std::rc::Rc::from("kit two")))
    );
}

#[test]
fn files_of_any_extension_import_each_other_as_modules() {
    // A module is its file's path without its extension, so a `.gcsl` file
    // and a host's file import each other as `.gcsl` files do.
    let mut workspace = given(&[
        ("workspace.gcsl", "", None),
        ("tools/vm.gcsl", "def boot(): String { \"vm\" }", None),
        (
            "tests/boot.prvm",
            "import boot from tools/vm\ndef machine(): String { boot() }",
            Some("kit"),
        ),
        (
            "main.gcsl",
            "import machine from tests/boot\nx = machine()",
            None,
        ),
    ])
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    let main = workspace.file("main.gcsl").unwrap();
    assert_eq!(
        workspace.run(main).unwrap().value.to_string(),
        "{x = \"vm\"}"
    );
}

#[test]
fn two_files_of_one_module_are_an_error() {
    assert_eq!(
        messages(&[
            ("workspace.gcsl", "", None),
            ("a.gcsl", "x = 1", None),
            ("a.prvm", "y = 2", Some("kit")),
        ]),
        ["`a.gcsl` and `a.prvm` are both the module `a`"]
    );
}

#[test]
fn module_paths_drop_the_last_extension_of_the_name_alone() {
    for (path, module) in [
        ("a/b.gcsl", "a/b"),
        ("a/b.prvm", "a/b"),
        ("<lib>/github.com/x/y.gcsl", "<lib>/github.com/x/y"),
        ("v1.2/c", "v1.2/c"),
        ("d.e.gcsl", "d.e"),
        (".hidden", ".hidden"),
    ] {
        assert_eq!(module_path(path), module, "{path}");
    }
}

#[test]
fn one_workspace_mixes_files_given_preludes_and_files_not() {
    // A build tool's file, given no prelude, and a harness's, given its
    // library, side by side, each with a `def test`: each names the `test`
    // its own view sees. The library's replaces the standard one by path,
    // so a host that lists `test` lists the harness's alone; the build
    // tool's definition, made with the standard `test`, is listed by no
    // `test` here, and a host that reads it reads no harness file.
    let workspace = given(&[
        ("workspace.gcsl", "", None),
        ("app/build.pekit", "def test t() { }", None),
        ("tests/boot.prvm", "def test u() { }", Some("kit")),
    ])
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    let listed = workspace.def_types().unwrap();
    let tests: Vec<&DefType> = listed.iter().filter(|found| found.path == "test").collect();
    assert_eq!(tests.len(), 1, "{listed:?}");
    let names: Vec<&str> = tests[0]
        .definitions
        .iter()
        .map(|definition| definition.path.as_str())
        .collect();
    assert_eq!(names, ["u"]);
    assert_eq!(tests[0].file, workspace.file("<lib>/kit/kit.gcsl").unwrap());
    // Without the harness's files, as a build tool reads the workspace, the
    // standard `test` has the build tool's definition.
    let sources = [
        SourceFile::new("workspace.gcsl", "").unwrap(),
        SourceFile::new("app/build.pekit", "def test t() { }").unwrap(),
    ];
    let alone = check_with_standard(&sources, &[], SCRATCH).1.unwrap();
    let listed = alone.def_types().unwrap();
    let tests: Vec<&DefType> = listed.iter().filter(|found| found.path == "test").collect();
    assert_eq!(tests.len(), 1, "{listed:?}");
    assert_eq!(tests[0].definitions[0].path, "t");
}

/// The library `kit/vm`, beside `kit` ([`KIT`]).
const VM: &str = "def boot(): String { \"vm\" }\nlimit = 9\n";

/// The workspace of `files`, each a path, a text and the imports of the
/// prelude given it, beside the libraries `kit` and `kit/vm`, checked.
fn given_imports(files: &[(&str, &str, &[&str])]) -> Result<Workspace, Vec<Diagnostic>> {
    let mut sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text, imports)| {
            SourceFile::new(path, text)
                .unwrap()
                .with_prelude(imports.iter().copied())
        })
        .collect();
    sources.push(SourceFile::new("<lib>/kit/kit.gcsl", KIT).unwrap());
    sources.push(SourceFile::new("<lib>/kit/vm/vm.gcsl", VM).unwrap());
    check_with_standard(&sources, &[], SCRATCH).1
}

/// The value of the file `path` of `files`, as [`given_imports`] checks
/// them, run.
fn given_value(files: &[(&str, &str, &[&str])], path: &str) -> String {
    let mut workspace = given_imports(files).unwrap_or_else(|errors| panic!("{errors:?}"));
    let file = workspace.file(path).unwrap();
    workspace.run(file).unwrap().value.to_string()
}

#[test]
fn a_prelude_is_imports_in_order_of_each_form() {
    let kit_and_vm: &[&str] = &["import * from \"kit\"", "import \"kit/vm\""];
    assert_eq!(
        given_value(
            &[
                ("workspace.gcsl", "", &[]),
                ("t.prvm", "x = vm.boot()\ny = limit", kit_and_vm),
            ],
            "t.prvm"
        ),
        "{x = \"vm\"; y = 3}"
    );
    // Whole as another name, and by members.
    assert_eq!(
        given_value(
            &[
                ("workspace.gcsl", "", &[]),
                (
                    "t.prvm",
                    "x = machine.boot()\ny = most",
                    &[
                        "import \"kit/vm\" as machine",
                        "import limit as most from \"kit\""
                    ],
                ),
            ],
            "t.prvm"
        ),
        "{x = \"vm\"; y = 3}"
    );
    // A later import shadows an earlier one's name.
    assert_eq!(
        given_value(
            &[
                ("workspace.gcsl", "", &[]),
                (
                    "t.prvm",
                    "y = limit",
                    &["import * from \"kit\"", "import * from \"kit/vm\""],
                ),
            ],
            "t.prvm"
        ),
        "{y = 9}"
    );
}

#[test]
fn an_import_a_prelude_does_not_take_is_an_error_and_the_others_stand() {
    let errors = given_imports(&[
        ("workspace.gcsl", "", &[]),
        ("lib/x.gcsl", "w = 1", &[]),
        (
            "t.prvm",
            "y = limit",
            &[
                "import lib/x",
                "import * from \"kit\"",
                "import * from \"absent\"",
            ],
        ),
    ])
    .unwrap_err();
    let messages: Vec<&str> = errors
        .iter()
        .filter(|error| error.severity == crate::diagnostic::Severity::Error)
        .map(|error| error.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "the prelude given for this file imports only libraries, whole, `*` or by name: \
             `import lib/x`",
            "the prelude `absent` given for this file is not found",
        ]
    );
    let source = SourceFile::new("t.prvm", "x = 1").unwrap().with_prelude([
        "import \"a\"",
        "import lib/x",
        "import b from \"c/d\"",
    ]);
    assert_eq!(library_imports(&source), ["a", "c/d"]);
}
