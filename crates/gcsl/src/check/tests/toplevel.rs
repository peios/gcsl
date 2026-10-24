//! Tests of the checker: top-level functions and fields, spreads and soft
//! fields.

use super::*;

/// The span of the operand of the first spread `spread`, written with
/// its `...`, in `text`.
pub(super) fn operand(text: &str, spread: &str) -> Span {
    let spread = at(text, spread);
    Span::new(spread.start + 3, spread.end)
}

/// The note on a spread that defines a field defined before it.
pub(super) const SPREAD_TWICE_NOTE: &str =
    "a spread cannot replace a field defined before it; put the spread first";

#[test]
fn a_def_is_a_field_whose_value_is_the_function() {
    // A field of the record, in order among the other fields.
    assert_eq!(
        file_type("def f(): Int { 1 }\na = 2"),
        file_type("f = fn(): Int { 1 }\na = 2")
    );
    assert_eq!(
        file_type("a = 2\ndef f(n: Int) ->> Int { n }"),
        "{a: Int, f: fn(n: Int) ->> Int}"
    );
    // In an inner record block, from the next item on.
    assert_eq!(
        file_type("r = { def f(): Int { 1 }; g = f() }"),
        "{r: {f: fn(): Int, g: Int}}"
    );
    // Above a field its body reads: the body is checked when something
    // mentions the function, or after the last item, not at the
    // field's point, since a closure reads the field when it runs.
    assert_eq!(
        file_type("def f(): Int { v }\nv = 3"),
        "{f: fn(): Int, v: Int}"
    );
    assert_eq!(
        file_type("def f(): Int { v.a }\nlet k = 1\nv.a = k\nx = f()"),
        "{f: fn(): Int, v: {a: Int}, x: Int}"
    );
    assert_eq!(
        messages("x = f()\ndef f(): Int { v }\nlet k = 1\nv = k"),
        ["`f` cannot be used here yet"]
    );
    // A dotted `let` of a field's name is reported once.
    assert_eq!(
        messages("let z.w = 1\nz = 3"),
        ["`z` is a field of this file"]
    );
    // Its body sees its own name as the function being called.
    assert_eq!(
        file_type("def down(n: Int): Int { if n == 0 { 0 } else { down(n - 1) } }"),
        "{down: fn(n: Int): Int}"
    );
    // Defined twice, with a field and with another `def`.
    for text in [
        "def f(): Int { 1 }\nf = 2",
        "f = 2\ndef f(): Int { 1 }",
        "def f(): Int { 1 }\ndef f(): Int { 2 }",
    ] {
        assert_eq!(messages(text), ["field `f` is defined twice"], "{text}");
    }
    // It overrides a soft field from a spread (which keeps its place:
    // see the evaluator's tests).
    assert_eq!(
        file_type("...{f = 1; a = 2}\ndef f(): Int { 3 }"),
        "{a: Int, f: fn(): Int}"
    );
    // A block with a `def` is a record block.
    assert_eq!(
        messages("let x = { def f(): Int { 1 }; = 2 }"),
        ["a block with fields cannot have a result"]
    );
}

#[test]
fn top_level_functions_are_visible_in_the_whole_input() {
    // Used above its declaration.
    assert_eq!(file_type("x = f()\nfn f(): Int { 1 }"), "{x: Int}");
    assert_eq!(
        file_type("x = f()\ndef f(): Int { 1 }"),
        "{f: fn(): Int, x: Int}"
    );
    // Mutual recursion of two and of three, in any order.
    let even = "fn even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }";
    let odd = "fn odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
    for text in [
        format!("x = even(4)\n{even}\n{odd}"),
        format!("{odd}\nx = even(4)\n{even}"),
        format!("{even}\n{odd}\nx = even(4)"),
    ] {
        assert_eq!(file_type(&text), "{x: Bool}", "{text}");
    }
    assert_eq!(
        file_type(
            "x = a(9)\nfn a(n: Int): Int { if n == 0 { 0 } else { b(n - 1) } }\n\
                 fn b(n: Int): Int { c(n) }\nfn c(n: Int): Int { a(n) + 1 }"
        ),
        "{x: Int}"
    );
}

#[test]
fn a_top_level_function_outside_a_group_is_seen_resolved() {
    // `a`'s parameter `b` shadows the function `b`, so `a` does not
    // mention `b`: they are not a group, and `b` sees `a` resolved
    // (its result a future) in either order.
    let a = "fn a(b: Int) -> Int { lazy b }";
    let b = "fn b() -> Int { a(1) }";
    for text in [format!("{a}\n{b}\nx = b"), format!("{b}\n{a}\nx = b")] {
        assert_eq!(file_type(&text), "{x: fn() -> Future<Int>}", "{text}");
    }
    // Inside a group, a use of another member sees its declared type,
    // as a recursive call through a function's own name does, from
    // either member and in either order.
    let a = "fn a(n: Int) -> Int { if n == 0 { lazy 5 } else { b(n - 1) } }";
    let b = "fn b(n: Int) -> Int { a(n) }";
    let first = file_type(&format!("{a}\n{b}\nx = a\ny = b"));
    let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
    assert_eq!(first, format!("{{x: {either}, y: {either}}}"));
    // As a function calling itself is.
    assert_eq!(
        file_type("fn a(n: Int) -> Int { if n == 0 { lazy 5 } else { a(n - 1) } }\nx = a"),
        format!("{{x: {either}}}")
    );
    assert_eq!(first, file_type(&format!("{b}\n{a}\nx = a\ny = b")));
    assert_eq!(first, file_type(&format!("x = a\ny = b\n{a}\n{b}")));
}

#[test]
fn a_def_named_by_a_dotted_path_is_a_top_level_function() {
    // The leaf of its path, used above its declaration, recursive by
    // path, and in its prefix's record with the fields beside it.
    let summary = "total = dev.peios.count(4)\n\
             def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.count(n: Int): Int {\n\
             \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
             }\n\
             def dev.peios.name(): String { \"world\" }\n\
             dev.peios.version = \"1\"\n\
             greeting = dev.peios.hello()\n\
             all = dev.peios";
    let record = "{count: fn(n: Int): Int, hello: fn(): String, name: fn(): String, \
                      version: String}";
    assert_eq!(
        file_type(summary),
        format!("{{all: {record}, dev: {{peios: {record}}}, greeting: String, total: Int}}")
    );
    // With no lone file's prelude too.
    let (ast, types, output) = check_text(summary);
    let output = output.unwrap();
    assert_eq!(
        types.display(output.ty, ast.names()).to_string(),
        file_type(summary)
    );
    // Mutual recursion with a plain `def`, and between two dotted ones,
    // in any order.
    let even = "def even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }";
    let odd = "def dev.odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
    for text in [
        format!("x = dev.odd(4)\n{even}\n{odd}"),
        format!("{odd}\nx = even(4)\n{even}"),
    ] {
        assert_eq!(
            file_type(&text),
            "{dev: {odd: fn(n: Int): Bool}, even: fn(n: Int): Bool, x: Bool}",
            "{text}"
        );
    }
    let f = "def a.x.f(n: Int): Int { if n == 0 { 0 } else { a.y.g(n - 1) } }";
    let g = "def a.y.g(n: Int): Int { a.x.f(n) }";
    for text in [
        format!("{f}\n{g}\nr = a.x.f(2)"),
        format!("r = a.x.f(2)\n{g}\n{f}"),
    ] {
        assert!(file_type(&text).ends_with("r: Int}"), "{text}");
    }
    // There is no short name: inside `def dev.count`, `count` is not
    // in scope unless something else binds it.
    assert_eq!(
        errors("def dev.count(n: Int): Int { if n == 0 { 0 } else { count(n - 1) } }"),
        [("undefined variable `count`".to_owned(), Span::new(52, 57))]
    );
    assert_eq!(
        file_type(
            "fn count(n: Int): Int { n * 10 }\n\
                 def dev.count(n: Int): Int { count(n) }\nx = dev.count(2)"
        ),
        "{dev: {count: fn(n: Int): Int}, x: Int}"
    );
}

#[test]
fn a_dotted_def_is_typed_as_a_plain_def_is_by_its_group() {
    // Outside its group, seen resolved; inside, its declared type, from
    // either member and in either order, as a plain name is.
    let a = "def p.a(b: Int) -> Int { lazy b }";
    let b = "def p.b() -> Int { p.a(1) }";
    for text in [format!("{a}\n{b}\nx = p.b"), format!("x = p.b\n{b}\n{a}")] {
        assert!(
            file_type(&text).contains("x: fn() -> Future<Int>"),
            "{text}: {}",
            file_type(&text)
        );
    }
    let a = "def p.a(n: Int) -> Int { if n == 0 { lazy 5 } else { p.b(n - 1) } }";
    let b = "def p.b(n: Int) -> Int { p.a(n) }";
    let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
    for text in [
        format!("{a}\n{b}\nx = p.a\ny = p.b"),
        format!("x = p.a\ny = p.b\n{b}\n{a}"),
    ] {
        assert!(
            file_type(&text).contains(&format!("x: {either}, y: {either}")),
            "{text}: {}",
            file_type(&text)
        );
    }
    // A prefix read whole above the declaration has the type the body
    // gives the function, not the one its signature declares.
    let resolved = "def dev.get(n: Int) -> Int { n + 1 }";
    for text in [
        format!("r = dev\n{resolved}"),
        format!("{resolved}\nr = dev"),
    ] {
        assert_eq!(
            file_type(&text),
            "{dev: {get: fn(n: Int | Future<Int>) -> Int}, \
                 r: {get: fn(n: Int | Future<Int>) -> Int}}",
            "{text}"
        );
    }
    // A call by path is of a known callee once its group is checked: the
    // call-site form of the implicit-await error names the path.
    assert_eq!(
        messages("def dev.inc(n: Int): Int { n + 1 }\nfn p(f: Future<Int>): Int { dev.inc(f) }"),
        ["argument `f` would be awaited inside `dev.inc`, which is pure"]
    );
    // A pure dotted `def` calling an impure one is the existing error.
    let text = "def dev.p(): Int { dev.q() }\ndef dev.q() -> Int { 1 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        "cannot call an impure function from a pure function"
    );
    assert_eq!(found[0].span, Span::new(19, 26));
    assert_eq!(
        labels(&found[0]),
        [
            (Span::new(19, 24), "this is fn() -> Int"),
            (
                Span::new(11, 12),
                "this function is pure, declared with `:`"
            ),
        ]
    );
}

#[test]
fn a_dotted_def_cannot_be_used_before_what_it_reads_is_bound() {
    // By its path, and by a prefix read whole, which holds its closure.
    let text = "x = dev.f()\nall = dev\nlet k = 1\ndef dev.f(): Int { k }";
    let found = diagnostics(text);
    let label = [(Span::new(22, 31), "`dev.f` reads `k`, which is bound here")];
    assert_eq!(found.len(), 2, "{found:?}");
    for (diagnostic, span) in found.iter().zip([Span::new(4, 9), Span::new(18, 21)]) {
        assert_eq!(diagnostic.message, "`dev.f` cannot be used here yet");
        assert_eq!(diagnostic.span, span);
        assert_eq!(labels(diagnostic), label);
    }
    // Through another function, the chain names each by its path.
    assert_eq!(
        messages("x = a.b()\nlet k = 1\ndef a.b(): Int { c.d() }\ndef c.d(): Int { k }"),
        ["`a.b` cannot be used here yet"]
    );
    let found = diagnostics("x = a.b()\nlet k = 1\ndef a.b(): Int { c.d() }\ndef c.d(): Int { k }");
    assert_eq!(
        labels(&found[0]),
        [(
            Span::new(10, 19),
            "`a.b` uses `c.d`, and `c.d` reads `k`, which is bound here"
        )]
    );
    // Below what it reads, it is fine.
    assert_eq!(
        file_type("let k = 1\ndef dev.f(): Int { k }\nx = dev.f()\nall = dev"),
        "{all: {f: fn(): Int}, dev: {f: fn(): Int}, x: Int}"
    );
}

#[test]
fn a_dotted_def_conflicts_as_a_leaf_does() {
    // With a field leaf, and with a field or a `def` at a prefix, in
    // either order: the existing texts at the later item.
    for (text, expected) in [
        (
            "def a.b(): Int { 1 }\na.b = 2",
            "field `a.b` is defined twice",
        ),
        (
            "a.b = 2\ndef a.b(): Int { 1 }",
            "field `a.b` is defined twice",
        ),
        (
            "def a.b(): Int { 1 }\ndef a.b(): Int { 2 }",
            "field `a.b` is defined twice",
        ),
        (
            "a = 1\ndef a.b(): Int { 1 }",
            "field `a` is already defined",
        ),
        ("def a.b(): Int { 1 }\na = 1", "field `a` is defined twice"),
        (
            "def a(): Int { 1 }\ndef a.b(): Int { 1 }",
            "field `a` is already defined",
        ),
        (
            "def a.b(): Int { 1 }\ndef a(): Int { 1 }",
            "field `a` is defined twice",
        ),
    ] {
        assert_eq!(messages(text), [expected], "{text}");
    }
    // As a dotted field does, with the same labels and notes.
    let shown = |text: &str| -> Vec<(String, Vec<String>, Vec<String>)> {
        diagnostics(text)
            .into_iter()
            .map(|d| {
                let labels = d.labels.iter().map(|l| l.text.clone()).collect();
                (d.message, labels, d.notes)
            })
            .collect()
    };
    for (dotted, field) in [
        (
            "def a.b(): Int { 1 }\na = 1",
            "a.b = fn(): Int { 1 }\na = 1",
        ),
        (
            "a = 1\ndef a.b(): Int { 1 }",
            "a = 1\na.b = fn(): Int { 1 }",
        ),
        (
            "def a.b(): Int { 1 }\na.b = 1",
            "a.b = fn(): Int { 1 }\na.b = 1",
        ),
    ] {
        assert_eq!(shown(dotted), shown(field), "{dotted}");
    }
    // A rejected one is no top-level function: the groups of the others
    // are found as ever, and nothing else is reported.
    assert_eq!(
        messages(
            "a.b = 1\ndef a.b(): Int { 1 }\nfn p(): Int { q() }\nfn q(): Int { p() }\n\
                 x = p"
        ),
        ["field `a.b` is defined twice"]
    );
    // Its root clashes as a dotted field's does.
    assert_eq!(
        messages("fn e(): Int { 1 }\ndef e.f(): Int { 1 }"),
        ["`e` is a function of this file"]
    );
    assert_eq!(
        messages("let g = 1\ndef g.h(): Int { 1 }"),
        ["`g` is a field of this file"]
    );
}

#[test]
fn a_dotted_def_is_shadowable_as_any_def_is() {
    // Shadowed by a later plain one and by a field, for the whole input.
    assert_eq!(
        file_type(
            "before = dev.x()\n#[shadowable] def dev.x(): String { \"a\" }\n\
                 def dev.x(): Int { 2 }"
        ),
        "{before: Int, dev: {x: fn(): Int}}"
    );
    assert_eq!(
        file_type("#[shadowable] def dev.y(): Int { 3 }\ndev.y = \"s\"\nz = dev.y"),
        "{dev: {y: String}, z: String}"
    );
    // A shadowed one is still checked as it is written, by itself.
    assert_eq!(
        messages("#[shadowable] def dev.x(): Int { \"no\" }\ndef dev.x(): Int { 2 }"),
        ["mismatched types"]
    );
    // A later declaration overlapping a plain one conflicts.
    assert_eq!(
        messages("def dev.x(): Int { 1 }\n#[shadowable] dev.x = 2"),
        ["field `dev.x` is defined twice"]
    );
}

#[test]
fn a_dotted_def_in_an_inner_block_is_its_dotted_field() {
    assert_eq!(
        file_type("r = { def b.c(n: Int): Int { n + 1 }\nd = b.c(1) }"),
        "{r: {b: {c: fn(n: Int): Int}, d: Int}}"
    );
    // Ordered: not visible above its item, and it sees no short name.
    assert_eq!(
        messages("r = { d = b.c(1)\ndef b.c(n: Int): Int { n + 1 } }"),
        ["undefined variable `b`"]
    );
    assert_eq!(
        messages("r = { def b.c(n: Int): Int { c(n) } }"),
        ["undefined variable `c`"]
    );
}

#[test]
fn a_top_level_functions_body_sees_the_bindings_above_it() {
    // Checked early, at the item above its declaration that uses it.
    assert_eq!(
        file_type("let k = 1\nx = g()\nfn g(): Int { k }"),
        "{x: Int}"
    );
    // Checked after the last item: the `k` above it, not the one below.
    let text = "let k = 1\nfn g(): Int { k }\nlet k = \"s\"\nx = k";
    assert_eq!(file_type(text), "{x: String}");
    // First used by an item below a `let` that is below the function:
    // it does not see that `let`.
    let text = "let k = 1\nfn g(): Int { k }\nlet k = \"s\"\nx = g()";
    assert_eq!(file_type(text), "{x: Int}");
}

#[test]
fn a_top_level_function_shadows_a_soft_field_of_its_name() {
    // The spread still defines the record's field `f`, but every read of
    // `f` in the input is the function, above the spread and below it.
    let spread = "let r = {f = 2; g = 3}";
    assert_eq!(
        file_type(&format!(
            "{spread}\n...r\nx = f()\ny = g\nfn f(): String {{ \"fn\" }}"
        )),
        "{f: Int, g: Int, x: String, y: Int}"
    );
    assert_eq!(
        file_type(&format!(
            "{spread}\nx = f()\n...r\nfn f(): String {{ \"fn\" }}"
        )),
        "{f: Int, g: Int, x: String}"
    );
    // A `def` of that name overrides the soft field, as a field does.
    assert_eq!(
        file_type(&format!(
            "{spread}\nx = f()\n...r\ndef f(): String {{ \"fn\" }}"
        )),
        "{f: fn(): String, g: Int, x: String}"
    );
}

/// Each error of `text`, as its message and the text it is at, then
/// each label's text and the text it is at, then each note.
pub(super) fn explained(text: &str) -> Vec<String> {
    let at = |span: Span| {
        text.get(span.start as usize..span.end as usize)
            .unwrap_or("")
    };
    diagnostics(text)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| {
            let mut parts = vec![format!(
                "{} at `{}`",
                diagnostic.message,
                at(diagnostic.span)
            )];
            for label in &diagnostic.labels {
                parts.push(format!("{} at `{}`", label.text, at(label.span)));
            }
            parts.extend(diagnostic.notes.iter().cloned());
            parts.join(" | ")
        })
        .collect()
}

#[test]
fn a_top_level_function_is_used_once_what_it_reads_is_bound() {
    const BOUND: &str = "a function can be used once every variable it reads is bound";
    const MUTABLE: &str = "a function copies a mutable variable where the function is written";
    // Allowed: it copies nothing, or a `let` above the use.
    assert_eq!(file_type("x = f()\nfn f(): Int { 1 }"), "{x: Int}");
    assert_eq!(
        file_type("let k = 1\nx = f()\nfn f(): Int { k }"),
        "{x: Int}"
    );
    // Not allowed: a `let` below the use.
    assert_eq!(
        explained("let x = total()\nlet k = 10\nfn total(): Int { k }"),
        vec![format!(
            "`total` cannot be used here yet at `total` | `total` reads `k`, which is bound \
                 here at `let k = 10` | {BOUND}"
        )]
    );
    // A mutable, even above the use: it exists from its function's line.
    assert_eq!(
        explained("let mut n = 1\nx = f()\nfn f(): Int { n }"),
        vec![format!(
            "`f` cannot be used here yet at `f` | `f` reads the mutable variable `n`, so it \
                 exists from here at `fn f(): Int {{ n }}` | {MUTABLE}"
        )]
    );
    // Through one, two and four other functions.
    let chain = "let k = 1\nfn b(): Int { c() }\nfn c(): Int { d() }\nfn d(): Int { e() }\n\
                     fn e(): Int { k }";
    let reads = "reads `k`, which is bound here at `let k = 1`";
    assert_eq!(
        explained(&format!("x = d()\n{chain}")),
        vec![format!(
            "`d` cannot be used here yet at `d` | `d` uses `e`, and `e` {reads} | {BOUND}"
        )]
    );
    assert_eq!(
        explained(&format!("x = c()\n{chain}")),
        vec![format!(
            "`c` cannot be used here yet at `c` | `c` uses `d`, `d` uses `e`, and `e` \
                 {reads} | {BOUND}"
        )]
    );
    assert_eq!(
        explained(&format!("x = a()\nfn a(): Int {{ b() }}\n{chain}")),
        vec![format!(
            "`a` cannot be used here yet at `a` | `a` uses `b`, ..., and `e` {reads} | \
                 {BOUND}"
        )]
    );
    // Mentioned only inside a `lazy` or a function expression the item
    // builds.
    for item in ["x = lazy f()", "x = fn(): Int { f() }"] {
        let found = explained(&format!("{item}\nlet k = 1\nfn f(): Int {{ k }}"));
        assert_eq!(found.len(), 1, "{item}");
        assert!(
            found[0].starts_with("`f` cannot be used here yet"),
            "{item}"
        );
    }
    // No error where a parameter or a local shadows the name.
    for text in [
        "x = fn(f: Int): Int { f }\nlet k = 1\nfn f(): Int { k }",
        "x = { let f = 2; = f }\nlet k = 1\nfn f(): Int { k }",
        // Inside function bodies there is no check.
        "fn g(): Int { f() }\nlet k = 1\nfn f(): Int { k }",
    ] {
        assert!(
            check(&parse_text(text), &mut TypeTable::new()).is_ok(),
            "{text}"
        );
    }
}

#[test]
fn a_top_level_functions_closure_is_made_once_what_it_copies_is_bound() {
    let text = "let k = 1\nlet mut n = 2\nfn a(): Int { 1 }\nfn b(): Int { k }\n\
                    fn c(): Int { n }\nfn d(): Int { e() }\nfn e(): Int { 1 }";
    let ast = parse_text(text);
    let (checked, _) = check(&ast, &mut TypeTable::new()).unwrap();
    let root = ast.items(ast.root());
    assert_eq!(
        checked.resolution.top_functions(),
        [
            // Copies nothing: made as the input starts.
            (root[2], MadeAt::Start),
            // Copies `k`: made just after its `let`.
            (root[3], MadeAt::After(root[0])),
            // Copies a mutable: made at its own declaration.
            (root[4], MadeAt::Own),
            // Reads another top-level function, which it does not copy.
            (root[5], MadeAt::Start),
            (root[6], MadeAt::Start),
        ]
    );
}

#[test]
fn a_body_checked_early_does_not_see_what_is_declared_below_the_use() {
    const BOUND: &str = "a function can be used once every variable it reads is bound";
    // The function reads the second `k`, the latest above itself.
    assert_eq!(
        explained("let k = 1\nx = g()\nlet k = 2\nfn g(): Int { k }"),
        vec![format!(
            "`g` cannot be used here yet at `g` | `g` reads `k`, which is bound here at \
                 `let k = 2` | {BOUND}"
        )]
    );
    // Declared again between the use and the function by `:=` and by a
    // dotted field.
    assert_eq!(
        explained("let r = {a = 1}\nx = g()\nr.a := 2\nfn g(): Int { r.a }"),
        vec![format!(
            "`g` cannot be used here yet at `g` | `g` reads `r`, which is bound here at \
                 `r.a := 2` | {BOUND}"
        )]
    );
    // A top-level field is order-free: `g` reads `r.a` through the top
    // level's frame, and `r.a` is computed before `x`.
    assert_eq!(
        file_type("x = g()\nr.a = 2\nfn g(): Int { r.a }"),
        "{r: {a: Int}, x: Int}"
    );
    // A spread between may define any name.
    assert_eq!(
        explained("let s = {port = 1}\nx = g()\n...s\nfn g(): Int { port }"),
        vec![format!(
            "`g` cannot be used here yet at `g` | `g` reads `port`, which this spread may \
                 define at `...s` | {BOUND}"
        )]
    );
    // The function is then erroneous for its users, and its body's
    // errors are reported once.
    let text = "x = g()\nlet k = 1\nfn g(): Int { k + true }\ny = g()";
    let found = messages(text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0], "`g` cannot be used here yet");
}

#[test]
fn a_top_level_functions_name_is_not_also_a_variable() {
    const NOTE: &str = "a function declared at the top level is visible in the whole \
                            file, so its name cannot also be a variable or a field";
    assert_eq!(
        explained("fn f(): Int { 1 }\nfn f(): Int { 2 }"),
        vec!["function `f` is declared twice at `f` | first declared here at `f`"]
    );
    for (text, at) in [
        ("fn f(): Int { 1 }\nlet f = 2", "f"),
        ("let f = 2\nfn f(): Int { 1 }", "f"),
        ("fn f(): Int { 1 }\nlet mut f = 2", "f"),
        ("fn f(): Int { 1 }\nf = 2", "f"),
        ("let mut x = 1\nfn f(): Int { 1 }\nf := 2", "f"),
        ("def f(): Int { 1 }\nlet f = 2", "f"),
    ] {
        let found = explained(text);
        assert_eq!(
            found.first(),
            Some(&format!(
                "`f` is a function of this file at `{at}` | declared here at `f` | {NOTE}"
            )),
            "{text}"
        );
    }
    // A `def` and a field, or two `def`s, are a field defined twice.
    for text in [
        "def f(): Int { 1 }\nf = 2",
        "def f(): Int { 1 }\ndef f(): Int { 2 }",
    ] {
        assert_eq!(messages(text), vec!["field `f` is defined twice"], "{text}");
    }
    // Inner blocks still shadow.
    assert_eq!(
        file_type("fn f(): Int { 1 }\nx = { let f = \"s\"; = f }"),
        "{x: String}"
    );
}

#[test]
fn a_top_level_field_cannot_also_be_a_variable() {
    const NOTE: &str = "a field declared at the top level is visible in the whole file, \
                            so its name cannot also be a variable";
    for text in [
        "a = 1\nlet a = 2",
        "let a = 2\na = 1",
        "a.b = 1\nlet mut a = 2",
        "let mut a = 2\na.b = 1",
        "let mut x = 1\na = 1\na := 2",
        "a.b = 1\nlet a.c = 2",
    ] {
        let found = explained(text);
        assert_eq!(
            found.first(),
            Some(&format!(
                "`a` is a field of this file at `a` | declared here at `a` | {NOTE}"
            )),
            "{text}"
        );
    }
    // A function and a field of one name: the function's error, at the
    // field, in either order.
    for text in ["a = 1\nfn a(): Int { 1 }", "fn a(): Int { 1 }\na.b = 1"] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some("`a` is a function of this file"),
            "{text}"
        );
    }
    // Inner blocks still shadow.
    assert_eq!(
        file_type("a = 1\nx = { let a = \"s\"; = a }"),
        "{a: Int, x: String}"
    );
}

#[test]
fn top_level_fields_are_visible_throughout_the_input() {
    let summary = "let major = 2\ncom.x.app.deps = [com.x.lib]\ncom.x.app.name = \
                       \"app-{version}\"\ncom.x.lib.name = \"lib\"\nversion = describe(major)\n\
                       fn describe(n: Int): String { \"v{n}\" }";
    assert_eq!(
        file_type(summary),
        "{com: {x: {app: {deps: List<{name: String}>, name: String}, lib: {name: String}}}, \
             version: String}"
    );
    // A field used above its declaration, from every kind of item and
    // position: allowed, since it reads nothing that stops it.
    for text in [
        "let x = y\ny = 1",
        "x = y\ny = 1",
        "{ let z = y }\ny = 1",
        "...r\nr = {a = 1}",
        "let x = lazy { y }\ny = 1",
        "let f = fn(): Int { y }\ny = 1",
        "let s = \"{y}\"\ny = 1",
        "let l = [1, 2]\nlet e = l[y]\ny = 0",
        "let mut m = 0\nm := y\ny = 1",
        "let l = for i in [y] { i }\ny = 1",
        "let l = for let i = y; i < y; i := i + y { i }\ny = 1",
        "let x = g(v = y)\nfn g(v: Int): Int { v }\ny = 1",
        "let x = g() { y }\nfn g(v: Int): Int { v }\ny = 1",
        // A leaf by its path while a sibling is not ready yet.
        "x = a.b\na.b = 1\nlet k = 2\na.c = k",
    ] {
        // `file_type` fails on an error.
        assert!(file_type(text).starts_with('{'), "{text}");
    }
    // Types of a root, a prefix, a leaf and an access beyond a leaf.
    assert_eq!(
        file_type("w = r\nx = r.q\ny = r.q.k\nz = r.q.k.m\nr.q.k = {m = 1}"),
        "{r: {q: {k: {m: Int}}}, w: {q: {k: {m: Int}}}, x: {k: {m: Int}}, y: {m: Int}, z: \
             Int}"
    );
    // A `def` as a leaf.
    assert_eq!(
        file_type("x = f()\ndef f(): Int { 1 }"),
        "{f: fn(): Int, x: Int}"
    );
}

#[test]
fn a_field_used_before_it_can_be_computed_is_an_error() {
    const FIELD: &str = "a field can be computed once every variable it reads is bound";
    const FUNCTION: &str = "a function can be used once every variable it reads is bound";
    let cases = [
        (
            "x = a\na.b = 1\nlet k = 2\na.c = k",
            format!(
                "`a` cannot be used here yet at `a` | `a` has `a.c`, and `a.c` reads `k`, \
                     which is bound here at `let k = 2` | {FIELD}"
            ),
        ),
        (
            "x = v\nlet k = 1\nv = k",
            format!(
                "`v` cannot be used here yet at `v` | `v` reads `k`, which is bound here \
                     at `let k = 1` | {FIELD}"
            ),
        ),
        (
            "let mut n = 1\nx = v\nv = n",
            "`v` cannot be used here yet at `v` | `v` reads the mutable variable `n`, so it \
                 is computed here at `v = n` | a field that reads a mutable variable is \
                 computed where it is written"
                .to_string(),
        ),
        (
            "x = v\nlet k = 1\nv = f()\nfn f(): Int { k }",
            format!(
                "`v` cannot be used here yet at `v` | `v` uses `f`, and `f` reads `k`, \
                     which is bound here at `let k = 1` | {FUNCTION}"
            ),
        ),
        (
            "x = g()\nlet k = 1\nv = k\nfn g(): Int { v }",
            format!(
                "`g` cannot be used here yet at `g` | `g` uses `v`, and `v` reads `k`, \
                     which is bound here at `let k = 1` | {FIELD}"
            ),
        ),
        (
            "x = v\n...{q = 1}\nv = q",
            format!(
                "`v` cannot be used here yet at `v` | `v` reads `q`, which this spread may \
                     define at `...{{q = 1}}` | {FIELD}"
            ),
        ),
        (
            "let k = 1\nx = v\nlet k = 2\nv = k",
            format!(
                "`v` cannot be used here yet at `v` | `v` reads `k`, which is bound here \
                     at `let k = 2` | {FIELD}"
            ),
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(explained(text), vec![expected], "{text}");
    }
    // Below the field: undefined, with the new label.
    assert_eq!(
        explained("v = k\nlet k = 1"),
        vec![
            "undefined variable `k` at `k` | `k` is declared here, below the field at `let \
                 k = 1` | a field's initialiser sees the variables declared above it"
                .to_string()
        ]
    );
}

#[test]
fn a_cycle_through_a_field_is_an_error() {
    const NOTE: &str = "a field cannot be computed from itself, even through a function";
    let cases = [
        (
            "a = b\nb = a",
            vec!["`a` uses `b` here at `b`", "`b` uses `a` here at `a`"],
        ),
        (
            "a = f()\nfn f(): Int { a }",
            vec!["`a` uses `f` here at `f`", "`f` uses `a` here at `a`"],
        ),
        (
            "a = f()\ndef f(): Int { a }",
            vec!["`a` uses `f` here at `f`", "`f` uses `a` here at `a`"],
        ),
        ("s = lazy { s }", vec!["`s` uses `s` here at `s`"]),
        (
            "a = b\nb = c\nc = a",
            vec![
                "`a` uses `b` here at `b`",
                "`b` uses `c` here at `c`",
                "`c` uses `a` here at `a`",
            ],
        ),
    ];
    for (text, labels) in cases {
        let found = explained(text);
        let first = text.split(' ').next().unwrap_or("");
        let expected = format!(
            "`{first}` depends on itself at `{first}` | {} | {NOTE}",
            labels.join(" | ")
        );
        assert_eq!(found, vec![expected], "{text}");
    }
    // Through an interior node: one error, at the leaf.
    let found = messages("a.b = a\nx = 1");
    assert_eq!(found, vec!["`a.b` depends on itself".to_string()]);
    // A cycle of functions alone is fine.
    assert_eq!(
        file_type("fn f(): Int { g() }\nfn g(): Int { f() }\nx = f()"),
        "{x: Int}"
    );
}

#[test]
fn a_spread_and_a_top_level_field_agree_in_either_order() {
    for text in ["...{a = 1}\na = \"s\"", "a = \"s\"\n...{a = 1}"] {
        assert_eq!(file_type(text), "{a: String}", "{text}");
    }
    for text in [
        "...{a = 1}\ndef a(): Int { 2 }",
        "def a(): Int { 2 }\n...{a = 1}",
    ] {
        assert_eq!(file_type(text), "{a: fn(): Int}", "{text}");
    }
    // A soft field as a dotted root, a leaf under a leaf, a leaf twice:
    // errors in either order.
    for (first, second) in [
        ("...{a = {b = 1}}", "a.c = 2"),
        ("a.b = 1", "a.b.c = 2"),
        ("a.b = 1", "a.b = 2"),
        ("a = 1", "a.b = 2"),
    ] {
        for text in [format!("{first}\n{second}"), format!("{second}\n{first}")] {
            assert_eq!(messages(&text).len(), 1, "{text}: {:?}", messages(&text));
        }
    }
    // Quoted names.
    for text in ["\"a\" = 1\nb = a", "b = a\n\"a\" = 1"] {
        assert_eq!(file_type(text).len(), "{a: Int, b: Int}".len(), "{text}");
    }
}

#[test]
fn an_undefined_variable_declared_below_the_function_says_so() {
    assert_eq!(
        explained("fn d(): Int { q }\nlet q = 2"),
        vec![
            "undefined variable `q` at `q` | `q` is declared here, below the function at \
                 `let q = 2` | a function sees the variables declared above it"
        ]
    );
    // Not for a name declared nowhere.
    assert_eq!(
        explained("fn d(): Int { q }"),
        vec!["undefined variable `q` at `q`"]
    );
}

#[test]
fn a_spread_defines_the_fields_of_its_record() {
    let text = "let defaults = { host = \"localhost\"; port = 80 }\n\
                    let conf = {\n  ...defaults\n  port = 8080\n  url = \"{host}:{port}\"\n}\n\
                    conf";
    assert_eq!(shown_result(text), "{host: String, port: Int, url: String}");
    // A block whose only item is a spread is a record block, and so is a
    // file's body.
    assert_eq!(shown_result("let r = { a = 1 }\n{ ...r }"), "{a: Int}");
    assert_eq!(file_type("...{ a = 1; b = \"s\" }"), "{a: Int, b: String}");
    // So it cannot have a result, with the existing error.
    let text = "let r = { a = 1 }\n{ ...r; = 2 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "a block with fields cannot have a result");
    assert_eq!(found[0].span, first_bytes(at(text, "= 2"), 1));
    assert_eq!(
        labels(&found[0]),
        vec![(at(text, "...r"), "first field declared here")]
    );
    // Its final expression is a statement.
    assert_eq!(
        messages("...{ a = 1 }\na + 1"),
        vec!["unused value of type Int"]
    );
}

#[test]
fn a_spread_defines_exactly_the_fields_of_the_static_type() {
    // `y` is hidden by width subtyping, so it is neither a field nor a
    // name, and a field item may define it with another type.
    let hidden = "let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\n";
    assert_eq!(shown_result(&format!("{hidden}{{ ...p }}")), "{x: Int}");
    assert_eq!(
        messages(&format!("{hidden}{{ ...p; z = y }}")),
        vec!["undefined variable `y`"]
    );
    assert_eq!(
        shown_result(&format!("{hidden}{{ ...p; y = \"s\"; x = true }}")),
        "{x: Bool, y: String}"
    );
}

#[test]
fn an_ascribed_spread_selects_fields() {
    let r = "let r = { a = 2; b = 1 }\n";
    assert_eq!(
        shown_result(&format!("{r}{{ ...r : {{b: Int}} }}")),
        "{b: Int}"
    );
    assert_eq!(
        messages(&format!("{r}{{ ...r : {{b: Int}}; c = a }}")),
        vec!["undefined variable `a`"]
    );
    // A bare type in an ascription is present, so a field that may be a
    // future is awaited by it; the either shape keeps it lazy.
    let r = "let r: {a: Int, b: Int | Future<Int>} = { a = 1; b = lazy 2 }\n";
    let text = format!("{r}{{ ...r : {{b: Int}} }}");
    assert_eq!(shown_result(&text), "{b: Int}");
    assert_eq!(marks(&text), vec![("r", AwaitMark::Deep)]);
    let text = format!("{r}{{ ...r : {{b: Int | Future<Int>}} }}");
    assert_eq!(shown_result(&text), "{b: Int | Future<Int>}");
    assert_eq!(marks(&text), vec![]);
    // An ascribed literal warns about the field its type lacks, as any
    // ascribed literal does.
    let text = "{ ...{ a = 1; b = 2 } : {b: Int} }";
    assert_eq!(shown_result(text), "{b: Int}");
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is not in `{b: Int}`");
    assert_eq!(found[0].span, at(text, "a"));
}

#[test]
fn a_soft_field_may_be_overridden() {
    // By a field item, with another type: the field is then hard.
    assert_eq!(
        shown_result("let r = { a = 1; b = 2 }\n{ ...r; a = \"s\" }"),
        "{a: String, b: Int}"
    );
    // By a later spread, and then still soft.
    assert_eq!(
        shown_result("{ ...{ a = 1 }; ...{ a = \"s\"; b = 2 } }"),
        "{a: String, b: Int}"
    );
    assert_eq!(
        shown_result("{ ...{ a = 1 }; ...{ a = \"s\" }; a = true }"),
        "{a: Bool}"
    );
    // Every definition of the field writes its one slot; the spread's
    // operand is held in the next free slot, which it leaves free.
    let text = "let r = { a = 1; b = 2 }\n...r\na = \"s\"\nlet c = 3";
    let (ast, resolution) = resolved(text);
    // `a`, a top-level field, has slot 0 from the start of the input,
    // and the spread writes nothing for it.
    let items = ast.items(ast.root());
    assert_eq!(resolution.item_slot(items[1]), Slot(3));
    assert_eq!(resolution.spread_slots(items[1]), &[Slot::NONE, Slot(2)]);
    assert_eq!(resolution.item_slot(items[2]), Slot(0));
    assert!(resolution.spread_slots(items[2]).is_empty());
    assert_eq!(resolution.item_slot(items[3]), Slot(3));
    assert_eq!(resolution.frame_size(), 4);
    let (ast, resolution) = resolved("...{ a = 1 }\n...{ a = 2; b = 3 }");
    let items = ast.items(ast.root());
    assert_eq!(resolution.spread_slots(items[0]), &[Slot(0)]);
    assert_eq!(resolution.item_slot(items[0]), Slot(1));
    assert_eq!(resolution.spread_slots(items[1]), &[Slot(0), Slot(1)]);
    assert_eq!(resolution.item_slot(items[1]), Slot(2));
}

#[test]
fn a_spread_cannot_replace_a_hard_field() {
    let text = "let r = { a = 1 }\n{ a = 2; ...r }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(found[0].span, operand(text, "...r"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("`a` is a field of this, of type Int")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a = 2"), 1), "first defined here")]
    );
    assert_eq!(found[0].notes, vec![SPREAD_TWICE_NOTE]);
    // One error per spread, for the first such field in the type's
    // order, counting the others.
    let r = "let r = { a = 1; b = 2; c = 3 }\n";
    let text = format!("{r}{{ c = 0; a = 0; ...r }}");
    let found = diagnostics(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(&text, "a = 0"), 1), "first defined here")]
    );
    assert_eq!(
        found[0].notes,
        vec![format!(
            "{SPREAD_TWICE_NOTE} (it also has 1 other field defined before it)"
        )]
    );
    let found = diagnostics(&format!("{r}{{ a = 0; b = 0; c = 0; ...r }}"));
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].notes,
        vec![format!(
            "{SPREAD_TWICE_NOTE} (it also has 2 other fields defined before it)"
        )]
    );
    // A dotted field is hard, and so is one a field item overrode.
    let text = "{ a.x = 1; ...{ a = 2 } }";
    let found = diagnostics(text);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a.x"), 1), "first defined here")]
    );
    let text = "{ ...{ a = 1 }; a = 2; ...{ a = 3 } }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].span, at(text, "{ a = 3 }"));
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a = 2"), 1), "first defined here")]
    );
    // The spread's other fields are still defined, and the block's
    // record is erroneous, so neither cascades.
    assert_eq!(
        messages("{ a = 1; ...{ a = 2; b = 3 }; c = b + 1 }"),
        vec!["field `a` is defined twice"]
    );
    assert_eq!(
        messages("let q = { a = 1; ...{ a = 2 } }\nq.z"),
        vec!["field `a` is defined twice"]
    );
    // A field item after a field item is defined twice as before; after
    // one that overrode a soft field, against that one.
    let text = "{ ...{ a = 1 }; a = 2; a = 3 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `a` is defined twice");
    assert_eq!(found[0].span, first_bytes(at(text, "a = 3"), 1));
    assert_eq!(
        labels(&found[0]),
        vec![(first_bytes(at(text, "a = 2"), 1), "first defined here")]
    );
}

#[test]
fn a_spreads_fields_are_names_in_scope() {
    // From the next item on, shadowing an outer variable of the name.
    assert_eq!(
        shown_result("let x = \"s\"\n{ ...{ x = 1 }; y = x + 1 }"),
        "{x: Int, y: Int}"
    );
    // Between a spread and an override the name is the spread's, and
    // after it the override's.
    assert_eq!(
        shown_result("{ ...{ a = 1 }; b = a; a = \"s\"; c = a }"),
        "{a: String, b: Int, c: String}"
    );
    assert_eq!(
        shown_result("{ ...{ a = 1 }; b = a; ...{ a = \"s\" }; c = a }"),
        "{a: String, b: Int, c: String}"
    );
    // A later `let` shadows the field without changing it.
    assert_eq!(
        shown_result("{ ...{ a = 1 }; let a = \"s\"; b = a }"),
        "{a: Int, b: String}"
    );
    // Not visible before its spread, nor after its block.
    assert_eq!(
        messages("{ b = a; ...{ a = 1 } }"),
        vec!["undefined variable `a`"]
    );
    assert_eq!(
        messages("let r = { ...{ a = 1 } }\na"),
        vec!["undefined variable `a`"]
    );
    // `:=` on one is the error for a field.
    let text = "let r = { a = 1 }\n{ ...r; a := 2 }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot rebind `a`: it is a field, not a variable"
    );
    assert_eq!(found[0].span, last(text, "a"));
    assert_eq!(labels(&found[0]), vec![(at(text, "...r"), "declared here")]);
}

#[test]
fn a_dotted_path_under_a_soft_field_is_an_error() {
    let text = "let r = { server = { port = 1 } }\n{ ...r; server.port = 2 }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `server` is already defined");
    assert_eq!(found[0].span, at(text, "server.port"));
    assert_eq!(
        labels(&found[0]),
        vec![(operand(text, "...r"), "defined whole here")]
    );
    assert_eq!(found[0].notes, vec![WHOLE_FIELD_NOTE]);
    // A dotted `let` of the name updates the variable as ever.
    assert_eq!(
        shown_result(
            "let r = { server = { port = 1 } }\n\
                 { ...r; let server.port = \"s\"; p = server.port }"
        ),
        "{p: String, server: {port: Int}}"
    );
}

#[test]
fn a_soft_field_is_checked_against_the_expected_type() {
    let text = "let r = { a = 1 }\nlet x: {a: String} = { ...r }";
    let found = diagnostics(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, operand(text, "...r"));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected String, found Int for field `a`")
    );
    assert_eq!(
        labels(&found[0]),
        vec![(
            at(text, "{a: String}"),
            "expected because of this annotation"
        )]
    );
    assert!(found[0].notes.is_empty());
    // It cannot be awaited to fit.
    let text = "let r = { a = lazy 1 }\nlet x: {a: Int} = { ...r }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected Int, found Future<Int> for field `a`")
    );
    assert_eq!(
        found[0].notes,
        vec![
            "a spread's field cannot be awaited; define the field with its value, as in \
                 `a = r.a`"
        ]
    );
    assert_eq!(
        diagnostics("let x: {a: Int} = { ...{ a = lazy 1 } }")[0].notes,
        vec!["a spread's field cannot be awaited; define the field with its value"]
    );
    // Only the definition that stands is checked.
    assert_eq!(
        shown_result("let r = { a = 1 }\nlet x: {a: String} = { ...r; a = \"s\" }\nx"),
        "{a: String}"
    );
    assert_eq!(
        messages("let r = { a = \"s\" }\nlet x: {a: String} = { ...r; a = 1 }"),
        vec!["mismatched types"]
    );
    // A subtype fits.
    assert_eq!(
        shown_result("let r = { a = { b = 1; c = 2 } }\nlet x: {a: {b: Int}} = { ...r }\nx"),
        "{a: {b: Int}}"
    );
    // A soft field the type lacks is not warned about; a field item is.
    assert!(warnings("let r = { a = 1; z = 2 }\nlet x: {a: Int} = { ...r }").is_empty());
    let found = warnings("let x: {a: Int} = { ...{ a = 1 }; z = 2 }");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "field `z` is not in `{a: Int}`");
    // A missing field is reported as ever.
    let text = "let r = { a = 1 }\nlet x: {a: Int, b: Int} = { ...r }";
    assert_eq!(
        errors(text),
        vec![("missing field `b` in record".to_owned(), last(text, "}"))]
    );
}

#[test]
fn a_spreads_operand_is_measured_and_must_be_a_record() {
    // A future is awaited at the top, once.
    let text = "let f = lazy { a = 1; b = 2 }\n{ ...f; c = a + b }";
    assert_eq!(shown_result(text), "{a: Int, b: Int, c: Int}");
    assert_eq!(marks(text), vec![("f", AwaitMark::Top)]);
    // So is the either shape; nothing inside it is awaited.
    let text = "fn f(r: {a: Int}) -> {a: Int} { ...r }\nf";
    assert_eq!(marks(text), vec![("r", AwaitMark::Top)]);
    // A pure function's bare parameter is measured, which makes it
    // present.
    let text = "fn f(r: {a: Int}): {a: Int} { ...r }";
    assert_eq!(type_of_f(text), "fn(r: {a: Int}): {a: Int}");
    assert_eq!(demands(text, 0), vec![Some("r")]);
    // One in a field of the operand is measured where the expected type
    // is present, at the operand.
    let text = "fn f(x: Int): {a: Int} { ...{ a = x } }";
    assert_eq!(type_of_f(text), "fn(x: Int): {a: Int}");
    assert_eq!(demands(text, 0), vec![Some("{ a = x }")]);
    let text = "fn f(x: Int): {a: Int | Future<Int>} { ...{ a = x } }";
    assert_eq!(
        type_of_f(text),
        "fn(x: Int | Future<Int>): {a: Int | Future<Int>}"
    );
    assert_eq!(demands(text, 0), vec![None]);
    // In a pure function, one that may be a future is the purity error.
    let text = "fn f(r: Future<{a: Int}>): {a: Int} { ...r }";
    assert_implicit_await_error(text, last(text, "r"), "Future<{a: Int}>");
    // Anything else is not a record.
    let text = "{ ...5 }";
    let found = diagnostics(text);
    assert_eq!(
        found[0].message,
        "cannot spread a value of type Int: it is not a record"
    );
    assert_eq!(found[0].span, at(text, "...5"));
    assert_eq!(labels(&found[0]), vec![(at(text, "5"), "this is Int")]);
    assert_eq!(
        messages("{ ...[1] }"),
        vec!["cannot spread a value of type `List<Int>`: it is not a record"]
    );
    assert_eq!(
        messages("let f = lazy 1\n{ ...f }"),
        vec!["cannot spread a value of type Int: it is not a record"]
    );
    // Neither that nor an erroneous operand cascades.
    assert_eq!(
        messages("let q = { ...5 }\nq.a + 1"),
        vec!["cannot spread a value of type Int: it is not a record"]
    );
    assert_eq!(
        messages("let q = { ...z; b = 1 }\nq.a"),
        vec!["undefined variable `z`"]
    );
    // `{}` spreads nothing.
    assert_eq!(shown_result("{ ...{} }"), "{}");
    assert_eq!(shown_result("let e = {}\n{ ...e; a = 1 }"), "{a: Int}");
}

#[test]
fn a_soft_field_in_an_inferred_result() {
    for (text, expected) in [
        // From the type the operand gives it, as it is.
        (
            "fn f() -> {a: Int, b: Int} { ...{ a = 1; b = lazy 2 } }",
            "fn() -> {a: Int, b: Future<Int>}",
        ),
        (
            "fn f(r: Present<{a: Int, b: Int}>) -> {a: Int, b: Int} { ...r }",
            "fn(r: {a: Int, b: Int}) -> {a: Int, b: Int}",
        ),
        (
            "fn f(r: {a: Int}) -> {a: Int} { ...r }",
            "fn(r: {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>) -> {a: Int | \
                 Future<Int>}",
        ),
        (
            "fn f(r: {a: Int}) ->> {a: Int} { ...r }",
            "fn(r: {a: Int}) ->> {a: Int}",
        ),
        // An overridden one from the definition that stands.
        (
            "fn f() -> {a: Int, b: Int} { ...{ a = 1; b = lazy 2 }; b = 3 }",
            "fn() -> {a: Int, b: Int}",
        ),
        (
            "fn f() -> {a: Int, b: Int} { ...{ a = 1; b = 3 }; ...{ b = lazy 2 } }",
            "fn() -> {a: Int, b: Future<Int>}",
        ),
        // A field item before the spread, and a record block nested in
        // the result.
        (
            "fn f() -> {a: Int, b: Int} { a = lazy 1; ...{ b = 2 } }",
            "fn() -> {a: Future<Int>, b: Int}",
        ),
        (
            "fn f() -> {x: {a: Int}} { x = { ...{ a = lazy 1 } } }",
            "fn() -> {x: {a: Future<Int>}}",
        ),
        // The operand's other fields play no part.
        (
            "fn f() -> {a: Int} { ...{ a = 1; z = lazy 2 } }",
            "fn() -> {a: Int}",
        ),
    ] {
        assert_eq!(type_of_f(text), expected, "{text}");
    }
}

#[test]
fn a_soft_field_of_an_earlier_input_shares_its_slot() {
    let mut names = Names::new();
    let mut types = TypeTable::new();
    let parse_with = |text: &str, names: &mut Names| {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, strings) = tokenize_with(&source, names).unwrap();
        parse(&tokens, names.clone(), strings, &source).unwrap()
    };
    let first = parse_with("...{ a = 1; b = 2 }\na = \"s\"", &mut names);
    let scope = Checker::new(&first, &mut types).check().unwrap().scope;
    // Two bindings, in two slots: the top-level field `a`, declared as
    // the input starts, and `b`; the spread defines nothing for `a`.
    assert_eq!(scope.len(), 2);
    let second = parse_with("let c = a", &mut names);
    let output = Checker::with_scope(&second, scope, &mut types)
        .check()
        .unwrap();
    let resolution = &output.checked.resolution;
    let items = second.items(second.root());
    assert_eq!(resolution.item_slot(items[0]), Slot(2));
    let slot_names: Vec<&str> = resolution
        .slot_names()
        .iter()
        .map(|&name| second.names().get(name))
        .collect();
    assert_eq!(slot_names, vec!["a", "b", "c"]);
    let ItemKind::Let { value, .. } = second.item(items[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(output.checked.types.get(value), TyId::STRING);
}
