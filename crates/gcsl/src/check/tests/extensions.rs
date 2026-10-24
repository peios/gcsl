//! Tests of the checker: rest-fields, quantities, attributes, generics, methods
//! and enums.

use super::*;

#[test]
fn a_record_literal_where_a_rest_field_is_expected_has_it() {
    // Each field the type does not name is checked against the
    // rest-field, and the record has it: no field is extra.
    let text = "let m: {*: String} = {\"bin/*\" = \"bin/*\"; b = \"x\"}\nm";
    assert_eq!(warnings(text), []);
    assert_eq!(shown_result(text), "{*: String}");
    let text = "let f: {to: String, *: Int} = {to = \"a\"; x = 1}\nf";
    assert_eq!(shown_result(text), "{to: String, *: Int}");
    // `{}`, and a literal that a block results in, an `if`'s branches, a
    // list's elements, a loop's results and an argument alike.
    for text in [
        "let e: {*: Int} = {}\ne",
        "let e: {*: Int} = { let x = 1; {a = x} }\ne",
        "let e: {*: Int} = if true { {a = 1} } else { {} }\ne",
        "let e: List<{*: Int}> = [{a = 1}, {}]\ne[0]",
        "let e: List<{*: Int}> = for n in [1, 2] { {a = n} }\ne[0]",
        "fn f(r: {*: Int}): {*: Int} { r }\nf({a = 1})",
    ] {
        assert_eq!(warnings(text), [], "{text}");
        assert_eq!(shown_result(text), "{*: Int}", "{text}");
    }
    // A nested literal is checked against the rest-field as any field's
    // value is.
    let text = "let e: {*: {*: Int}} = {a = {b = 1}}\ne";
    assert_eq!(warnings(text), []);
    assert_eq!(shown_result(text), "{*: {*: Int}}");
}

#[test]
fn a_field_a_rest_field_covers_must_fit_it() {
    let found = diagnostics("let m: {*: String} = {a = \"x\"; b = 1}");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(found[0].span, Span::new(35, 36));
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected String, found Int")
    );
    // An annotated field, and a spread's.
    assert_eq!(
        messages("let m: {*: String} = {a: Int = 1}"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("let s = {a = 1}\nlet m: {*: String} = {...s}"),
        vec!["mismatched types"]
    );
    // A named field is exempt.
    assert_eq!(
        warnings("let f: {to: String, *: Int} = {to = \"a\"}\nf"),
        []
    );
    // A dotted path builds a record without one, which cannot have it.
    let found = diagnostics("let m: {*: {*: Int}} = {a.b = 1}");
    assert_eq!(found[0].message, "mismatched types");
    assert_eq!(
        found[0].label.as_deref(),
        Some("expected {*: Int}, found {b: Int}")
    );
    assert_eq!(
        found[0].notes,
        vec![
            "dotted paths build a record without a rest-field; define the field whole, as \
             in `a = {...}`"
        ]
    );
}

#[test]
fn a_value_whose_type_hides_fields_does_not_fit_a_rest_field() {
    // `x` may hold fields its type does not show, of any type.
    assert_eq!(
        messages("let x = {a = 1}\nlet m: {*: Int} = x"),
        vec!["mismatched types"]
    );
    assert_eq!(
        messages("fn f(r: {*: Int}): Int { 1 }\nlet x = {a = 1}\nf(x)"),
        vec!["mismatched types"]
    );
    // One whose type has a rest-field that fits does.
    assert_eq!(
        shown_result("let x: {a: Int, *: Int} = {a = 1}\nlet m: {*: Int} = x\nm"),
        "{*: Int}"
    );
}

#[test]
fn rest_field_errors() {
    assert_eq!(
        errors("1 : {*: Int, a: Int, *: String}"),
        vec![(
            "the rest-field appears twice in this type".to_owned(),
            Span::new(21, 22)
        )]
    );
    // A spread defines the fields its operand's type names.
    let found = diagnostics("let m: {*: Int} = {a = 1}\nlet c = {...m}");
    assert_eq!(
        found[0].message,
        "cannot spread a value of type `{*: Int}`: it has a rest-field"
    );
    // A field the rest-field covers is not read by name.
    let found = diagnostics("let m: {*: Int} = {a = 1}\nm.a");
    assert_eq!(found[0].message, "no field `a` in `{*: Int}`");
    assert_eq!(
        found[0].notes,
        vec!["a field that only the rest-field `*: ...` covers cannot be read by name"]
    );
    let found = diagnostics("let f: {to: String, *: Int} = {to = \"a\"}\nf.x");
    assert_eq!(
        found[0].notes,
        vec![
            "the only named field is `to`",
            "a field that only the rest-field `*: ...` covers cannot be read by name"
        ]
    );
    // A named field is read as ever.
    assert_eq!(
        shown_result("let f: {to: String, *: Int} = {to = \"a\"; x = 1}\nf.to"),
        "String"
    );
}

#[test]
fn a_rest_field_s_fields_are_tested_by_is_and_as() {
    // A record whose type hides its fields is tested at runtime; one that
    // can never be one is an error, as for any type.
    assert_eq!(
        shown_result("let x = {a = 1}\nlet y: {} = x\ny as {*: Int}"),
        "{*: Int}"
    );
    assert_eq!(
        messages("let f: {to: String, *: Int} = {to = \"a\"}\nf is {*: Int}"),
        vec!["`f` is never a `{*: Int}`"]
    );
}

#[test]
fn quantities_have_their_own_arithmetic() {
    let cases = [
        ("30s", "Duration"),
        ("2GiB", "Size"),
        ("1s + 1m", "Duration"),
        ("1B - 1KiB", "Size"),
        ("2 * 1s", "Duration"),
        ("1s * 2", "Duration"),
        ("1s // 2", "Duration"),
        ("1h // 1m", "Int"),
        ("1h % 7m", "Duration"),
        ("-1s", "Duration"),
        ("1s < 2s", "Bool"),
        ("1s == 1000ms", "Bool"),
        ("\"{1s}\"", "String"),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
    // An untyped integer beside one is an `Int`.
    let types = all_types("1s * 2");
    assert!(types.contains(&TyId::INT), "{types:?}");
}

#[test]
fn quantities_mix_with_nothing_else() {
    let cases = [
        ("1s + 1", "cannot add Duration and Int"),
        ("1s + 1B", "cannot add Duration and Size"),
        ("1s * 1.5", "cannot multiply Duration and Float"),
        ("1s * 1s", "cannot multiply Duration and Duration"),
        ("1s / 2", "cannot divide Duration and Int"),
        ("1s % 2", "cannot take modulo of Duration and Int"),
        ("2 // 1s", "cannot floor-divide Int and Duration"),
        ("1s // 1B", "cannot floor-divide Duration and Size"),
        ("0 == 0s", "cannot compare Int and Duration"),
        ("1s < 1B", "cannot compare Duration and Size"),
        ("let x: Duration = 5\nx", "mismatched types"),
    ];
    for (text, message) in cases {
        assert_eq!(messages(text), [message], "{text}");
    }
    // `/` says to use `//`, and `//` and `%` what they take.
    let found = diagnostics("1B / 2");
    assert_eq!(
        found[0].notes,
        ["use `//`; a Size is a whole number of bytes"]
    );
    let found = diagnostics("1s % 2");
    assert_eq!(
        found[0].notes,
        [
            "a Duration divides with `//` by an Int, giving a Duration, or by a Duration, giving \
          an Int, and `%` gives the remainder of one Duration by another"
        ]
    );
    // `Duration` and `Size` are built-in type names.
    assert_eq!(messages("type Size = Int"), ["`Size` is a built-in type"]);
}

#[test]
fn a_quantity_literal_is_a_constant_default() {
    let text = "fn f(t: Duration = 30s, s: Size = -1KiB): Int { 0 }";
    assert_eq!(
        defaults_of(text, 0),
        [
            Some(Constant::Quantity(Quantity::Duration, 30_000_000_000)),
            Some(Constant::Quantity(Quantity::Size, -1024)),
        ]
    );
    // A field's default too, shown in its type.
    let text = "type T = {t: Duration = 1m30s}\nfn f(r: T): T { r }\nf";
    assert_eq!(
        file_type(text),
        "fn(r: {t: Duration = 1m30s}): {t: Duration = 1m30s}"
    );
}

#[test]
fn the_checker_s_own_attributes_are_checked_without_a_workspace() {
    let text = "#[retry(3)] #[timeout(1m)]\nfn f() -> Int { 1 }\nf()";
    let (_, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(
        output.checked.resolution.attributes(FnId::new(0)),
        [
            Attribute {
                id: AttributeId::Builtin("retry"),
                args: Box::new([(Box::from("times"), Constant::Int(3))]),
                defers: false,
            },
            Attribute {
                id: AttributeId::Builtin("timeout"),
                args: Box::new([(
                    Box::from("limit"),
                    Constant::Quantity(Quantity::Duration, 60_000_000_000)
                )]),
                defers: false,
            },
        ]
    );
    let cases = [
        ("#[retry]", "`#[retry]` needs `times`"),
        ("#[retry(\"3\")]", "mismatched types"),
        ("#[retry(3, 4)]", "`#[retry]` takes one argument"),
        ("#[retry(tims = 3)]", "`#[retry]` has no argument `tims`"),
        ("#[retry(times = 1, times = 2)]", "`times` is given twice"),
        ("#[timeout(30)]", "mismatched types"),
        ("#[rety(3)]", "unknown attribute `rety`"),
        (
            "#[retry(1 + 2)]",
            "an attribute's argument must be a constant",
        ),
    ];
    for (attribute, message) in cases {
        let text = format!("{attribute}\nfn f() -> Int {{ 1 }}\nf()");
        assert_eq!(messages(&text), [message], "{attribute}");
    }
    // A misspelt one is suggested, and a declared one is unknown without a
    // workspace to declare it.
    let found = diagnostics("#[rety(3)]\nfn f() -> Int { 1 }\nf()");
    assert_eq!(
        found[0].suggestion.as_ref().map(|s| s.replacement.as_str()),
        Some("retry")
    );
}

#[test]
fn retry_and_timeout_need_arguments_they_can_mean() {
    let cases = [
        ("#[retry(-1)]", "`#[retry]` needs a `times` of at least 0"),
        ("#[timeout(0s)]", "`#[timeout]` needs a positive `limit`"),
        ("#[timeout(-1s)]", "`#[timeout]` needs a positive `limit`"),
    ];
    for (attribute, message) in cases {
        let text = format!("{attribute}\nfn f() -> Int {{ 1 }}\nf()");
        assert_eq!(messages(&text), [message], "{attribute}");
    }
    // Retrying a pure function is a warning.
    assert_eq!(
        warnings("#[retry(2)]\nfn f(): Int { 1 }\nf()")
            .iter()
            .map(|warning| warning.message.as_str())
            .collect::<Vec<_>>(),
        ["retrying a pure function cannot change its result"]
    );
    assert!(warnings("#[retry(2)]\nfn f() -> Int { 1 }\nf()").is_empty());
}

#[test]
fn track_caller_is_the_checker_s_own_and_takes_no_arguments() {
    let text = "#[track_caller]\nfn f() -> Int { 1 }\nf()";
    let (_, _, output) = check_prelude(text);
    let output = output.unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(
        output.checked.resolution.attributes(FnId::new(0)),
        [Attribute {
            id: AttributeId::Builtin("track_caller"),
            args: Box::new([]),
            defers: false,
        }]
    );
    // On a pure function too: where an error is reported is no effect.
    assert!(
        check_prelude("#[track_caller]\nfn f(): Int { 1 }\nf()")
            .2
            .is_ok()
    );
    assert!(
        check_prelude("#[track_caller()]\nfn f(): Int { 1 }\nf()")
            .2
            .is_ok()
    );
    assert_eq!(
        messages("#[track_caller(1)]\nfn f(): Int { 1 }\nf()"),
        ["`#[track_caller]` takes no argument"]
    );
}

#[test]
fn a_call_of_a_generic_function_binds_its_type_parameters() {
    let cases = [
        ("fn id<T>(x: T): T { x }\nid(1)", "Int"),
        ("fn id<T>(x: T): T { x }\nid(\"a\")", "String"),
        ("fn id<T>(x: T): T { x }\nid([1.5])", "List<Float>"),
        (
            "fn pair<A, B>(a: A, b: B): {first: A, second: B} { { first = a; second = b } }\n\
             pair(1, \"x\")",
            "{first: Int, second: String}",
        ),
        (
            "fn first<T>(xs: List<T>, fallback: T): T { fallback }\nfirst([], \"none\")",
            "String",
        ),
        // An integer literal adapts to what another argument binds.
        (
            "fn first<T>(xs: List<T>, fallback: T): T { fallback }\nfirst([1.5], 0)",
            "Float",
        ),
        (
            "fn apply<T, U>(f: fn(x: T): U, x: T): U { f(x) }\n\
             fn double(n: Int): Int { n * 2 }\napply(double, 21)",
            "Int",
        ),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
    // A generic function's type shows its type parameters.
    let (ast, types, output) = check_prelude("def keep<T>(x: T): {v: T} { v = x }");
    let output = output.unwrap();
    assert_eq!(
        types.display(output.ty, ast.names()).to_string(),
        "{keep: fn<T>(x: T): {v: T}}"
    );
}

#[test]
fn a_generic_body_holds_its_type_parameters_opaque() {
    let cases = [
        ("fn f<T>(x: T): T { x + 1 }", "cannot add T and Int"),
        (
            "fn f<T>(x: T): String { \"{x}\" }",
            "cannot interpolate a value of type T, of a type parameter",
        ),
        ("fn f<T>(x: T): Int { x }", "mismatched types"),
        ("fn f<T>(x: T): Bool { x == x }", "cannot compare T and T"),
        (
            "fn id<T>(x: T): T { x }\nlet g = id",
            "a generic function must be called",
        ),
        (
            "fn none<T>(): List<T> { [] }\nnone()",
            "cannot infer `T` for this call of `none`",
        ),
        (
            "fn id<T>(x: T): T { x }\nlet x: Int = id(\"s\")\nx",
            "mismatched types",
        ),
    ];
    for (text, message) in cases {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
}

#[test]
fn arguments_binding_a_type_parameter_differently_widen_it_with_a_warning() {
    let pick = "fn pick<T>(a: T, b: T): T { a }\n";
    for (call, ty, warned) in [
        ("let a = 1\npick(a, \"s\")", "Int | String", true),
        ("pick(1, \"s\")", "Int | String", true),
        // A literal adapts to what the other argument binds.
        ("pick(2.5, 1)", "Float", false),
        ("pick(1, 2)", "Int", false),
        ("pick((1 : Int | String), \"s\")", "Int | String", false),
    ] {
        let text = format!("{pick}{call}");
        assert_eq!(file_type(&text), ty, "{text}");
        let messages = warning_messages(&text);
        assert_eq!(
            messages
                .iter()
                .any(|m| m == "the arguments for `T` have different types"),
            warned,
            "{text}: {messages:?}"
        );
    }
}

#[test]
fn a_built_in_type_takes_one_type_argument() {
    for (text, message) in [
        (
            "let x: List<Int, Int> = []\nx",
            "type `List` takes 1 type argument, given 2",
        ),
        (
            "let x: Future<Int, Int> = 1\nx",
            "type `Future` takes 1 type argument, given 2",
        ),
        (
            "let x: Int<String> = 1\nx",
            "type `Int` takes no type argument",
        ),
        ("let x: Nope<Int, Int> = 1\nx", "unknown type `Nope`"),
    ] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
}

#[test]
fn a_method_is_chosen_by_its_receiver_type() {
    let cases = [
        ("fn (x: Int) twice(): Int { x * 2 }\n3.twice()", "Int"),
        (
            "fn (x: Int) twice(): Int { x * 2 }\nlet a = 3\na.twice()",
            "Int",
        ),
        (
            "fn (xs: List<T>) first<T>(f: T): T { f }\n[\"a\"].first(\"b\")",
            "String",
        ),
        // The most specific receiver wins.
        (
            "fn (xs: List<T>) size<T>(): String { \"any\" }\nfn (xs: List<Int>) size(): Int { 1 }\n[1].size()",
            "Int",
        ),
        (
            "fn (xs: List<T>) size<T>(): String { \"any\" }\nfn (xs: List<Int>) size(): Int { 1 }\n[true].size()",
            "String",
        ),
        // A record with more fields than the receiver's has.
        (
            "fn (r: {a: Int}) get(): Int { r.a }\n{ a = 1; b = 2 }.get()",
            "Int",
        ),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
}

#[test]
fn method_call_errors() {
    for (text, message) in [
        (
            "fn (x: String) shout(): String { x }\n3.shout()",
            "no method `shout` for type Int",
        ),
        (
            "fn (r: {a: Int}) b(): Int { r.a }\nlet v = { a = 1; b = 2 }\nv.b()",
            "`b` is both a field of `{a: Int, b: Int}` and a method",
        ),
        (
            "fn (r: {a: Int}) m(): Int { 1 }\nfn (r: {b: Int}) m(): Int { 2 }\n{ a = 1; b = 2 }.m()",
            "method `m` is ambiguous for type `{a: Int, b: Int}`",
        ),
        (
            "fn (x: Int) m(): Int { 1 }\nfn (y: Int) m(): Int { 2 }\n1",
            "method `m` is declared twice for type Int",
        ),
        (
            "fn (r: {a: Int}) a(): Int { 1 }\n1",
            "method `a` has the name of a field of its receiver type `{a: Int}`",
        ),
        (
            "fn (x: Int) m(y: Int): Int { y }\n3.m(\"s\")",
            "mismatched types",
        ),
        ("fn (x: Int) m(): Int { x }\nm", "undefined variable `m`"),
    ] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
    let text = "def (x: Int) loud() -> Int { x }\nfn quiet(): Int { 3.loud() }\n1";
    let found = messages(text);
    assert!(
        found
            .iter()
            .any(|m| m == "cannot call an impure function from a pure function"),
        "{found:?}"
    );
}

#[test]
fn a_method_s_name_alone_notes_how_to_call_it() {
    let (_, _, output) = check_text("fn (x: Int) twice(): Int { x }\ntwice");
    let found = output.unwrap_err();
    let error = found
        .iter()
        .find(|d| d.message == "undefined variable `twice`")
        .unwrap();
    assert_eq!(
        error.notes,
        ["`twice` is a method: call it on a value, as in `x.twice()`"]
    );
}

#[test]
fn a_variant_is_a_callable_value_of_its_enum() {
    let cases = [
        ("enum S extensible\nenum S::A<Int>\nS::A", "Variant<S, Int>"),
        ("enum S extensible\nenum S::A<Int>\nS::A(1)", "S"),
        ("enum S extensible\nenum S::B\nS::B({})", "S"),
        (
            "enum S extensible\nenum S::A<Int>\nfn make(k: Variant<S, Int>): S { k(2) }\nmake(S::A)",
            "S",
        ),
        (
            "enum S extensible<Int>\nenum S::A<Int>\nS::A(1) == S::A(2)",
            "Bool",
        ),
        (
            "enum S extensible\nenum S::A<Int>\nlet v: S | Int = 3\nv is S",
            "Bool",
        ),
        (
            "enum S extensible\nenum S::A<Int>\nS::A.payload(S::A(4))",
            "Int",
        ),
        (
            "type P = {a: Int, b: Int}\nlet p: Partial<P> = { a = 1 }\np.matches({ a = 1; b = 2 })",
            "Bool",
        ),
    ];
    for (text, ty) in cases {
        assert_eq!(file_type(text), ty, "{text}");
    }
}

#[test]
fn enum_errors() {
    for (text, message) in [
        ("type T = Int\nenum T::V\n1", "Int is not an enum"),
        ("enum S extensible\nS::Nope", "no variant `Nope` of `S`"),
        (
            "enum S extensible<{*: Int}>\nenum S::A<{x: String}>\n1",
            "the payload of `S::A` must fit `{*: Int}`",
        ),
        (
            "enum S extensible\nenum S::A\nenum S::A\n1",
            "variant `S::A` is declared twice",
        ),
        (
            "enum S extensible\nenum S::A<Int>\nS::A(1) == S::A(1)",
            "cannot compare S: its variants' payloads are not bounded, so they may hold functions",
        ),
        (
            "let k: Variant<Int, Int> = 1\nk",
            "`Variant`'s first type argument must be an enum, not Int",
        ),
        (
            "let k: Variant<Int> = 1\nk",
            "type `Variant` takes 2 type arguments, given 1",
        ),
        (
            "let p: Partial<Int> = 1\np",
            "`Partial` takes a record type with no rest-field, not Int",
        ),
        (
            "type P = {a: Int}\nlet p: Partial<P> = { a = 1 }\np.a",
            "field `a` of `Partial<{a: Int}>` may be absent",
        ),
        (
            "type P = {a: Int}\nlet p: Partial<P> = { a = \"x\" }\np",
            "mismatched types",
        ),
        ("type Variant = Int\n1", "`Variant` is a built-in type"),
    ] {
        assert_eq!(
            messages(text).first().map(String::as_str),
            Some(message),
            "{text}"
        );
    }
}
