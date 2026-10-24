//! Tests of workspaces: references to definitions, metas, and definitions with
//! `+` and digits in their paths.

use super::*;

/// The field of record `value` whose name is `name`.
pub(super) fn field_named(value: &Value, name: &str) -> Value {
    let Value::Record(record) = value else {
        panic!("{value} is no record");
    };
    let at = record.layout.texts.iter().position(|text| &**text == name);
    record.values[at.unwrap()].clone()
}

/// [`Workspace::definition_of`] as a pair of the def-type's path and the
/// definition's.
pub(super) fn named_by(workspace: &Workspace, value: &Value) -> Option<(String, String)> {
    workspace
        .definition_of(value)
        .map(|found| (found.def_type, found.path))
}

/// A `build` def-type whose meta names a `source`, and a `package` one
/// whose record holds a `build`, as pekit's are: in `lib.types`, which
/// `app` imports `build` from by another name.
pub(super) const BUILDS: [(&str, &str); 3] = [
    ("workspace.gcsl", "x = 1"),
    (
        "lib/types.gcsl",
        "#[def_type]\n\
         def source(name: String, f: fn(): String): String { f() }\n\
         #[def_type]\n\
         def build(name: String, f: fn(out: String) ->> String META: {source: fn(): String}, \
         out: String) ->> String { f(out) }\n\
         #[def_type]\n\
         def package(name: String, f: fn(): {build: fn(out: String) ->> String}): \
         {build: fn(out: String) ->> String} { f() }",
    ),
    (
        "app/a.gcsl",
        "import source, package from lib/types\n\
         import build as make from lib/types\n\
         def source up() { \"https://x\" }\n\
         defmeta make b { source = source up }\n\
         def make b(out) { out }\n\
         def package dev.x() { build = make b }\n\
         n = 1\n\
         plain = fn(out: String): String { out }\n\
         calls = fn(out: String) -> String { make b(out) }\n\
         builtin = len",
    ),
];

#[test]
fn a_reference_names_its_definition() {
    let mut workspace = rooted(&BUILDS).unwrap();
    // The reference an invocation returns names its def-type as declared,
    // not by the alias it is written with.
    let package = workspace.invoke("package", "dev.x", &[]).unwrap().value;
    let build = field_named(&package, "build");
    let named = Some(("build".to_owned(), "b".to_owned()));
    assert_eq!(named_by(&workspace, &build), named);
    // So does one read from a meta, which names the definition to read.
    let source = workspace.meta_field("build", "b", "source").unwrap();
    let source = source.unwrap().value;
    let named = Some(("source".to_owned(), "up".to_owned()));
    assert_eq!(named_by(&workspace, &source), named);
    // What it names invokes as the reference does.
    let invoked = workspace.invoke("source", "up", &[]).unwrap().value;
    assert_eq!(invoked.to_string(), "https://x");
    // It knows its directory, and so the path qualified by it, which names
    // this very definition whatever others share its path.
    let referenced = workspace.definition_of(&source).unwrap();
    assert_eq!(referenced.dir, "app");
    assert_eq!(referenced.qualified(), "app/up");
    let invoked = workspace.invoke("source", &referenced.qualified(), &[]);
    assert_eq!(invoked.unwrap().value.to_string(), "https://x");
    let source = workspace.meta_field("build", "app/b", "source").unwrap();
    assert_eq!(named_by(&workspace, &source.unwrap().value), named);
    // Nothing else names a definition: no other value, no closure that
    // calls one, no built-in function.
    let index = workspace.file("app/a.gcsl").unwrap();
    let file = workspace.run(index).unwrap().value;
    for name in ["n", "plain", "calls", "builtin"] {
        let value = field_named(&file, name);
        assert_eq!(named_by(&workspace, &value), None, "{name}");
    }
}

#[test]
fn a_closure_that_captures_a_definition_names_none() {
    // The def-type's closure captures `n` and then the definition, as a
    // reference's captures the def-type and then the definition, but it is
    // no reference.
    let files = [(
        "workspace.gcsl",
        "#[def_type]\n\
         def wrap(name: String, f: fn(): Int, n: Int): fn(): Int { fn(): Int { n + f() } }\n\
         def wrap w() { 1 }",
    )];
    let mut workspace = rooted(&files).unwrap();
    let n = ("n".to_owned(), Given::Int(1));
    let wrapped = workspace.invoke("wrap", "w", &[n]).unwrap().value;
    let Value::Fn(closure) = &wrapped else {
        panic!("{wrapped} is no closure");
    };
    assert!(matches!(closure.captures.get(1), Some(Value::Fn(_))));
    assert_eq!(named_by(&workspace, &wrapped), None);
}

#[test]
fn a_reference_made_in_a_child_names_an_ancestor_s_definition() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
             def tool(name: String, f: fn(): Int): Int { f() }\n\
             def tool t() { 1 }",
        ),
        ("app/deep/a.gcsl", "r = tool t"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let index = workspace.file("app/deep/a.gcsl").unwrap();
    let file = workspace.run(index).unwrap().value;
    let named = Some(("tool".to_owned(), "t".to_owned()));
    assert_eq!(named_by(&workspace, &field_named(&file, "r")), named);
}

pub(super) const TAGGED: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\n\
         def tagged(name: String, f: fn(): String META: {tag: String, n: Int}): String {\n\
         \x20   \"{name}: {plain(f)} {tag(f)} {f.n}\"\n\
         }\n\
         fn tag(g: fn(): String META: {tag: String}): String { g.tag }\n\
         fn plain(g: fn(): String): String { g() }",
);

#[test]
fn a_meta_is_a_leaf_of_its_directory_s_metas() {
    // The meta's leaf is its path as one name, under the meta key of its
    // def-type and directory; the definition and the meta are top-level
    // functions, and the definition's item mentions the meta's leaf and
    // function, by its read of its meta.
    let files = [
        TAGGED,
        (
            "app/a.gcsl",
            "def tagged dev.x() { \"x\" }\ndefmeta tagged dev.x { tag = \"t\"; n = 1 }",
        ),
    ];
    let scan = ws(&files).unwrap();
    let app = scan.files.iter().find(|f| f.path == "app/a.gcsl").unwrap();
    let tree = &app.mentions.tree;
    let meta_root = &tree.nodes[tree.roots[1] as usize];
    let key = scan.names.get(tree.nodes[tree.roots[0] as usize].name);
    assert_eq!(
        Some(scan.names.get(meta_root.name)),
        names::meta_key(key, app.dir).as_deref()
    );
    let leaf = &tree.nodes[meta_root.children[0] as usize];
    assert_eq!(scan.names.get(leaf.name), "dev.x");
    assert!(leaf.children.is_empty());
    assert_eq!(app.mentions.functions.len(), 2);
    assert_eq!(app.mentions.items[0], [1]);
    assert!(app.mentions.fields[0].contains(&meta_root.children[0]));
    assert!(app.unpaired.is_empty());
}

#[test]
fn a_meta_is_read_as_fields_of_its_definition() {
    // The meta above or below its definition, in its file or in another
    // of its directory, a block or an expression; read in pure functions,
    // one whose meta's type is less than the definition's and one that
    // forgets the meta.
    let files = [
        TAGGED,
        (
            "app/a.gcsl",
            "defmeta tagged x { tag = \"t-x\"; n = 1 }\n\
                 def tagged x() { \"x\" }\n\
                 def tagged y() { \"y\" }\n\
                 defmeta tagged y = { tag = \"t-y\"; n = 2 }",
        ),
        (
            "app/b.gcsl",
            "defmeta tagged dev.z { tag = \"t-z\"; n = 3 }",
        ),
        (
            "app/c.gcsl",
            "def tagged dev.z() { \"z\" }\n\
                 r = [tagged x(), tagged y(), tagged dev.z()]",
        ),
    ];
    assert_eq!(
        value(&files, "app/c.gcsl"),
        "{r = [\"x: x t-x 1\", \"y: y t-y 2\", \"dev.z: z t-z 3\"]}"
    );
    // A def-type with no meta works as it always did.
    let files = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }\nr = build x(\"m\")"),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = \"x: m\"}");
}

#[test]
fn each_marker_reads_its_meta_as_its_effect() {
    let path = std::env::temp_dir().join(format!("gcsl-slice-38-{}", std::process::id()));
    let path = path.to_str().unwrap();
    let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
    let _ = std::fs::remove_file(path);
    // `META->` runs its body at each read, `META->>` once for the run.
    let impure = "#[def_type]\n\
             def twice(name: String, f: fn(): Int META-> {n: Int}) -> List<Int> { [f.n, f.n] }";
    let app = format!(
        "defmeta twice x {{ n = exec([\"sh\", \"-c\", \"echo x >> {path}\"]).status }}\n\
             def twice x() {{ 1 }}\n\
             r = twice x()"
    );
    let files = [("workspace.gcsl", impure), ("app.gcsl", app.as_str())];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]}");
    assert_eq!(count(), 2);
    let memo = impure.replace("META->", "META->>");
    let files = [
        ("workspace.gcsl", memo.as_str()),
        ("app.gcsl", app.as_str()),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]}");
    assert_eq!(count(), 3);
    std::fs::remove_file(path).unwrap();
    // A `META: Future<...>` meta is read purely, giving a future, which
    // `defmeta ... = lazy` makes: its field is awaited, which only
    // impure code may.
    let future = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def later(name: String, f: fn(): Int META: Future<{v: Int}>) -> Int {\n\
                 \x20   f.v + f()\n\
                 }",
        ),
        (
            "app.gcsl",
            "defmeta later x = lazy { v = 41 }\ndef later x() { 1 }\nr = later x()",
        ),
    ];
    assert_eq!(value(&future, "app.gcsl"), "{r = 42}");
    let pure = future[0].1.replace(") -> Int", "): Int");
    let files = [("workspace.gcsl", pure.as_str()), future[1]];
    assert_eq!(
        check_errors(&files),
        [
            "cannot use a value that may be a future in a pure function at workspace.gcsl \
                 (this is Future<{v: Int}>); workspace.gcsl: this function is pure, declared \
                 with `:`; note: using the value here would await it, which may run effects; a \
                 pure function may hold and pass on a future but not await it. Declare the \
                 function with `->` to make it impure"
        ]
    );
    // Under `META->`, a field that may be a future is awaited where it is
    // used.
    let lazy = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def sum(name: String, f: fn(): Int META-> {ns: List<Int>}) -> Int {\n\
                 \x20   let mut total = f()\n\
                 \x20   for n in f.ns { total := total + n }\n\
                 \x20   total\n\
                 }",
        ),
        (
            "app.gcsl",
            "defmeta sum x { ns = lazy [1, 2, lazy 3] }\ndef sum x() { 4 }\nr = sum x()",
        ),
    ];
    assert_eq!(value(&lazy, "app.gcsl"), "{r = 10}");
}

#[test]
fn a_meta_s_caller_is_its_reader_s() {
    // The meta's `fileroot.this` is its own file's directory, and its
    // caller the directory of the code that reads it: the def-type's, or
    // through its `fileroot.helper()`, the def-type's own caller's.
    let files = [
        (
            "workspace.gcsl",
            "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn(): String META-> {this: String, caller: String}) \
                 -> String {\n\
                 \x20   \"{f.this} {f.caller}\"\n\
                 }\n\
                 #[def_type]\n\
                 def helped(name: String, f: fn(): String META-> {this: String, caller: String}) \
                 -> String {\n\
                 \x20   fileroot.helper()\n\
                 \x20   \"{f.this} {f.caller}\"\n\
                 }",
        ),
        (
            "a/a.gcsl",
            "import fileroot\n\
                 defmeta where x { this = fileroot.this; caller = fileroot.caller }\n\
                 def where x() { \"\" }\n\
                 defmeta helped x { this = fileroot.this; caller = fileroot.caller }\n\
                 def helped x() { \"\" }",
        ),
        ("a/sub/c.gcsl", "r = [where x(), helped x()]"),
    ];
    assert_eq!(
        rooted_text(&files, "a/sub/c.gcsl"),
        "{r = [\"<root>/a <root>\", \"<root>/a <root>/a/sub\"]}"
    );
}

#[test]
fn a_meta_pairs_with_its_definition_in_its_directory() {
    // A `#[shadowable]` definition shadowed in a subdirectory, which
    // brings its own meta: each view has its own pair.
    let files = [
        (
            "workspace.gcsl",
            format!(
                "{}\n#[shadowable]\ndef tagged s() {{ \"top\" }}\n\
                     defmeta tagged s {{ tag = \"top-meta\"; n = 0 }}",
                TAGGED.1
            ),
        ),
        ("root.gcsl", "r = tagged s()".to_owned()),
        (
            "sub/a.gcsl",
            "def tagged s() { \"sub\" }\n\
                 defmeta tagged s { tag = \"sub-meta\"; n = 1 }\n\
                 below = tagged s()"
                .to_owned(),
        ),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    assert_eq!(value(&files, "root.gcsl"), "{r = \"s: top top-meta 0\"}");
    assert_eq!(
        value(&files, "sub/a.gcsl"),
        "{below = \"s: sub sub-meta 1\"}"
    );
    // A chain of `#[shadowable]` definitions in one directory has one
    // meta.
    let files = [
        TAGGED,
        ("app/a.gcsl", "#[shadowable]\ndef tagged c() { \"first\" }"),
        (
            "app/b.gcsl",
            "def tagged c() { \"second\" }\n\
                 defmeta tagged c { tag = \"one\"; n = 1 }\n\
                 r = tagged c()",
        ),
    ];
    assert_eq!(value(&files, "app/b.gcsl"), "{r = \"c: second one 1\"}");
    // A meta in a subdirectory of its definition is no meta of it: the
    // definition has none in its own directory either.
    let files = [
        TAGGED,
        ("app/a.gcsl", "def tagged x() { \"x\" }"),
        ("app/sub/b.gcsl", "defmeta tagged x { tag = \"t\"; n = 1 }"),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "`tagged x` has no `defmeta` in its directory at app/a.gcsl; note: `tagged` \
                 declares a meta, `META: {tag: String, n: Int}`, for each of its definitions",
            "`defmeta tagged x` has no definition `x` in its directory at app/sub/b.gcsl"
        ]
    );
    let files = [TAGGED, ("app/a.gcsl", "def tagged x() { \"x\" }")];
    assert_eq!(
        check_errors(&files),
        [
            "`tagged x` has no `defmeta` in its directory at app/a.gcsl; note: `tagged` \
                 declares a meta, `META: {tag: String, n: Int}`, for each of its definitions"
        ]
    );
}

#[test]
fn a_meta_is_invoked_with_its_definition() {
    // The meta in another file of the definition's directory: invoking
    // the definition by name runs that file too.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def tag(name: String, f: fn(): String META->> {t: String}) -> String {\n\
                 \x20   \"{f()} {f.t}\"\n\
                 }",
        ),
        ("pkg/a.gcsl", "defmeta tag p { t = \"meta\" }"),
        ("pkg/p.gcsl", "def tag p() { \"p\" }"),
    ];
    assert_eq!(invoked_text(&files, "tag", "p"), "p meta");
}

#[test]
fn meta_errors() {
    // Each error with its labels and notes, joined by `; `, the errors
    // by newlines.
    let errors = |text: &str| {
        let files = [TAGGED, ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    assert_eq!(
        errors("x: fn(): Int META: Int = 1"),
        "a meta must be a record type: its fields are read from the function at app.gcsl"
    );
    assert_eq!(
        errors("x: fn(): Int META-> Future<List<Int>> = 1"),
        "a meta must be a record type: its fields are read from the function at app.gcsl"
    );
    assert_eq!(
        errors("fn f(g: fn(): Int META: {a: Int, b: Int}): Int { g.c }"),
        "the meta of `g` has no field `c` at app.gcsl; note: the fields are `a` and `b`"
    );
    assert_eq!(
        errors("fn f(g: fn(): Int META-> {a: Int}): Int {\nlet held = g.a\n1\n}"),
        "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: reading the \
             meta of `g` is impure (`META->`); app.gcsl: this function is pure, declared with \
             `:`; note: a pure function, declared with `:`, may read only pure metas, `META:`; \
             declare it with `->` to make it impure"
    );
    // In a memoised function, as calling an impure function is.
    let memo = [
        TAGGED,
        (
            "app.gcsl",
            "fn f(g: fn(): Int META-> {a: Int}) ->> Int { g.a }\n\
                 fn h(g: fn(): Int META->> {a: Int}) -> Int { g.a }",
        ),
    ];
    assert_eq!(value(&memo, "app.gcsl"), "{}");
    // A function with no meta has no fields, as before.
    assert_eq!(
        errors("fn f(g: fn(): Int): Int { g.a }"),
        "cannot access field `a` of `fn(): Int` at app.gcsl; app.gcsl: this is fn(): Int"
    );
    // A function with no meta, or with a meta of a greater marker, does
    // not fit a type with one.
    assert_eq!(
        errors("fn f(g: fn(): Int META: {a: Int}): Int { g.a }\nx = f(fn(): Int { 1 })"),
        "mismatched types at app.gcsl (expected fn(): Int META: {a: Int}, found fn(): Int); \
             app.gcsl: the function is fn(g: fn(): Int META: {a: Int}): Int"
    );
    assert_eq!(
        errors(
            "fn f(g: fn(): Int META: {a: Int}): Int { g.a }\n\
                 fn h(g: fn(): Int META-> {a: Int}): Int { f(g) }"
        ),
        "mismatched types at app.gcsl (expected fn(): Int META: {a: Int}, found fn(): Int \
             META-> {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>); app.gcsl: the \
             function is fn(g: fn(): Int META: {a: Int}): Int"
    );
    // The pairing.
    assert_eq!(
        errors(
            "def tagged x() { \"x\" }\n\
                 defmeta tagged x { tag = \"a\"; n = 1 }\n\
                 defmeta tagged x { tag = \"b\"; n = 2 }\n\
                 defmeta tagged y { tag = \"c\"; n = 3 }"
        ),
        "`defmeta tagged x` is declared twice in this directory at app.gcsl; app.gcsl: \
             first declared here\n\
             `defmeta tagged y` has no definition `y` in its directory at app.gcsl"
    );
    assert_eq!(
        errors(
            "#[def_type]\ndef plain(name: String, f: fn(): Int): Int { f() }\n\
                 def plain p() { 1 }\n\
                 defmeta plain p { tag = \"x\" }"
        ),
        "`plain` declares no meta for its definitions at app.gcsl; note: a def-type \
             declares one by its definition parameter's type, as in `func: fn(): PackageDef \
             META: PackageMeta`"
    );
    assert_eq!(
        errors("defmeta nope x { a = 1 }"),
        "undefined variable `nope` at app.gcsl"
    );
    // The meta's body is checked against the meta's type, with its marker.
    assert_eq!(
        errors("def tagged x() { \"x\" }\ndefmeta tagged x { tag = 1; n = 1 }"),
        "mismatched types at app.gcsl (expected String, found Int); app.gcsl: the return \
             type is declared here"
    );
    assert_eq!(
        errors(
            "def tagged x() { \"x\" }\n\
                 defmeta tagged x { tag = exec([\"true\"]).stdout; n = 1 }"
        ),
        "cannot call an impure function from a pure function at app.gcsl; app.gcsl: this \
             is fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}; \
             app.gcsl: this function is pure, declared with `:`; note: a pure function, \
             declared with `:`, may call only pure functions; declare it with `->` to make it \
             impure"
    );
    // A definition needs its meta: one that reads a variable bound later
    // is shown as written in the chain.
    let found = errors(
        "r = tagged x()\nlet k = 1\ndefmeta tagged x { tag = \"t\"; n = k }\n\
             def tagged x() { \"x\" }",
    );
    assert!(
        found.contains("uses `defmeta tagged x`, and `defmeta tagged x` reads `k`"),
        "{found}"
    );
}

/// A def-type of the root of a workspace, for the tests of reading a
/// meta through a reference: `info`, whose definitions give a number and
/// carry a pure meta of a record, a list and a string.
pub(super) const INFO: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\n\
         def info(name: String, f: fn(): Int META: {about: {owner: String}, ns: List<Int>, \
         peios: String}): Int {\n\
         \x20   f() + len(f.ns)\n\
         }",
);

#[test]
fn a_reference_reads_its_definition_s_meta() {
    // In parentheses and by a path past the definition, deeper reads, a
    // reference stored and passed as a function with less of the meta,
    // in a pure function: the meta is pure.
    let def_types = format!("{}\n{}", INFO.1, TAGGED.1);
    let files = [
        ("workspace.gcsl", def_types.as_str()),
        (
            "app/a.gcsl",
            "defmeta info dev.x { about = { owner = \"jack\" }; ns = [4, 5]; peios = \"p\" }\n\
                 def info dev.x() { 1 }\n\
                 defmeta tagged dev.x { tag = \"t\"; n = 7 }\n\
                 def tagged dev.x() { \"x\" }\n\
                 fn owner(): String { info dev.x.about.owner }\n\
                 fn tag(g: fn(): String META: {tag: String}): String { g.tag }\n\
                 r = [\n\
                 \x20   info dev.x.peios,\n\
                 \x20   (info dev.x).peios,\n\
                 \x20   owner(),\n\
                 \x20   \"{info dev.x.ns[1]}\",\n\
                 \x20   \"{(info dev.x).ns[0]}\",\n\
                 ]\n\
                 let stored = tagged dev.x\n\
                 s = [stored.tag, tag(stored), tag(tagged dev.x), \"{stored.n}\"]\n\
                 called = info dev.x()",
        ),
    ];
    assert_eq!(
        value(&files, "app/a.gcsl"),
        "{r = [\"p\", \"p\", \"jack\", \"5\", \"4\"]; s = [\"t\", \"t\", \"t\", \"7\"]; \
             called = 3}"
    );
    // Under `META->`, each field past the definition that may be a future
    // is awaited where the next reads it, a lazy one included.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def t(name: String, f: fn(): Int META-> {about: {owner: String}}) -> Int { f() }",
        ),
        (
            "app.gcsl",
            "defmeta t x { about = { owner = \"o\" } }\n\
                 def t x() { 1 }\n\
                 defmeta t y { about = lazy { owner = lazy \"lazy\" } }\n\
                 def t y() { 2 }\n\
                 r = \"{t x.about.owner} {t y.about.owner} {(t y).about.owner}\"",
        ),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = \"o lazy lazy\"}");
}

#[test]
fn a_reference_means_its_longest_definition() {
    // `info dev` is a definition, and so is `tagged dev.peios.x`: each
    // path means the definition of its own def-type, `info
    // dev.peios` the field `peios` of `info dev`'s meta.
    let def_types = format!("{}\n{}", INFO.1, TAGGED.1);
    let files = [
        ("workspace.gcsl", def_types.as_str()),
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"meta of dev\" }\n\
                 def info dev() { 1 }\n\
                 defmeta tagged dev.peios.x { tag = \"t\"; n = 1 }\n\
                 def tagged dev.peios.x() { \"x\" }\n\
                 r = [info dev.peios, tagged dev.peios.x.tag]",
        ),
    ];
    assert_eq!(
        value(&files, "app/a.gcsl"),
        "{r = [\"meta of dev\", \"t\"]}"
    );
    // A name past the definition that is no field of its meta: the
    // meta's error, noted with how the path was read.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = info dev.peios.x",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        ["cannot access field `x` of String at app/a.gcsl; app/a.gcsl: this is String"]
    );
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = info dev.pieos.x",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "the meta of `info dev` has no field `pieos` at app/a.gcsl; note: the fields \
                 are `about`, `ns` and `peios`; note: `info dev.pieos` is read as the field \
                 `pieos` of the meta of `info dev`, its longest prefix that is a definition"
        ]
    );
    // In parentheses the reference is all there is: no second note.
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
        check_errors(&files),
        [
            "the meta of `info dev` has no field `pieos` at app/a.gcsl; note: the fields \
                 are `about`, `ns` and `peios`"
        ]
    );
    // A path no prefix of which is a definition: as before.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev.x { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev.x() { 1 }\n\
                 r = info dev.y.peios\n\
                 s = info dev",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "`info` has no definition `dev.y.peios` at app/a.gcsl; note: the only \
                 definition of `info` here is `dev.x`",
            "`dev` is not a `info` definition at app/a.gcsl"
        ]
    );
    // The result of a call has the fields of the result, not of the meta.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev()).peios",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        ["cannot access field `peios` of Int at app/a.gcsl; app/a.gcsl: this is Int"]
    );
}

#[test]
fn a_def_type_without_a_meta_reads_no_path_past_a_definition() {
    // `build x.y` names the definition `x.y`, as it always did.
    let files = [BUILD, ("app.gcsl", "def build x(t) { t }\nr = build x.y")];
    assert_eq!(
        check_errors(&files),
        [
            "`build` has no definition `x.y` at app.gcsl; note: the only definition of \
                 `build` here is `x`"
        ]
    );
    // Nor does its reference have a meta, nor any field.
    let files = [BUILD, ("app.gcsl", "def build x(t) { t }\nr = (build x).y")];
    assert_eq!(
        check_errors(&files),
        [
            "cannot access field `y` of `fn(mode: String): String` at app.gcsl; app.gcsl: \
                 this is fn(mode: String): String"
        ]
    );
}

#[test]
fn a_meta_read_through_a_reference_needs_the_meta() {
    // The meta in one file, the definition in another, the read in a
    // directory below: each runs before what reads it.
    let files = [
        INFO,
        (
            "pkg/m.gcsl",
            "defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = \"m\" }",
        ),
        ("pkg/d.gcsl", "def info x() { 1 }"),
        ("pkg/sub/app.gcsl", "r = info x.peios"),
    ];
    assert_eq!(value(&files, "pkg/sub/app.gcsl"), "{r = \"m\"}");
    // Read in the definition's file, above its meta, which reads a field
    // of the file.
    let files = [
        INFO,
        (
            "pkg/d.gcsl",
            "r = info x.peios\n\
                 def info x() { 1 }\n\
                 defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = owner }\n\
                 owner = \"later\"",
        ),
    ];
    assert_eq!(
        value(&files, "pkg/d.gcsl"),
        "{r = \"later\"; owner = \"later\"}"
    );
    // A meta that reads a variable bound below the read cannot be read
    // there: the chain names the definition and its meta.
    let files = [
        INFO,
        (
            "app.gcsl",
            "r = info x.peios\n\
                 let k = \"k\"\n\
                 def info x() { 1 }\n\
                 defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = k }",
        ),
    ];
    let found = check_errors(&files).join("\n");
    assert!(
        found.starts_with("`info x` cannot be used here yet at app.gcsl")
            && found.contains("`info x` uses `defmeta info x`, and `defmeta info x` reads `k`"),
        "{found}"
    );
}

#[test]
fn a_meta_read_through_a_reference_is_the_definition_s_meta() {
    let path = std::env::temp_dir().join(format!("gcsl-slice-39-{}", std::process::id()));
    let path = path.to_str().unwrap();
    let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
    let _ = std::fs::remove_file(path);
    // `META->>`: one run for the def-type's two reads and the
    // references' three, in parentheses, past the definition and
    // stored; at the top level of a file, which is impure.
    let memo = "#[def_type]\n\
             def twice(name: String, f: fn(): Int META->> {n: Int}) -> List<Int> { [f.n, f.n] }";
    let app = format!(
        "defmeta twice x {{ n = exec([\"sh\", \"-c\", \"echo x >> {path}\"]).status }}\n\
             def twice x() {{ 1 }}\n\
             r = twice x()\n\
             let stored = twice x\n\
             s = [(twice x).n, twice x.n, stored.n]"
    );
    let files = [("workspace.gcsl", memo), ("app.gcsl", app.as_str())];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]; s = [0, 0, 0]}");
    assert_eq!(count(), 1);
    // `META->`: one run for each read.
    let impure = memo.replace("META->>", "META->");
    let files = [
        ("workspace.gcsl", impure.as_str()),
        ("app.gcsl", app.as_str()),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]; s = [0, 0, 0]}");
    assert_eq!(count(), 6);
    std::fs::remove_file(path).unwrap();
    // In a pure function, the read is the error, however the reference
    // is written; in an impure one, it is allowed.
    let files = [
        ("workspace.gcsl", impure.as_str()),
        (
            "app.gcsl",
            "defmeta twice x { n = 1 }\n\
                 def twice x() { 1 }\n\
                 fn a(): Int {\nlet held = twice x.n\n1\n}\n\
                 fn b(): Int {\nlet held = (twice x).n\n1\n}\n\
                 fn c() -> Int { twice x.n }",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: \
                 reading the meta of `twice x` is impure (`META->`); app.gcsl: this function is \
                 pure, declared with `:`; note: a pure function, declared with `:`, may read \
                 only pure metas, `META:`; declare it with `->` to make it impure",
            "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: \
                 reading the meta of `twice x` is impure (`META->`); app.gcsl: this function is \
                 pure, declared with `:`; note: a pure function, declared with `:`, may read \
                 only pure metas, `META:`; declare it with `->` to make it impure"
        ]
    );
}

#[test]
fn dumps_show_a_reference_past_its_definition() {
    let mut workspace = checked(&[
        INFO,
        (
            "app.gcsl",
            "defmeta info x { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info x() { 1 }\n\
                 r = info x.about.owner",
        ),
    ]);
    let index = workspace.file("app.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    let meta = "META: {about: {owner: String}, ns: List<Int>, peios: String}";
    assert!(
        dump.contains(&format!(
            "(field r @84..106\n    (. owner : String @88..106\n      (. about : {{owner: \
                 String}} @88..100\n        (def-type-ref info x : fn(): Int {meta} @88..94))))"
        )),
        "{dump}"
    );
    // The syntax tree alone has the reference as it is written.
    let source = SourceFile::new("app.gcsl", "r = info x.about.owner").unwrap();
    let (tokens, names, strings) = crate::lexer::tokenize(&source).unwrap();
    let ast = crate::parser::parse(&tokens, names, strings, &source).unwrap();
    assert!(
        ast.pretty()
            .to_string()
            .contains("(def-type-ref info x.about.owner @4..22)"),
        "{}",
        ast.pretty()
    );
    // The reads of the meta read it through the definition's cell.
    let ir = workspace.dump_ir(index).unwrap();
    assert!(
        ir.contains("(field owner @93..106\n      (field about @93..100\n        (call @93..100\n"),
        "{ir}"
    );
}

/// A workspace whose definitions have `+` in their paths' names: the
/// names of real packages, `a+b` and `c++11`, defined with their metas in
/// two files of one directory, `pkgs`, and referred to from a third file
/// of it and from a directory below it, called and read past.
pub(super) const PLUS: [(&str, &str); 5] = [
    (
        "workspace.gcsl",
        "#[def_type]\n\
             def package(name: String, f: fn(v: String): String META: {versions: List<String>}, \
             version: String) -> String { \"{name} {f(version)}\" }",
    ),
    (
        "pkgs/gnu.gcsl",
        "defmeta package org.gnu.gmp-c++ { versions = [\"6.3.0\"] }\n\
             def package org.gnu.gmp-c++(v) { \"gmp {v}\" }\n\
             defmeta package org.gnu.gcc-c++ { versions = [\"14\"] }\n\
             def package org.gnu.gcc-c++(v) { \"g++ {v}\" }\n\
             defmeta package org.gnu.gcc-libstdc++ { versions = [\"14.1\"] }\n\
             def package org.gnu.gcc-libstdc++(v) { \"libstdc++ {v}\" }\n\
             defmeta package org.gnu.gcc-libstdc++-devel { versions = [\"14.2\"] }\n\
             def package org.gnu.gcc-libstdc++-devel(v) { \"devel {v}\" }",
    ),
    (
        "pkgs/more.gcsl",
        "defmeta package dev.peios.build-essentials-c++ { versions = [\"1\"] }\n\
             def package dev.peios.build-essentials-c++(v) { \"essentials {v}\" }\n\
             defmeta package org.gnu.a+b { versions = [] }\n\
             def package org.gnu.a+b(v) { \"a+b {v}\" }\n\
             defmeta package c++11 { versions = [\"11\"] }\n\
             def package c++11(v) { \"c++11 {v}\" }",
    ),
    (
        "pkgs/use.gcsl",
        "x = package org.gnu.gmp-c++(\"6.3.0\")\n\
             y = package org.gnu.gmp-c++.versions\n\
             gxx = package org.gnu.gcc-c++.versions[0]\n\
             lib = package org.gnu.gcc-libstdc++(\"1\")\n\
             devel = (package org.gnu.gcc-libstdc++-devel).versions\n\
             essentials = package dev.peios.build-essentials-c++(\"2\")\n\
             ab = package org.gnu.a+b(\"3\")\n\
             c11 = package c++11.versions\n\
             z = 1 + 2\n\
             let a = 4\n\
             let b = 5\n\
             c = a+b",
    ),
    (
        "pkgs/sub/below.gcsl",
        "let f = package org.gnu.gcc-libstdc++-devel\n\
             below = f(\"4\")\n\
             none = package org.gnu.a+b.versions",
    ),
];

#[test]
fn definitions_with_plus_in_their_paths_are_ordinary_definitions() {
    // Referred to from another file of their directory and from one
    // below it: called, with their paths as written, and their metas
    // read; `a+b` anywhere else is the addition it always was.
    assert_eq!(
        rooted_text(&PLUS, "pkgs/use.gcsl"),
        "{x = \"org.gnu.gmp-c++ gmp 6.3.0\"; y = [\"6.3.0\"]; gxx = \"14\"; \
             lib = \"org.gnu.gcc-libstdc++ libstdc++ 1\"; devel = [\"14.2\"]; \
             essentials = \"dev.peios.build-essentials-c++ essentials 2\"; \
             ab = \"org.gnu.a+b a+b 3\"; c11 = [\"11\"]; z = 3; c = 9}"
    );
    assert_eq!(
        rooted_text(&PLUS, "pkgs/sub/below.gcsl"),
        "{below = \"org.gnu.gcc-libstdc++-devel devel 4\"; none = []}"
    );
    // Listed, invoked and their metas read by name, by their paths as
    // written.
    let mut workspace = rooted(&PLUS).unwrap();
    assert_eq!(
        listed(&workspace, &PLUS),
        [
            "package in workspace.gcsl: org.gnu.gmp-c++ in pkgs/gnu.gcsl, \
                 org.gnu.gcc-c++ in pkgs/gnu.gcsl, org.gnu.gcc-libstdc++ in pkgs/gnu.gcsl, \
                 org.gnu.gcc-libstdc++-devel in pkgs/gnu.gcsl, \
                 dev.peios.build-essentials-c++ in pkgs/more.gcsl, \
                 org.gnu.a+b in pkgs/more.gcsl, c++11 in pkgs/more.gcsl"
        ]
    );
    for (path, value) in [
        ("org.gnu.gmp-c++", "gmp"),
        ("org.gnu.gcc-c++", "g++"),
        ("org.gnu.gcc-libstdc++", "libstdc++"),
        ("org.gnu.gcc-libstdc++-devel", "devel"),
        ("dev.peios.build-essentials-c++", "essentials"),
        ("org.gnu.a+b", "a+b"),
        ("c++11", "c++11"),
    ] {
        let outcome = workspace.invoke("package", path, &[string("version", "9")]);
        assert_eq!(
            outcome.unwrap().value.to_string(),
            format!("{path} {value} 9")
        );
    }
    let versions = workspace.meta_field("package", "org.gnu.gmp-c++", "versions");
    assert_eq!(versions.unwrap().unwrap().value.to_string(), "[\"6.3.0\"]");
    // A path that names none is the error it is, its definitions listed
    // as written.
    assert_eq!(
        request_error_with(&PLUS, ("package", "org.gnu.gmp-c"), &[("version", "1")]),
        "`package` has no definition `org.gnu.gmp-c`; note: its definitions are \
             `c++11`, `dev.peios.build-essentials-c++`, `org.gnu.a+b`, `org.gnu.gcc-c++`, \
             `org.gnu.gcc-libstdc++`, `org.gnu.gcc-libstdc++-devel`, `org.gnu.gmp-c++`"
    );
    // The dumps show them as written, of files not yet run.
    let mut workspace = rooted(&PLUS).unwrap();
    let index = workspace.file("pkgs/sub/below.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(
        dump.contains("(def-type-ref package org.gnu.gcc-libstdc++-devel : fn(version:"),
        "{dump}"
    );
    assert!(
        dump.contains("(def-type-ref package org.gnu.a+b : fn(version:"),
        "{dump}"
    );
    let index = workspace.file("pkgs/gnu.gcsl").unwrap();
    let ir = workspace.dump_ir(index).unwrap();
    assert!(ir.contains("fn #1 package org.gnu.gmp-c++ meta #"), "{ir}");
}

#[test]
fn definitions_with_plus_in_their_paths_conflict_as_any_do() {
    // Two of one path in one directory: the language's error, the path
    // shown as written.
    let one = [
        PACKAGE,
        (
            "a.gcsl",
            "def package x.a+b() { \"1\" }\ndef package x.a+b() { \"2\" }",
        ),
    ];
    assert_eq!(
        check_errors(&one),
        ["field `package x.a+b` is defined twice at a.gcsl; a.gcsl: first defined here"]
    );
    // In two sibling directories, which see neither the other: named with
    // its directory when invoked by name.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package x.a+b() { \"a\" }"),
        ("b/y.gcsl", "def package x.a+b() { \"b\" }"),
    ];
    assert_eq!(
        request_error(&siblings, "package", "x.a+b"),
        "`package x.a+b` is defined in 2 directories, so it is named with its directory; \
             note: name one of `a/x.a+b`, `b/x.a+b`"
    );
    assert_eq!(invoked_text(&siblings, "package", "b/x.a+b"), "x.a+b b");
    // One lying under another in a directory above, which it shadows.
    let under = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package x.a+b() { \"a\" }",
        ),
        ("b/y.gcsl", "def package x.a+b.c() { \"b\" }"),
    ];
    assert_eq!(
        listing_errors(&under),
        [format!(
            "`package x.a+b.c` lies under another `package` definition at b/y.gcsl; \
                 workspace.gcsl: `x.a+b` is defined here; {UNDER}"
        )]
    );
    // `a+b` is a name of its own, under no `a`.
    let apart = [
        PACKAGE,
        ("a/x.gcsl", "def package x.a() { \"a\" }"),
        ("b/y.gcsl", "def package x.a+b() { \"b\" }"),
    ];
    assert_eq!(invoked_text(&apart, "package", "x.a"), "x.a a");
    assert_eq!(invoked_text(&apart, "package", "x.a+b"), "x.a+b b");
    // A reference followed by `+` and a space is added to, as it was.
    let added = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef package(name: String, func: fn(): Int) -> Int { func() }",
        ),
        ("a.gcsl", "def package x.a() { 1 }\nr = package x.a + 1"),
    ];
    assert_eq!(
        check_errors(&added),
        ["cannot add `fn() -> Int` and Int at a.gcsl; a.gcsl: fn() -> Int; a.gcsl: Int"]
    );
}

/// A workspace whose definitions have names starting with a digit in
/// their paths: the names of real toolchain lanes, a float's split at its
/// `.`, an integer's glued to a name, written with `_` and with an
/// exponent, defined with their metas in two files of one directory,
/// `pkgs`, and referred to from a third file of it and from a directory
/// below it, called and read past.
pub(super) const DIGITS: [(&str, &str); 5] = [
    (
        "workspace.gcsl",
        "#[def_type]\n\
             def package(name: String, f: fn(v: String): String META: {versions: List<String>}, \
             version: String) -> String { \"{name} {f(version)}\" }",
    ),
    (
        "pkgs/lanes.gcsl",
        "defmeta package org.golang.go-stage0-1.24 { versions = [\"1.24.0\"] }\n\
             def package org.golang.go-stage0-1.24(v) { \"go {v}\" }\n\
             defmeta package org.rust-lang.rust-1.83 { versions = [\"1.83.0\"] }\n\
             def package org.rust-lang.rust-1.83(v) { \"rust {v}\" }",
    ),
    (
        "pkgs/more.gcsl",
        "defmeta package x.1.83.0 { versions = [\"0\"] }\n\
             def package x.1.83.0(v) { \"x {v}\" }\n\
             defmeta package x.24rc1 { versions = [] }\n\
             def package x.24rc1(v) { \"rc {v}\" }\n\
             defmeta package x.1.83rc1 { versions = [\"rc1\"] }\n\
             def package x.1.83rc1(v) { \"rc1 {v}\" }\n\
             defmeta package x.1_0 { versions = [\"10\"] }\n\
             def package x.1_0(v) { \"ten {v}\" }\n\
             defmeta package x.1e5 { versions = [\"1e5\"] }\n\
             def package x.1e5(v) { \"exp {v}\" }",
    ),
    (
        "pkgs/use.gcsl",
        "a = package org.golang.go-stage0-1.24(\"1.24.0\")\n\
             b = package org.rust-lang.rust-1.83.versions\n\
             c = package x.1.83.0(\"9\")\n\
             d = package x.24rc1.versions\n\
             e = package x.1.83rc1(\"1\")\n\
             f = package x.1_0.versions[0]\n\
             g = package x.1e5(\"2\")",
    ),
    (
        "pkgs/sub/below.gcsl",
        "let r = package org.rust-lang.rust-1.83\n\
             below = r(\"1.83.0\")\n\
             none = package x.24rc1.versions",
    ),
];

#[test]
fn definitions_with_digits_in_their_paths_are_ordinary_definitions() {
    // Referred to from another file of their directory and from one
    // below it: called, their def-type given their paths as written,
    // and their metas read.
    assert_eq!(
        rooted_text(&DIGITS, "pkgs/use.gcsl"),
        "{a = \"org.golang.go-stage0-1.24 go 1.24.0\"; b = [\"1.83.0\"]; \
             c = \"x.1.83.0 x 9\"; d = []; e = \"x.1.83rc1 rc1 1\"; f = \"10\"; \
             g = \"x.1e5 exp 2\"}"
    );
    assert_eq!(
        rooted_text(&DIGITS, "pkgs/sub/below.gcsl"),
        "{below = \"org.rust-lang.rust-1.83 rust 1.83.0\"; none = []}"
    );
    // Listed, invoked and their metas read by name, by their paths as
    // written.
    let mut workspace = rooted(&DIGITS).unwrap();
    assert_eq!(
        listed(&workspace, &DIGITS),
        [
            "package in workspace.gcsl: org.golang.go-stage0-1.24 in pkgs/lanes.gcsl, \
                 org.rust-lang.rust-1.83 in pkgs/lanes.gcsl, x.1.83.0 in pkgs/more.gcsl, \
                 x.24rc1 in pkgs/more.gcsl, x.1.83rc1 in pkgs/more.gcsl, \
                 x.1_0 in pkgs/more.gcsl, x.1e5 in pkgs/more.gcsl"
        ]
    );
    for (path, value) in [
        ("org.golang.go-stage0-1.24", "go"),
        ("org.rust-lang.rust-1.83", "rust"),
        ("x.1.83.0", "x"),
        ("x.24rc1", "rc"),
        ("x.1.83rc1", "rc1"),
        ("x.1_0", "ten"),
        ("x.1e5", "exp"),
    ] {
        let outcome = workspace.invoke("package", path, &[string("version", "9")]);
        assert_eq!(
            outcome.unwrap().value.to_string(),
            format!("{path} {value} 9")
        );
    }
    let versions = workspace.meta_field("package", "org.golang.go-stage0-1.24", "versions");
    assert_eq!(versions.unwrap().unwrap().value.to_string(), "[\"1.24.0\"]");
    let versions = workspace.meta_field("package", "org.rust-lang.rust-1.83", "versions");
    assert_eq!(versions.unwrap().unwrap().value.to_string(), "[\"1.83.0\"]");
    // A path that names none is the error it is, its definitions listed
    // as written; a number's value is not its name.
    assert_eq!(
        request_error_with(
            &DIGITS,
            ("package", "org.golang.go-stage0-1.2"),
            &[("version", "1")]
        ),
        "`package` has no definition `org.golang.go-stage0-1.2`; note: its definitions are \
             `org.golang.go-stage0-1.24`, `org.rust-lang.rust-1.83`, `x.1.83.0`, `x.1.83rc1`, \
             `x.1_0`, `x.1e5`, `x.24rc1`"
    );
    assert_eq!(
        request_error_with(&DIGITS, ("package", "x.10"), &[("version", "1")]),
        "`package` has no definition `x.10`; note: its definitions are \
             `org.golang.go-stage0-1.24`, `org.rust-lang.rust-1.83`, `x.1.83.0`, `x.1.83rc1`, \
             `x.1_0`, `x.1e5`, `x.24rc1`"
    );
    // The dumps show them as written, of files not yet run.
    let mut workspace = rooted(&DIGITS).unwrap();
    let index = workspace.file("pkgs/sub/below.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(
        dump.contains("(def-type-ref package org.rust-lang.rust-1.83 : fn(version:"),
        "{dump}"
    );
    assert!(
        dump.contains("(def-type-ref package x.24rc1 : fn(version:"),
        "{dump}"
    );
    let index = workspace.file("pkgs/lanes.gcsl").unwrap();
    let ir = workspace.dump_ir(index).unwrap();
    assert!(
        ir.contains("fn #1 package org.golang.go-stage0-1.24 meta #"),
        "{ir}"
    );
}

#[test]
fn definitions_with_digits_in_their_paths_conflict_as_any_do() {
    // In one directory, a definition under another, and two of one path:
    // the language's errors, the paths shown as written.
    let under = [
        PACKAGE,
        (
            "a.gcsl",
            "def package x.1() { \"1\" }\ndef package x.1.83() { \"2\" }",
        ),
    ];
    assert_eq!(
        check_errors(&under),
        [
            "cannot define `package x.1.83`: `package x.1` is `fn(): String`, not a record \
                 at a.gcsl; a.gcsl: `package x.1` defined here"
        ]
    );
    let twice = [
        PACKAGE,
        (
            "a.gcsl",
            "def package x.1.83() { \"1\" }\ndef package x.1.83() { \"2\" }",
        ),
    ];
    assert_eq!(
        check_errors(&twice),
        ["field `package x.1.83` is defined twice at a.gcsl; a.gcsl: first defined here"]
    );
    // In two sibling directories, which see neither the other: named with
    // its directory when invoked by name.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package x.1.83() { \"a\" }"),
        ("b/y.gcsl", "def package x.1.83() { \"b\" }"),
    ];
    assert_eq!(
        request_error(&siblings, "package", "x.1.83"),
        "`package x.1.83` is defined in 2 directories, so it is named with its directory; \
             note: name one of `a/x.1.83`, `b/x.1.83`"
    );
    assert_eq!(invoked_text(&siblings, "package", "b/x.1.83"), "x.1.83 b");
    // One lying under another in a directory above, which it shadows.
    let under = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package x.1() { \"a\" }",
        ),
        ("b/y.gcsl", "def package x.1.83() { \"b\" }"),
    ];
    assert_eq!(
        listing_errors(&under),
        [format!(
            "`package x.1.83` lies under another `package` definition at b/y.gcsl; \
                 workspace.gcsl: `x.1` is defined here; {UNDER}"
        )]
    );
    // `10` is a name of its own, under no `1`.
    let apart = [
        PACKAGE,
        ("a/x.gcsl", "def package x.1() { \"a\" }"),
        ("b/y.gcsl", "def package x.10() { \"b\" }"),
    ];
    assert_eq!(invoked_text(&apart, "package", "x.1"), "x.1 a");
    assert_eq!(invoked_text(&apart, "package", "x.10"), "x.10 b");
}
