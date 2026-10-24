//! Tests of the checker: the built-in functions.

use super::*;

#[test]
fn len_is_a_built_in_function() {
    assert_eq!(shown_result("len([1, 2])"), "Int");
    assert_eq!(shown_result("len(list = [1, 2])"), "Int");
    assert_eq!(shown_result("len([lazy 1])"), "Int");
    // Its argument is measured at the top, its elements not at all.
    let text = "let xs = lazy [lazy 1]\nlen(xs)";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Top)]);
    // It is pure, so a pure function may call it.
    assert_eq!(
        type_of_f("fn f(xs: Int[]): Int { len(xs) + 1 }"),
        "fn(xs: List<Int>): Int"
    );
    // Its callee has the function type of the call.
    let text = "len([1])";
    assert_eq!(type_at(text, at(text, "len")), "fn(List<Int>): Int");
    let (ast, _, output) = check_prelude(text);
    let resolution = output.unwrap().checked.resolution;
    let callee = ast
        .expr_ids()
        .find(|&id| ast.get(id).span == at(text, "len"))
        .unwrap();
    assert_eq!(resolution.expr_var(callee), VarRef::Builtin(Builtin::Len));
    // A future list in a pure function is the purity error.
    let text = "fn f(xs: Future<Int[]>): Int { len(xs) }";
    assert_implicit_await_error(text, last(text, "xs"), "Future<List<Int>>");
}

#[test]
fn a_built_in_is_shadowed_as_any_outer_name() {
    assert_eq!(shown_result("let len = 5\nlen"), "Int");
    assert_eq!(
        shown_result("let len = fn(x: String): String { x }\nlen(\"a\")"),
        "String"
    );
    assert_eq!(
        type_of_f("fn f(len: Int): Int { len + 1 }"),
        "fn(len: Int): Int"
    );
    assert_eq!(file_type("len = 1\nn = len"), "{len: Int, n: Int}");
    // `:=` never finds it.
    assert_eq!(
        messages("len := 1"),
        vec!["cannot rebind `len`: no variable named `len` is declared"]
    );
}

#[test]
fn a_built_in_is_not_a_value_yet() {
    for (text, span) in [
        ("len", Span::new(0, 3)),
        ("let mut f = len", Span::new(12, 15)),
        ("(len)([1])", Span::new(1, 4)),
        ("\"{len}\"", Span::new(2, 5)),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message,
            "`len` is a built-in function and cannot be used as a value yet"
        );
        assert_eq!(found[0].span, span, "{text}");
        assert_eq!(
            found[0].notes,
            vec![
                "a built-in function with a type for any list cannot be written as a type \
                     yet; call it, or bind it to a name"
            ]
        );
    }
}

#[test]
fn the_alias_rule_lets_a_special_built_in_stand_only_where_it_says() {
    const NOT_A_VALUE: &str = "`len` is a built-in function and cannot be used as a value yet";
    // Every other use is the error, at the reference: an argument, a
    // list element, a field of an inner record, a function's result,
    // an operand of `==` and `===`, an `is`, an `as`, an ascription, a
    // `let` that is not a top-level one of one name, a branch, a
    // `lazy`, a rebind, a dotted field and a record read whole.
    for (text, needle) in [
        ("fn f(g: Int): Int { g }\nx = f(len)", "len)"),
        ("x = [len]", "len]"),
        ("x = { inner = len }", "len }"),
        ("fn h(): Int { len }", "len }"),
        ("x = len == 1", "len =="),
        ("x = 1 === len", "len"),
        ("x = len is Int", "len is"),
        ("x = len as Int", "len as"),
        ("x = len : Int", "len :"),
        ("x = { let l = len; 1 }", "len;"),
        ("let mut l = len", "len"),
        ("let l: fn(xs: Int[]): Int = len", "len"),
        ("x = if true { len } else { len }", "len }"),
        ("x = lazy len", "len"),
        ("let l = len\nl := len", "len"),
        ("com.l = len", "len"),
        ("let l = len\nx = [l]", "l]"),
        ("l = len\nx = { l }", "l }"),
    ] {
        let found = diagnostics(text);
        assert!(!found.is_empty(), "{text}");
        assert!(
            found.iter().all(|error| error.message == NOT_A_VALUE),
            "{text}: {found:?}"
        );
        let start = u32::try_from(text.rfind(needle).unwrap()).unwrap();
        assert!(
            found.iter().any(|error| error.span.start == start),
            "{text}: {found:?}"
        );
    }
    // Where it may: the callee of a call, and the whole initialiser of
    // a top-level field or `let` of one name, each then an alias, of
    // the opaque type, called by its rule.
    assert_eq!(file_type("l = len\nn = l([1])"), "{l: builtin len, n: Int}");
    assert_eq!(
        file_type("let l = len\nm = l\nn = m([1])"),
        "{m: builtin len, n: Int}"
    );
    assert_eq!(shown_result("let l = len\nlet m = l\nm([[1]])"), "Int");
    assert_eq!(
        messages("let l = len\nn = l(1)"),
        ["`len` takes a list, found Int"]
    );
    // Through `builtins`, a path to its cell, never the record whole.
    assert_eq!(shown_result("import builtins\nbuiltins.len([1])"), "Int");
    assert_eq!(
        shown_result("import builtins\nlet l = builtins.len\nl([1])"),
        "Int"
    );
    assert_eq!(messages("import builtins\nlet b = builtins"), [NOT_A_VALUE]);
}

#[test]
fn int_and_float_have_function_types() {
    assert_eq!(shown_result("int"), "fn(x: Float): Int");
    assert_eq!(shown_result("float"), "fn(x: Int): Float");
    assert_eq!(shown_result("int(2.5)"), "Int");
    assert_eq!(shown_result("float(x = 2)"), "Float");
    assert_eq!(shown_result("half(1)"), "Float");
    assert_eq!(
        shown_result("import builtins\nbuiltins.int"),
        "fn(x: Float): Int"
    );
    assert_eq!(
        file_type("i = int\nf = [float]"),
        "{f: List<fn(x: Int): Float>, i: fn(x: Float): Int}"
    );
    // Called as any function is: an integer literal adapts to `Float`.
    assert_eq!(shown_result("int(1)"), "Int");
    assert_eq!(messages("int(1 // 1)"), ["mismatched types"]);
    assert_eq!(messages("float(1.5)"), ["mismatched types"]);
    // No equality, as for any function.
    assert_eq!(messages("int === int"), ["cannot compare functions"]);
    assert_eq!(messages("int == int"), ["cannot compare functions"]);
    // Pure: a pure function may call them.
    assert_eq!(
        type_of_f("fn f(x: Int): Int { int(float(x) * 1.5) }"),
        "fn(x: Int): Int"
    );
}

#[test]
fn the_string_builtins_have_function_types() {
    for (text, shown) in [
        ("split", "fn(s: String, sep: String): List<String>"),
        ("join", "fn(parts: List<String>, sep: String): String"),
        ("starts_with", "fn(s: String, prefix: String): Bool"),
        ("ends_with", "fn(s: String, suffix: String): Bool"),
        ("trim", "fn(s: String): String"),
        (
            "import builtins\nbuiltins.split",
            "fn(s: String, sep: String): List<String>",
        ),
        ("split(\"a/b\", \"/\")", "List<String>"),
        ("join([\"a\"], sep = \",\")", "String"),
        ("starts_with(prefix = \"/\", s = \"/usr\")", "Bool"),
        ("ends_with(\"x.gz\", \".gz\")", "Bool"),
        ("trim(\" x \")", "String"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // The summary's string lines.
    assert_eq!(
        file_type(
            "parts = split(\"a/b/c\", \"/\")\n\
                 s = join(parts, \"-\")\n\
                 yes = starts_with(\"/usr\", \"/\")\n\
                 gz = ends_with(\"x.tar.gz\", \".gz\")\n\
                 hash = trim(sh(\"echo h\"))"
        ),
        "{gz: Bool, hash: String, parts: List<String>, s: String, yes: Bool}"
    );
    // Their arguments are checked as any call's.
    assert_eq!(messages("split(1, \"/\")"), ["mismatched types"]);
    assert_eq!(messages("join([1], \",\")"), ["mismatched types"]);
    assert_eq!(messages("trim([\"x\"])"), ["mismatched types"]);
    assert_eq!(messages("split(\"a\")"), ["expected 2 arguments, found 1"]);
    assert_eq!(
        messages("ends_with(s = \"a\", prefix = \"b\")"),
        ["unknown argument `prefix`", "missing argument `suffix`"]
    );
    // Pure: a pure function may call them.
    assert_eq!(
        type_of_f(
            "fn f(p: String): Bool { starts_with(trim(join(split(p, \"/\"), \"-\")), \"-\") }"
        ),
        "fn(p: String): Bool"
    );
    // A file's own `join` shadows the prelude's silently.
    assert_eq!(
        file_type("join = \"mine\"\nv = join"),
        "{join: String, v: String}"
    );
}

#[test]
fn the_more_string_builtins_and_compare_versions_have_function_types() {
    for (text, shown) in [
        ("strip_prefix", "fn(s: String, prefix: String): String"),
        ("strip_suffix", "fn(s: String, suffix: String): String"),
        ("lines", "fn(s: String): List<String>"),
        ("fields", "fn(s: String): List<String>"),
        ("compare_versions", "fn(a: String, b: String): Int"),
        (
            "import builtins\nbuiltins.fields",
            "fn(s: String): List<String>",
        ),
        ("strip_prefix(\"ab\", \"a\")", "String"),
        ("strip_suffix(suffix = \".gz\", s = \"a.gz\")", "String"),
        ("lines(\"a\\nb\")", "List<String>"),
        ("fields(\" a b \")", "List<String>"),
        ("compare_versions(b = \"6.6\", a = \"6.10\")", "Int"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // The summary of slice 41.
    assert_eq!(
        file_type(
            "a = strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")\n\
                 b = strip_prefix(\"x\", \"y\")\n\
                 c = strip_suffix(\"a.tar.gz\", \".gz\")\n\
                 d = lines(\"a\\nb\\n\")\n\
                 e = fields(\"  tcp\\t6  TCP \")\n\
                 f = compare_versions(\"6.10\", \"6.6\")"
        ),
        "{a: String, b: String, c: String, d: List<String>, e: List<String>, f: Int}"
    );
    // Their arguments are checked as any call's.
    assert_eq!(messages("lines([\"x\"])"), ["mismatched types"]);
    assert_eq!(messages("compare_versions(1, \"2\")"), ["mismatched types"]);
    assert_eq!(
        messages("fields(\"a\", \"b\")"),
        ["expected 1 argument, found 2"]
    );
    assert_eq!(
        messages("strip_suffix(s = \"a\", prefix = \"b\")"),
        ["unknown argument `prefix`", "missing argument `suffix`"]
    );
    // Pure: a pure function may call them.
    assert_eq!(
        type_of_f(
            "fn f(v: String, w: String): Bool { \
                 compare_versions(strip_prefix(v, \"v\"), strip_suffix(w, \"\\n\")) > 0 }"
        ),
        "fn(v: String, w: String): Bool"
    );
    assert_eq!(
        type_of_f("fn f(s: String): Int { len(fields(lines(s)[0])) }"),
        "fn(s: String): Int"
    );
    // A file's own `fields` shadows the prelude's silently.
    assert_eq!(file_type("fields = 1\nv = fields"), "{fields: Int, v: Int}");
}

#[test]
fn the_file_system_builtins_have_impure_function_types() {
    for (text, shown) in [
        ("import fs\nfs.exists", "fn(p: String) -> Bool"),
        ("import fs\nfs.read", "fn(p: String) -> String"),
        ("import fs\nfs.mkdir", "fn(p: String) -> {}"),
        ("import fs\nfs.copy", "fn(from: String, to: String) -> {}"),
        ("import fs\nfs.temp_dir", "fn() -> String"),
        (
            "import builtins\nbuiltins.fs_read",
            "fn(p: String) -> String",
        ),
        ("import read from fs\nread", "fn(p: String) -> String"),
        ("import fs\nfs.exists(\"/\")", "Bool"),
        ("import fs\nfs.read(p = \"/x\")", "String"),
        ("import fs\nfs.mkdir(\"/x\")", "{}"),
        ("import fs\nfs.copy(to = \"/b\", from = \"/a\")", "{}"),
        ("import fs\nfs.temp_dir()", "String"),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // Not the prelude's: without `import fs`, no such names.
    assert_eq!(
        messages("fs_read(\"/x\")"),
        ["undefined variable `fs_read`"]
    );
    assert_eq!(messages("read(\"/x\")"), ["undefined variable `read`"]);
    // Their arguments are checked as any call's.
    assert_eq!(messages("import fs\nfs.read(1)"), ["mismatched types"]);
    assert_eq!(
        messages("import fs\nfs.temp_dir(\"/x\")"),
        ["expected 0 arguments, found 1"]
    );
    assert_eq!(
        messages("import fs\nfs.copy(\"/a\")"),
        ["expected 2 arguments, found 1"]
    );
    // Impure: from a pure function, the existing error, through the
    // module, a member import, an alias and `builtins` alike.
    for text in [
        "import fs\nfn f(p: String): String { fs.read(p) }",
        "import read from fs\nfn f(p: String): String { read(p) }",
        "import fs as files\nfn f(): String { files.temp_dir() }",
        "import builtins\nfn f(p: String): Bool { builtins.fs_exists(p) }",
        "import fs\nfn f(p: String): {} { fs.mkdir(p) }",
    ] {
        assert_eq!(
            messages(text),
            ["cannot call an impure function from a pure function"],
            "{text}"
        );
    }
    // From a `lazy` in a pure function, an impure or memoised function,
    // and the top level, it is fine.
    assert_eq!(
        type_of_f("import fs\nfn f(p: String): Future<String> { lazy { fs.read(p) } }"),
        "fn(p: String | Future<String>): Future<String>"
    );
    assert_eq!(
        type_of_f("import fs\nfn f() -> String { fs.read(\"/x\") }"),
        "fn() -> String"
    );
    assert_eq!(
        type_of_f("import fs\nfn f() ->> String { fs.temp_dir() }"),
        "fn() ->> String"
    );
}

#[test]
fn fs_write_and_sort_versions_have_function_types() {
    for (text, shown) in [
        (
            "import fs\nfs.write",
            "fn(p: String, contents: String) -> {}",
        ),
        (
            "import write from fs\nwrite",
            "fn(p: String, contents: String) -> {}",
        ),
        (
            "import builtins\nbuiltins.fs_write",
            "fn(p: String, contents: String) -> {}",
        ),
        ("import fs\nfs.write(\"/a\", \"x\")", "{}"),
        ("import fs\nfs.write(contents = \"x\", p = \"/a\")", "{}"),
        ("sort_versions", "fn(vs: List<String>): List<String>"),
        ("sort_versions([\"1\", \"0\"])", "List<String>"),
        ("sort_versions([])", "List<String>"),
        ("sort_versions(vs = [\"1\"])", "List<String>"),
        (
            "import builtins\nbuiltins.sort_versions",
            "fn(vs: List<String>): List<String>",
        ),
    ] {
        assert_eq!(shown_result(text), shown, "{text}");
    }
    // Not the prelude's: without `import fs`, no `write`.
    assert_eq!(
        messages("write(\"/a\", \"x\")"),
        ["undefined variable `write`"]
    );
    // Their arguments are checked as any call's.
    assert_eq!(
        messages("import fs\nfs.write(\"/a\", 1)"),
        ["mismatched types"]
    );
    assert_eq!(
        messages("import fs\nfs.write(\"/a\")"),
        ["expected 2 arguments, found 1"]
    );
    assert_eq!(messages("sort_versions([1])"), ["mismatched types"]);
    assert_eq!(messages("sort_versions(\"1\")"), ["mismatched types"]);
    // `fs.write` is impure: from a pure function, the existing error, by
    // any road; `sort_versions` is pure.
    for text in [
        "import fs\nfn f(p: String): {} { fs.write(p, \"x\") }",
        "import write from fs\nfn f(p: String): {} { write(p, \"x\") }",
        "import builtins\nfn f(p: String): {} { builtins.fs_write(p, \"x\") }",
    ] {
        assert_eq!(
            messages(text),
            ["cannot call an impure function from a pure function"],
            "{text}"
        );
    }
    assert_eq!(
        type_of_f("import fs\nfn f(p: String) -> {} { fs.write(p, \"x\") }"),
        "fn(p: String | Future<String>) -> {}"
    );
    assert_eq!(
        type_of_f("fn f(vs: List<String>): List<String> { sort_versions(vs) }"),
        "fn(vs: List<String>): List<String>"
    );
}

#[test]
fn show_is_typed_by_its_rule() {
    // Any type without a function in it, to `String`.
    for text in [
        "show(1)",
        "show(1.5)",
        "show(true)",
        "show(\"a\")",
        "show({})",
        "show({a = 1; b = [\"x\"]})",
        "show([[1], [2]])",
        "let u: Int | String = 1\nshow(u)",
        "type Pin = {version: String, commit: String}\nlet p: Pin = {version = \"1\"; commit = \"c\"}\nshow(p)",
        "show(value = 1)",
        "import builtins\nbuiltins.show(1)",
        "let s = show\ns(1)",
        "import show as render from prelude\nrender(1)",
        "show(lazy 1)",
    ] {
        assert_eq!(shown_result(text), "String", "{text}");
    }
    // A special built-in: shown as itself, and no value.
    assert_eq!(file_type("s = show"), "{s: builtin show}");
    let found = diagnostics("v = [show]");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "`show` is a built-in function and cannot be used as a value yet"
    );
    assert_eq!(
        found[0].notes,
        [
            "a built-in function with a type for any value cannot be written as a type yet; \
              call it, or bind it to a name"
        ]
    );
    // `len`'s note is as it was.
    assert_eq!(
        diagnostics("v = [len]")[0].notes,
        [
            "a built-in function with a type for any list cannot be written as a type yet; \
              call it, or bind it to a name"
        ]
    );
    // Its arguments bind as any call's.
    assert_eq!(messages("show()"), ["expected 1 argument, found 0"]);
    assert_eq!(messages("show(1, 2)"), ["expected 1 argument, found 2"]);
    assert_eq!(
        messages("show(v = 1)"),
        ["unknown argument `v`", "missing argument `value`"]
    );
    // `[]` alone has no element type to infer, as anywhere.
    assert_eq!(
        messages("show([])"),
        ["cannot infer the element type of `[]`"]
    );
    // Pure, and awaiting deeply as an interpolation does: in a pure
    // function a present argument is fine, its bare parameter made
    // present, and one that may be a future is the implicit-await error.
    assert_eq!(
        type_of_f("fn f(r: {a: Int}): String { show(r) }"),
        "fn(r: {a: Int}): String"
    );
    assert_eq!(
        type_of_f("fn f(xs: List<Int>): String { show(xs) }"),
        "fn(xs: List<Int>): String"
    );
    assert_eq!(
        messages("fn f(r: {a: Future<Int>}): String { show(r) }"),
        ["cannot use a value that may be a future in a pure function"]
    );
    assert_eq!(
        type_of_f("fn f(r: {a: Future<Int>}) -> String { show(r) }"),
        "fn(r: {a: Future<Int>} | Future<{a: Future<Int>}>) -> String"
    );
    // A value of type `Never` is never there to show.
    assert_eq!(shown_result("show(panic(\"p\"))"), "Never");
}

#[test]
fn show_rejects_a_function_anywhere_in_its_argument() {
    // A function anywhere in the argument's type is an error at the
    // argument, its type in a note: the function itself, a field, an
    // element, a member of a union, an alias's expansion, a built-in.
    for (text, argument, shown) in [
        (
            "show(fn(x: Int): Int { x })",
            "fn(x: Int): Int { x }",
            "`fn(x: Int): Int`",
        ),
        (
            "show({f = fn(): Int { 1 }})",
            "{f = fn(): Int { 1 }}",
            "`{f: fn(): Int}`",
        ),
        (
            "show([fn(): Int { 1 }])",
            "[fn(): Int { 1 }]",
            "`List<fn(): Int>`",
        ),
        (
            "let u: Int | fn(): Int = 1\nshow(u)",
            "u)",
            "`Int | (fn(): Int)`",
        ),
        (
            "type Handler = {run: fn(): Int}\nlet h: Handler = {run = fn(): Int { 1 }}\nshow(h)",
            "h)",
            "`{run: fn(): Int}`",
        ),
        ("show(int)", "int)", "`fn(x: Float): Int`"),
        (
            "show({e = exec})",
            "{e = exec}",
            "`{e: fn(argv: List<Revealed<String>>, cwd: String, env: {*: Revealed<String>}, \
             interactive: Bool, panic: Bool, until: String) -> {status: Int, stderr: String, \
             stdout: String}}`",
        ),
        (
            "show(lazy fn(): Int { 1 })",
            "lazy fn(): Int { 1 }",
            "`Future<fn(): Int>`",
        ),
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(
            found[0].message, "cannot show a function: `show` renders data as source",
            "{text}"
        );
        // The argument of the last `show(` of the text.
        let argument = argument.trim_end_matches(')');
        let start = text.rfind("show(").unwrap() + "show(".len();
        let span = Span::new(
            u32::try_from(start).unwrap(),
            u32::try_from(start + argument.len()).unwrap(),
        );
        assert_eq!(&text[start..start + argument.len()], argument, "{text}");
        assert_eq!(found[0].span, span, "{text}");
        assert_eq!(
            found[0].notes,
            [format!("the argument is of type {shown}")],
            "{text}"
        );
        assert!(found[0].labels.is_empty(), "{text}");
    }
    // `len` as the argument is the alias rule's error, not this one.
    assert_eq!(
        messages("show(len)"),
        ["`len` is a built-in function and cannot be used as a value yet"]
    );
    // One error, not two, for a function that may be a future in a pure
    // function.
    assert_eq!(
        messages("fn f(g: Future<fn(): Int>): String { show(g) }"),
        ["cannot show a function: `show` renders data as source"]
    );
}

/// The type `exec` returns, as the type table shows it.
pub(super) const EXEC_RESULT: &str = "{status: Int, stderr: String, stdout: String}";

/// The type of the prelude's `exec`, which takes the built-in's options by
/// name.
const EXEC_TYPE: &str = "fn(argv: List<Revealed<String>>, cwd: String, env: {*: Revealed<String>}, \
                         interactive: Bool, panic: Bool, until: String) -> {status: Int, stderr: \
                         String, stdout: String}";

/// The type of the built-in `exec`, which takes its options as a record.
const BUILTIN_EXEC_TYPE: &str = "fn(argv: List<Revealed<String>>, options: {cwd: String, env: {*: \
                                 Revealed<String>}, interactive: Bool, panic: Bool, until: \
                                 String}) -> {status: Int, stderr: String, stdout: String}";

#[test]
fn exec_and_panic_have_function_types() {
    assert_eq!(shown_result("exec"), EXEC_TYPE);
    assert_eq!(shown_result("panic"), "fn(message: String): Never");
    assert_eq!(
        shown_result("import builtins\nbuiltins.exec"),
        BUILTIN_EXEC_TYPE
    );
    // The summary of slice 29.
    assert_eq!(
        file_type(
            "r = exec([\"ls\", \"-1\", \"src\"])\n\
                 lines = r.stdout\n\
                 ok = r.status == 0\n\
                 out = sh(\"git rev-parse HEAD\")"
        ),
        format!("{{lines: String, ok: Bool, out: String, r: {EXEC_RESULT}}}")
    );
    assert_eq!(
        type_of_f(
            "fn f(name: String): { build: Future<String> } { { build = lazy { sh(\"make {name}\") } } }"
        ),
        "fn(name: String | Future<String>): {build: Future<String>}"
    );
    assert_eq!(
        type_of_f("fn f(n: Int): Int {\nif n < 0 { panic(\"negative size {n}\") }\nn\n}"),
        "fn(n: Int): Int"
    );
    // Arguments bind by position or by name, as any call's.
    assert_eq!(shown_result("exec(argv = [\"true\"])"), EXEC_RESULT);
    assert_eq!(shown_result("panic(message = \"no\")"), "Never");
    assert_eq!(messages("exec([1])"), ["mismatched types"]);
    assert_eq!(messages("exec(\"true\")"), ["mismatched types"]);
    assert_eq!(messages("panic(1)"), ["mismatched types"]);
    assert_eq!(
        messages("exec(args = [\"true\"])"),
        ["unknown argument `args`", "missing argument `argv`"]
    );
    assert_eq!(messages("panic()"), ["expected 1 argument, found 0"]);
    // A value of the built-in's type: the display of each ordinary
    // built-in is the type table's, so that the two never drift.
    let mut types = TypeTable::new();
    let mut names = Names::new();
    for builtin in Builtin::ALL {
        let ty = builtin.ty(&mut types, &mut names);
        let shown = types.display(ty, &names).to_string();
        match builtin.typing() {
            Typing::Ordinary => assert_eq!(builtin.display(), shown),
            Typing::Special => {
                assert_eq!(builtin.display(), format!("builtin {}", builtin.name()));
            }
        }
        assert!(builtin.params().len() <= Builtin::MAX_PARAMS);
    }
}

#[test]
fn exec_is_impure_and_called_as_any_impure_function() {
    // From a pure function, the existing error.
    let text = "fn f(): Int { exec([\"true\"]).status }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, at(text, "exec([\"true\"])"));
    assert_eq!(
        labels(&found[0])[0],
        (at(text, "exec"), format!("this is {EXEC_TYPE}").as_str())
    );
    // Through a variable or a parameter, the same.
    assert_eq!(
        messages("let e = sh\nfn f(): String { e(\"true\") }"),
        ["cannot call an impure function from a pure function"]
    );
    // From a `lazy` in a pure function, an impure or memoised function,
    // and the top level, it is fine.
    assert_eq!(
        type_of_f("fn f(): Future<Int> { lazy { exec([\"true\"]).status } }"),
        "fn(): Future<Int>"
    );
    assert_eq!(
        type_of_f("fn f() -> Int { exec([\"true\"]).status }"),
        "fn() -> Int"
    );
    assert_eq!(
        type_of_f("fn f(a: String) ->> String { exec([a]).stdout }"),
        "fn(a: String) ->> String"
    );
    assert_eq!(shown_result("exec([\"true\"]).status"), "Int");
    // As a value: passed, stored, called through a variable; it has no
    // equality, as no function has. Its parameter is a present list, so
    // it fits a written impure function type whose parameter says
    // `Present`, as an impure function declared so does, and not one
    // whose bare parameter may be a future.
    let options = "import builtins\n\
                   type Options = {cwd: String, env: {*: String}, interactive: Bool, panic: Bool, \
                   until: String}\n";
    let given = "{cwd = \"\"; env = {}; interactive = false; panic = false; until = \"\"}";
    let pick = "fn pick(f: fn(argv: Present<List<String>>, options: Present<Options>) -> \
                    {status: Int}): fn(Present<List<String>>, Present<Options>) -> {status: Int} \
                    { f }\n";
    assert_eq!(
        shown_result(&format!(
            "{options}{pick}let e = pick(builtins.exec)\ne([\"true\"], {given}).status"
        )),
        "Int | Future<Int>"
    );
    assert_eq!(
        messages(&format!(
            "{options}fn pick(f: fn(argv: List<String>, options: Options) -> {{status: Int}}): \
             Int {{ 1 }}\npick(builtins.exec)"
        )),
        ["mismatched types"]
    );
    // The prelude's `exec`, through a value, is given every argument: only
    // a call by its own name uses its defaults.
    assert_eq!(
        shown_result(
            "let r = { e = exec }\nlet e = r.e\ne([\"true\"], \"\", {}, false, false, \"\").stdout"
        ),
        "String"
    );
    assert_eq!(
        messages("let e = exec\ne([\"true\"])"),
        ["expected 6 arguments, found 1"]
    );
    assert_eq!(messages("exec === exec"), ["cannot compare functions"]);
    assert_eq!(messages("panic === panic"), ["cannot compare functions"]);
}

#[test]
fn exec_live_is_typed_as_exec_and_impure() {
    assert_eq!(shown_result("exec_live"), shown_result("exec"));
    assert_eq!(
        shown_result("import builtins\nbuiltins.exec_live"),
        BUILTIN_EXEC_TYPE
    );
    assert_eq!(shown_result("exec_live([\"true\"])"), EXEC_RESULT);
    assert_eq!(shown_result("exec_live(argv = [\"true\"])"), EXEC_RESULT);
    assert_eq!(messages("exec_live([1])"), ["mismatched types"]);
    // From a pure function, the existing error, through its name or a
    // variable; from a `lazy`, an impure function or the top level, fine.
    let text = "fn f(): Int { exec_live([\"true\"]).status }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, at(text, "exec_live([\"true\"])"));
    assert_eq!(
        messages("let e = sh_live\nfn f(): String { e(\"true\") }"),
        ["cannot call an impure function from a pure function"]
    );
    assert_eq!(
        messages("fn f(): String { sh_live(\"true\") }"),
        ["cannot call an impure function from a pure function"]
    );
    assert_eq!(
        type_of_f("fn f(): Future<Int> { lazy { exec_live([\"true\"]).status } }"),
        "fn(): Future<Int>"
    );
    assert_eq!(
        type_of_f("fn f() -> String { sh_live(\"true\") }"),
        "fn() -> String"
    );
    assert_eq!(messages("exec_live === exec"), ["cannot compare functions"]);
}

#[test]
fn exec_awaits_a_future_argument_as_any_call_does() {
    // At the top level and in an impure function, a future list is
    // awaited at the top, as an argument of any function whose
    // parameter is a present `List<String>`.
    assert_eq!(
        marks("let xs = lazy [\"true\"]\nexec(xs)"),
        vec![("xs", AwaitMark::Top)]
    );
    assert_eq!(
        marks("fn f(xs: Future<List<String>>) -> Int { exec(xs).status }"),
        vec![("xs", AwaitMark::Top)]
    );
    // A list of futures is awaited deeply, as for any parameter of a
    // present `List<String>`.
    assert_eq!(
        marks("let xs = [lazy \"true\"]\nexec(xs)"),
        vec![("xs", AwaitMark::Deep)]
    );
    // In a pure function, the call is the purity error, and so is the
    // await, as for any call.
    assert_eq!(
        messages("fn f(xs: Future<List<String>>): Int { exec(xs).status }"),
        [
            "cannot call an impure function from a pure function",
            "cannot use a value that may be a future in a pure function"
        ]
    );
}

#[test]
fn a_call_of_panic_is_of_type_never() {
    assert_eq!(shown_result("panic(\"x\")"), "Never");
    assert_eq!(
        shown_result("let c = true\nif c { panic(\"x\") } else { 1 }"),
        "Int"
    );
    assert_eq!(
        shown_result("let c = true\nif c { 1 } else { panic(\"x\") }"),
        "Int"
    );
    // An `if` whose branch panics needs no `else`.
    assert_eq!(
        type_of_f("fn f(c: Bool): Int {\nif c { panic(\"x\") }\n1\n}"),
        "fn(c: Bool): Int"
    );
    // A `let` of it and a statement of it are fine; what follows is
    // never run, which is a warning, not an error.
    for text in ["let x = panic(\"no\")\n1", "panic(\"no\")\n1"] {
        let found = warnings(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, "this is never run", "{text}");
    }
    assert!(warnings("let x = panic(\"no\")").is_empty());
    assert!(warnings("panic(\"no\")").is_empty());
    // `panic` fits a parameter of type `fn(String): Int`, since `Never`
    // fits every result.
    assert_eq!(
        shown_result("fn apply(f: fn(String): Int): Int { f(\"a\") }\napply(panic)"),
        "Int"
    );
    assert_eq!(
        shown_result("let p = panic\nlet fs: List<fn(m: String): Int> = [p]\nfs"),
        "List<fn(m: String): Int>"
    );
    // Pure: a pure function may call it.
    assert_eq!(
        type_of_f("fn f(s: String): Int { panic(s) }"),
        "fn(s: String): Int"
    );
}

#[test]
fn exec_panic_and_sh_are_shadowed_as_any_prelude_name() {
    assert_eq!(file_type("exec = 1\nn = exec + 1"), "{exec: Int, n: Int}");
    assert_eq!(
        file_type("def sh(n: Int): Int { n * 2 }\nx = sh(2)"),
        "{sh: fn(n: Int): Int, x: Int}"
    );
    assert_eq!(shown_result("let panic = \"no\"\npanic"), "String");
    assert_eq!(
        shown_result("fn panic(m: String): Int { 0 }\npanic(\"x\")"),
        "Int"
    );
}

#[test]
fn a_built_in_s_arguments_are_bound_as_a_function_s() {
    let text = "len(1)";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].message, "`len` takes a list, found Int");
    assert_eq!(found[0].span, at(text, "1"));
    assert_eq!(found[0].label.as_deref(), Some("this is Int"));
    assert_eq!(
        messages("len({ a = 1 })"),
        vec!["`len` takes a list, found `{a: Int}`"]
    );
    let text = "len([1], [2])";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "expected 1 argument, found 2");
    assert_eq!(found[0].span, Span::new(0, 13));
    assert_eq!(
        labels(&found[0]),
        vec![
            (at(text, "len"), "`len` is a built-in function"),
            (at(text, "[2]"), "unexpected argument")
        ]
    );
    let text = "len()";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "expected 1 argument, found 0");
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "len"), "`len` is a built-in function")]
    );
    let text = "len(lst = [1])";
    assert_eq!(
        errors(text),
        vec![
            ("unknown argument `lst`".to_owned(), at(text, "lst")),
            ("missing argument `list`".to_owned(), at(text, ")"))
        ]
    );
    let text = "len([1], list = [2])";
    assert_eq!(
        errors(text),
        vec![(
            "argument `list` is given twice".to_owned(),
            at(text, "list = [2]")
        )]
    );
    // Every argument is checked whatever goes wrong.
    assert_eq!(
        messages("len(y, z)"),
        vec![
            "expected 1 argument, found 2",
            "undefined variable `y`",
            "undefined variable `z`"
        ]
    );
}

#[test]
fn lists_compare_as_records_do() {
    assert_eq!(shown_result("[1] == [1]"), "Bool");
    assert_eq!(shown_result("[[1]] !== [[2]]"), "Bool");
    assert_eq!(shown_result("[{ a = 1 }] == [{ a = 2 }]"), "Bool");
    // Either elements are awaited deeply first.
    let text = "let xs = [1, lazy 2]\nxs == xs";
    assert_eq!(
        marks(text),
        vec![("xs", AwaitMark::Deep), ("xs", AwaitMark::Deep)]
    );
    // Their types must be identical, without a future or a function.
    assert_eq!(
        messages("[1] == [1.5]"),
        vec!["cannot compare `List<Int>` and `List<Float>`"]
    );
    let text = "let xs: List<Future<Int>> = [lazy 1]\nxs == xs";
    assert_eq!(
        messages(text),
        vec!["cannot compare `List<Future<Int>>`: it contains a future"]
    );
    let text = "let fs = [fn(): Int { 1 }]\nfs == fs";
    assert_eq!(
        messages(text),
        vec!["cannot compare `List<fn(): Int>`: it contains a function"]
    );
    // They have no order, and no arithmetic.
    let text = "[1] < [2]";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "cannot order `List<Int>`");
    assert_eq!(
        labels(&found[0]),
        vec![
            (at(text, "[1]"), "List<Int>"),
            (at(text, "[2]"), "List<Int>")
        ]
    );
    assert_eq!(
        messages("[1] + [2]"),
        vec!["cannot add `List<Int>` and `List<Int>`"]
    );
    assert_eq!(messages("-[1]"), vec!["cannot negate `List<Int>`"]);
    assert_eq!(messages("[true] and true"), vec!["mismatched types"]);
}

#[test]
fn interpolating_a_list_awaits_it_deeply() {
    let text = "let xs = [lazy 1]\n\"{xs}\"";
    assert_eq!(marks(text), vec![("xs", AwaitMark::Deep)]);
    assert_eq!(shown_result(text), "String");
    let text = "let xs = [1]\n\"{xs}\"";
    assert_eq!(marks(text), vec![]);
}
