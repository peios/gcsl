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
        ("-(-2)", "2 : Int"),
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
fn newlines_in_parentheses_and_after_operators_are_insignificant() {
    let text = "# a calculation\n(1 +   # one\n   2\n) *\n\t3 # times three\n";
    assert_eq!(show(text), "9 : Int");
}

#[test]
fn programs_with_items_and_blocks() {
    let cases = [
        ("", "() : Unit"),
        ("let x = 1", "() : Unit"),
        ("let x = 1\nx", "1 : Int"),
        ("let x = 1; let y = x + 1; = x * y", "2 : Int"),
        (
            "# totals\nlet price = 250\nlet count = 3\n\nprice * count\n",
            "750 : Int",
        ),
        ("let x = { let y = 1; y + 1 }\nx", "2 : Int"),
        ("let x = 1\nlet x = x * 2\nx", "2 : Int"),
        ("let x = 1.0\nlet x = x + 0.5\nx", "1.5 : Float"),
        ("let x = 1\nx := x + 1\nx := x * 10\nx", "20 : Int"),
        ("let rate: Float = 3\nrate / 2", "1.5 : Float"),
        ("let n = 7.0\nn * 2 : Float", "14.0 : Float"),
        ("let n = 7\n(n // 2 : Int) / 2", "1.5 : Float"),
        ("let hello-world = 2\nhello-world - 1", "1 : Int"),
        ("let x-1 = 5\nlet x = 1\nx-1 - x", "4 : Int"),
        ("{ { {} } }", "() : Unit"),
        (
            "let total = {\n  let a = 1\n  let b = 2\n  a + b\n}\n= total",
            "3 : Int",
        ),
        ("{ 1 } : Float", "1.0 : Float"),
    ];
    for (text, expected) in cases {
        assert_eq!(show(text), expected, "{text}");
    }
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
         \x20--> calc.gcsl:2:10\n\
         \x20 |\n\
         2 | 1 + (7.0 // 2)\n\
         \x20 |      --- ^^ - Int\n\
         \x20 |      |\n\
         \x20 |      Float\n"
    );
}

#[test]
fn every_type_error_is_reported() {
    assert_eq!(
        rendered("(7.0 // 2) +\n(3 % 2.5)"),
        "error: cannot floor-divide Float\n\
         \x20--> calc.gcsl:1:6\n\
         \x20 |\n\
         1 | (7.0 // 2) +\n\
         \x20 |  --- ^^ - Int\n\
         \x20 |  |\n\
         \x20 |  Float\n\
         \n\
         error: cannot take modulo of Float\n\
         \x20--> calc.gcsl:2:4\n\
         \x20 |\n\
         2 | (3 % 2.5)\n\
         \x20 |  - ^ --- Float\n\
         \x20 |  |\n\
         \x20 |  Int\n"
    );
}

#[test]
fn runtime_error_on_a_two_digit_line_renders_exactly() {
    let text = format!("{}1 +\n  (9223372036854775807 + 1)\n", "#\n".repeat(9));
    assert_eq!(
        rendered(&text),
        "error: integer overflow\n\
         \x20 --> calc.gcsl:11:24\n\
         \x20  |\n\
         11 |   (9223372036854775807 + 1)\n\
         \x20  |    ------------------- ^ - 1\n\
         \x20  |    |\n\
         \x20  |    9223372036854775807\n"
    );
}

#[test]
fn operands_on_different_lines_are_each_labelled() {
    assert_eq!(
        rendered("1 //\n0"),
        "error: division by zero\n\
         \x20--> calc.gcsl:1:3\n\
         \x20 |\n\
         1 | 1 //\n\
         \x20 | - ^^\n\
         \x20 | |\n\
         \x20 | 1\n\
         2 | 0\n\
         \x20 | - 0\n"
    );
}

#[test]
fn lex_and_parse_errors_render_through_run() {
    assert_eq!(
        rendered("1 + 2_"),
        "error: trailing `_` in number literal\n\
         \x20--> calc.gcsl:1:6\n\
         \x20 |\n\
         1 | 1 + 2_\n\
         \x20 |      ^\n"
    );
    assert_eq!(
        rendered("(1 + 2\n"),
        "error: expected `)`, found end of input\n\
         \x20--> calc.gcsl:1:7\n\
         \x20 |\n\
         1 | (1 + 2\n\
         \x20 | -     ^\n\
         \x20 | |\n\
         \x20 | unclosed `(`\n"
    );
    assert_eq!(
        rendered("99999999999999999999"),
        "error: integer literal out of range for Int\n\
         \x20--> calc.gcsl:1:1\n\
         \x20 |\n\
         1 | 99999999999999999999\n\
         \x20 | ^^^^^^^^^^^^^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: the largest Int literal is 9223372036854775807\n"
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
        // Nested blocks: two levels of parser recursion each.
        let blocks = (depth - 1) / 2;
        let nested_blocks = source(&format!("{}1{}", "{".repeat(blocks), "}".repeat(blocks)));
        // Blocks with a declaration each, around a chain using it: as tall as
        // the limit allows, and nesting both ways at once, so the later
        // stages recurse through blocks, items and operators together.
        let outer = 4_000;
        let terms = depth - outer;
        let mixed = source(&format!(
            "{}1{}{}",
            "{ let v = 1\n".repeat(outer),
            " + v".repeat(terms - 1),
            "\n}".repeat(outer)
        ));
        let mixed_value = Value::Int(i64::try_from(terms).unwrap());
        for (file, expected) in [
            (&chain, chain_value),
            (&nested, nested_value),
            (&nested_blocks, Value::Int(1)),
            (&mixed, mixed_value),
        ] {
            let (tokens, names) = gcsl::lexer::tokenize(file).unwrap();
            let ast = gcsl::parser::parse(&tokens, names, file).unwrap();
            let checked = gcsl::check::check(&ast).unwrap();
            let ir = gcsl::lower::lower(&ast, &checked).unwrap();
            assert_eq!(gcsl::eval::eval(&ir).unwrap(), expected);
            // The printers recurse too.
            assert!(!ast.pretty().to_string().is_empty());
            assert!(!ast.pretty_with(&checked.types).to_string().is_empty());
            assert!(!ir.pretty(ast.names()).to_string().is_empty());
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
        let text = format!("{}1{}", "{".repeat(200_000), "}".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
    });
}

#[test]
fn stages_can_be_run_individually() {
    let file = source("let x = 1 : Float\nx + 2.5");
    let (tokens, names) = gcsl::lexer::tokenize(&file).unwrap();
    assert_eq!(tokens.len(), 11);
    let ast = gcsl::parser::parse(&tokens, names, &file).unwrap();
    let checked = gcsl::check::check(&ast).unwrap();
    assert_eq!(checked.resolution.frame_size(), 1);
    let ir = gcsl::lower::lower(&ast, &checked).unwrap();
    assert_eq!(ir.ty_of(ir.root()), Ty::Float);
    assert_eq!(gcsl::eval::eval(&ir).unwrap(), Value::Float(3.5));
}

#[test]
fn stages_report_every_new_kind_of_error() {
    let message = |text: &str| run(text).unwrap_err()[0].message.clone();
    assert_eq!(message("1-2"), "`-` here is ambiguous");
    assert_eq!(message("{ 1"), "expected `}`, found end of input");
    assert_eq!(message("1 + 2\n3"), "unused value of type Int");
    assert_eq!(message("x"), "undefined variable `x`");
}

#[test]
fn rebind_in_an_enclosing_block_renders_exactly() {
    assert_eq!(
        rendered("let total = 0\n{\n  total := 1\n}\n"),
        "error: cannot rebind `total` here: it is declared in an enclosing block\n\
         \x20--> calc.gcsl:3:3\n\
         \x20 |\n\
         1 | let total = 0\n\
         \x20 | ------------- declared here\n\
         2 | {\n\
         3 |   total := 1\n\
         \x20 |   ^^^^^\n\
         \x20 |\n\
         \x20 = note: shadow it with `let total = ...` instead\n"
    );
}

#[test]
fn annotation_mismatch_renders_exactly() {
    assert_eq!(
        rendered("let x: Int = 5.0\n"),
        "error: mismatched types\n\
         \x20--> calc.gcsl:1:14\n\
         \x20 |\n\
         1 | let x: Int = 5.0\n\
         \x20 |        ---   ^^^ expected Int, found Float\n\
         \x20 |        |\n\
         \x20 |        expected because of this annotation\n"
    );
}
