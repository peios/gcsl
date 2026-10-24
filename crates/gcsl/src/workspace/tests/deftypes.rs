//! Tests of workspaces: def-types and their definitions.

use super::*;

/// Jack's `/home/jack/projects/peios/newpkgs` as it is written, with
/// `&` written as a spread and the commas of the package's block as
/// newlines, the bottom line `result = package dev.peios.testpkg()`,
/// and `ppk.commit` stubbed by a module of the test's own, which joins
/// the content to its caller's directory as the real one does.
pub(super) const JACKS_PACKAGE: [(&str, &str); 3] = [
    (
        "workspace.gcsl",
        "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
             import _lib/ppk\n\
             import fileroot\n\
             \n\
             type PackageDef = {\n\
             \x20 version: String,\n\
             \x20 arch: String,\n\
             \x20 content: String,\n\
             }\n\
             \n\
             type Package = { ...PackageDef, name: String }\n\
             \n\
             \n\
             #[def_type]\n\
             def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
             \x20   fileroot.helper()\n\
             \x20   pkg: Package = { ...func(); name = name }\n\
             \n\
             \x20   manifest = ppk.commit(\n\
             \x20       name = pkg.name,\n\
             \x20       version = pkg.version,\n\
             \x20       arch = pkg.arch,\n\
             \x20       content = pkg.content,\n\
             \x20   )\n\
             }\n",
    ),
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
             }\n\
             \n\
             \n\
             result = package dev.peios.testpkg()",
    ),
];

#[test]
fn jacks_package_is_defined_and_called_with_its_def_type() {
    // The stub is given the package's directory as the content's base,
    // through the def-type's `fileroot.helper()`; the definition binds
    // nothing of the file's record.
    assert_eq!(
        rooted_text(&JACKS_PACKAGE, "dev.peios.testpkg/test.gcsl"),
        "{result = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
             <root>/dev.peios.testpkg/usr\"}}"
    );
}

/// A def-type of the root of a workspace, for the def-type tests:
/// `build`, whose definitions take a target and give a string, and
/// which takes a mode when it is invoked.
pub(super) const BUILD: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\n\
         def build(name: String, f: fn(target: String): String, mode: String): String {\n\
         \x20   \"{name}: {f(mode)}\"\n\
         }",
);

#[test]
fn a_definition_takes_its_types_and_its_invocation_its_arguments() {
    // A definition with a parameter, used above it; invocation
    // parameters; a reference stored and called later; `def x()` beside
    // `def build x()`, a bare `x` seeing only the former.
    let files = [
        BUILD,
        (
            "app.gcsl",
            "y = build x(\"release\")\n\
                 def build x(target) { \"built {target}\" }\n\
                 def x(): Int { 7 }\n\
                 let later = build x\n\
                 z = later(mode = \"debug\")\n\
                 plain = x()",
        ),
    ];
    assert_eq!(
        value(&files, "app.gcsl"),
        "{y = \"x: built release\"; x = fn(): Int; z = \"x: built debug\"; plain = 7}"
    );
    // From another file of the directory and from a directory below.
    let files = [
        BUILD,
        ("a.gcsl", "def build lib.core(t) { \"core-{t}\" }"),
        ("b.gcsl", "b = build lib.core(\"b\")"),
        ("sub/c.gcsl", "c = build lib.core(\"c\")"),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{b = \"lib.core: core-b\"}");
    assert_eq!(value(&files, "sub/c.gcsl"), "{c = \"lib.core: core-c\"}");
    assert_eq!(value(&files, "a.gcsl"), "{}");
}

#[test]
fn a_definition_is_a_leaf_under_its_namespace_s_key() {
    // The scan names the namespace, makes the definition a top-level
    // function and a leaf under its key, and a reference above it
    // mentions it, as a read of its path.
    let files = [
        BUILD,
        ("app.gcsl", "y = build x(\"m\")\ndef build x(t) { t }"),
    ];
    let scan = ws(&files).unwrap();
    let app = scan.files.iter().find(|f| f.path == "app.gcsl").unwrap();
    let workspace = scan
        .files
        .iter()
        .position(|f| f.path == "workspace.gcsl")
        .unwrap();
    let tree = &app.mentions.tree;
    // The roots: `y`, then the namespace, whose child `x` is the leaf.
    let root = &tree.nodes[tree.roots[1] as usize];
    // The `def`'s item, the second the parser made, after its body's.
    assert_eq!(scan.names.get(root.name), format!("<build#{workspace}.1>"));
    assert_eq!(root.children, [2]);
    assert_eq!(tree.path(2).len(), 2);
    assert_eq!(app.mentions.functions.len(), 1);
    assert_eq!(app.mentions.functions[0].name, None);
    assert_eq!(app.mentions.items[0], [0]);
    assert_eq!(app.mentions.fields[0], [2]);
    // The definition's item mentions its def-type, another file's `def`:
    // its node and its function.
    assert_eq!(app.mentions.foreign[1].len(), 2);
}

#[test]
fn each_def_type_has_a_namespace_of_its_own() {
    // Two def-types, each with a definition of one path, and a field of
    // that path too: none meets another.
    let files = [
        BUILD,
        (
            "app.gcsl",
            "#[def_type]\n\
                 def test(name: String, f: fn(target: String): String): String { f(name) }\n\
                 def build lib.core(t) { \"built {t}\" }\n\
                 def test lib.core(t) { \"tested {t}\" }\n\
                 lib.core = 1\n\
                 a = build lib.core(\"r\")\n\
                 b = test lib.core()",
        ),
    ];
    assert_eq!(
        value(&files, "app.gcsl"),
        "{test = fn(name: String, f: fn(target: String): String): String; \
             lib = {core = 1}; a = \"lib.core: built r\"; b = \"tested lib.core\"}"
    );
}

#[test]
fn definitions_conflict_and_shadow_as_dotted_defs_do() {
    let conflict = [
        BUILD,
        ("a.gcsl", "def build x(t) { t }"),
        ("b.gcsl", "def build x(t) { t }"),
    ];
    assert_eq!(
        error(&conflict).0,
        "field `build x` is defined in two files"
    );
    let under = [
        BUILD,
        ("a.gcsl", "def build x(t) { t }"),
        ("b.gcsl", "def build x.y(t) { t }"),
    ];
    assert_eq!(error(&under).0, "field `build x` is already defined");
    // In one file, as the dotted fields of a block.
    let files = [
        BUILD,
        (
            "a.gcsl",
            "def build x(t) { t }\ndef build x(t) { t }\ny = build x(\"m\")",
        ),
    ];
    assert_eq!(
        messages(&files, "a.gcsl"),
        ["field `build x` is defined twice"]
    );
    // Shadowable, across files in path order and within one file.
    let shadowed = [
        BUILD,
        ("a.gcsl", "#[shadowable] def build x(t) { \"a {t}\" }"),
        (
            "b.gcsl",
            "#[shadowable] def build x(t) { \"b {t}\" }\ndef build x(t) { \"c {t}\" }",
        ),
        ("d.gcsl", "y = build x(\"m\")"),
    ];
    assert_eq!(value(&shadowed, "d.gcsl"), "{y = \"x: c m\"}");
    let reversed = [
        BUILD,
        ("a.gcsl", "def build x(t) { t }"),
        ("b.gcsl", "#[shadowable] def build x(t) { t }"),
    ];
    assert_eq!(
        error(&reversed).0,
        "field `build x` is defined in two files"
    );
}

#[test]
fn a_def_type_is_reached_through_its_imports() {
    let lib = (
        "lib/types.gcsl",
        "#[def_type]\n\
             def build(name: String, f: fn(): String): String { \"{name} {f()}\" }",
    );
    // Through a renamed member, defining and invoking; through a whole
    // import, qualified; and a definition through one alias is in the
    // def-type's namespace whatever another file calls it.
    let files = [
        ("workspace.gcsl", ""),
        lib,
        (
            "app/a.gcsl",
            "import build as b from lib/types\n\
                 def b x() { \"via b\" }\n\
                 one = b x()",
        ),
        (
            "app/c.gcsl",
            "import lib/types as t\n\
                 two = t.build x()",
        ),
    ];
    assert_eq!(value(&files, "app/a.gcsl"), "{one = \"x via b\"}");
    assert_eq!(value(&files, "app/c.gcsl"), "{two = \"x via b\"}");
    // `*` brings a file's definitions; a whole import of it does not.
    let defs = (
        "lib/defs.gcsl",
        "import * from lib/types\ndef build x() { \"x\" }",
    );
    let files = [
        ("workspace.gcsl", ""),
        lib,
        defs,
        (
            "app/a.gcsl",
            "import * from lib/defs\nimport * from lib/types\na = build x()",
        ),
        ("app/c.gcsl", "import lib/defs\nc = defs"),
    ];
    assert_eq!(value(&files, "app/a.gcsl"), "{a = \"x x\"}");
    // Read whole, a file holds no namespace.
    assert_eq!(value(&files, "app/c.gcsl"), "{c = {}}");
    let files = [
        ("workspace.gcsl", ""),
        lib,
        defs,
        (
            "app/b.gcsl",
            "import lib/defs\nimport * from lib/types\nb = build x()",
        ),
    ];
    assert_eq!(
        messages(&files, "app/b.gcsl"),
        ["`build` has no definition `x`"]
    );
}

#[test]
fn a_def_type_s_caller_is_where_its_reference_is_called() {
    // The reference is made in `a/`, called in `b/`: the def-type runs
    // for `b/`, and the definition for the def-type's file, but through
    // its helper.
    let files = [
        (
            "workspace.gcsl",
            "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn() -> String) -> { called: String, defined: String } {\n\
                 \x20   called = fileroot.caller\n\
                 \x20   defined = f()\n\
                 }\n\
                 #[def_type]\n\
                 def helped(name: String, f: fn() -> String) -> String {\n\
                 \x20   fileroot.helper()\n\
                 \x20   f()\n\
                 }",
        ),
        (
            "a/a.gcsl",
            "import fileroot\n\
                 def where x() { fileroot.caller }\n\
                 def helped x() { fileroot.caller }\n\
                 def made(): fn() -> { called: String, defined: String } { where x }\n\
                 def made_helped(): fn() -> String { helped x }",
        ),
        (
            "b/b.gcsl",
            "import a/a\n\
                 there = a.made()()\n\
                 through = a.made_helped()()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "b/b.gcsl"),
        "{there = {called = \"<root>/b\"; defined = \"<root>\"}; through = \"<root>/b\"}"
    );
}

#[test]
fn a_pure_definition_cannot_call_an_impure_function() {
    let files = [
        BUILD,
        (
            "app.gcsl",
            "fn noisy() -> String { \"n\" }\n\
                 def build x(t) { noisy() }",
        ),
    ];
    assert_eq!(
        messages(&files, "app.gcsl"),
        ["cannot call an impure function from a pure function"]
    );
    // And an impure def-type's reference from a pure function.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn(): String) -> String { f() }",
        ),
        (
            "app.gcsl",
            "def run x() { \"x\" }\nfn p(): String { run x() }",
        ),
    ];
    assert_eq!(
        messages(&files, "app.gcsl"),
        ["cannot call an impure function from a pure function"]
    );
}

#[test]
fn def_type_errors() {
    // Each error with its labels and notes, joined by `; `, the errors
    // by newlines.
    let errors = |text: &str| {
        let files = [BUILD, ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    let signature = "a def-type takes the name as a `String` and then the definition as a \
                         function, as in `def bad(name: String, func: fn(): T)` at app.gcsl; \
                     note: or, for definitions made with a variant, the name, the variant and \
                     the definition, as in `def Toolchain<T>(name: String, kind: \
                     Variant<Toolchain, T>, func: fn(): T)`";
    assert_eq!(
        errors("#[def_type]\ndef bad(name: Int, f: fn(): Int): Int { 1 }"),
        signature
    );
    assert_eq!(errors("#[def_type]\ndef bad(): Int { 1 }"), signature);
    let not_one = "note: a def-type is a `def` marked `#[def_type]`";
    assert_eq!(
        errors("def plain(): Int { 1 }\ndef plain x() { 1 }"),
        format!("`plain` is not a def-type at app.gcsl; {not_one}")
    );
    assert_eq!(
        errors("def build x() { \"x\" }"),
        "`build` defines functions of 1 parameter, `target` at app.gcsl"
    );
    assert_eq!(
        errors(
            "#[def_type]\ndef two(name: String, f: fn(a: Int, b: Int): Int): Int { 1 }\n\
                 def two x(a) { a }"
        ),
        "`two` defines functions of 2 parameters, `a` and `b` at app.gcsl"
    );
    assert_eq!(
        errors(
            "#[def_type]\ndef none(name: String, f: fn(): Int): Int { 1 }\n\
                 def none x(a) { a }"
        ),
        "`none` defines functions of no parameters at app.gcsl"
    );
    assert_eq!(
        errors("def build a.b(t) { t }\nx = build a.c\ny = build a\nz = build q"),
        "`build` has no definition `a.c` at app.gcsl; \
             note: the only definition of `build` here is `a.b`\n\
             `a` is not a `build` definition at app.gcsl\n\
             `build` has no definition `q` at app.gcsl; \
             note: the only definition of `build` here is `a.b`"
    );
    assert_eq!(
        errors("def build a(t) { t }\ndef build b.c(t) { t }\nx = build d"),
        "`build` has no definition `d` at app.gcsl; \
             note: the definitions of `build` here are `a` and `b.c`"
    );
    assert_eq!(
        errors("x = build d"),
        "`build` has no definition `d` at app.gcsl; note: `build` has no definitions here"
    );
    // Two names in a row, the first no def-type, or undefined.
    let juxtaposed = "note: two names in a row are read as a def-type reference, as in \
                          `package dev.x()`; is an operator or a line break missing?";
    assert_eq!(
        errors("let a = 1\nx = a b"),
        format!("`a` is not a def-type at app.gcsl; {not_one}; {juxtaposed}")
    );
    assert!(
        errors("x = nope b").starts_with("undefined variable `nope` at app.gcsl")
            && errors("x = nope b").ends_with(juxtaposed),
        "{}",
        errors("x = nope b")
    );
    // A parameter of the def-type's name is no def-type.
    assert_eq!(
        errors("def build x(t) { t }\nfn f(build: Int): Int { build x() }"),
        format!("`build` is not a def-type at app.gcsl; {not_one}; {juxtaposed}")
    );
}

#[test]
fn no_source_can_write_a_namespace_s_key() {
    // A field of the key's form is no field at all, so it cannot reach
    // a namespace's definitions.
    let files = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }\n\"<build#1.0>\" = 1"),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "a field cannot be named `\"<build#1.0>\"`: names of that form are def-types' \
                 namespaces at app.gcsl"
        ]
    );
    let files = [BUILD, ("app.gcsl", "x = {}.\"<build#1.0>\"")];
    assert_eq!(
        check_errors(&files),
        [
            "a field cannot be named `\"<build#1.0>\"`: names of that form are def-types' \
                 namespaces at app.gcsl"
        ]
    );
}

#[test]
fn dump_ast_shows_definitions_and_references() {
    let mut workspace = checked(&[
        BUILD,
        ("app.gcsl", "def build x(t) { t }\ny = build x(\"m\")"),
    ]);
    let index = workspace.file("app.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(def-type-def build x @0..20\n"), "{dump}");
    assert!(dump.contains("(params t)\n"), "{dump}");
    assert!(
        dump.contains("(def-type-ref build x : fn(mode: String): String @25..32)"),
        "{dump}"
    );
}

/// A workspace of a def-type at its root, `build`, definitions of it in
/// `pkgs/` at two depths, and `app.gcsl`, in `image/`, of `app`.
fn globbed(app: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "workspace.gcsl",
            "#[def_type]\n\
             def build(name: String, f: fn(): String): String { \"{name}: {f()}\" }"
                .to_owned(),
        ),
        (
            "pkgs/top.gcsl",
            "def build top() { \"in pkgs\" }".to_owned(),
        ),
        ("pkgs/a/a.gcsl", "def build x() { \"a\" }".to_owned()),
        (
            "pkgs/b/b.gcsl",
            "def build y() { \"b\" }\ndef build y2() { \"b2\" }".to_owned(),
        ),
        (
            "pkgs/b/deep/d.gcsl",
            "def build z() { \"deep\" }".to_owned(),
        ),
        (
            "pkgs/.hidden/h.gcsl",
            "def build h() { \"hidden\" }".to_owned(),
        ),
        ("image/app.gcsl", app.to_owned()),
    ]
}

/// The files of `files`, borrowed.
fn borrowed<'a>(files: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    files
        .iter()
        .map(|(path, text)| (*path, text.as_str()))
        .collect()
}

#[test]
fn a_def_type_s_definitions_are_imported_from_every_directory_a_glob_names() {
    let cases = [
        // Directly in each directory under `pkgs/`, not deeper, nor in it.
        (
            "import build * from pkgs/*\nv = [build x(), build y(), build y2()]",
            "{v = [\"x: a\", \"y: b\", \"y2: b2\"]}",
        ),
        // At any depth under it.
        (
            "import build * from pkgs/**\nv = [build x(), build z()]",
            "{v = [\"x: a\", \"z: deep\"]}",
        ),
        // In the directory itself, as `import * from` would.
        (
            "import build * from pkgs\nv = build top()",
            "{v = \"top: in pkgs\"}",
        ),
        // The root's directories; none of them is `pkgs/b`.
        (
            "import build * from *\nv = build top()",
            "{v = \"top: in pkgs\"}",
        ),
        // A glob that finds none is no error.
        ("import build * from pkgs/a/*\nv = 1", "{v = 1}"),
    ];
    for (app, expected) in cases {
        let files = globbed(app);
        assert_eq!(
            value(&borrowed(&files), "image/app.gcsl"),
            expected,
            "{app}"
        );
    }
    // Only the definitions: one depth only, and nothing else of the files.
    for (app, message) in [
        (
            "import build * from pkgs/*\nv = build z()",
            "`build` has no definition `z`",
        ),
        (
            "import build * from pkgs/*\nv = build top()",
            "`build` has no definition `top`",
        ),
        (
            "import build * from pkgs/**\nv = build h()",
            "`build` has no definition `h`",
        ),
    ] {
        let files = globbed(app);
        assert_eq!(
            messages(&borrowed(&files), "image/app.gcsl"),
            [message],
            "{app}"
        );
    }
}

#[test]
fn definitions_a_glob_imports_twice_are_an_error_at_the_import() {
    let mut files = globbed("import build * from pkgs/*\nv = build y()");
    files.push(("pkgs/c/c.gcsl", "def build x() { \"c\" }".to_owned()));
    let files = borrowed(&files);
    assert_eq!(
        messages(&files, "image/app.gcsl"),
        ["field `build x` is defined in two files"]
    );
}

#[test]
fn an_import_of_definitions_names_a_def_type_and_directories() {
    for (app, message) in [
        ("import nope * from pkgs/*\nv = 1", "`nope` is no def-type"),
        (
            "import build * from pkgs/a/a/*\nv = 1",
            "a glob names directories, and `pkgs/a/a` is a file",
        ),
        (
            "import build * from pkgs/none/*\nv = 1",
            "no module `pkgs/none`",
        ),
    ] {
        let files = globbed(app);
        assert_eq!(
            messages(&borrowed(&files), "image/app.gcsl"),
            [message],
            "{app}"
        );
    }
}
