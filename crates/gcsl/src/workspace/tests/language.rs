//! Tests of workspaces: attributes, generics, methods, enums, contexts,
//! `assert`, `recover` and secrets across files.

use super::*;

/// The attributes of the closure that field `name` of `value`, a record,
/// holds, each as its name and its arguments shown, `on = true`.
pub(super) fn attributes_of(value: &Value, name: &str) -> Vec<String> {
    let Value::Record(record) = value else {
        panic!("{value} is no record");
    };
    let at = record
        .layout
        .texts
        .iter()
        .position(|text| &**text == name)
        .unwrap();
    let Value::Fn(closure) = &record.values[at] else {
        panic!("{name} is no closure");
    };
    closure
        .attributes()
        .iter()
        .map(|attribute| {
            let args: Vec<String> = attribute
                .args
                .iter()
                .map(|(field, constant)| format!("{field} = {constant:?}"))
                .collect();
            let id = match &attribute.id {
                crate::ast::AttributeId::Declared { module, name } => format!("{module}.{name}"),
                crate::ast::AttributeId::Builtin(name) => (*name).to_owned(),
            };
            format!("{id}({})", args.join(", "))
        })
        .collect()
}

#[test]
fn a_function_s_attributes_reach_its_closures_with_their_declarations() {
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] type hermetic = {on: Bool = true}\n\
             #[hermetic] #[retry(2)]\n\
             def build() -> Int { 1 }\n\
             def plain(): Int { 2 }",
        ),
        (
            "lib/attrs.gcsl",
            "#[attribute] type scratch = {keep: Size = 1GiB}",
        ),
        (
            "app/main.gcsl",
            "import scratch from lib/attrs\n#[scratch(512MiB)] def go(): Int { 1 }",
        ),
    ];
    let root = rooted_value(&files, "workspace.gcsl");
    assert_eq!(
        attributes_of(&root, "build"),
        [
            "workspace.hermetic(on = Bool(true))",
            "retry(times = Int(2))"
        ]
    );
    assert!(attributes_of(&root, "plain").is_empty());
    // By the module that declares it, wherever it is used.
    assert_eq!(
        attributes_of(&rooted_value(&files, "app/main.gcsl"), "go"),
        ["lib/attrs.scratch(keep = Quantity(Size, 536870912))"]
    );
    // One not imported is unknown there.
    let files = [
        files[0],
        files[1],
        ("app/main.gcsl", "#[scratch] def go(): Int { 1 }"),
    ];
    assert_eq!(rooted_messages(&files), ["unknown attribute `scratch`"]);
}

#[test]
fn a_def_type_s_attributes_are_defaults_for_its_definitions() {
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] type hermetic = {on: Bool = true}\n\
             #[def_type] #[hermetic]\n\
             def test(name: String, f: fn(): Int): fn(): Int { f }",
        ),
        (
            "tests/t.gcsl",
            "def test a() { 1 }\n\
             #[hermetic(false)] def test b() { 2 }\n\
             #[retry(3)] def test c() { 3 }\n\
             ra = (test a)()\nrb = (test b)()\nrc = (test c)()",
        ),
    ];
    let value = rooted_value(&files, "tests/t.gcsl");
    assert_eq!(
        attributes_of(&value, "ra"),
        ["workspace.hermetic(on = Bool(true))"]
    );
    // Replaced whole by its own.
    assert_eq!(
        attributes_of(&value, "rb"),
        ["workspace.hermetic(on = Bool(false))"]
    );
    // Its own others after the def-type's.
    assert_eq!(
        attributes_of(&value, "rc"),
        [
            "workspace.hermetic(on = Bool(true))",
            "retry(times = Int(3))"
        ]
    );
}

#[test]
fn a_generic_def_is_called_from_another_file_and_its_parameters_shadow_types() {
    let files = [
        ("workspace.gcsl", "type T = Int"),
        ("lib/box.gcsl", "def wrap<T>(x: T): {v: T} { v = x }"),
        (
            "app/main.gcsl",
            "import wrap from lib/box\nr = wrap(5).v + 1\ns = wrap(\"a\").v",
        ),
    ];
    // `T` in `wrap` is its parameter, not the root's `type T = Int`.
    assert_eq!(rooted_text(&files, "app/main.gcsl"), "{r = 6; s = \"a\"}");
}

#[test]
fn a_generic_type_is_its_definition_with_its_arguments_substituted() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib/types.gcsl",
            "type Pair<A, B> = {first: A, second: B}\ntype Named<T> = Pair<String, T>\n\
             type Id<T> = T\ntype F<T> = fn(x: T): T",
        ),
        (
            "app/main.gcsl",
            "import lib/types as t\nimport Named, Id, F from lib/types\n\
             p: t.Pair<Int, Bool> = { first = 1; second = true }\n\
             n: Named<List<Int>> = { first = \"a\"; second = [2] }\n\
             i: Id<Int> = 3\n\
             fn twice(x: Int): Int { x * 2 }\nf: F<Int> = twice\n\
             r = p.first + n.second[0] + i + f(1)",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{p = {first = 1; second = true}; n = {first = \"a\"; second = [2]}; i = 3; \
         f = fn(x: Int): Int; r = 8}"
    );
}

#[test]
fn a_generic_type_takes_as_many_arguments_as_it_has_parameters() {
    let lib = (
        "lib/types.gcsl",
        "type Pair<A, B> = {first: A, second: B}\ntype Box = {v: Int}",
    );
    for (main, message) in [
        (
            "import Pair from lib/types\nx: Pair = { first = 1 }",
            "type `Pair` needs its type arguments",
        ),
        (
            "import Pair from lib/types\nx: Pair<Int> = { first = 1 }",
            "type `Pair` takes 2 type arguments, given 1",
        ),
        (
            "import lib/types as t\nx: t.Pair<Int, Int, Int> = { first = 1 }",
            "type `t.Pair` takes 2 type arguments, given 3",
        ),
        (
            "import lib/types as t\nx: t.Pair = { first = 1 }",
            "type `t.Pair` needs its type arguments",
        ),
        (
            "import Box from lib/types\nx: Box<Int> = { v = 1 }",
            "type `Box` takes no type argument",
        ),
        // A type refers to itself through its own instance.
        ("type L<T> = {next: L<T>}", "type `L` refers to itself"),
        (
            "#[attribute]\ntype Tag<T> = {v: T}",
            "attribute `Tag` takes no type parameters",
        ),
        (
            "#[attribute] type track_caller = {}",
            "`track_caller` is a built-in attribute",
        ),
    ] {
        let files = [("workspace.gcsl", ""), lib, ("app/main.gcsl", main)];
        assert_eq!(rooted_messages(&files), [message], "{main}");
    }
}

#[test]
fn a_generic_type_s_parameters_shadow_declared_types_and_instances_are_fresh() {
    let files = [
        ("workspace.gcsl", "type T = Int"),
        (
            "app/main.gcsl",
            "type Box<T> = {v: T}\n\
             a: Box<String> = { v = \"s\" }\n\
             b: Box<Box<Bool>> = { v = { v = true } }\n\
             fn get<U>(box: Box<U>): U { box.v }\n\
             r = get(a)\ns = get(b).v",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = {v = \"s\"}; b = {v = {v = true}}; r = \"s\"; s = true}"
    );
}

#[test]
fn a_def_method_is_visible_through_every_import_and_the_directory_chain() {
    let lib = (
        "lib/ints.gcsl",
        "def (x: Int) twice(): Int { x * 2 }\ndef (x: Int) add(y: Int = 10): Int { x + y }\n\
         fn (x: Int) secret(): Int { 0 }\nk = 1",
    );
    let files = [
        ("workspace.gcsl", ""),
        lib,
        (
            "app/helpers.gcsl",
            "def (s: String) shout(): String { \"{s}!\" }",
        ),
        (
            "app/whole.gcsl",
            "import lib/ints\nwhole = 3.twice() + 1.add()",
        ),
        (
            "app/member.gcsl",
            "import k from lib/ints\nmember = 4.twice() + k",
        ),
        (
            "app/star.gcsl",
            "import * from lib/ints\nstar = 5.add(y = 1)",
        ),
        ("app/sub/deep.gcsl", "deep = \"hi\".shout()"),
    ];
    for (path, value) in [
        ("app/whole.gcsl", "{whole = 17}"),
        ("app/member.gcsl", "{member = 9}"),
        ("app/star.gcsl", "{star = 6}"),
        ("app/sub/deep.gcsl", "{deep = \"hi!\"}"),
    ] {
        assert_eq!(rooted_text(&files, path), value, "{path}");
    }
    // A `fn` method is its file's alone.
    let files = [
        ("workspace.gcsl", ""),
        lib,
        ("app/main.gcsl", "import lib/ints\nbad = 3.secret()"),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["cannot access field `secret` of Int"]
    );
}

#[test]
fn methods_of_other_files_are_chosen_and_conflict_as_a_file_s_own() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib/any.gcsl",
            "def (xs: List<T>) size<T>(): String { \"any\" }",
        ),
        (
            "lib/ints.gcsl",
            "def (xs: List<Int>) size(): String { \"ints\" }",
        ),
        (
            "app/main.gcsl",
            "import lib/any\nimport lib/ints\na = [1].size()\nb = [true].size()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"ints\"; b = \"any\"}"
    );
    let files = [
        ("workspace.gcsl", ""),
        ("lib/one.gcsl", "def (x: Int) m(): Int { 1 }"),
        ("lib/two.gcsl", "def (y: Int) m(): Int { 2 }"),
        ("app/main.gcsl", "import lib/one\nimport lib/two\nr = 3.m()"),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["method `m` is declared twice for type Int"]
    );
}

#[test]
fn a_call_of_a_field_needs_the_files_of_the_methods_of_its_name() {
    // Conservatively: the call is of a record's field, but a method of its
    // name is visible, so the file needs the method's, which needs it back.
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib.gcsl",
            "import app/main\ndef (x: Int) get(): Int { x }\nseen = main.v",
        ),
        (
            "app/main.gcsl",
            "v = 1\nrec = { get = fn(): Int { 2 } }\nw = rec.get()",
        ),
    ];
    let messages = rooted_messages(&files);
    assert_eq!(messages, ["`app/main.gcsl` and `lib.gcsl` need each other"]);
}

#[test]
fn variants_are_visible_where_def_methods_are_and_conflict_at_a_use() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/tool.gcsl", "enum Tool extensible<{*: String}>"),
        (
            "lib/c.gcsl",
            "import Tool from lib/tool\nenum Tool::C<{cc: String}>",
        ),
        (
            "app/main.gcsl",
            "import Tool from lib/tool\nimport lib/c\nt = Tool::C({ cc = \"gcc\" })\nok = Tool::C.matches(t)",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{t = Tool::C({cc = \"gcc\"}); ok = true}"
    );
    // Without the import of the variant's module, it is not seen.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/tool.gcsl", "enum Tool extensible"),
        ("lib/c.gcsl", "import Tool from lib/tool\nenum Tool::C"),
        (
            "app/main.gcsl",
            "import Tool from lib/tool\nt = Tool::C({})",
        ),
    ];
    assert_eq!(rooted_messages(&files), ["no variant `C` of `Tool`"]);
    // Two files' variants of one name are one name too many.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/tool.gcsl", "enum Tool extensible"),
        ("lib/a.gcsl", "import Tool from lib/tool\nenum Tool::C"),
        ("lib/b.gcsl", "import Tool from lib/tool\nenum Tool::C"),
        (
            "app/main.gcsl",
            "import Tool from lib/tool\nimport lib/a\nimport lib/b\nt = Tool::C({})",
        ),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["variant `Tool::C` is declared in two files"]
    );
}

#[test]
fn values_of_enums_are_made_compared_tested_and_unwrapped() {
    let text = "enum Tool extensible<{*: String}>\n\
                enum Tool::C<{cc: String}>\n\
                enum Tool::Rust<{rustc: String}>\n\
                fn get<T>(kind: Variant<Tool, T>, tools: List<Tool>, fallback: T): T {\n\
                \x20   for t in tools { if kind.matches(t) { return kind.payload(t) } }\n\
                \x20   fallback\n\
                }\n\
                let tools = [Tool::Rust({ rustc = \"rustc\" }), Tool::C({ cc = \"gcc\" })]\n\
                a = get(Tool::C, tools, { cc = \"none\" }).cc\n\
                b = get(Tool::Rust, tools, { rustc = \"none\" }).rustc\n\
                c = Tool::C({ cc = \"x\" }) == Tool::C({ cc = \"x\" })\n\
                d = Tool::C({ cc = \"x\" }) == Tool::Rust({ rustc = \"x\" })\n\
                e = Tool::C == Tool::C\n\
                f = Tool::C\n\
                g = tools[1]\n\
                let v: Tool | Int = 3\n\
                h = v is Tool\n\
                type P = {arch: String, os: String}\n\
                let p: Partial<P> = { arch = \"x86_64\" }\n\
                i = p.matches({ arch = \"x86_64\"; os = \"linux\" })\n\
                j = p.matches({ arch = \"arm\"; os = \"linux\" })";
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"gcc\"; b = \"rustc\"; c = true; d = false; e = true; f = Tool::C; \
         g = Tool::C({cc = \"gcc\"}); h = false; i = true; j = false}"
    );
    let files = [
        ("workspace.gcsl", ""),
        (
            "app/main.gcsl",
            "enum T extensible\nenum T::A<Int>\nenum T::B<Int>\nr = T::A.payload(T::B(1))",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let main = workspace.file("app/main.gcsl").unwrap();
    let error = workspace.run(main).unwrap_err().remove(0);
    assert_eq!(error.message, "the value is `T::B`, not `T::A`");
}

/// An enum of toolchains and a generic def-type whose definitions are made
/// with its variants, as a workspace's root declares them.
pub(super) const TOOLCHAIN: (&str, &str) = (
    "workspace.gcsl",
    "enum Toolchain extensible<{*: String}>\n\
     enum Toolchain::C<{cc: String}>\n\
     enum Toolchain::Rust<{rustc: String}>\n\
     type Def<T> = {arch: String, provides: T}\n\
     type Registered = {arch: String, provides: Toolchain}\n\
     #[def_type]\n\
     def Toolchain<T>(name: String, kind: Variant<Toolchain, T>, f: fn(): Def<T>): Registered {\n\
     \x20   let d = f()\n\
     \x20   arch = d.arch\n\
     \x20   provides = kind(d.provides)\n\
     }",
);

#[test]
fn a_definition_made_with_a_variant_binds_its_def_type_s_type_parameters() {
    let files = [
        TOOLCHAIN,
        (
            "tools.gcsl",
            "def Toolchain::C gcc() {\n    arch = \"x86_64\"\n    provides = { cc = \"gcc\" }\n}\n\
             def Toolchain::Rust gcc() {\n    arch = \"aarch64\"\n    provides = { rustc = \"rustc\" }\n}",
        ),
        (
            "app/main.gcsl",
            "import Toolchain from workspace\nimport workspace as w\n\
             let c = Toolchain::C gcc()\nlet rust = w.Toolchain::Rust gcc()\n\
             cc = Toolchain::C.payload(c.provides).cc\nrustc = Toolchain::Rust.payload(rust.provides).rustc\n\
             arches = [c.arch, rust.arch]",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{cc = \"gcc\"; rustc = \"rustc\"; arches = [\"x86_64\", \"aarch64\"]}"
    );
    // Not listed to be invoked by name: its definitions are reached by
    // references alone.
    let workspace = rooted(&files).unwrap();
    assert!(workspace.def_types().unwrap().is_empty());
}

#[test]
fn variant_def_type_errors() {
    let errors = |text: &str| {
        let files = [TOOLCHAIN, ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    for (text, message) in [
        (
            "def Toolchain::C x() {\n    arch = \"a\"\n    provides = { cc = 1 }\n}",
            "mismatched types",
        ),
        (
            "def Toolchain x() { 1 }",
            "def-type `Toolchain` needs a variant: `def Toolchain::V name()`",
        ),
        (
            "def Toolchain::Nope x() { 1 }",
            "no variant `Nope` of `Toolchain`",
        ),
        (
            "r = Toolchain::C nope()",
            "`Toolchain::C` has no definition `nope`",
        ),
        (
            "#[def_type]\ndef pk(name: String, f: fn(): Int): Int { f() }\ndef pk::C x() { 1 }",
            "`pk` takes no variant",
        ),
        (
            "#[def_type]\ndef g<T>(name: String, f: fn(): T): Int { 1 }",
            "a generic def-type takes its definitions' variant after the name, as in `def \
             Toolchain<T>(name: String, kind: Variant<Toolchain, T>, func: fn(): T)`",
        ),
        (
            "#[def_type]\ndef g<T, U>(name: String, kind: Variant<Toolchain, T>, f: fn(): U): Int { 1 }",
            "type parameter `U` of a def-type must appear in its variant parameter's type",
        ),
        (
            "#[def_type]\ndef s(name: String, kind: Variant<Toolchain, {cc: String}>, f: fn(): Int): \
             Int { f() }\ndef s::Rust x() { 1 }",
            "the variant of a `s` definition is a `Variant<Toolchain, {cc: String}>`, not \
             `Variant<Toolchain, {rustc: String}>`",
        ),
    ] {
        let found = errors(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
}

/// Contexts of platforms, as a library declares them, and a function that
/// reads them.
pub(super) const PLATFORMS: (&str, &str) = (
    "lib/plat.gcsl",
    "type Platform = {arch: String, os: String}\n\
     context arch: String = \"x86_64\"\n\
     context host_platform: Platform = { arch = ^arch; os = \"linux\" }\n\
     context platform: Platform = ^host_platform\n\
     context required: Int\n\
     def describe(): String { \"{^platform.arch}-{^platform.os}\" }\n\
     def tool(^arch): String { \"cc-{arch}\" }\n\
     def needs(): Int { ^required }",
);

#[test]
fn contexts_are_read_bound_and_defaulted_where_they_are_read() {
    let files = [
        ("workspace.gcsl", ""),
        PLATFORMS,
        (
            "app/main.gcsl",
            "import lib/plat\n\
             a = plat.describe()\n\
             b = plat.describe() with { host_platform = { arch = \"aarch64\"; os = \"linux\" } }\n\
             c = plat.describe() with { platform.arch = \"riscv64\"; platform.os = \"peios\" }\n\
             d = with { arch = \"arm\" } { plat.describe() }\n\
             e = plat.tool()\n\
             f = plat.tool() with { arch = \"mips\" }\n\
             g = plat.tool(arch = \"explicit\")\n\
             h = plat.needs() with { required = 3 }\n\
             i = ^platform.os",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"x86_64-linux\"; b = \"aarch64-linux\"; c = \"riscv64-peios\"; d = \"arm-linux\"; \
         e = \"cc-x86_64\"; f = \"cc-mips\"; g = \"cc-explicit\"; h = 3; i = \"linux\"}"
    );
    // A call of a function of another file that reads one no `with` binds,
    // with no default, is an error where it is called.
    let files = [
        ("workspace.gcsl", ""),
        PLATFORMS,
        ("app/main.gcsl", "import lib/plat\nr = plat.needs()"),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "context `required` has no value here at app/main.gcsl; note: `needs` needs it, and no \
          `with` around this call sets it"
        ]
    );
}

#[test]
fn a_future_keeps_its_context_and_memoised_calls_key_on_what_they_read() {
    let text = "context x: Int = 1\n\
                fn m(n: Int) ->> Int { n + ^x }\n\
                fn inner() ->> Int { ^x }\n\
                fn outer() ->> Int { inner() * 100 }\n\
                fn outer2() ->> Int { inner() * 1000 }\n\
                fn later() ->> Future<Int> { lazy { ^x } }\n\
                fn fixed() ->> Int { with { x = 7 } { inner() } }\n\
                fn force(v: Int): Int { v }\n\
                a = m(1)\n\
                b = m(1) with { x = 10 }\n\
                c = m(1)\n\
                d = inner() with { x = 5 }\n\
                e = outer()\n\
                f = outer() with { x = 5 }\n\
                g = outer2() with { x = 5 }\n\
                h = outer2()\n\
                i = force(later())\n\
                j = force(later()) with { x = 42 }\n\
                k = fixed()\n\
                l = fixed() with { x = 9 }\n\
                let made = lazy { ^x } with { x = 99 }\n\
                n = force(made)";
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = 2; b = 11; c = 2; d = 5; e = 100; f = 500; g = 5000; h = 1000; i = 1; j = 42; \
         k = 7; l = 7; n = 99}"
    );
}

#[test]
fn context_errors() {
    let errors = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    for (text, message) in [
        ("r = ^nope", "no context `nope` is declared"),
        (
            "context arch: String = \"x\"\nr = ^arhc",
            "no context `arhc` is declared",
        ),
        ("context x: Int\nr = ^x", "context `x` has no value here"),
        (
            "context x: Int = 1\ncontext x: Int = 2",
            "context `x` is declared twice",
        ),
        ("context x: Int = \"s\"", "mismatched types"),
        (
            "context x: Int = 1\nr = 1 with { x = \"s\" }",
            "mismatched types",
        ),
        (
            "context p: {a: Int} = {a = 1}\nr = 1 with { p.b = 2 }",
            "context `p` has no field `b`",
        ),
        (
            "context x: Int = 1\nr = 1 with { x = 2; x = 3 }",
            "context `x` is bound twice in this `with`",
        ),
        (
            "context x: Int = 1\nr = 1 with { y = 2 }",
            "no context `y` is declared",
        ),
        (
            "fn imp() -> Int { 1 }\ncontext x: Int = imp()",
            "the default of a context must be pure",
        ),
        ("context x: Nope = 1", "unknown type `Nope`"),
        ("fn f(^nope): Int { 1 }", "no context `nope` is declared"),
    ] {
        let found = errors(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
    // A context is declared once in a workspace.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "context x: Int = 1"),
        ("b.gcsl", "context x: Int = 2"),
    ];
    assert!(
        check_errors(&files)
            .join("\n")
            .starts_with("context `x` is declared twice")
    );
}

#[test]
fn a_context_s_default_is_scheduled_with_what_it_reads() {
    let files = [
        ("workspace.gcsl", ""),
        ("app.gcsl", "r = ^a\ncontext a: Int = b + 1\nb = 3\ns = ^a"),
    ];
    assert_eq!(rooted_text(&files, "app.gcsl"), "{r = 4; b = 3; s = 4}");
}

/// A def-type whose definitions read contexts, and a file of a platform's
/// record, to give an invocation its context with.
pub(super) const HOSTED: [(&str, &str); 4] = [
    (
        "workspace.gcsl",
        "type Platform = {arch: String, os: String}\n\
         context host_platform: Platform = { arch = \"x86_64\"; os = \"linux\" }\n\
         context label: String = \"default\"\n\
         #[def_type]\n\
         def build(name: String, f: fn() -> String) -> String { f() }\n\
         def tagged(n: Int) ->> String { \"{n}:{^label}\" }\n\
         top = ^label",
    ),
    (
        "app.gcsl",
        "def build x() { \"{^host_platform.arch}-{tagged(1)}-{top}\" }",
    ),
    ("plat.gcsl", "arch = \"aarch64\"\nos = \"linux\""),
    ("other.gcsl", "n = 1"),
];

#[test]
fn an_invocation_runs_in_the_context_its_host_sets() {
    let mut workspace = rooted(&HOSTED).unwrap();
    // A host can tell which contexts it can set.
    assert!(workspace.declares_context("host_platform"));
    assert!(!workspace.declares_context("hostplatform"));
    let plat = workspace.file("plat.gcsl").unwrap();
    let platform = workspace.run(plat).unwrap().value;
    let set = |label: &str| {
        vec![
            ("host_platform".to_owned(), platform.clone()),
            ("label".to_owned(), Value::Str(Rc::from(label))),
        ]
    };
    let invoked = |workspace: &mut Workspace, contexts: &[(String, Value)]| {
        let outcome = workspace.invoke_with(("build", "x"), &[], contexts);
        outcome.unwrap().value.to_string()
    };
    // What a file computes reads the defaults, whatever the invocation.
    assert_eq!(
        invoked(&mut workspace, &set("first")),
        "aarch64-1:first-default"
    );
    // A memoised call is not found from another context.
    assert_eq!(
        invoked(&mut workspace, &set("second")),
        "aarch64-1:second-default"
    );
    assert_eq!(invoked(&mut workspace, &[]), "x86_64-1:default-default");
    let error = |contexts: Vec<(String, Value)>| {
        let mut workspace = rooted(&HOSTED).unwrap();
        match workspace.invoke_with(("build", "x"), &[], &contexts) {
            Err(InvokeError::Request(error)) => {
                let mut shown = vec![error.message()];
                shown.extend(error.notes());
                shown.join("; ")
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        error(vec![("lable".to_owned(), Value::Int(1))]),
        "no context `lable` is declared; the contexts are `host_platform`, `label`"
    );
    assert_eq!(
        error(vec![("label".to_owned(), Value::Int(1))]),
        "the value given for context `label` is not a `String`; a context is given data of \
         its type: no function or future"
    );
    let twice = vec![
        ("label".to_owned(), Value::Str(Rc::from("a"))),
        ("label".to_owned(), Value::Str(Rc::from("b"))),
    ];
    assert_eq!(error(twice), "context `label` is given more than once");
    // A record lacking a field of the type is not of it.
    let mut workspace = rooted(&HOSTED).unwrap();
    let other = workspace.file("other.gcsl").unwrap();
    let wrong = workspace.run(other).unwrap().value;
    assert_eq!(
        error(vec![("host_platform".to_owned(), wrong)]),
        "the value given for context `host_platform` is not a `{arch: String, os: String}`; a \
         context is given data of its type: no function or future"
    );
}

#[test]
fn a_recover_catches_what_retry_would_retry_and_an_assertion_shows_its_operands() {
    let text = "fn check(x: Int): Int {\n\
                \x20   assert x > 0, \"x must be positive\"\n\
                \x20   x * 2\n\
                }\n\
                a = recover { check(3) } else { 0 }\n\
                b = recover { check(-1) } else (e) { e.message }\n\
                c = recover { panic(\"boom\") } else (failure) { \"caught {failure.message}\" }\n\
                d = recover { [1, 2][5] } else { -1 }\n\
                f = recover { recover { panic(\"inner\") } else (e) { panic(\"re: {e.message}\") } } \
                else (e) { e.message }\n\
                let s = \"hello\"\n\
                g = recover { assert s == \"hullo\"; \"passed\" } else (e) { e.message }\n\
                fn leave(): Int {\n\
                \x20   let _ = for i in [1, 2, 3] {\n\
                \x20       recover { if i == 2 { return i * 10 }; {} } else { {} }\n\
                \x20   }\n\
                \x20   0\n\
                }\n\
                h = leave()";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = 6; b = \"assertion failed: x must be positive\"; c = \"caught boom\"; d = -1; \
         f = \"re: inner\"; g = \"assertion failed: `s == \\\"hullo\\\"`\"; h = 20}"
    );
    // A failed assertion labels each side of its comparison with its value.
    let files = [
        ("workspace.gcsl", ""),
        ("app.gcsl", "let s = \"hello\"\nassert s == \"hullo\""),
    ];
    let mut workspace = rooted(&files).unwrap();
    let app = workspace.file("app.gcsl").unwrap();
    let error = workspace.run(app).unwrap_err().remove(0);
    assert_eq!(error.message, "assertion failed: `s == \"hullo\"`");
    let labels: Vec<&str> = error
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .collect();
    assert_eq!(labels, ["this is `\"hello\"`", "this is `\"hullo\"`"]);
}

#[test]
fn a_recover_inside_a_timed_call_leaves_its_timeout_to_the_call() {
    let text = "#[timeout(200ms)]\n\
                fn slow() -> String {\n\
                \x20   recover { let r = exec([\"sleep\", \"3\"]); \"slept {r.status}\" } else (e) { \
                \"inner caught: {e.message}\" }\n\
                }\n\
                r = recover { slow() } else (e) { \"outer caught: {e.message}\" }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{r = \"outer caught: `slow` timed out after 200ms\"}"
    );
}

#[test]
fn assertion_and_recover_errors() {
    let errors = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    for (text, message) in [
        ("assert 1", "mismatched types"),
        ("assert true, 1", "mismatched types"),
        (
            "r = recover { 1 } else (e) { e.nope }",
            "no field `nope` in `{message: String}`",
        ),
        (
            "fn imp() -> Int { 1 }\nfn p(): Int { recover { imp() } else { 0 } }",
            "cannot call an impure function from a pure function",
        ),
    ] {
        let found = errors(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
}

#[test]
fn what_is_computed_on_a_secret_is_a_secret_and_unsafe_value_takes_its_value() {
    let text = "let s = secret(\"hunter2\", \"password\")\n\
                let n = secret(41, \"answer\")\n\
                a = s + \"!\"\n\
                b = (s + \"!\").unsafe_value()\n\
                c = (n + 1).unsafe_value()\n\
                d = \"--token={s}\".unsafe_value()\n\
                e = (s == \"hunter2\").unsafe_value()\n\
                f = (-n).unsafe_value()\n\
                let o = secret({k = s; n = 1}, \"login\")\n\
                g = o.k.unsafe_value()\n\
                h = (o == {k = \"hunter2\"; n = 1}).unsafe_value()\n\
                i = ({k = s} == {k = \"guess\"}).unsafe_value()\n\
                j = ([s, s] == [\"hunter2\", \"hunter2\"]).unsafe_value()\n\
                k = secret([1, 2], \"list\")[1].unsafe_value()\n\
                l = show({k = s})\n\
                fn same<T>(x: T): T { x }\n\
                m = same(s)\n\
                enum B extensible<{*: Secret<String>}>\n\
                enum B::X<{a: Secret<String>}>\n\
                p = (B::X({a = s}) == B::X({a = secret(\"hunter2\", \"other\")})).unsafe_value()";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        value(&files, "app.gcsl"),
        "{a = ***; b = \"hunter2!\"; c = 42; d = \"--token=hunter2\"; e = true; f = -41; \
         g = \"hunter2\"; h = true; i = false; j = true; k = 2; l = \"{{k = ***}}\"; m = ***; \
         p = true}"
    );
}

#[test]
fn a_secret_computed_on_secrets_is_identified_by_theirs() {
    let text = "let s = secret(\"pw\", \"password\")\n\
                let u = secret(\"jack\", \"user\")\n\
                a = s\n\
                b = u + \":\" + s\n\
                c = \"{u}:{s}\"\n\
                d = {k = s} == {k = \"pw\"}\n\
                e = secret({k = s}, \"login\").k\n\
                f = secret(s, \"again\")";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    let Value::Record(record) = rooted_value(&files, "app.gcsl") else {
        panic!("not a record");
    };
    let identities: Vec<(&str, &str)> = record
        .layout
        .texts
        .iter()
        .zip(&record.values)
        .map(|(name, value)| match value {
            Value::Secret(secret) => (name.as_ref(), secret.identity.as_ref()),
            value => panic!("{name} is {value}, not a secret"),
        })
        .collect();
    assert_eq!(
        identities,
        [
            ("a", "password"),
            (
                "b",
                "str-concat($(str-concat($(user\\), \":\"\\)), $(password))"
            ),
            ("c", "interp($(user), \":\", $(password))"),
            ("d", "record-eq({k = $(password)}, {k = \"pw\"})"),
            ("e", "field.k($(login))"),
            ("f", "again"),
        ]
    );
}

#[test]
fn a_derived_secret_s_identity_is_its_derivation_so_it_names_one_value() {
    // Two secrets of one identity are one secret to memoisation and to
    // total equality: secrets derived differently must not share one.
    let text = "let s = secret(\"pw\", \"password\")\n\
                let o = secret({user = \"jack\"; pass = \"pw\"}, \"login\")\n\
                fn f(x: Secret<String>) ->> Secret<String> { x }\n\
                a = f(s + \"a\").unsafe_value()\n\
                b = f(s + \"b\").unsafe_value()\n\
                c = f(\"x{s}\").unsafe_value()\n\
                d = f(\"y{s}\").unsafe_value()\n\
                e = f(o.user).unsafe_value()\n\
                g = f(o.pass).unsafe_value()\n\
                let p: {n: Int} = {n = 1; hidden = s + \"a\"}\n\
                let q: {n: Int} = {n = 1; hidden = s + \"b\"}\n\
                h = p === q";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = \"pwa\"; b = \"pwb\"; c = \"xpw\"; d = \"ypw\"; e = \"jack\"; g = \"pw\"; h = false}"
    );
}

#[test]
fn a_host_reads_the_identities_of_the_secrets_a_value_holds() {
    let text = "let s = secret(\"pw\", \"password\")\n\
                let u = secret(\"jack\", \"user\")\n\
                step = {argv = [\"sign\", s.reveal()]; who = u; again = [s]; plain = 1}";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    let value = rooted_value(&files, "app.gcsl");
    let identities: Vec<String> = value
        .secret_identities()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(identities, ["password", "user"]);
}

#[test]
fn a_secret_decides_nothing_and_is_not_its_value() {
    let errors = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        let error = rooted(&files)
            .err()
            .unwrap_or_else(|| panic!("{text}: no error"));
        (error[0].message.clone(), error[0].notes.clone())
    };
    let prefix = "let s = secret(\"pw\", \"id\")\n";
    for (text, message) in [
        ("r = if s == \"pw\" { 1 } else { 2 }", "mismatched types"),
        (
            "r = s == \"pw\" and true",
            "a secret cannot decide what runs",
        ),
        (
            "r = true or s == \"pw\"",
            "a secret cannot decide what runs",
        ),
        (
            "let v: Secret<String> | Int = s\nr = v is Int",
            "a secret cannot decide what runs",
        ),
        (
            "let v: Secret<String> | Int = s\nr = v as Int",
            "a secret cannot decide what runs",
        ),
        (
            "r = for c in secret([1], \"l\") { c }",
            "cannot iterate over a value",
        ),
        ("assert s == \"pw\"", "mismatched types"),
        (
            "r = [1, 2][secret(0, \"i\")]",
            "a list index must be an Int",
        ),
        ("r = trim(s)", "mismatched types"),
        ("r = s + 1", "cannot add `Secret<String>` and Int"),
        (
            "r = {...secret({k = 1}, \"r\")}",
            "cannot spread a value of type",
        ),
        (
            "let r: Secret<Future<Int>> = secret(1, \"f\")",
            "a secret's value is present",
        ),
    ] {
        let (found, notes) = errors(&format!("{prefix}{text}"));
        assert!(found.starts_with(message), "{text}: {found}");
        if found == "mismatched types" {
            assert!(
                notes.iter().any(|note| note.contains("`.unsafe_value()`")),
                "{text}: {notes:?}"
            );
        }
    }
}

#[test]
fn a_failure_on_a_secret_names_nothing_and_nothing_recovers_it() {
    for text in [
        "let n = secret(0, \"zero\")\nr = recover { 1 // n } else (e) { e.message }",
        "let l = secret([1, 2], \"l\")\nr = recover { l[5] } else { 0 }",
        "let n = secret(9223372036854775807, \"big\")\nr = recover { n + 1 } else { 0 }",
        "let n = secret(0, \"zero\")\nr = recover { (n + 1) // (n * 2) } else { 0 }",
    ] {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        let mut workspace = rooted(&files).unwrap();
        let app = workspace.file("app.gcsl").unwrap();
        let error = workspace.run(app).unwrap_err().remove(0);
        assert_eq!(error.message, "an operation on a secret failed", "{text}");
        assert!(error.labels.is_empty(), "{text}: {:?}", error.labels);
        assert!(error.notes.is_empty(), "{text}: {:?}", error.notes);
    }
}

#[test]
fn a_failure_of_an_operand_of_an_operation_on_a_secret_is_the_operand_s() {
    let text = "let s = secret(\"pw\", \"id\")\n\
                a = recover { s + panic(\"boom\") } else (e) { e.message }\n\
                b = recover { \"{s}{[\"x\"][3]}\" } else (e) { e.message }\n\
                let n = secret(0, \"zero\")\n\
                c = recover { (1 // n) + 1 } else { 0 }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    let mut workspace = rooted(&files).unwrap();
    let app = workspace.file("app.gcsl").unwrap();
    // The operand's own failures are recovered; the failure of an operand
    // that is itself an operation on a secret is that operation's, which
    // nothing recovers, wherever it reaches.
    let error = workspace.run(app).unwrap_err().remove(0);
    assert_eq!(error.message, "an operation on a secret failed");
    let files = [
        ("workspace.gcsl", ""),
        ("app.gcsl", &text[..text.find("let n").unwrap()]),
    ];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = \"boom\"; b = \"index out of range\"}"
    );
}

#[test]
fn a_timeout_inside_an_operation_on_a_secret_is_its_call_s() {
    let text = "let s = secret(\"pw\", \"id\")\n\
                #[timeout(200ms)]\n\
                fn slow() -> Secret<String> { s + exec([\"sleep\", \"3\"]).stdout }\n\
                r = recover { slow().unsafe_value() } else (e) { e.message }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{r = \"`slow` timed out after 200ms\"}"
    );
}

#[test]
fn a_secret_is_one_with_another_of_its_identity_where_a_comparison_is_plain() {
    // Total equality compares fields the type does not name, and a cache
    // hit is seen in whether a call runs: neither tells of the value.
    let text = "let s = secret(\"pw\", \"id\")\n\
                let q: {x: Int} = {x = 1; y = s}\n\
                let wrong: {x: Int} = {x = 1; y = secret(\"guess\", \"id\")}\n\
                let other: {x: Int} = {x = 1; y = secret(\"pw\", \"other\")}\n\
                a = q === wrong\n\
                b = q === other\n\
                fn f(x: Secret<String>) ->> Secret<String> { x }\n\
                let first = f(s)\n\
                c = f(secret(\"guess\", \"id\")).unsafe_value()\n\
                d = (s == secret(\"guess\", \"id\")).unsafe_value()";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = true; b = false; c = \"pw\"; d = false}"
    );
}

#[test]
fn a_revealed_secret_keeps_its_identity_and_is_told_apart_from_the_secret() {
    let text = "let s = secret(\"pw\", \"token\")\n\
                a = s.reveal()\n\
                b = s.reveal() + \"!\"\n\
                c = \"{s.reveal()}{s}\"\n\
                fn f(x: Secret<String>) ->> Secret<String> { x }\n\
                d = f(s)\n\
                e = f(s.reveal())";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    let Value::Record(record) = rooted_value(&files, "app.gcsl") else {
        panic!("not a record");
    };
    let secrets: Vec<(&str, &str, bool)> = record
        .layout
        .texts
        .iter()
        .zip(&record.values)
        .map(|(name, value)| match value {
            Value::Secret(secret) => (name.as_ref(), secret.identity.as_ref(), secret.revealed),
            value => panic!("{name} is {value}, not a secret"),
        })
        .collect();
    // A memoised call of a revealed secret is not one of the secret.
    assert_eq!(
        secrets,
        [
            ("a", "token", true),
            ("b", "str-concat($(token), \"!\")", true),
            ("c", "interp($(token), $(token))", false),
            ("d", "token", false),
            ("e", "token", true),
        ]
    );
}

#[test]
fn a_tracked_function_reports_the_errors_of_its_body_at_its_call() {
    let error = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        let mut workspace = rooted(&files).unwrap();
        let app = workspace.file("app.gcsl").unwrap();
        let error = workspace.run(app).unwrap_err().remove(0);
        let at = text[error.span.start as usize..error.span.end as usize].to_owned();
        (error.message, at)
    };
    // Raised in its body: at the call.
    let tracked = "#[track_caller]\ndef fail(n: Int) -> Int { if n > 0 { panic(\"no\") }\nn }\n";
    assert_eq!(
        error(&format!("{tracked}x = fail(1)")),
        ("no".to_owned(), "fail(1)".to_owned())
    );
    // A tracked function called from another's body fails at that call, and
    // the other moves it on to its own call if it is tracked too.
    let text = format!("#[track_caller]\ndef outer() -> Int {{ fail(2) }}\n{tracked}x = outer()");
    assert_eq!(error(&text), ("no".to_owned(), "outer()".to_owned()));
    let text = format!("def outer() -> Int {{ fail(2) }}\n{tracked}x = outer()");
    assert_eq!(error(&text), ("no".to_owned(), "fail(2)".to_owned()));
    // An untracked function it calls fails where it raises its error.
    let text = "def inner() -> Int { panic(\"in\") }\n\
                #[track_caller]\ndef outer() -> Int { inner() }\nx = outer()";
    assert_eq!(error(text), ("in".to_owned(), "panic(\"in\")".to_owned()));
    // So does a callback, whose body is not the tracked function's.
    let text = "#[track_caller]\ndef call(f: fn() -> Int) -> Int { f() }\n\
                fn boom() -> Int { panic(\"cb\") }\nx = call(boom)";
    assert_eq!(error(text), ("cb".to_owned(), "panic(\"cb\")".to_owned()));
    // A memoised or retried tracked function, the same.
    let text = "#[track_caller]\ndef once(n: Int) ->> Int { panic(\"m{n}\") }\nx = once(1)";
    assert_eq!(error(text), ("m1".to_owned(), "once(1)".to_owned()));
    let text = "#[track_caller] #[retry(1)]\ndef again() -> Int { panic(\"r\") }\nx = again()";
    assert_eq!(error(text), ("r".to_owned(), "again()".to_owned()));
}

#[test]
fn a_program_given_a_secret_reports_a_secret() {
    let text = "let s = secret(\"hunter2\", \"token\")\n\
                let r = exec_secret([\"sh\", \"-c\", \"echo $T\"], secrets = {T = s.reveal()})\n\
                a = r\n\
                b = r.stdout\n\
                c = r.stdout.unsafe_value()\n\
                d = r.status.unsafe_value()\n\
                let flag = \"--token={s.reveal()}\"\n\
                e = flag\n\
                f = exec_secret([\"sh\", \"-c\", \"echo $F\"], secrets = {F = flag}).stdout.unsafe_value()\n\
                let kept: Secret<String> = s.reveal()\n\
                g = kept";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = ***; b = ***; c = \"hunter2\\n\"; d = 0; e = ***; f = \"--token=hunter2\\n\"; \
         g = ***}"
    );
}

#[test]
fn a_run_given_a_secret_is_one_secret_with_another_only_if_it_is_the_same_run() {
    // A memoised function keys its calls by a secret's identity: two runs
    // of one command are one key, and a run of another command another.
    let counter = std::env::temp_dir().join(format!("gcsl-12d-memo-{}", std::process::id()));
    let counter = counter.to_str().unwrap();
    let _ = std::fs::remove_file(counter);
    let text = format!(
        "let s = secret(\"pw\", \"token\")\n\
         fn run(word: String) -> Secret<{{status: Int, stdout: String, stderr: String}}> {{\n\
             exec_secret([\"echo\", word], secrets = {{T = s.reveal()}})\n\
         }}\n\
         fn mark(r: Secret<{{status: Int, stdout: String, stderr: String}}>) ->> Int {{\n\
             exec([\"sh\", \"-c\", \"echo x >> {counter}\"]).status\n\
         }}\n\
         = [mark(run(\"a\")), mark(run(\"a\")), mark(run(\"b\"))]"
    );
    let files = [("workspace.gcsl", ""), ("app.gcsl", text.as_str())];
    assert_eq!(rooted_text(&files, "app.gcsl"), "[0, 0, 0]");
    let runs = std::fs::read_to_string(counter).unwrap().lines().count();
    std::fs::remove_file(counter).unwrap();
    assert_eq!(runs, 2);
}

#[test]
fn a_program_is_given_a_secret_only_by_exec_secret_and_only_revealed() {
    let prefix = "let s = secret(\"pw\", \"id\")\n";
    // A word or a variable of `exec`'s, or of a file's, is never a secret.
    for text in [
        "r = exec([\"echo\", s.reveal()])",
        "r = exec([\"true\"], env = {T = s.reveal()})",
        "r = exec_live([\"echo\", s.reveal()])",
        "r = file(\"/bin/echo\").exec(s.reveal())",
        "r = exec_secret([\"echo\", s.reveal()], secrets = {})",
    ] {
        let files = [
            ("workspace.gcsl", ""),
            ("app.gcsl", &format!("{prefix}{text}")),
        ];
        let errors = rooted(&files)
            .err()
            .unwrap_or_else(|| panic!("{text}: no error"));
        assert_eq!(errors[0].message, "mismatched types", "{text}");
        assert!(
            errors[0]
                .notes
                .iter()
                .any(|note| note.contains("exec_secret(argv, secrets")),
            "{text}: {:?}",
            errors[0].notes
        );
    }
    // A secret given is revealed, whole.
    for text in [
        "r = exec_secret([\"true\"], secrets = {T = s})",
        "r = exec_secret([\"true\"], secrets = {T = \"{s.reveal()}{s}\"})",
        "r = exec_secret([\"true\"], secrets = {T = secret(s.reveal(), \"again\")})",
        "let r: Revealed<String> = s",
    ] {
        let files = [
            ("workspace.gcsl", ""),
            ("app.gcsl", &format!("{prefix}{text}")),
        ];
        let errors = rooted(&files)
            .err()
            .unwrap_or_else(|| panic!("{text}: no error"));
        assert_eq!(errors[0].message, "mismatched types", "{text}");
        assert!(
            errors[0]
                .notes
                .iter()
                .any(|note| note.contains("`.reveal()`")),
            "{text}: {:?}",
            errors[0].notes
        );
    }
}
