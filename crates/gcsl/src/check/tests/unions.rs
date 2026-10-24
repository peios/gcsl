//! Tests of the checker: unions, `is`, `as` and narrowing.

use super::*;

/// The message of each warning of `text`, which must check.
pub(super) fn warning_messages(text: &str) -> Vec<String> {
    warnings(text).into_iter().map(|w| w.message).collect()
}

#[test]
fn written_unions_are_accepted() {
    assert_eq!(shown_result("let w: Int | String = 1\nw"), "Int | String");
    assert_eq!(
        shown_result("let w: {a: Int} | List<Int> | Bool = true\nw"),
        "Bool | {a: Int} | List<Int>"
    );
    // A `Future` member makes it the either shape of the others.
    assert_eq!(
        shown_result("let w: Int | String | Future<String | Int> = 1\nw"),
        "Int | String | Future<Int | String>"
    );
    assert_eq!(
        shown_result("let w: Future<Int | String> | String | Int = 1\nw"),
        "Int | String | Future<Int | String>"
    );
    // Anything else with a `Future` member is rejected, at the last `|`.
    let text = "let w: Int | String | Future<Int> = 1";
    assert_eq!(
        errors(text),
        vec![(
            "a union with a `Future` member must be `T | Future<T>`".to_owned(),
            last(text, "|")
        )]
    );
}

#[test]
fn unions_display_as_they_are_written_and_round_trip() {
    for ty in [
        "Int | String",
        "Bool | {a: Int} | List<Int>",
        "Int | (fn(): Int)",
        "(fn(): Int) | (fn(): String)",
        "Int | String | Future<Int | String>",
        "List<Int | String>",
        "{a: Int | String}",
        "fn(Int | String): Int | String",
        "Bool | (fn(): Int | String)",
    ] {
        let text = format!("let f = fn(x: Present<{ty}>): {{}} {{ }}\nf");
        assert_eq!(shown_result(&text), format!("fn(x: {ty}): {{}}"), "{ty}");
    }
    // Written in any order, they show in one.
    assert_eq!(
        shown_result("let f = fn(x: Present<List<Int> | String | {a: Int}>): {} { }\nf"),
        "fn(x: String | {a: Int} | List<Int>): {}"
    );
}

#[test]
fn bare_unions_follow_the_bare_type_rule() {
    // An impure function's parameter and result are the either shape of
    // the union, each member's inside following the rule too.
    assert_eq!(
        type_of_f("fn f(v: Int | {a: Int}) -> Int { 1 }"),
        "fn(v: Int | {a: Int | Future<Int>} | Future<Int | {a: Int | Future<Int>}>) -> Int"
    );
    // A memoised function's parameter is present; its result inferred.
    assert_eq!(
        type_of_f("fn f(v: Int | String) ->> Int | String { v }"),
        "fn(v: Int | String) ->> Int | String"
    );
    // A pure function's parameter is inferred: present if measured, the
    // either shape if passed through.
    assert_eq!(
        type_of_f("fn f(v: Int | String): Int { if v is Int { v } else { 0 } }"),
        "fn(v: Int | String): Int"
    );
    assert_eq!(
        type_of_f("fn f(v: Int | String): {k: Int | String | Future<Int | String>} { k = v }"),
        "fn(v: Int | String | Future<Int | String>): {k: Int | String | Future<Int | String>}"
    );
    // A function type member follows its own marker.
    assert_eq!(
        type_of_f("fn f(v: Int | fn(Int) -> Int) -> Int { 1 }"),
        "fn(v: Int | (fn(Int | Future<Int>) -> Int | Future<Int>) | Future<Int | (fn(Int | \
             Future<Int>) -> Int | Future<Int>)>) -> Int"
    );
    // The result of an impure function is inferred at its top.
    assert_eq!(
        type_of_f("fn f(c: Bool) -> Int | String { if c { 1 } else { \"a\" } }"),
        "fn(c: Bool | Future<Bool>) -> Int | String"
    );
    assert_eq!(
        type_of_f("fn f() -> Int | String { lazy 1 }"),
        "fn() -> Future<Int | String>"
    );
}

#[test]
fn a_widened_join_is_a_warning() {
    let text = "let v = if true { 1 } else { \"one\" }\nv";
    assert_eq!(shown_result(text), "Int | String");
    let warning = &warnings(text)[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(
        warning.notes,
        vec![
            "the `if` is `Int | String`; annotate it with that type if both are meant, or \
                 tell them apart with `is`"
        ]
    );
    // A join that finds a supertype is not widened.
    assert!(warnings("if true { { a = 1; b = 2 } } else { { a = 3 } }").is_empty());
    assert!(warnings("let u: Int | String = 1\nif true { u } else { 2 }").is_empty());
    // Literals adapt before the join.
    assert!(warnings("if true { 1 } else { 2.5 }").is_empty());
    assert_eq!(result_type("if true { 1 } else { 2.5 }"), TyId::FLOAT);
    // Lists: the elements, with the type the list has.
    let text = "let xs = [1, \"a\", true]\nxs";
    assert_eq!(shown_result(text), "List<Int | Bool | String>");
    let found = warnings(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].span, at(text, "\"a\""));
    assert_eq!(
        found[0].notes,
        vec![
            "the list is `List<Int | Bool | String>`; annotate it with that type if both \
                 are meant, or tell them apart with `is`"
        ]
    );
    // Inside: a record field the branches disagree on.
    let text = "if true { { a = 1; b = 2 } } else { { a = \"x\" } }";
    assert_eq!(shown_result(text), "{a: Int | String}");
    assert_eq!(warning_messages(text).len(), 1);
}

#[test]
fn an_expected_union_makes_no_join() {
    for text in [
        "let w: Int | String = if true { 1 } else { \"one\" }",
        "let w = (if true { 1 } else { \"one\" }) : Int | String",
        "fn f(v: Int | String): Int { 1 }\nf(if true { 1 } else { \"one\" })",
        "fn f(c: Bool): Int | String { if c { 1 } else { \"one\" } }",
        "let r: {a: Int | String} = { a = if true { 1 } else { \"one\" } }",
        "let w: Int | String = { let c = true\nif c { 1 } else { \"one\" } }",
        "let xs: List<Int | String> = [1, \"one\", if true { 2 } else { \"two\" }]",
        "let w: List<Int | String> | Bool = [1, \"one\"]",
    ] {
        assert!(warnings(text).is_empty(), "{text}: {:?}", warnings(text));
    }
    // Literals adapt to a union's numeric member.
    let text = "let w: Float | String = 1\nw";
    assert_eq!(shown_result(text), "Float | String");
    assert_eq!(all_types(text)[0], TyId::FLOAT);
    // A record block is checked against a union's one record member.
    assert_eq!(
        messages("let w: {a: Int} | String = { b = 1 }"),
        vec![
            "field `b` is not in `{a: Int}`",
            "missing field `a` in record"
        ]
    );
    // An annotation the value does not fit is still an error.
    assert_eq!(
        messages("let w: Int | String = true"),
        vec!["mismatched types"]
    );
}

#[test]
fn a_misused_union_is_an_error_with_a_note() {
    let note = "tell them apart first, as in `if v is Int { ... }`";
    let union = "let v: Int | String = 1\n";
    for (rest, message) in [
        ("v + 1", "cannot add `Int | String` and Int"),
        ("v < 1", "cannot order `Int | String`"),
        ("-v", "cannot negate `Int | String`"),
        ("if v { 1 } else { 2 }", "mismatched types"),
        (
            "[1, 2][v]",
            "a list index must be an Int, found `Int | String`",
        ),
        ("v..3", "a range's bounds must be Int, found `Int | String`"),
        ("v.a", "cannot access field `a` of `Int | String`"),
        (
            "v[0]",
            "cannot index a value of type `Int | String`: it is not a list",
        ),
        (
            "for x in v { x }",
            "cannot iterate over a value of type `Int | String`: it is not a list",
        ),
        ("len(v)", "`len` takes a list, found `Int | String`"),
        ("v(1)", "cannot call a value of type `Int | String`"),
        ("v and true", "mismatched types"),
    ] {
        let text = format!("{union}{rest}");
        let found = diagnostics(&text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        assert_eq!(found[0].message, message, "{text}");
        assert!(
            found[0].notes.iter().any(|n| n == note),
            "{text}: {:?}",
            found[0].notes
        );
    }
    // Not a variable: a placeholder name.
    let found = diagnostics("fn f(): Int | String { 1 }\nf() + 1");
    assert_eq!(
        found[0].notes,
        vec!["tell them apart first, as in `if x is Int { ... }`"]
    );
    // Functions alone cannot be told apart.
    let found =
        diagnostics("let g = if true { fn(a: Int): Int { a } } else { fn(): Int { 1 } }\ng(1)");
    assert_eq!(
        found
            .iter()
            .find(|d| d.severity == Severity::Error)
            .map(|d| d.notes.clone()),
        Some(vec![
            "only functions can be called".to_owned(),
            "`is` cannot tell function types apart; annotate the value with one function \
                 type that each of them fits"
                .to_owned()
        ])
    );
    // `==` needs one type that is not a union; `===` compares values.
    let found = diagnostics(&format!("{union}v == v"));
    assert_eq!(found[0].message, "cannot compare `Int | String` with `==`");
    assert_eq!(found[0].notes, vec!["narrow it, or compare with `===`"]);
    assert_eq!(result_type(&format!("{union}v === v")), TyId::BOOL);
    assert_eq!(result_type(&format!("{union}v !== v")), TyId::BOOL);
    // Interpolation shows any value.
    assert_eq!(result_type(&format!("{union}\"{{v}}\"")), TyId::STRING);
}

#[test]
fn records_and_lists_of_unions_are_used_as_their_members_agree() {
    // A field every member has: the join of its types, no warning.
    let text = "let r: {a: Int, b: Int} | {a: String} = { a = 1; b = 2 }\nr.a";
    assert_eq!(shown_result(text), "Int | String");
    assert!(warnings(text).is_empty());
    let found = diagnostics("let r: {a: Int} | {b: Int} = { a = 1 }\nr.b");
    assert_eq!(
        found[0].message,
        "cannot access field `b` of `{a: Int} | {b: Int}`"
    );
    assert_eq!(
        found[0].notes,
        vec!["tell them apart first, as in `if r is {a: Int} { ... }`"]
    );
    // Indexing, iterating and `len` of a union of lists.
    let lists = "let xs: List<Int> | List<String> = [1]\n";
    assert_eq!(shown_result(&format!("{lists}xs[0]")), "Int | String");
    assert_eq!(
        shown_result(&format!("{lists}for x in xs {{ x }}")),
        "List<Int | String>"
    );
    assert_eq!(result_type(&format!("{lists}len(xs)")), TyId::INT);
}

#[test]
fn is_tests_a_type() {
    let union = "let v: Int | List<Int> = 1\n";
    assert_eq!(result_type(&format!("{union}v is Int")), TyId::BOOL);
    assert!(warnings(&format!("{union}v is List<Int>")).is_empty());
    // A test that always passes is a warning.
    let text = "let n = 1\nn is Int";
    let found = warnings(text);
    assert_eq!(found[0].message, "`n` is always an Int");
    assert_eq!(found[0].span, at(text, "n is Int"));
    let found = warnings("let r = { a = 1 }\n(r) is {}");
    assert_eq!(found[0].message, "`r` is always a `{}`");
    // One that cannot is an error, at the type.
    let text = format!("{union}v is String");
    let found = diagnostics(&text);
    assert_eq!(found[0].message, "`v` is never a String");
    assert_eq!(found[0].span, last(&text, "String"));
    assert_eq!(
        labels(&found[0]),
        vec![(last(&text, "v"), "this is Int | List<Int>")]
    );
    assert_eq!(messages("[1, 2] is {}"), vec!["this value is never a `{}`"]);
    // Width: `{}` may be any member record.
    assert!(warnings("let x: Int | {a: Int} = 1\nx is {}").is_empty());
    // A member that overlaps the type may pass, though neither is a
    // subtype of the other.
    let text = "let x: {kind: String, value: Int | String} = { kind = \"n\"; value = 1 }\n\
                    x is {value: Int}";
    assert_eq!(result_type(text), TyId::BOOL);
    // Futures and function types cannot be tested.
    let found = diagnostics(&format!("{union}v is Future<Int>"));
    assert_eq!(found[0].message, "`is` cannot test a future");
    let found = diagnostics(&format!("{union}v is {{a: Int | Future<Int>}}"));
    assert_eq!(found[0].message, "`is` cannot test a future");
    let found = diagnostics(&format!("{union}v is fn(): Int"));
    assert_eq!(found[0].message, "`is` cannot test a function type");
    assert_eq!(
        messages(&format!("{union}v is Nope")),
        vec!["unknown type `Nope`"]
    );
}

#[test]
fn is_awaits_as_deep_as_its_plan_looks() {
    // At the top, for a test of the kind alone.
    assert_eq!(
        marks(
            "let x: List<Int | Future<Int>> | Int | Future<List<Int | Future<Int>> | Int> = 1\n\
                 x is Int"
        ),
        vec![("x", AwaitMark::Top)]
    );
    // Deeply, for one that looks at the elements.
    assert_eq!(
        marks("let x: List<Int | Future<Int>> | List<String> = []\nx is List<Int>"),
        vec![("x", AwaitMark::Deep)]
    );
    // In a pure function, an await is the purity error; a parameter is
    // measured instead.
    let text = "fn f(): Bool { let l: Int | String | Future<Int | String> = 1\nl is Int }";
    assert_eq!(
        messages(text),
        vec!["cannot use a value that may be a future in a pure function"]
    );
    assert_eq!(
        type_of_f("fn f(v: Int | String): Bool { v is Int }"),
        "fn(v: Int | String): Bool"
    );
    assert_eq!(
        demands("fn f(v: Int | String): Bool { v is Int }", 0),
        vec![Some("v")]
    );
}

#[test]
fn as_narrows_with_a_check() {
    let union = "let v: Int | String = 1\n";
    assert_eq!(result_type(&format!("{union}v as Int")), TyId::INT);
    // As precise as the value's type allows.
    assert_eq!(
        shown_result("let r: {a: Int, b: Int} | String = \"s\"\nr as {a: Int}"),
        "{a: Int, b: Int}"
    );
    let found = warnings("let n = 1\nn as Int");
    assert_eq!(
        found[0].message,
        "`as` changes nothing here: `n` is already an Int"
    );
    assert_eq!(found[0].notes, vec!["`: Int` says the same without a test"]);
    assert_eq!(
        messages(&format!("{union}v as Bool")),
        vec!["`v` is never a Bool"]
    );
    let found = diagnostics(&format!("{union}v as Future<Int>"));
    assert_eq!(found[0].message, "`as` cannot test a future");
    // It is pure, where its operand needs no await.
    assert_eq!(
        type_of_f("fn f(v: Int | String): Int { v as Int }"),
        "fn(v: Int | String): Int"
    );
    // Its resolution names what it tests.
    let (ast, resolution) = resolved(&format!("{union}v as Int"));
    let id = result_expr(&ast);
    let test = resolution.test_of(id).unwrap();
    assert_eq!(&*test.subject, "`v`");
    assert_eq!(&*test.target, "an Int");
    assert_eq!(test.test, ValueTest::Kind(ValueKind::Int));
    assert_eq!(test.slot, Slot::NONE);
}

#[test]
fn an_if_narrows_its_branches() {
    let text = "let v: Int | String = 1\nif v is Int { v + 1 } else { len([v]) }";
    assert_eq!(type_at(text, nth(text, "v", 2)), "Int");
    assert_eq!(type_at(text, nth(text, "v", 3)), "String");
    // The branches read a slot of their own, which the test writes,
    // after the standard library's.
    let (ast, _, output) = check_prelude(text);
    let resolution = output.unwrap().checked.resolution;
    let uses = uses(&ast, &resolution, "v");
    let base = standard_slots();
    assert_eq!(
        uses,
        vec![
            VarRef::Local(Slot(base)),
            VarRef::Local(Slot(base + 1)),
            VarRef::Local(Slot(base + 1))
        ]
    );
    // `else if` narrows the remainder in turn.
    let text = "let v: Int | String | Bool = 1\n\
                    if v is Int { 0 } else if v is String { len([v]) } else { if v { 1 } else { 2 } }";
    assert_eq!(type_at(text, nth(text, "v", 3)), "String");
    assert_eq!(type_at(text, nth(text, "v", 4)), "Bool");
    // A mutable variable, a parameter, a field and a `for`'s variable:
    // the use in the `then` branch, `{ n }`.
    let inside = |text: &str| {
        let block = at(text, "{ n }");
        Span::new(block.start + 2, block.start + 3)
    };
    for text in [
        "let mut n: Int | String = 1\nif n is Int { n } else { 0 }",
        "fn f(n: Int | String): Int { if n is Int { n } else { 0 } }",
        "{ n = if true { 1 } else { \"s\" }; b = if n is Int { n } else { 0 } }",
        "let ns: List<Int | String> = []\nfor n in ns { if n is Int { n } else { 0 } }",
    ] {
        assert_eq!(type_at(text, inside(text)), "Int", "{text}");
    }
    // The `else` branch keeps the variable when nothing always passes
    // and nothing was awaited, so a mutable variable is still one.
    let text = "let mut w: {a: Int} | String = \"s\"\n\
                    if w is {a: Int, b: Int} { 0 } else { w := \"t\"\n0 }";
    assert!(diagnostics_or_none(text).is_empty());
    // Where a member always passes, it is a copy of the remainder
    // there, which `:=` may not rebind, in either branch, at any depth,
    // by a plain or a dotted path: it would not change the variable.
    for text in [
        "let mut w: Int | String = 1\nif w is Int { 0 } else { w := \"t\"\n0 }",
        "let mut w: Int | String = 1\nif w is Int { w := 2\n0 } else { 0 }",
        "let mut w: Int | String = 1\nif w is Int { for i in 0..2 { w := i }\n0 } else { 0 }",
        "let w: Int | String = 1\nif w is Int { w := 2\n0 } else { 0 }",
        "let mut w: {a: Int} | String = \"s\"\nif w is String { 0 } else { w.a := 2\n0 }",
    ] {
        let found = diagnostics(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(
            found[0].message, "cannot rebind `w` here: it is narrowed by `is`",
            "{text}"
        );
        let narrowing = if text.contains("w is Int") {
            at(text, "w is Int")
        } else {
            at(text, "w is String")
        };
        assert_eq!(
            labels(&found[0]),
            vec![(narrowing, "narrowed here")],
            "{text}"
        );
        assert_eq!(
            found[0].notes,
            vec![
                "in this branch `w` is a copy of the value that was tested, so rebinding it \
                     would not change the variable; test a copy under another name instead, as \
                     in `let v = w` and `if v is ...`, which leaves `w` free to rebind"
            ],
            "{text}"
        );
    }
    // Tested under another name, the variable is free to rebind, and a
    // `let` in the branch shadows the copy as any `let` does.
    for text in [
        "let mut w: Int | String = 1\nlet v = w\nif v is Int { w := v + 1 }",
        "let w: Int | String = 1\nif w is Int { let w = w + 1\nw } else { 0 }",
    ] {
        assert!(diagnostics_or_none(text).is_empty(), "{text}");
    }
    // ...and has the remainder when the test awaited the value.
    let text = "let q: Int | String | Future<Int | String> = 1\nif q is Int { q } else { q }";
    assert_eq!(type_at(text, nth(text, "q", 2)), "Int");
    assert_eq!(type_at(text, nth(text, "q", 3)), "String");
    // Through parentheses.
    let text = "let v: Int | String = 1\nif ((v) is Int) { v } else { 0 }";
    assert_eq!(type_at(text, nth(text, "v", 2)), "Int");
}

#[test]
fn other_conditions_narrow_nothing() {
    let union = "let v: Int | String = 1\nlet r = { a = v }\n";
    for rest in [
        "if v is Int and true { v + 1 } else { 0 }",
        "if not (v is Int) { 0 } else { v + 1 }",
        "if r.a is Int { r.a + 1 } else { 0 }",
    ] {
        let found = diagnostics(&format!("{union}{rest}"));
        assert!(
            found.iter().any(|d| d.severity == Severity::Error),
            "{rest}: {found:?}"
        );
    }
    // Nothing narrows after the `if`.
    let text = format!("{union}let n = if v is Int {{ v }} else {{ 0 }}\nv + 1");
    assert_eq!(messages(&text), vec!["cannot add `Int | String` and Int"]);
}

#[test]
fn a_union_of_functions_narrowed_in_else_is_called() {
    let text = "let g: Int | fn(): Int = fn(): Int { 7 }\nif g is Int { g } else { g() }";
    assert_eq!(result_type(text), TyId::INT);
}
