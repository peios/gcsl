//! End-to-end tests: source text through every stage via the public API.

use gcsl::{Diagnostic, SourceFile, Ty, Value};

fn source(text: &str) -> SourceFile {
    SourceFile::new("calc.gcsl", text).unwrap()
}

fn run(text: &str) -> Result<Value, Vec<Diagnostic>> {
    gcsl::run(&source(text))
}

/// The value and its type, as the REPL shows them.
fn show(text: &str) -> String {
    match run(text) {
        Ok(value) => format!("{value} : {}", value.ty()),
        Err(diagnostics) => panic!("{text}: unexpected errors {diagnostics:?}"),
    }
}

/// All diagnostics, rendered and joined as the CLI prints them.
fn rendered(text: &str) -> String {
    let file = source(text);
    let diagnostics = gcsl::run(&file).unwrap_err();
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.render(&file))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn values_and_types() {
    let cases = [
        ("1 + 2 * 3", "7 : Int"),
        ("(1 + 2) * 3", "9 : Int"),
        ("8 - 3 - 2", "3 : Int"),
        ("2 * 3 // 4", "1 : Int"),
        ("-2 * 3", "-6 : Int"),
        ("- 2 * 3", "-6 : Int"),
        ("--2", "2 : Int"),
        ("2 * -3", "-6 : Int"),
        ("((((4))))", "4 : Int"),
        ("1_000_000 * 3", "3000000 : Int"),
        ("7 / 2", "3.5 : Float"),
        ("7 / 2.0", "3.5 : Float"),
        ("6 / 2", "3.0 : Float"),
        ("(7 // 2) / 2", "1.5 : Float"),
        ("(1 + 2) * 3.0", "9.0 : Float"),
        ("1.5 * 2", "3.0 : Float"),
        ("-7 // 2", "-4 : Int"),
        ("-7 % 2", "1 : Int"),
        ("7 % -2", "-1 : Int"),
        ("-0.0", "-0.0 : Float"),
        ("1e300", "1e300 : Float"),
        ("1e300 * 1e300", "inf : Float"),
        ("-1.0 / 0.0", "-inf : Float"),
        ("0.0 / 0.0", "nan : Float"),
        ("1.5e-3", "0.0015 : Float"),
        ("1 / 3", "0.3333333333333333 : Float"),
    ];
    for (text, expected) in cases {
        assert_eq!(show(text), expected, "{text}");
    }
}

#[test]
fn whitespace_newlines_and_comments_are_insignificant() {
    let text = "# a calculation\n(1 +   # one\n   2)\n\t* 3 # times three\n";
    assert_eq!(show(text), "9 : Int");
}

#[test]
fn value_display_does_not_include_the_type() {
    assert_eq!(run("7 / 2").unwrap().to_string(), "3.5");
    assert_eq!(run("7 // 2").unwrap().ty(), Ty::Int);
}

#[test]
fn static_error_renders_exactly() {
    assert_eq!(
        rendered("# first line\n1 + (7.0 // 2)\n"),
        "error: cannot floor-divide Float\n\
         \x20 --> calc.gcsl:2:6\n\
         \x20  |\n\
         \x202 | 1 + (7.0 // 2)\n\
         \x20  |      ^^^^^^^^\n"
    );
}

#[test]
fn every_type_error_is_reported() {
    assert_eq!(
        rendered("(7.0 // 2) +\n(3 % 2.5)"),
        "error: cannot floor-divide Float\n\
         \x20 --> calc.gcsl:1:2\n\
         \x20  |\n\
         \x201 | (7.0 // 2) +\n\
         \x20  |  ^^^^^^^^\n\
         \n\
         error: cannot take modulo of Float\n\
         \x20 --> calc.gcsl:2:2\n\
         \x20  |\n\
         \x202 | (3 % 2.5)\n\
         \x20  |  ^^^^^^^\n"
    );
}

#[test]
fn runtime_error_on_a_two_digit_line_renders_exactly() {
    let text = format!("{}1 +\n  (9223372036854775807 + 1)\n", "#\n".repeat(9));
    assert_eq!(
        rendered(&text),
        "error: integer overflow\n\
         \x20  --> calc.gcsl:11:4\n\
         \x20   |\n\
         \x2011 |   (9223372036854775807 + 1)\n\
         \x20   |    ^^^^^^^^^^^^^^^^^^^^^^^\n"
    );
}

#[test]
fn multi_line_operator_span_is_underlined_to_end_of_line() {
    assert_eq!(
        rendered("1 //\n0"),
        "error: division by zero\n\
         \x20 --> calc.gcsl:1:1\n\
         \x20  |\n\
         \x201 | 1 //\n\
         \x20  | ^^^^\n"
    );
}

#[test]
fn lex_and_parse_errors_render_through_run() {
    assert_eq!(
        rendered("1 + 2_"),
        "error: trailing `_` in number literal\n\
         \x20 --> calc.gcsl:1:6\n\
         \x20  |\n\
         \x201 | 1 + 2_\n\
         \x20  |      ^\n"
    );
    assert_eq!(
        rendered("(1 + 2\n"),
        "error: expected `)`\n\
         \x20 --> calc.gcsl:1:7\n\
         \x20  |\n\
         \x201 | (1 + 2\n\
         \x20  |       ^\n"
    );
    assert_eq!(
        rendered("99999999999999999999"),
        "error: integer literal out of range for Int\n\
         \x20 --> calc.gcsl:1:1\n\
         \x20  |\n\
         \x201 | 99999999999999999999\n\
         \x20  | ^^^^^^^^^^^^^^^^^^^^\n\
         \x20  |\n\
         \x20  = note: the largest Int literal is 9223372036854775807\n"
    );
}

#[test]
fn left_operand_error_wins() {
    let diagnostics = run("(1 // 0) + (9223372036854775807 + 1)").unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "division by zero");
}

/// Runs `f` on a thread with the stack size the library asks for.
fn on_recommended_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Every stage, including the dump printers, survives the deepest tree the
/// parser accepts, on the stack size the library recommends. This is the
/// test that keeps `STACK_SIZE` honest: it runs unoptimised under
/// `cargo test`, where stack frames are largest.
#[test]
fn every_stage_handles_maximum_depth() {
    on_recommended_stack(|| {
        let depth = gcsl::MAX_DEPTH;
        // Left-nested chain: N terms is N levels.
        let chain = source(&format!("1{}", " + 1".repeat(depth - 1)));
        let chain_value = Value::Int(i64::try_from(depth).unwrap());
        // Nested parentheses and negations: recursion in the parser too.
        let pairs = (depth - 1) / 2;
        let nested = source(&format!("{}1{}", "-(".repeat(pairs), ")".repeat(pairs)));
        let nested_value = Value::Int(if pairs.is_multiple_of(2) { 1 } else { -1 });
        for (file, expected) in [(&chain, chain_value), (&nested, nested_value)] {
            let tokens = gcsl::lexer::tokenize(file).unwrap();
            let ast = gcsl::parser::parse(&tokens, file).unwrap();
            let types = gcsl::check::check(&ast).unwrap();
            let ir = gcsl::lower::lower(&ast, &types).unwrap();
            assert_eq!(gcsl::eval::eval(&ir).unwrap(), expected);
            // The printers recurse too.
            assert!(!ast.pretty().to_string().is_empty());
            assert!(!ast.pretty_with(&types).to_string().is_empty());
            assert!(!ir.pretty().to_string().is_empty());
        }
    });
}

#[test]
fn nesting_beyond_the_limit_is_a_diagnostic_not_a_crash() {
    on_recommended_stack(|| {
        // Far beyond the limit, where a missing check would overflow.
        let text = format!("{}1{}", "(".repeat(200_000), ")".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        let text = format!("1{}", " + 1".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
    });
}

#[test]
fn stages_can_be_run_individually() {
    let file = source("1 + 2.5");
    let tokens = gcsl::lexer::tokenize(&file).unwrap();
    assert_eq!(tokens.len(), 4);
    let ast = gcsl::parser::parse(&tokens, &file).unwrap();
    let types = gcsl::check::check(&ast).unwrap();
    assert_eq!(types.get(ast.root()), Ty::Float);
    let ir = gcsl::lower::lower(&ast, &types).unwrap();
    assert_eq!(gcsl::eval::eval(&ir).unwrap(), Value::Float(3.5));
}
