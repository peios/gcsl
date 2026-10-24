//! Tests of workspaces: types across files, dotted defs and annotated fields.

use super::*;

/// Every diagnostic of checking the workspace of `files`, in order,
/// each shown as `MESSAGE at FILE (LABEL); FILE: LABEL; note: NOTE`,
/// its file and each label's by path.
pub(super) fn check_errors(files: &[(&str, &str)]) -> Vec<String> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], "/");
    let Err(errors) = checked else {
        panic!("{files:?}: checked without an error");
    };
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    all.extend(paths);
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
                let file = path(label.input.or(error.input));
                parts.push(format!("{file}: {}", label.text));
            }
            for note in &error.notes {
                parts.push(format!("note: {note}"));
            }
            parts.join("; ")
        })
        .collect()
}

/// The files of the slice 33 summary: named record types, one built
/// from another by spread, an annotated field, a qualified name and a
/// file below reading a type it imports.
pub(super) const TYPES: [(&str, &str); 3] = [
    (
        "_lib/types.gcsl",
        "type PackageDef = {\n\
             \x20   version: String,\n\
             \x20   arch: String,\n\
             \x20   content: String,\n\
             }\n\
             type Package = { ...PackageDef, name: String }",
    ),
    (
        "workspace.gcsl",
        "import _lib/types as t\n\
             type Steps = List<fn() -> String>\n\
             def make(name: String, f: fn(): t.PackageDef): { pkg: t.Package } {\n\
             \x20   pkg: t.Package = { ...f(); name = name }\n\
             }\n\
             built = make(\"hello\", fn(): t.PackageDef { version = \"1\"; arch = \"x\"; content = \"usr\" })",
    ),
    (
        "pkg/app.gcsl",
        "import Package from _lib/types\n\
             p: Package = { version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\" }\n\
             steps: Steps = []\n\
             n = built.pkg.name",
    ),
];

#[test]
fn the_types_summary_runs() {
    assert_eq!(
        value(&TYPES, "pkg/app.gcsl"),
        "{p = {version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\"}; \
             steps = []; n = \"hello\"}"
    );
    // A type is shown as what it stands for, never by its name.
    assert_eq!(
        value(&TYPES, "workspace.gcsl"),
        "{make = fn(name: String, f: fn(): {arch: String, content: String, version: \
             String}): {pkg: {arch: String, content: String, name: String, version: String}}; \
             built = {pkg = {arch = \"x\"; content = \"usr\"; version = \"1\"; name = \"hello\"}}}"
    );
}

#[test]
fn jacks_workspace_checks_with_spread_for_intersection() {
    // `/home/jack/projects/peios/newpkgs/workspace.gcsl`, with `&`
    // written as a spread and `#[def_type]` removed, and `ppk.commit`
    // stubbed by a module of the test's own.
    let files = [
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
            "def commit(name: String, version: String, arch: String, content: String) -> String {\n\
                 \x20   \"{name} {version} {arch} {content}\"\n\
                 }",
        ),
        (
            "dev.peios.testpkg/test.gcsl",
            "built = package(\"dev.peios.testpkg\", fn(): PackageDef {\n\
                 \x20   version = \"0.1.0\"\n\
                 \x20   arch = \"x86_64\"\n\
                 \x20   content = \"usr\"\n\
                 })",
        ),
    ];
    checked(&files);
    assert_eq!(
        value(&files, "dev.peios.testpkg/test.gcsl"),
        "{built = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}"
    );
}

/// The files of the slice 34 summary as it is written: `hello` in
/// `a.gcsl` reads `name` of `b.gcsl`, which reads `hello`.
pub(super) const DOTTED_DEFS: [(&str, &str); 3] = [
    ("workspace.gcsl", ""),
    (
        "pkgs/a.gcsl",
        "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.count(n: Int): Int {\n\
             \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
             }",
    ),
    (
        "pkgs/b.gcsl",
        "def dev.peios.name(): String { \"world\" }\n\
             dev.peios.version = \"1\"\n\
             greeting = dev.peios.hello()     # \"hello world\"\n\
             all = dev.peios                  # record of hello, count, name, version",
    ),
];

#[test]
fn the_dotted_defs_summary_is_two_files_that_need_each_other() {
    // As written, the two files need each other, as they do with every
    // `def` named by one name: a file runs as a whole before a file
    // that needs it.
    let (message, input, labels) = error(&DOTTED_DEFS);
    assert_eq!(message, "`pkgs/a.gcsl` and `pkgs/b.gcsl` need each other");
    assert_eq!((input, labels), (Some(1), vec![Some(2)]));
    let plain = [
        ("workspace.gcsl", ""),
        ("pkgs/a.gcsl", "def hello(): String { \"hello {name()}\" }"),
        (
            "pkgs/b.gcsl",
            "def name(): String { \"world\" }\ngreeting = hello()",
        ),
    ];
    assert_eq!(error(&plain).0, message);
    // With `name` beside `hello`, it runs: each file to its own value,
    // and `all` the record of the directory's leaves under the prefix,
    // the functions among them.
    let files = [
        ("workspace.gcsl", ""),
        (
            "pkgs/a.gcsl",
            "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
                 def dev.peios.count(n: Int): Int {\n\
                 \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
                 }\n\
                 def dev.peios.name(): String { \"world\" }",
        ),
        (
            "pkgs/b.gcsl",
            "dev.peios.version = \"1\"\n\
                 greeting = dev.peios.hello()\n\
                 all = dev.peios\n\
                 counted = dev.peios.count(3)",
        ),
    ];
    assert_eq!(
        value(&files, "pkgs/b.gcsl"),
        "{dev = {peios = {version = \"1\"}}; greeting = \"hello world\"; \
             all = {hello = fn(): String; count = fn(n: Int): Int; name = fn(): String; \
             version = \"1\"}; counted = 3}"
    );
    assert_eq!(
        value(&files, "pkgs/a.gcsl"),
        "{dev = {peios = {hello = fn(): String; count = fn(n: Int): Int; \
             name = fn(): String}}}"
    );
}

#[test]
fn dotted_defs_across_the_files_of_a_directory() {
    // Called by path from another file of the directory, and from a
    // directory below.
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def dev.odd(n: Int): Bool { if n == 0 { false } else { dev.even(n - 1) } }\n\
                 def dev.even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
        ),
        ("b.gcsl", "x = dev.even(10)"),
        ("sub/c.gcsl", "y = dev.odd(3)\nall = dev"),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{x = true}");
    assert_eq!(
        value(&files, "sub/c.gcsl"),
        "{y = true; all = {odd = fn(n: Int): Bool; even = fn(n: Int): Bool}}"
    );
    // Two in two files that call each other need each other, as two
    // `def`s of one name do.
    let mutual = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def dev.even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
        ),
        (
            "b.gcsl",
            "def dev.odd(n: Int): Bool { if n == 0 { false } else { dev.even(n - 1) } }",
        ),
    ];
    let plain = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }",
        ),
        (
            "b.gcsl",
            "def odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }",
        ),
    ];
    assert_eq!(error(&mutual), error(&plain));
    assert_eq!(error(&mutual).0, "`a.gcsl` and `b.gcsl` need each other");
    // A leaf conflict across files, as any leaf's.
    let conflict = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "def dev.x(): Int { 1 }"),
        ("b.gcsl", "dev.x = 2"),
    ];
    assert_eq!(error(&conflict).0, "field `dev.x` is defined in two files");
    // Shadowable across the files of a directory, in path order.
    let shadowed = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "#[shadowable] def dev.x(): Int { 1 }"),
        ("b.gcsl", "def dev.x(): Int { 2 }\ny = dev.x()"),
        ("c.gcsl", "z = dev.x()"),
    ];
    assert_eq!(value(&shadowed, "c.gcsl"), "{z = 2}");
    // A call from another file is of a known callee, as a `def` of one
    // name is: the call-site form of the implicit-await error.
    let known = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def dev.inc(n: Int): Int { n + 1 }\ndef inc(n: Int): Int { n + 1 }",
        ),
        (
            "b.gcsl",
            "fn p(f: Future<Int>): Int { dev.inc(f) }\n\
                 fn q(f: Future<Int>): Int { inc(f) }",
        ),
    ];
    assert_eq!(
        messages(&known, "b.gcsl"),
        [
            "argument `f` would be awaited inside `dev.inc`, which is pure",
            "argument `f` would be awaited inside `inc`, which is pure",
        ]
    );
}

#[test]
fn dotted_defs_through_each_form_of_import() {
    let lib = (
        "pkgs/a.gcsl",
        "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.name(): String { \"world\" }",
    );
    for (text, expected) in [
        (
            "import dev from pkgs/a\nh = dev.peios.hello()",
            "{h = \"hello world\"}",
        ),
        (
            "import dev as d from pkgs/a\nh = d.peios.name()",
            "{h = \"world\"}",
        ),
        (
            "import * from pkgs/a\ndef dev.own(): String { dev.peios.name() }\n\
                 h = dev.peios.hello()\no = dev.own()\nall = dev",
            "{dev = {own = fn(): String}; h = \"hello world\"; o = \"world\"; \
                 all = {own = fn(): String; peios = {hello = fn(): String; \
                 name = fn(): String}}}",
        ),
        (
            "import pkgs/a\nh = a.dev.peios.hello()\nw = a",
            "{h = \"hello world\"; w = {dev = {peios = {hello = fn(): String; \
                 name = fn(): String}}}}",
        ),
        ("import pkgs\nh = pkgs.dev.peios.name()", "{h = \"world\"}"),
    ] {
        let files = [("workspace.gcsl", ""), lib, ("app/main.gcsl", text)];
        assert_eq!(value(&files, "app/main.gcsl"), expected, "{text}");
    }
    // Through an import, a call is of a known callee.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/x.gcsl", "def dev.inc(n: Int): Int { n + 1 }"),
        (
            "app/main.gcsl",
            "import dev from lib/x\nimport lib\n\
                 fn p(f: Future<Int>): Int { dev.inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.dev.inc(f) }",
        ),
    ];
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        [
            "argument `f` would be awaited inside `dev.inc`, which is pure",
            "argument `f` would be awaited inside `lib.dev.inc`, which is pure",
        ]
    );
}

#[test]
fn a_view_read_whole_has_its_dotted_defs_resolved() {
    // `dev` merges the root's, the import's and the file's own leaves:
    // read whole above the `def`, it holds the type its body gives it,
    // in either order of the file's items.
    let resolved = "{base: Int, lib: Int, own: {get: fn(n: Int | Future<Int>) -> Int}}";
    for text in [
        "import * from lib\nwhole = dev\ndef dev.own.get(n: Int) -> Int { n + 1 }\n\
             got = whole.own.get(1)",
        "import * from lib\ndef dev.own.get(n: Int) -> Int { n + 1 }\nwhole = dev\n\
             got = whole.own.get(1)",
    ] {
        let files = [
            ("workspace.gcsl", "dev.base = 1"),
            ("lib/l.gcsl", "dev.lib = 2"),
            ("app/main.gcsl", text),
        ];
        let mut workspace = checked(&files);
        let index = workspace.file("app/main.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(
            dump.contains(&format!("(name dev : {resolved}")),
            "{text}\n{dump}"
        );
        assert!(
            value(&files, "app/main.gcsl").ends_with("got = 2}"),
            "{text}"
        );
    }
    // And one that cannot be had there yet is the error at the read.
    let files = [
        ("workspace.gcsl", "dev.base = 1"),
        ("lib/l.gcsl", "dev.lib = 2"),
        (
            "app/main.gcsl",
            "import * from lib\nwhole = dev\nlet k = 1\ndef dev.own(): Int { k }",
        ),
    ];
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        ["`dev.own` cannot be used here yet"]
    );
}

#[test]
fn jacks_package_is_a_dotted_def_given_to_package() {
    // `/home/jack/projects/peios/newpkgs`, with `&` written as a spread,
    // `#[def_type]` removed and `ppk.commit` stubbed as in the slice 33
    // test, and the package written as a `def` named by its dotted path
    // passed to `package`.
    let files = [
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
            "def commit(name: String, version: String, arch: String, content: String) -> String {\n\
                 \x20   \"{name} {version} {arch} {content}\"\n\
                 }",
        ),
        (
            "dev.peios.testpkg/test.gcsl",
            "def dev.peios.testpkg(): PackageDef {\n\
                 \x20   version = \"0.1.0\"\n\
                 \x20   arch = \"x86_64\"\n\
                 \x20   content = \"usr\"\n\
                 }\n\
                 \n\
                 result = package(\"dev.peios.testpkg\", dev.peios.testpkg)",
        ),
    ];
    checked(&files);
    assert_eq!(
        value(&files, "dev.peios.testpkg/test.gcsl"),
        "{dev = {peios = {testpkg = fn(): {arch: String, content: String, \
             version: String}}}; result = {pkg = {arch = \"x86_64\"; content = \"usr\"; \
             version = \"0.1.0\"; name = \"dev.peios.testpkg\"}; \
             manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}"
    );
}

#[test]
fn a_type_is_used_above_its_declaration_and_across_files() {
    // In its own file above its line, from another file of its
    // directory, and from a directory below, through the chain.
    let root = "a: Point = { x = 1; y = 2 }\ntype Point = {x: Int, y: Int}";
    let files = [
        ("workspace.gcsl", root),
        ("other.gcsl", "b: Point = { x = 3; y = 4 }"),
        ("sub/deep.gcsl", "c: Point = { x = 5; y = 6 }"),
    ];
    assert_eq!(value(&files, "workspace.gcsl"), "{a = {x = 1; y = 2}}");
    assert_eq!(value(&files, "other.gcsl"), "{b = {x = 3; y = 4}}");
    assert_eq!(value(&files, "sub/deep.gcsl"), "{c = {x = 5; y = 6}}");
    // A sibling directory does not see them.
    let hidden = [
        ("workspace.gcsl", ""),
        ("a/types.gcsl", "type Point = {x: Int}"),
        ("b/use.gcsl", "p: Point = { x = 1 }"),
    ];
    assert_eq!(messages(&hidden, "b/use.gcsl"), ["unknown type `Point`"]);
}

/// A library of types for the import tests: `lib/types.gcsl` with a
/// record type and a type also a value's name, and `lib/more.gcsl`.
pub(super) const TYPE_LIB: [(&str, &str); 3] = [
    ("workspace.gcsl", ""),
    (
        "lib/types.gcsl",
        "type P = {x: Int}\ntype Both = Int\nBoth = 7",
    ),
    ("lib/more.gcsl", "type Q = {...P, y: String}"),
];

#[test]
fn types_are_reached_through_every_import_form() {
    let with = |main: &str| {
        let mut files = TYPE_LIB.to_vec();
        files.push(("app/main.gcsl", main));
        value(&files, "app/main.gcsl")
    };
    // Whole, by its path's last segment and `as`.
    assert_eq!(
        with("import lib/types\nv: types.P = { x = 1 }"),
        "{v = {x = 1}}"
    );
    assert_eq!(
        with("import lib/types as t\nv: t.P = { x = 1 }"),
        "{v = {x = 1}}"
    );
    // A directory's merged tree has its files' types.
    assert_eq!(
        with("import lib\nv: lib.Q = { x = 1; y = \"a\" }"),
        "{v = {x = 1; y = \"a\"}}"
    );
    // A member, bare or renamed.
    assert_eq!(
        with("import P from lib/types\nv: P = { x = 2 }"),
        "{v = {x = 2}}"
    );
    assert_eq!(
        with("import P as R from lib/types\nv: R = { x = 2 }"),
        "{v = {x = 2}}"
    );
    // `*`, with the fields.
    assert_eq!(
        with("import * from lib/types\nv: P = { x = Both }"),
        "{v = {x = 7}}"
    );
    // A member that is both a type and a value binds both.
    assert_eq!(
        with("import Both from lib/types\nv: Both = Both + 1"),
        "{v = 8}"
    );
}

#[test]
fn qualified_names_go_two_deep() {
    // A deep module path, and a type whose definition names another
    // module's type qualified, used qualified from a third.
    let files = [
        ("workspace.gcsl", ""),
        ("a/b/inner.gcsl", "type Leaf = {v: Int}"),
        (
            "a/b/types.gcsl",
            "import a/b/inner as i\ntype Node = {leaf: i.Leaf, name: String}",
        ),
        (
            "app/main.gcsl",
            "import a/b/types as t\nn: t.Node = { leaf = { v = 1 }; name = \"n\" }\nw = n.leaf.v",
        ),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{n = {leaf = {v = 1}; name = \"n\"}; w = 1}"
    );
}

#[test]
fn a_type_and_a_value_of_one_name_coexist() {
    let files = [(
        "workspace.gcsl",
        "type Foo = {a: Int}\nFoo = 3\nlet x: Foo = { a = Foo }\nfn bar(Foo: Foo): Int { Foo.a }\ny = bar(x)",
    )];
    assert_eq!(value(&files, "workspace.gcsl"), "{Foo = 3; y = 3}");
    // `type` is still a name, as it was, for a variable and a field.
    let files = [(
        "workspace.gcsl",
        "type = 1\nlet t = type\nx = { type = t }.type\ny: Int = type",
    )];
    assert_eq!(value(&files, "workspace.gcsl"), "{type = 1; x = 1; y = 1}");
}

#[test]
fn spread_in_a_record_type_overrides_as_value_spread_does() {
    let run = |types: &str, value_text: &str| {
        let prefix = "type A = {a: Int, b: Int}\ntype B = {b: String}";
        let text = format!("{prefix}\n{types}\n{value_text}");
        value(&[("workspace.gcsl", &text)], "workspace.gcsl")
    };
    // A later field overrides a spread's, its type differing.
    assert_eq!(
        run("type T = {...A, b: String}", "v: T = { a = 1; b = \"x\" }"),
        "{v = {a = 1; b = \"x\"}}"
    );
    // A later spread overrides an earlier one's.
    assert_eq!(
        run("type T = {...A, ...B}", "v: T = { a = 1; b = \"y\" }"),
        "{v = {a = 1; b = \"y\"}}"
    );
    assert_eq!(
        run("type T = {...B, ...A}", "v: T = { a = 1; b = 2 }"),
        "{v = {a = 1; b = 2}}"
    );
    // The same spread twice is idempotent.
    assert_eq!(
        run("type T = {...A, ...A}", "v: T = { a = 1; b = 2 }"),
        "{v = {a = 1; b = 2}}"
    );
    // A spread of a type built by a spread.
    assert_eq!(
        run(
            "type T = {...A, c: Bool}\ntype U = {...T, d: Int}",
            "v: U = { a = 1; b = 2; c = true; d = 4 }"
        ),
        "{v = {a = 1; b = 2; c = true; d = 4}}"
    );
    // A record type written in place spread.
    assert_eq!(
        run(
            "type T = {...{z: Int}, ...A}",
            "v: T = { z = 0; a = 1; b = 2 }"
        ),
        "{v = {z = 0; a = 1; b = 2}}"
    );
}

#[test]
fn spread_problems_in_record_types() {
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    let twice = "field `b` is defined twice at workspace.gcsl (`b` is a field of this, of \
                     type Int); workspace.gcsl: first defined here; note: a spread cannot \
                     replace a field defined before it; put the spread first";
    let text = "type A = {b: Int}\ntype T = {b: String, ...A}";
    assert_eq!(one(text), [twice]);
    let text = "type A = {b: Int, c: Int}\nx: {b: String, c: Bool, ...A} = { b = \"x\" }";
    let others = format!("{twice} (it also has 1 other field defined before it)");
    assert_eq!(one(text), [others]);
    assert_eq!(
        one("type T = {...Int, a: Int}"),
        ["cannot spread type Int: it is not a record type at workspace.gcsl"]
    );
    assert_eq!(
        one("type N = List<Int>\ntype T = {...N}"),
        ["cannot spread type `List<Int>`: it is not a record type at workspace.gcsl"]
    );
    // Only a record type, written so or named, is spread: the type is
    // shown as what it stands for.
    assert_eq!(
        one("x: {...Future<{a: Int}>} = { a = 1 }"),
        ["cannot spread type `Future<{a: Int}>`: it is not a record type at workspace.gcsl"]
    );
    let union = "cannot spread type `{a: Int} | {b: Int}`: it is not a record type at \
                     workspace.gcsl";
    assert_eq!(
        one("type A = {a: Int}\ntype B = {b: Int}\nx: {...A | B} = { a = 1 }"),
        [union]
    );
    // A record type written in place with a name twice, spread, is the
    // repeated-name error it is anywhere else, at the same place, in a
    // `type` and in a field's annotation alike.
    let repeated = |text: &str| {
        let source = SourceFile::new("workspace.gcsl", text).unwrap();
        let errors = check(std::slice::from_ref(&source)).unwrap_err();
        assert_eq!(errors.len(), 1, "{text}: {errors:?}");
        let at = u32::try_from(text.rfind("a: String").unwrap()).unwrap();
        assert_eq!(errors[0].span, Span::new(at, at + 1), "{text}");
        errors[0].message.clone()
    };
    let twice = "field `a` appears twice in this type";
    for text in [
        "type R = { a: Int, a: String }",
        "type R = { ...{ a: Int, a: String } }",
        "x: { a: Int, a: String } = { a = 1 }",
        "x: { ...{ a: Int, a: String } } = { a = 1 }",
        "type B = {b: Int}\nx: { ...B, ...{ a: Int, a: String } } = { a = 1; b = 2 }",
    ] {
        assert_eq!(repeated(text), twice, "{text}");
    }
    // And a type so defined is erroneous wherever it is used, reported
    // once, where it is written.
    assert_eq!(
        check_errors(&[
            ("workspace.gcsl", "type R = { ...{ a: Int, a: String } }"),
            ("use.gcsl", "x: R = { a = 1 }"),
        ]),
        [format!("{twice} at workspace.gcsl")]
    );
    // An erroneous type spread is reported where it is written, once.
    assert_eq!(
        one("type A = {a: Strng}\ntype T = {...A}\nx: T = { a = 1 }"),
        ["unknown type `Strng` at workspace.gcsl"]
    );
}

#[test]
fn a_spread_of_a_record_type_carries_its_rest_field() {
    let run = |text: &str| value(&[("workspace.gcsl", text)], "workspace.gcsl");
    // A spread's rest-field joins the record type, softly, as its fields
    // do: a later one replaces it.
    assert_eq!(
        run("type M = {*: Int}\ntype T = {...M, to: String}\nv: T = { to = \"a\"; x = 1 }"),
        "{v = {to = \"a\"; x = 1}}"
    );
    assert_eq!(
        run("type M = {*: Int}\ntype T = {...M, *: String}\nv: T = { x = \"y\" }"),
        "{v = {x = \"y\"}}"
    );
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    // One written before the spread is not replaced.
    assert_eq!(
        one("type M = {*: Int}\ntype T = {*: String, ...M}"),
        [
            "the rest-field is defined twice at workspace.gcsl (this has a rest-field, of \
             type Int); workspace.gcsl: first defined here; note: a spread cannot replace a \
             field defined before it; put the spread first"
        ]
    );
    // Two in a record type written in place and spread.
    assert_eq!(
        one("type T = {...{*: Int, *: String}}"),
        ["the rest-field appears twice in this type at workspace.gcsl"]
    );
}

#[test]
fn a_def_type_s_definitions_can_return_a_record_of_any_fields() {
    // pekit's packages map paths to paths: `files` is a map, which each
    // definition writes as a record literal.
    let types = "type Package = {build: String, files: {*: String}}\n\
                 #[def_type]\n\
                 def package(name: String, func: fn(): Package) -> {*: String} {\n\
                 \x20   func().files\n\
                 }\n";
    let busybox = "def package net.busybox.busybox() {\n\
                   \x20   build = \"busybox\"\n\
                   \x20   files = {\n\
                   \x20       \"bin/*\" = \"bin/*\"\n\
                   \x20       \"sbin/*\" = \"sbin/*\"\n\
                   \x20   }\n\
                   }\n\
                   files = package net.busybox.busybox()\n";
    let files = [
        ("workspace.gcsl", types),
        ("net.busybox.busybox/pekit.gcsl", busybox),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let checked = check(&sources).unwrap();
    assert_eq!(checked.warnings(), []);
    assert_eq!(
        value(&files, "net.busybox.busybox/pekit.gcsl"),
        "{files = {\"bin/*\" = \"bin/*\"; \"sbin/*\" = \"sbin/*\"}}"
    );
}

#[test]
fn type_declaration_errors() {
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    assert_eq!(
        one("type Int = String"),
        ["`Int` is a built-in type at workspace.gcsl"]
    );
    assert_eq!(
        one("type List = {a: Int}\ntype Future = Int"),
        [
            "`List` is a built-in type at workspace.gcsl",
            "`Future` is a built-in type at workspace.gcsl"
        ]
    );
    assert_eq!(
        one("type A = Int\nx = 1\ntype A = String"),
        ["type `A` is declared twice at workspace.gcsl; workspace.gcsl: first declared here"]
    );
    assert_eq!(
        one("type A = {a: Nope}\nx: A = { a = 1 }\ny: A = { a = 2 }"),
        ["unknown type `Nope` at workspace.gcsl"]
    );
    assert_eq!(
        one("type A = Int\nx: A<Int> = 1"),
        ["type `A` takes no type argument at workspace.gcsl"]
    );
    let mismatch = "mismatched types at workspace.gcsl (expected Int, found String); \
                        workspace.gcsl: expected because of this annotation";
    assert_eq!(one("type A = Int\nx: A = \"s\""), [mismatch]);
}

#[test]
fn an_erroneous_type_is_reported_once_whichever_file_is_checked_first() {
    // `a.gcsl` is checked before `z.gcsl`, whose type it uses; and
    // `n.gcsl`, which needs `z.gcsl`, after it.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x: Bad = { b = 1 }"),
        ("n.gcsl", "y = zed + 1"),
        ("z.gcsl", "type Bad = {b: Strng}\nzed = 1"),
    ];
    assert_eq!(check_errors(&files), ["unknown type `Strng` at z.gcsl"]);
}

#[test]
fn type_conflicts_are_the_conflicts_of_fields() {
    let errors = |files: &[(&str, &str)]| check_errors(files);
    // Two files of one directory.
    let two = "type `Foo` is declared in two files at b.gcsl (declared here); \
                   a.gcsl: declared here";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", ""),
            ("a.gcsl", "type Foo = Int"),
            ("b.gcsl", "type Foo = String"),
        ]),
        [two]
    );
    // An inner directory's and an outer one's.
    let above = "type `Foo` is declared in two files at workspace.gcsl (declared here); \
                     sub/a.gcsl: declared here; note: one is in a directory above";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", "type Foo = Int"),
            ("sub/a.gcsl", "type Foo = String"),
        ]),
        [above]
    );
    // An imported one and an own one.
    let imported = "type `Foo` is declared in two files at app/m.gcsl (imported here); \
                        lib/t.gcsl: declared here; app/m.gcsl: declared here";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", ""),
            ("lib/t.gcsl", "type Foo = Int"),
            ("app/m.gcsl", "import Foo from lib/t\ntype Foo = String"),
        ]),
        [imported]
    );
    // A `#[shadowable]` import brings types as plain ones, which an own
    // type of their name conflicts with.
    let star = "type `Foo` is declared in two files at app/m.gcsl (imported here); \
                    lib/t.gcsl: declared here; app/m.gcsl: declared here";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", ""),
            ("lib/t.gcsl", "type Foo = Int"),
            (
                "app/m.gcsl",
                "#[shadowable] import * from lib/t\ntype Foo = String"
            ),
        ]),
        [star]
    );
    // One type reached twice is one type.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/t.gcsl", "type Foo = Int"),
        (
            "lib/m.gcsl",
            "import * from lib\nimport Foo from lib/t\nx: Foo = 1",
        ),
    ];
    assert_eq!(value(&files, "lib/m.gcsl"), "{x = 1}");
}

#[test]
fn qualified_names_that_name_no_type() {
    let with = |main: &str| {
        let mut files = TYPE_LIB.to_vec();
        files.push(("app/main.gcsl", main));
        check_errors(&files)
    };
    assert_eq!(
        with("import lib/types as t\nx: t.Nope = 1"),
        ["`t` has no type `Nope` at app/main.gcsl; note: `t` has the types `P`, `Both`"]
    );
    assert_eq!(
        with("import lib/more as m\nimport builtins\nx: builtins.Int = 1"),
        ["`builtins` has no type `Int` at app/main.gcsl; note: `builtins` has no types"]
    );
    assert_eq!(
        with("import lib/types as t\nx: t = 1"),
        ["`t` is a module, not a type at app/main.gcsl"]
    );
    assert_eq!(
        with("import lib/types as t\nx: t.P.x = 1"),
        ["`t.P` is a type, not a module at app/main.gcsl"]
    );
    assert_eq!(
        with("x: nothing.P = 1"),
        ["unknown type `nothing.P` at app/main.gcsl"]
    );
}

#[test]
fn cycles_of_types() {
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    assert_eq!(
        one("type A = {a: A}"),
        ["type `A` refers to itself at workspace.gcsl; workspace.gcsl: `A` uses `A` here"]
    );
    assert_eq!(
        one("type A = B\ntype B = {a: A}"),
        ["type `A` refers to itself at workspace.gcsl; \
              workspace.gcsl: `A` uses `B` here; workspace.gcsl: `B` uses `A` here"]
    );
    // Across files, at the first in path then source order.
    let three = "type `A` refers to itself at a.gcsl; a.gcsl: `A` uses `B` here; \
                     b.gcsl: `B` uses `C` here; x.gcsl: `C` uses `A` here";
    assert_eq!(
        check_errors(&[
            ("workspace.gcsl", ""),
            ("x.gcsl", "type C = {c: A}"),
            ("a.gcsl", "type A = List<B>"),
            ("b.gcsl", "type B = {...C}"),
        ]),
        [three]
    );
}

#[test]
fn annotated_fields_have_their_annotations_types() {
    // Top-level, in an inner block, dotted and quoted, read from another
    // file, whose readers see the annotation's type, not the value's.
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "x: Int | String = 1\n\
                 conf.port: Int | String = 80\n\
                 \"a b\": Float = 2\n\
                 inner = { y: Int | String = \"s\"; z = y }",
        ),
        (
            "b.gcsl",
            "import a as m\n\
                 kind = if x is Int { \"int\" } else { \"string\" }\n\
                 port = conf.port\n\
                 f = m.\"a b\" + 0.5",
        ),
    ];
    assert_eq!(
        value(&files, "a.gcsl"),
        "{x = 1; conf = {port = 80}; \"a b\" = 2.0; inner = {y = \"s\"; z = \"s\"}}"
    );
    assert_eq!(
        value(&files, "b.gcsl"),
        "{kind = \"int\"; port = 80; f = 2.5}"
    );
    let wide = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x: Int | String = 1"),
        ("b.gcsl", "y = x + 1"),
    ];
    let add = "cannot add `Int | String` and Int at b.gcsl; b.gcsl: Int | String; \
                   b.gcsl: Int; note: tell them apart first, as in `if x is Int { ... }`";
    assert_eq!(check_errors(&wide), [add]);
}

#[test]
fn annotated_fields_check_as_annotated_lets_do() {
    let one = |text: &str| value(&[("workspace.gcsl", text)], "workspace.gcsl");
    let errors = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    // A value that fits once awaited is awaited, a literal adapts, and a
    // pure function's parameter the annotation needs present is
    // measured, present.
    assert_eq!(
        one("x: Int = lazy 1\ny: Float = 2\nf = fn(n: Int): {v: Int} { v: Int = n }"),
        "{x = 1; y = 2.0; f = fn(n: Int): {v: Int}}"
    );
    // In a record block checked against an expected record type.
    assert_eq!(
        one("let r: {a: Int | String} = { a: Int = 1 }\nv = r.a"),
        "{v = 1}"
    );
    let mismatch = "mismatched types at workspace.gcsl (expected Int, found Int | String); \
                        workspace.gcsl: expected because of this annotation";
    assert_eq!(
        errors("let r: {a: Int} = { a: Int | String = 1 }"),
        [mismatch]
    );
    // TEMPORARY (alias rule): an annotated field is no alias of a
    // built-in function, as an annotated `let` is none.
    let value_yet = "`len` is a built-in function and cannot be used as a value yet at \
                         workspace.gcsl; note: a built-in function with a type for any list \
                         cannot be written as a type yet; call it, or bind it to a name";
    assert_eq!(errors("l: fn(xs: List<Int>): Int = len"), [value_yet]);
}

#[test]
fn an_alias_is_bare_where_it_is_used() {
    // As if `Int` were written at each use: the either shape in an
    // impure function's bare parameter, inferred in a pure one's.
    let files = [(
        "workspace.gcsl",
        "type N = Int\n\
             type F = Future<Int>\n\
             def impure(x: N) -> N { x }\n\
             def pure(x: N): Int { x }\n\
             def either(x: Int | F): Int { 1 }",
    )];
    assert_eq!(
        value(&files, "workspace.gcsl"),
        "{impure = fn(x: Int | Future<Int>) -> Int | Future<Int>; \
              pure = fn(x: Int): Int; either = fn(x: Int | Future<Int>): Int}"
    );
    assert_eq!(
        check_errors(&[(
            "workspace.gcsl",
            "type F = Future<Int>\nx: Int | F | String = 1"
        )]),
        ["a union with a `Future` member must be `T | Future<T>` at workspace.gcsl"]
    );
}

#[test]
fn a_type_expanded_too_deep_is_an_error_where_it_is_written() {
    // Each type nests the one before it a level deeper, so the chain
    // passes `MAX_DEPTH` though no type is written deeply: `T10000` is
    // one level too deep where it is written, and `T10001`, which uses
    // it, is erroneous wherever it is used.
    let chain: Vec<String> = (1..=10_001)
        .map(|at| format!("type T{at} = {{a: T{}}}", at - 1))
        .collect();
    let text = format!("type T0 = Int\n{}\nx: T10001 = 1", chain.join("\n"));
    let errors = std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(move || check_errors(&[("workspace.gcsl", &text)]))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(errors, ["expression nested too deeply at workspace.gcsl"]);
}

#[test]
fn a_value_file_declares_types() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/v.gcsl", "type T = {a: Int}\n= 5"),
        (
            "app.gcsl",
            "import lib/v\nimport T from lib/v\nx: v.T = { a = v }\ny: T = x",
        ),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{x = {a = 5}; y = {a = 5}}");
}

#[test]
fn dump_types_shows_each_type_with_its_expansion() {
    let mut workspace = checked(&TYPES);
    let index = workspace.file("_lib/types.gcsl").unwrap();
    assert_eq!(
        workspace.dump_types(index).unwrap(),
        "(file\n\
             \x20 (type PackageDef {version: String, arch: String, content: String} = \
             {arch: String, content: String, version: String} @0..81)\n\
             \x20 (type Package {...PackageDef, name: String} = {arch: String, content: String, \
             name: String, version: String} @82..128))"
    );
    // A use prints as written.
    let index = workspace.file("pkg/app.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(field p : Package @"), "{dump}");
    let index = workspace.file("workspace.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(f fn(): t.PackageDef)"), "{dump}");
    assert!(
        dump.contains("(type Steps List<fn() -> String> = List<fn() -> String | Future<String>> @"),
        "{dump}"
    );
    // A type defined as another prints as written, and dumps as what
    // both stand for.
    let mut workspace = checked(&[("workspace.gcsl", "type A = Int\ntype B = A")]);
    let index = workspace.file("workspace.gcsl").unwrap();
    assert_eq!(
        workspace.dump_types(index).unwrap(),
        "(file\n  (type A Int = Int @0..12)\n  (type B A = Int @13..23))"
    );
}
