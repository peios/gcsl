//! Tests of the parser: attributes, `type` items, annotated fields, def-types,
//! metas, and definitions' paths.

use super::*;

/// The items of `text`'s body marked `#[shadowable]`, by position, and
/// whether the file is marked `#![no_prelude]`.
pub(super) fn attributes(text: &str) -> (Vec<usize>, Option<Span>) {
    let ast = parse_text(text).unwrap();
    let marked = ast
        .items(ast.root())
        .iter()
        .enumerate()
        .filter(|&(_, &item)| ast.is_shadowable(item))
        .map(|(position, _)| position)
        .collect();
    (marked, ast.no_prelude())
}

#[test]
fn attributes_mark_items_and_the_file() {
    // On its item's line or a line before it, comments and blank lines
    // between; a field, a dotted field, a `def` and an import of each
    // form.
    let text = "# a comment\n\n#![no_prelude]\n#[shadowable] a = 1\nb = 2\n#[shadowable]\n\n\
                    # between\nc.d = 3\n#[shadowable] def f(): Int { 1 }\n\
                    #[shadowable] import x\n#[shadowable] import * from y\n\
                    #[shadowable] import z as w\n";
    assert_eq!(
        attributes(text),
        (vec![0, 2, 3, 4, 5, 6], Some(Span::new(13, 27)))
    );
    assert_eq!(attributes("a = 1"), (vec![], None));
    // The printer shows them.
    assert_eq!(
        parse_text("#![no_prelude]\n#[shadowable] a = 1\nb = 2")
            .unwrap()
            .pretty()
            .to_string(),
        "(file #![no_prelude]\n  #[shadowable] (field a @29..34\n    (int 1 @33..34))\n  \
             (field b @35..40\n    (int 2 @39..40)))"
    );
}

#[test]
fn misplaced_and_unknown_attributes_are_errors() {
    let placed = MISPLACED_SHADOWABLE.to_owned();
    // An outer one that is none of the parser's is a use, which the
    // checker resolves, and applies only to a function.
    assert_eq!(
        error("#[sorted] a = 1"),
        (MISPLACED_USE.to_owned(), Span::new(0, 9))
    );
    assert_eq!(
        error("#![sorted]"),
        ("unknown attribute `sorted`".to_owned(), Span::new(0, 10))
    );
    assert_eq!(
        error("#![shadowable]\na = 1"),
        (
            "`shadowable` applies to a declaration: write `#[shadowable]`".to_owned(),
            Span::new(0, 14)
        )
    );
    assert_eq!(
        error("#[no_prelude]\na = 1"),
        (
            "`no_prelude` applies to the whole file: write `#![no_prelude]`".to_owned(),
            Span::new(0, 13)
        )
    );
    assert_eq!(
        error("#[shadowable] #[shadowable] a = 1"),
        (
            "`#[shadowable]` is written twice".to_owned(),
            Span::new(14, 27)
        )
    );
    assert_eq!(
        error("#![no_prelude]\n#![no_prelude]"),
        (
            "`#![no_prelude]` is written twice".to_owned(),
            Span::new(15, 29)
        )
    );
    for (text, at) in [
        ("a = 1\n#![no_prelude]", Span::new(6, 20)),
        ("#[shadowable]\n#![no_prelude]\na = 1", Span::new(14, 28)),
        ("a = { #![no_prelude] }", Span::new(6, 20)),
    ] {
        assert_eq!(error(text), (MISPLACED_NO_PRELUDE.to_owned(), at), "{text}");
    }
    // Before a `let`, a `let mut` or a `fn`, with the note.
    for text in [
        "#[shadowable] let a = 1",
        "#[shadowable] let mut a = 1",
        "#[shadowable] fn f(): Int { 1 }",
    ] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(
            (diagnostic.message.as_str(), diagnostic.span),
            (MISPLACED_SHADOWABLE, Span::new(0, 13)),
            "{text}"
        );
        assert_eq!(
            diagnostic.notes,
            vec![LET_SHADOWS_NOTE.to_owned()],
            "{text}"
        );
    }
    // Before a `:=`, an expression, a spread, a result or a `break`, or
    // in an inner block or a `for` header, without it.
    for (text, at) in [
        ("let a = 1\n#[shadowable] a := 2", Span::new(10, 23)),
        ("#[shadowable] f()", Span::new(0, 13)),
        ("#[shadowable] ...{}", Span::new(0, 13)),
        ("#[shadowable] = 1", Span::new(0, 13)),
        ("x = { #[shadowable] a = 1 }", Span::new(6, 19)),
        ("x = { #[shadowable] let a = 1; = a }", Span::new(6, 19)),
        ("for #[shadowable] let i = 0; i < 1; {}", Span::new(4, 17)),
    ] {
        let diagnostic = parse_text(text).unwrap_err();
        assert_eq!(
            (diagnostic.message.clone(), diagnostic.span),
            (placed.clone(), at),
            "{text}"
        );
        let note = text.contains("let a = 1;");
        assert_eq!(diagnostic.notes.is_empty(), !note, "{text}");
    }
    // With nothing after it.
    for (text, at) in [
        ("a = 1\n#[shadowable]", Span::new(6, 19)),
        ("#[shadowable]\n# a comment\n", Span::new(0, 13)),
        ("x = { a = 1; #[shadowable] }", Span::new(13, 26)),
    ] {
        assert_eq!(
            error(text),
            ("`#[shadowable]` is not followed by anything".to_owned(), at),
            "{text}"
        );
    }
    // Where an expression starts, as any token is.
    assert_eq!(
        message("a = #[shadowable] 1"),
        "expected expression, found `#[shadowable]`"
    );
}

#[test]
fn type_items_parse_and_print_back() {
    assert_eq!(shape("type N = Int"), "type N = Int");
    assert_eq!(
        shape("type P = {\n  version: String,\n  arch: String,\n}"),
        "type P = {version: String, arch: String}"
    );
    assert_eq!(
        shape("type Package = { ...PackageDef, name: String }"),
        "type Package = {...PackageDef, name: String}"
    );
    assert_eq!(
        shape("type Q = {...t.P, ...{a: Int}}"),
        "type Q = {...t.P, ...{a: Int}}"
    );
    assert_eq!(
        shape("type Steps =\n  List<fn() -> String>"),
        "type Steps = List<fn() -> String>"
    );
    // A qualified name in any type position.
    assert_eq!(
        shape("x : a.b.T; fn f(p: t.P): t.Q { p }"),
        "(x : a.b.T); fn f(p: t.P): t.Q {p}"
    );
    let ast = parse_text("type P = {a: Int}").unwrap();
    let item = ast.item(ast.items(ast.root())[0]);
    assert_eq!(item.span, Span::new(0, 17));
    let ItemKind::Type { name_span, .. } = item.kind else {
        panic!("expected a type item");
    };
    assert_eq!(name_span, Span::new(5, 6));
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n  (type P {a: Int} @0..17))"
    );
}

#[test]
fn type_is_a_contextual_keyword() {
    // Only `type` followed on its line by an identifier, at the start
    // of an item, begins a `type` item: everywhere else it is a name,
    // as it was.
    assert_eq!(shape("type = 1"), "type = 1");
    assert_eq!(shape("let type = 1; type"), "let type = 1; type");
    assert_eq!(shape("type.x = 1"), "type.x = 1");
    assert_eq!(shape("a = { type = 1 }.type"), "a = ({type = 1} . type)");
    assert_eq!(
        shape("fn f(type: Int): Int { type }"),
        "fn f(type: Int): Int {type}"
    );
    assert_eq!(shape("type\nFoo"), "type; Foo");
    assert_eq!(shape("type: Int = 1"), "type: Int = 1");
}

#[test]
fn type_item_errors() {
    for (text, at) in [
        ("x = { type T = Int }", Span::new(6, 10)),
        ("fn f(): Int { type T = Int; 1 }", Span::new(14, 18)),
        ("for { type T = Int }", Span::new(6, 10)),
    ] {
        assert_eq!(error(text), (MISPLACED_TYPE.to_owned(), at), "{text}");
    }
    assert_eq!(
        error("type Box<T, T> = {v: T}"),
        (
            "type parameter `T` is declared twice".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("type T Int"),
        (
            "expected `=` after the type's name, found `Int`".to_owned(),
            Span::new(7, 10)
        )
    );
    assert_eq!(
        error("#[shadowable] type T = Int"),
        (MISPLACED_SHADOWABLE.to_owned(), Span::new(0, 13))
    );
    assert_eq!(
        error("x : t."),
        (
            "expected a type's name after `.`".to_owned(),
            Span::new(6, 6)
        )
    );
    assert_eq!(
        error("type T = {...}"),
        ("expected a type".to_owned(), Span::new(13, 14))
    );
}

#[test]
fn annotated_fields_parse() {
    assert_eq!(shape("x: Int = 1"), "x: Int = 1");
    assert_eq!(shape("a.b: Int | String = 1"), "a.b: (Int | String) = 1");
    assert_eq!(shape("\"a b\": Float = 2"), "\"a b\": Float = 2");
    assert_eq!(
        shape("x: {\n  a: Int,\n} = { a = 1 }"),
        "x: {a: Int} = {a = 1}"
    );
    assert_eq!(shape("x:\n  Int = 1"), "x: Int = 1");
    assert_eq!(shape("x: Int |\n  String = 1"), "x: (Int | String) = 1");
    assert_eq!(shape("{ x: Int = 1 }"), "{x: Int = 1}");
    // Ascriptions as they were: a block's result, a statement, an item
    // ending before an `=` on the next line.
    assert_eq!(shape("{ x : Int }"), "{(x : Int)}");
    assert_eq!(shape("x : {}"), "(x : {})");
    assert_eq!(shape("x : Int\ny = 2"), "(x : Int); y = 2");
    assert_eq!(shape("x : Int { a = 1 }"), "(x : Int)(){a = 1}");
    // An annotation is followed by `=`; a rebind is not annotated.
    assert_eq!(
        error("x: Int := 1"),
        (
            "expected end of item, found `:=`".to_owned(),
            Span::new(7, 9)
        )
    );
    // The annotation counts towards the item's height, as a `let`'s.
    let ast = parse_text("a.b: Int = 1").unwrap();
    let ItemKind::Field { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated field");
    };
    assert_eq!(ast.type_expr(ty).span, Span::new(5, 8));
    assert_eq!(
        ast.pretty().to_string(),
        "(file\n  (field a.b : Int @0..12\n    (int 1 @11..12)))"
    );
}

#[test]
fn def_types_are_marked_defined_and_referred_to() {
    // `#[def_type]` on a `def` of one name or a path, with or without
    // `#[shadowable]`, in either order.
    let ast = parse_text(
        "#[def_type] def p(n: String, f: fn(): Int): Int { 1 }\n\
             #[shadowable]\n#[def_type]\ndef t.q(n: String, f: fn(): Int): Int { 1 }\n\
             #[def_type] #[shadowable] def r(n: String, f: fn(): Int): Int { 1 }",
    )
    .unwrap();
    let items = ast.items(ast.root());
    let marks: Vec<(bool, bool)> = items
        .iter()
        .map(|&item| (ast.is_def_type(item), ast.is_shadowable(item)))
        .collect();
    assert_eq!(marks, [(true, false), (true, true), (true, true)]);
    // A definition: its def-type a name or a path, its path a path, its
    // parameters by name alone; a reference, a def-type and a path on
    // one line, called or not, binding as a field access does.
    assert_eq!(
        shape("def package dev.peios.testpkg() { version = \"1\" }"),
        "def package dev.peios.testpkg() {version = \"1\"}"
    );
    assert_eq!(
        shape("def t.build x(a, b) { a }"),
        "def t.build x(a, b) {a}"
    );
    assert_eq!(
        shape("result = package dev.peios.testpkg()"),
        "result = (package dev.peios.testpkg)()"
    );
    assert_eq!(shape("let f = t.p a.b"), "let f = (t.p a.b)");
    assert_eq!(shape("x = -p a"), "x = (-(p a))");
    assert_eq!(shape("x = p a(1).y"), "x = ((p a)(1) . y)");
    // Two names on two lines are two items, or, inside parentheses, the
    // error they were.
    assert_eq!(shape("x = p\na"), "x = p; a");
    assert_eq!(
        error("f(p\na)"),
        ("expected `,` or `)`, found `a`".to_owned(), Span::new(4, 5))
    );
    // The dump shows them.
    assert_eq!(
        parse_text(
            "#[def_type] def p(n: String, f: fn(a: Int): Int): Int { 1 }\n\
                        def p x(a) { a }\ny = p x"
        )
        .unwrap()
        .pretty()
        .to_string(),
        "(file\n  #[def_type] (def p @12..59\n    (fn @12..59\n      \
             (params (n String) (f fn(a: Int): Int))\n      (ret pure Int)\n      \
             (block @54..59\n        (expr @56..57\n          (int 1 @56..57)))))\n  \
             (def-type-def p x @60..76\n    (fn @66..76\n      (params a)\n      \
             (block @71..76\n        (expr @73..74\n          (name a @73..74)))))\n  \
             (field y @77..84\n    (def-type-ref p x @81..84)))"
    );
}

#[test]
fn def_type_errors() {
    let misplaced = MISPLACED_DEF_TYPE.to_owned();
    assert_eq!(
        error("#[def_type] a = 1"),
        (misplaced.clone(), Span::new(0, 11))
    );
    assert_eq!(
        error("#[def_type] def p x() { 1 }"),
        (misplaced.clone(), Span::new(0, 11))
    );
    assert_eq!(
        error("#[def_type] fn f(): Int { 1 }"),
        (misplaced.clone(), Span::new(0, 11))
    );
    assert_eq!(
        error("x = { #[def_type] def f(): Int { 1 } }"),
        (misplaced, Span::new(6, 17))
    );
    assert_eq!(
        error("#[def_type]\n#[def_type] def f(): Int { 1 }"),
        (
            "`#[def_type]` is written twice".to_owned(),
            Span::new(12, 23)
        )
    );
    assert_eq!(
        error("#![def_type]\na = 1"),
        (
            "`def_type` applies to a declaration: write `#[def_type]`".to_owned(),
            Span::new(0, 12)
        )
    );
    assert_eq!(
        error("a = 1\n#[def_type]"),
        (
            "`#[def_type]` is not followed by anything".to_owned(),
            Span::new(6, 17)
        )
    );
    // A definition's own errors.
    assert_eq!(
        error("def p x(a: Int) { a }"),
        (
            "`a` is a parameter of the definition's own, which `p` calls with none: give it a \
             default, or leave its type to `p`"
                .to_owned(),
            Span::new(8, 14)
        )
    );
    assert_eq!(
        error("def t.p x(): Int { 1 }"),
        (
            "the result type of a `t.p` definition comes from `t.p`".to_owned(),
            Span::new(11, 12)
        )
    );
    assert_eq!(
        error("def p x.\"y\"() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(8, 11))
    );
    assert_eq!(
        error("def p x { 1 }"),
        (
            "expected `(` after the definition's path, found `{`".to_owned(),
            Span::new(8, 9)
        )
    );
    assert_eq!(
        error("def p x() 1"),
        (
            "expected `{` to start the definition's body, found `1`".to_owned(),
            Span::new(10, 11)
        )
    );
    assert_eq!(
        error("x = { def p x() { 1 } }"),
        (MISPLACED_DEF_TYPE_DEF.to_owned(), Span::new(6, 9))
    );
    // A reference's path is made of identifiers too.
    assert_eq!(
        error("x = p a.\"b\""),
        (DEF_PATH.to_owned(), Span::new(8, 11))
    );
    // But an index ends it, indexing what it reads; a definition's path
    // and a meta's have none.
    assert_eq!(shape("x = p a.b[0].c"), "x = (((p a.b) at 0) . c)");
    assert_eq!(
        error("def p a[0]() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(7, 10))
    );
    assert_eq!(
        error("defmeta p a[0] { b = 1 }"),
        (DEF_PATH.to_owned(), Span::new(11, 14))
    );
    // No source writes a namespace's key.
    assert_eq!(
        error("\"<p#1.2>\" = 1"),
        (
            "a field cannot be named `\"<p#1.2>\"`: names of that form are def-types' \
                 namespaces"
                .to_owned(),
            Span::new(0, 9)
        )
    );
}

/// The meta of the outermost function type of the annotation of the
/// only item of `text`, a `let`: its marker's effect and its type's
/// span.
pub(super) fn annotation_meta(text: &str) -> Option<(Effect, Span)> {
    let ast = parse_text(text).unwrap();
    let ItemKind::Let { ty: Some(ty), .. } = ast.item(ast.items(ast.root())[0]).kind else {
        panic!("expected an annotated `let`");
    };
    let TypeExprKind::Fn { meta, .. } = ast.type_expr(ty).kind else {
        panic!("expected a function type");
    };
    meta.map(|meta| (meta.effect, ast.type_expr(meta.ty).span))
}

#[test]
fn a_function_type_may_have_a_meta() {
    // After the return type, a union included, `META` and a marker.
    assert_eq!(
        shape("let f: fn(v: Int): Int META: {a: Int} = g"),
        "let f: fn(v: Int): Int META: {a: Int} = g"
    );
    assert_eq!(
        shape("let f: fn() -> Int | String META-> {a: Int} = g"),
        "let f: fn() -> (Int | String) META-> {a: Int} = g"
    );
    assert_eq!(
        annotation_meta("let f: fn() ->> Int META->> Future<{a: Int}> = g"),
        Some((Effect::Memo, Span::new(28, 44)))
    );
    assert_eq!(
        annotation_meta("let f: fn(): Int META-> {a: Int} = g"),
        Some((Effect::Impure, Span::new(24, 32)))
    );
    // A meta after the return type of a function type returned belongs
    // to the inner one, unless it is in parentheses.
    assert_eq!(
        annotation_meta("let f: fn(): fn(): Int META: {a: Int} = g"),
        None
    );
    assert_eq!(
        annotation_meta("let f: fn(): (fn(): Int) META: {a: Int} = g"),
        Some((Effect::Pure, Span::new(31, 39)))
    );
    // Newlines inside parentheses are skipped, after the marker too.
    assert_eq!(
        annotation_meta("let f: (fn(): Int\nMETA:\n{a: Int}) = g"),
        Some((Effect::Pure, Span::new(24, 32)))
    );
    // The dump prints it as written.
    assert_eq!(
        parse_text("x : fn(): (fn(): Int) META-> A")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (expr @0..30\n    (: fn(): (fn(): Int) META-> A @0..30\n      \
             (name x @0..1))))"
    );
    // `META` is a name everywhere else.
    assert_eq!(shape("META = 1\nlet y = META"), "META = 1; let y = META");
    assert_eq!(shape("let f: fn(): META = g"), "let f: fn(): META = g");
    // A meta's marker anywhere but after a function type's return type.
    assert_eq!(
        error("x = fn(): Int META: {a: Int} { 1 }"),
        (MISPLACED_META.to_owned(), Span::new(14, 19))
    );
    assert_eq!(
        error("def f() -> Int META-> {a: Int} { 1 }"),
        (MISPLACED_META.to_owned(), Span::new(15, 21))
    );
    assert_eq!(
        error("def p x() META->> {a: Int} { 1 }"),
        (MISPLACED_META.to_owned(), Span::new(10, 17))
    );
}

#[test]
fn a_defmeta_is_a_function_of_its_body() {
    assert_eq!(
        shape("defmeta p dev.x { a = 1 }"),
        "defmeta p dev.x {a = 1}"
    );
    assert_eq!(
        shape("defmeta t.p x = lazy { a = 1 }"),
        "defmeta t.p x {= (lazy {a = 1})}"
    );
    // The dump: its function, of no parameters, its body the block, or
    // a block whose result is the expression.
    assert_eq!(
        parse_text("defmeta p x = 1\ndefmeta p y { a = 2 }")
            .unwrap()
            .pretty()
            .to_string(),
        "(file\n  (defmeta p x @0..15\n    (fn @10..15\n      (params)\n      \
             (block @14..15\n        (result @12..15\n          (int 1 @14..15)))))\n  \
             (defmeta p y @16..37\n    (fn @26..37\n      (params)\n      \
             (block @28..37\n        (field a @30..35\n          (int 2 @34..35))))))"
    );
    // `defmeta` is a name everywhere else: a field, a variable, and the
    // def-type of a reference.
    assert_eq!(
        shape("defmeta = 1\nlet y = defmeta\nz = defmeta p\nw = defmeta p { 1 }"),
        "defmeta = 1; let y = defmeta; z = (defmeta p); w = (defmeta p)(){1}"
    );
    // Its errors.
    assert_eq!(
        error("x = { defmeta p a { b = 1 } }"),
        (MISPLACED_DEFMETA.to_owned(), Span::new(6, 13))
    );
    assert_eq!(
        error("#[shadowable] defmeta p a { b = 1 }"),
        (
            "`#[shadowable]` does not apply to a `defmeta`".to_owned(),
            Span::new(0, 13)
        )
    );
    assert_eq!(
        error("#[def_type] defmeta p a { b = 1 }"),
        (MISPLACED_DEF_TYPE.to_owned(), Span::new(0, 11))
    );
    assert_eq!(
        error("defmeta p a b"),
        (
            "expected `{` or `=` after the meta's path, found `b`".to_owned(),
            Span::new(12, 13)
        )
    );
    assert_eq!(
        error("defmeta p a.\"b\" { }"),
        (DEF_PATH.to_owned(), Span::new(12, 15))
    );
    assert_eq!(
        error("defmeta p a META: {b: Int} { }"),
        (MISPLACED_META.to_owned(), Span::new(12, 17))
    );
}

/// The path of each definition, meta and reference made with a def-type
/// in `text`, in the order they are parsed: each name's text, as its
/// source spells it, and its span.
pub(super) fn def_type_paths(text: &str) -> Vec<Vec<(String, Span)>> {
    let ast = parse_text(text).unwrap();
    ast.def_types()
        .map(|(_, def)| {
            ast.path(def.path)
                .iter()
                .map(|segment| {
                    let name = segment.name().unwrap();
                    (ast.names().get(name).to_owned(), segment.span)
                })
                .collect()
        })
        .collect()
}

#[test]
fn a_definition_s_path_may_have_plus_in_its_names() {
    // The names of real packages, each a definition, a meta and a
    // reference, called and read past, with one name of the source text
    // of its run of tokens.
    for path in [
        "org.gnu.gmp-c++",
        "org.gnu.gcc-c++",
        "org.gnu.gcc-libstdc++",
        "org.gnu.gcc-libstdc++-devel",
        "dev.peios.build-essentials-c++",
    ] {
        let text = format!(
            "defmeta package {path} {{ versions = [] }}\n\
                 def package {path}(v) {{ v }}\n\
                 x = package {path}(\"1\")\n\
                 y = package {path}.versions"
        );
        assert_eq!(
            shape(&text),
            format!(
                "defmeta package {path} {{versions = list()}}; def package {path}(v) {{v}}; \
                     x = (package {path})(\"1\"); y = (package {path}.versions)"
            )
        );
        let (_, last) = path.rsplit_once('.').unwrap();
        let paths = def_type_paths(&text);
        let lengths: Vec<usize> = paths.iter().map(Vec::len).collect();
        assert_eq!(lengths, [3, 3, 3, 4]);
        for names in &paths {
            let (name, span) = &names[2];
            assert_eq!(name, last);
            assert_eq!(&text[span.start as usize..span.end as usize], last);
        }
        assert_eq!(paths[3][3].0, "versions");
    }
    // Any name with `+` after its first character, `+` ending it or not,
    // and names after it.
    assert_eq!(
        def_type_paths("def p a+b() { 1 }\nx = p c++11\ny = p a+b.c+d.e"),
        [
            vec![("a+b".to_owned(), Span::new(6, 9))],
            vec![("c++11".to_owned(), Span::new(24, 29))],
            vec![
                ("a+b".to_owned(), Span::new(36, 39)),
                ("c+d".to_owned(), Span::new(40, 43)),
                ("e".to_owned(), Span::new(44, 45)),
            ],
        ]
    );
    // The rule is of the characters, whatever the tokens are: a keyword,
    // an integer with `_` and a float with an exponent are glued too.
    assert_eq!(
        def_type_paths("x = p a+in.c++1_0.c++1e5"),
        [vec![
            ("a+in".to_owned(), Span::new(6, 10)),
            ("c++1_0".to_owned(), Span::new(11, 17)),
            ("c++1e5".to_owned(), Span::new(18, 24)),
        ]]
    );
    // A `-` is glued only before what it is directly followed by, as in
    // an identifier: before `(`, it is the negation it was.
    assert_eq!(shape("x = p a+-b"), "x = (p a+-b)");
    assert_eq!(shape("x = p a+-1"), "x = (p a+-1)");
    assert_eq!(
        error("x = p a+-(1)"),
        (
            "expected end of item, found `-(`".to_owned(),
            Span::new(8, 10)
        )
    );
}

#[test]
fn plus_ends_a_definition_s_path_where_it_did_and_is_added_elsewhere() {
    // Anything not directly after the segment ends it: `+` with a space
    // before it, or after it, is the addition it was.
    assert_eq!(shape("x = p a + 1"), "x = ((p a) + 1)");
    assert_eq!(shape("x = p a +1"), "x = ((p a) + 1)");
    assert_eq!(shape("x = p a.b + c"), "x = ((p a.b) + c)");
    // But a reference directly followed by `+` reads it into its path,
    // where it was the addition of the reference and what followed.
    assert_eq!(
        error("x = p a+ 1"),
        (
            "expected end of item, found `1`".to_owned(),
            Span::new(9, 10)
        )
    );
    assert_eq!(shape("x = p a+(1)"), "x = (p a+)(1)");
    // A path's name starts with an identifier, so a `+` after `.` is the
    // error it was, and so is one after `def` and a name.
    assert_eq!(
        error("x = p a.+b"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(8, 9)
        )
    );
    assert_eq!(
        error("def p a.+b() { 1 }"),
        (
            "expected `(` after the definition's path, found `.`".to_owned(),
            Span::new(7, 8)
        )
    );
    assert_eq!(
        error("def a+b() { 1 }"),
        (
            "expected `(` after `fn`, found `+`".to_owned(),
            Span::new(5, 6)
        )
    );
    // Anywhere else, `a+b` is the addition it was: a field's path, a
    // variable, a `def`'s name, a def-type's.
    assert_eq!(shape("c = a+b"), "c = (a + b)");
    assert_eq!(shape("let y = x.a+b"), "let y = ((x . a) + b)");
    assert_eq!(shape("x = p+q a"), "x = (p + (q a))");
    assert_eq!(
        error("x.a+b = 1"),
        (
            "expected end of item, found `=`".to_owned(),
            Span::new(6, 7)
        )
    );
}

#[test]
fn a_definition_s_path_may_have_names_starting_with_a_digit() {
    // The names of real toolchain lanes, each a definition, a meta and a
    // reference, called and read past, the last name a number's.
    for path in ["org.golang.go-stage0-1.24", "org.rust-lang.rust-1.83"] {
        let text = format!(
            "defmeta package {path} {{ versions = [] }}\n\
                 def package {path}(v) {{ v }}\n\
                 x = package {path}(\"1\")\n\
                 y = package {path}.versions"
        );
        assert_eq!(
            shape(&text),
            format!(
                "defmeta package {path} {{versions = list()}}; def package {path}(v) {{v}}; \
                     x = (package {path})(\"1\"); y = (package {path}.versions)"
            )
        );
        let (_, last) = path.rsplit_once('.').unwrap();
        let paths = def_type_paths(&text);
        let lengths: Vec<usize> = paths.iter().map(Vec::len).collect();
        assert_eq!(lengths, [4, 4, 4, 5]);
        for names in &paths {
            let (name, span) = &names[3];
            assert_eq!(name, last);
            assert_eq!(&text[span.start as usize..span.end as usize], last);
        }
        assert_eq!(paths[3][4].0, "versions");
    }
    // A float is split at its `.`, each part a name with its own span;
    // an integer glues on what touches it; a name is its source text.
    assert_eq!(
        def_type_paths(
            "def p x.1.83.0() { 1 }\nx = p x.24rc1\ny = p x.1.83rc1.versions\nz = p x.1_0"
        ),
        [
            vec![
                ("x".to_owned(), Span::new(6, 7)),
                ("1".to_owned(), Span::new(8, 9)),
                ("83".to_owned(), Span::new(10, 12)),
                ("0".to_owned(), Span::new(13, 14)),
            ],
            vec![
                ("x".to_owned(), Span::new(29, 30)),
                ("24rc1".to_owned(), Span::new(31, 36)),
            ],
            vec![
                ("x".to_owned(), Span::new(43, 44)),
                ("1".to_owned(), Span::new(45, 46)),
                ("83rc1".to_owned(), Span::new(47, 52)),
                ("versions".to_owned(), Span::new(53, 61)),
            ],
            vec![
                ("x".to_owned(), Span::new(68, 69)),
                ("1_0".to_owned(), Span::new(70, 73)),
            ],
        ]
    );
    // A float with an exponent: whole without a `.`, split with one, the
    // exponent's sign kept.
    assert_eq!(
        def_type_paths("x = p a.1e5.2.5e-3.7.5E+3"),
        [vec![
            ("a".to_owned(), Span::new(6, 7)),
            ("1e5".to_owned(), Span::new(8, 11)),
            ("2".to_owned(), Span::new(12, 13)),
            ("5e-3".to_owned(), Span::new(14, 18)),
            ("7".to_owned(), Span::new(19, 20)),
            ("5E+3".to_owned(), Span::new(21, 25)),
        ]]
    );
    // A float written with `_` is split the same way.
    assert_eq!(
        def_type_paths("x = p a.1_0.2_5"),
        [vec![
            ("a".to_owned(), Span::new(6, 7)),
            ("1_0".to_owned(), Span::new(8, 11)),
            ("2_5".to_owned(), Span::new(12, 15)),
        ]]
    );
    // `+` glues as it does after an identifier, and `-` before a name.
    assert_eq!(
        def_type_paths("x = p a.1+2.c.3a-b.007"),
        [vec![
            ("a".to_owned(), Span::new(6, 7)),
            ("1+2".to_owned(), Span::new(8, 11)),
            ("c".to_owned(), Span::new(12, 13)),
            ("3a-b".to_owned(), Span::new(14, 18)),
            ("007".to_owned(), Span::new(19, 22)),
        ]]
    );
    // A float in a run, not after a `.`, is not split: the segment ends
    // before it, as it did after `c++`.
    assert_eq!(
        error("def p x.24+1.5() { 1 }"),
        (
            "expected `(` after the definition's path, found `1.5`".to_owned(),
            Span::new(11, 14)
        )
    );
    assert_eq!(
        error("def p x.c++1.5() { 1 }"),
        (
            "expected `(` after the definition's path, found `1.5`".to_owned(),
            Span::new(11, 14)
        )
    );
}

#[test]
fn a_digit_starts_a_name_nowhere_else() {
    // The first segment of a definition's path is what makes it one, an
    // identifier: a digit there is the error it was.
    assert_eq!(
        error("def p 1.x() { 1 }"),
        (
            "expected `(` after `fn`, found `1`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("x = p 1.x"),
        (
            "expected end of item, found `1`".to_owned(),
            Span::new(6, 7)
        )
    );
    // `x.0` is the error it was in a field's path, an expression, a
    // `def`'s name and a def-type.
    assert_eq!(
        error("c = a.0"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("let y = x.1.5"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(10, 13)
        )
    );
    assert_eq!(
        error("def x.0() { 1 }"),
        (
            "expected the function's name after `.`, found `0`".to_owned(),
            Span::new(6, 7)
        )
    );
    assert_eq!(
        error("x = p.1 a"),
        (
            "expected a field name after `.`".to_owned(),
            Span::new(6, 7)
        )
    );
    // After a reference's path, what follows its last name is read as it
    // was, so a range of one.
    assert_eq!(shape("x = p a.1..3"), "x = ((p a.1) .. 3)");
    // A quoted name or an index after a number is the error it is after
    // an identifier.
    assert_eq!(
        error("def p x.1.\"a\"() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(10, 13))
    );
    assert_eq!(
        error("def p x.1[0]() { 1 }"),
        (DEF_PATH.to_owned(), Span::new(9, 12))
    );
}
