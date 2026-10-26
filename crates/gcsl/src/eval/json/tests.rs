//! Parsing JSON text.

use super::parse::{Json, Number, parse};

fn number(text: &str, int: Option<i64>, float: Option<f64>) -> Json {
    Json::Number(Number {
        text: Box::from(text),
        int,
        float,
    })
}

#[test]
fn parses_every_kind_of_value() {
    let parsed = parse(r#" {"a": [1, -2.5, true, false, null], "b": "x\n\u00e9\ud83d\ude00"} "#);
    assert_eq!(
        parsed,
        Ok(Json::Object(vec![
            (
                "a".into(),
                Json::Array(vec![
                    number("1", Some(1), Some(1.0)),
                    number("-2.5", None, Some(-2.5)),
                    Json::Bool(true),
                    Json::Bool(false),
                    Json::Null,
                ])
            ),
            ("b".into(), Json::Str("x\n\u{e9}\u{1f600}".into())),
        ]))
    );
}

#[test]
fn numbers_are_ints_only_when_integral_and_in_range() {
    assert_eq!(parse("1e2"), Ok(number("1e2", None, Some(100.0))));
    assert_eq!(
        parse("9223372036854775808"),
        Ok(number(
            "9223372036854775808",
            None,
            Some(9_223_372_036_854_775_808.0)
        ))
    );
    assert_eq!(parse("1e400"), Ok(number("1e400", None, None)));
}

#[test]
fn rejects_what_rfc_8259_does_not_allow() {
    for (text, reason) in [
        ("[1,]", "expected a value, at line 1, column 4"),
        (
            "01",
            "a number cannot have a leading zero, at line 1, column 2",
        ),
        (
            "\"a\tb\"",
            "a control character must be escaped in a string, at line 1, column 3",
        ),
        (
            "\"\\ud800\"",
            "an escaped high surrogate must be followed by a low one, at line 1, column 8",
        ),
        ("\"\\x\"", "not an escape JSON has, at line 1, column 3"),
        ("1 2", "there is more after the value, at line 1, column 3"),
        (
            "",
            "the text ends where a value should be, at line 1, column 1",
        ),
        (
            "{\"a\": 1,\n \"a\": 2}",
            "the key \"a\" is in the object twice, at line 2, column 2",
        ),
        ("// x\n1", "expected a value, at line 1, column 1"),
    ] {
        assert_eq!(parse(text), Err(reason.to_owned()), "{text}");
    }
}

#[test]
fn limits_nesting() {
    let deep = "[".repeat(super::parse::MAX_DEPTH + 1);
    assert!(parse(&deep).is_err_and(|reason| reason.contains("nest more than")));
    let fine = format!(
        "{}{}",
        "[".repeat(super::parse::MAX_DEPTH),
        "]".repeat(super::parse::MAX_DEPTH)
    );
    assert!(parse(&fine).is_ok());
}

// ----- Decoding, end to end -----

/// The value `text`, a lone file, gives, shown as GCSL source.
fn decoded(text: &str) -> String {
    let source =
        crate::source::SourceFile::new("test.gcsl", format!("import json\n{text}")).unwrap();
    match crate::run(&source) {
        Ok(outcome) => outcome.value.to_string(),
        Err(errors) => panic!("{text}: {errors:?}"),
    }
}

/// The first error `text`, a lone file, fails with: its message and label.
fn failure(text: &str) -> (String, Option<String>) {
    let source =
        crate::source::SourceFile::new("test.gcsl", format!("import json\n{text}")).unwrap();
    let errors = crate::run(&source)
        .err()
        .unwrap_or_else(|| panic!("{text} ran"));
    let first = errors.into_iter().next().unwrap();
    (first.message, first.label.map(String::from))
}

#[test]
fn decodes_as_the_type_expected() {
    let text = r#"type T = {s: String, i: Int, f: Float, b: Bool, l: List<Int>, r: {x: Int}}
let t: T = json.decode("{{\"s\": \"a\", \"i\": -3, \"f\": 2, \"b\": true, \"l\": [1], \"r\": {{\"x\": 4}}, \"extra\": null}}")
= t == {s = "a"; i = -3; f = 2.0; b = true; l = [1]; r = {x = 4}}"#;
    assert_eq!(decoded(text), "true");
    // An ascription, a parameter and a result expect a type too.
    assert_eq!(decoded("= json.decode(\"[1, 2]\"): List<Int>"), "[1, 2]");
    assert_eq!(
        decoded("fn n(xs: List<Int>): Int { len(xs) }\n= n(json.decode(\"[1, 2, 3]\"))"),
        "3"
    );
    // The type reaches a block's result and an `if`'s branches.
    assert_eq!(
        decoded(
            "fn f(t: String): List<Int> { if t == \"\" { [] } else { json.decode(t) } }\n\
             = f(\"[3]\")"
        ),
        "[3]"
    );
    // It is pure: a pure function calls it.
    assert_eq!(
        decoded("fn f(t: String): {a: Int} { json.decode(t) }\n= f(\"{{\\\"a\\\": 1}}\").a"),
        "1"
    );
}

#[test]
fn a_missing_key_or_null_gives_the_default() {
    let text = r#"let t: {a: Int, b: String = "d", c: {d: List<Int>} = {d = [4]}} = json.decode("{{\"a\": 1, \"b\": null}}")
= t"#;
    assert_eq!(decoded(text), "{a = 1; b = \"d\"; c = {d = [4]}}");
    // A decoded record is equal to one written.
    assert_eq!(
        decoded("let t: {a: Int} = json.decode(\"{{\\\"a\\\": 1}}\")\n= t == {a = 1}"),
        "true"
    );
}

#[test]
fn a_union_takes_the_member_of_the_value_s_kind() {
    let text = "let t: List<Int | Float | String | List<String>> = json.decode(\"[1, 1.5, \\\"a\\\", [\\\"b\\\"]]\")\n= t";
    assert_eq!(decoded(text), "[1, 1.5, \"a\", [\"b\"]]");
}

#[test]
fn the_type_must_be_expected_and_decodable() {
    assert_eq!(
        failure("= json.decode(\"1\")").0,
        "`json.decode` needs to know the type to decode as"
    );
    assert_eq!(
        failure("let d: {a: Duration} = json.decode(\"1\")\n= d").0,
        "cannot decode JSON as a Duration"
    );
    assert_eq!(
        failure("let d: {a: Int} | {b: Int} = json.decode(\"1\")\n= d").0,
        "cannot decode JSON as a `{a: Int} | {b: Int}`"
    );
    assert_eq!(
        failure("let d: {*: Int} = json.decode(\"1\")\n= d").0,
        "cannot decode JSON as a `{*: Int}`"
    );
}

#[test]
fn a_document_that_does_not_fit_names_where() {
    let text = "let t: {a: List<{b: Int}>} = json.decode(\"{{\\\"a\\\": [{{\\\"b\\\": 1}}, {{\\\"b\\\": 1.5}}]}}\")\n= t";
    assert_eq!(
        failure(text),
        (
            "the document does not fit the type".to_owned(),
            Some("`a[1].b` is the number 1.5, not an integer".to_owned())
        )
    );
    assert_eq!(
        failure("let t: {a: Int} = json.decode(\"{{}}\")\n= t").1,
        Some("the document has no key \"a\", nor a default for it".to_owned())
    );
    assert_eq!(
        failure("let t: Int = json.decode(\"9223372036854775808\")\n= t").1,
        Some("the document is the number 9223372036854775808, not an integer".to_owned())
    );
    assert_eq!(
        failure("let t: Int = json.decode(\"[\")\n= t").0,
        "the text is not JSON: the text ends where a value should be, at line 1, column 2"
    );
}
