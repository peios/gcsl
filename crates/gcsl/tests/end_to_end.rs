//! End-to-end tests: source text through every stage via the public API.

use std::rc::Rc;

use gcsl::{Diagnostic, Session, Severity, SourceFile, TypeTable, Value, ValueKind};

fn source(text: &str) -> SourceFile {
    SourceFile::new("calc.gcsl", text).unwrap()
}

/// Runs `text`, which must not warn, and returns its value.
fn run(text: &str) -> Result<Value, Vec<Diagnostic>> {
    gcsl::run(&source(text)).map(|outcome| {
        assert!(
            outcome.warnings.is_empty(),
            "{text}: {:?}",
            outcome.warnings
        );
        outcome.value
    })
}

/// The value and its type, as the REPL shows them.
fn show(text: &str) -> String {
    let mut session = Session::new();
    match session.eval(&source(text)) {
        Ok(outcome) => format!(
            "{} : {}",
            outcome.value.repr(),
            session
                .types()
                .display(session.value_type(), session.names())
        ),
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
        ("", "{} : {}"),
        ("let x = 1", "{} : {}"),
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
        ("{ { {} } }", "{} : {}"),
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
fn programs_with_strings_bools_and_ifs() {
    let cases = [
        ("true", "true : Bool"),
        ("1 < 2 and not (2.5 == 2.5)", "false : Bool"),
        ("\"hi\"", "\"hi\" : String"),
        ("\"a\\tb\\n\"", "\"a\\tb\\n\" : String"),
        (
            "\"\"\"say \"hi\" now\"\"\"",
            "\"say \\\"hi\\\" now\" : String",
        ),
        (
            "let n = 3\n\"{n} / 2 = {n / 2}, {n} * {n} = {n * n}\"",
            "\"3 / 2 = 1.5, 3 * 3 = 9\" : String",
        ),
        (
            "let name = \"GCSL\"\nlet greeting = \"hello, \" + name\n\
             = if greeting == \"hello, GCSL\" { \"yes\" } else { \"no\" }",
            "\"yes\" : String",
        ),
        (
            "let n = 15\n\
             = if n % 15 == 0 { \"FizzBuzz\" }\n\
             else if n % 3 == 0 { \"Fizz\" }\n\
             else if n % 5 == 0 { \"Buzz\" }\n\
             else { \"{n}\" }",
            "\"FizzBuzz\" : String",
        ),
        (
            "let script = \"\n    set -e\n    echo {1 + 1}\n  \"\nscript",
            "\"set -e\\necho 2\" : String",
        ),
        ("if 1 > 2 { 1 } else { 2.5 }", "2.5 : Float"),
        ("let x = 1\nif x > 0 { let y = x }", "{} : {}"),
    ];
    for (text, expected) in cases {
        assert_eq!(show(text), expected, "{text}");
    }
}

#[test]
fn programs_with_records() {
    let cases = [
        (
            "{ x = 1; y = \"a\" }",
            "{x = 1; y = \"a\"} : {x: Int, y: String}",
        ),
        (
            "x = 1\ny = \"a\"",
            "{x = 1; y = \"a\"} : {x: Int, y: String}",
        ),
        (
            "let origin = { x = 0.0; y = 0.0 }\n\
             let p = { x = 1.5; y = origin.y + 2 }\n\
             = p.x * p.y",
            "3.0 : Float",
        ),
        (
            "let config = {\n  name = \"gcsl\"\n  build = { jobs = 4; \"dry run\" = false }\n}\n\
             config.build",
            "{jobs = 4; \"dry run\" = false} : {\"dry run\": Bool, jobs: Int}",
        ),
        (
            "let p: {x: Float, y: Float} = { x = 1; y = 2 }\np",
            "{x = 1.0; y = 2.0} : {x: Float, y: Float}",
        ),
        (
            "let q = { x = 1; y = 2 }\nlet p: {x: Int} = q\np",
            "{x = 1; y = 2} : {x: Int}",
        ),
        (
            "if 1 < 2 { { x = 1; y = 2 } } else { { x = 3 } }",
            "{x = 1; y = 2} : {x: Int}",
        ),
        ("{ x = 1 } == { x = 1 }", "true : Bool"),
        (
            "let r = { n = 3 }\n\"r is {r}\"",
            "\"r is {{n = 3}}\" : String",
        ),
        ("{ e = {} }.e", "{} : {}"),
    ];
    for (text, expected) in cases {
        assert_eq!(show(text), expected, "{text}");
    }
}

#[test]
fn programs_with_dotted_paths_and_equality() {
    let cases = [
        (
            "server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true",
            "{server = {host = \"localhost\"; port = 8080; tls = {enabled = true}}} : \
             {server: {host: String, port: Int, tls: {enabled: Bool}}}",
        ),
        (
            "let config = { name = \"gcsl\"; build = { jobs = 4 } }\n\
             let config.build.jobs = 8\n\
             let config.build.\"dry run\" = true\n\
             config.build.\"dry run\" := false\n\
             config",
            "{name = \"gcsl\"; build = {jobs = 8; \"dry run\" = false}} : \
             {build: {\"dry run\": Bool, jobs: Int}, name: String}",
        ),
        ("let a.b: Float = 1\na", "{b = 1.0} : {b: Float}"),
        (
            "let q: {x: Int} = { x = 1; y = 2 }\n\"{q == { x = 1 }} {q === { x = 1 }}\"",
            "\"true false\" : String",
        ),
        (
            "let p: {a: {b: Float}} = { a.b = 1 }\np",
            "{a = {b = 1.0}} : {a: {b: Float}}",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(show(text), expected, "{text}");
    }
}

#[test]
fn a_merge_error_renders_exactly() {
    assert_eq!(
        rendered("server.port = 8080\nserver.port.number = 1\n"),
        "error: cannot define `server.port.number`: `server.port` is Int, not a record\n\
         \x20--> calc.gcsl:2:1\n\
         \x20 |\n\
         1 | server.port = 8080\n\
         \x20 | ----------- `server.port` defined here\n\
         2 | server.port.number = 1\n\
         \x20 | ^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(
        rendered("server = { port = 1 }\nserver.host = \"h\"\n"),
        "error: field `server` is already defined\n\
         \x20--> calc.gcsl:2:1\n\
         \x20 |\n\
         1 | server = { port = 1 }\n\
         \x20 | ------ defined whole here\n\
         2 | server.host = \"h\"\n\
         \x20 | ^^^^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: a field defined as a whole cannot be extended with dotted paths\n"
    );
}

#[test]
fn an_update_error_renders_exactly() {
    assert_eq!(
        rendered("let limits = { cpu = 2 }\nlet limits.cpu.max = 4\n"),
        "error: cannot update `limits.cpu.max`: `limits.cpu` is Int, not a record\n\
         \x20--> calc.gcsl:2:5\n\
         \x20 |\n\
         2 | let limits.cpu.max = 4\n\
         \x20 |     ------^^^^^^^^\n\
         \x20 |     |\n\
         \x20 |     this is {cpu: Int}\n"
    );
}

#[test]
fn value_display_does_not_include_the_type() {
    assert_eq!(run("7 / 2").unwrap().to_string(), "3.5");
    assert_eq!(run("7 // 2").unwrap().kind(), ValueKind::Int);
    // A record displays as it would be written, strings quoted.
    assert_eq!(
        run("{ x = 1; y = \"a\" }").unwrap().to_string(),
        "{x = 1; y = \"a\"}"
    );
    // A string displays as its text, and shows as a literal only in `repr`.
    let value = run("\"a\\\"b\\n\"").unwrap();
    assert_eq!(value.to_string(), "a\"b\n");
    assert_eq!(value.repr().to_string(), "\"a\\\"b\\n\"");
}

#[test]
fn if_branch_mismatch_renders_exactly() {
    assert_eq!(
        rendered("let x = if true { 1 } else { \"no\" }\n"),
        "error: `if` branches have different types\n\
         \x20--> calc.gcsl:1:9\n\
         \x20 |\n\
         1 | let x = if true { 1 } else { \"no\" }\n\
         \x20 |         ^^      -----      -------- this is String\n\
         \x20 |                 |\n\
         \x20 |                 this is Int\n"
    );
}

#[test]
fn unterminated_triple_quoted_string_renders_exactly() {
    assert_eq!(
        rendered("let s = \"\"\"\n  She said \"\"hi\"\"\n"),
        "error: unterminated string literal\n\
         \x20--> calc.gcsl:1:9\n\
         \x20 |\n\
         1 | let s = \"\"\"\n\
         \x20 |         ^^^\n\
         \x20 |\n\
         \x20 = note: this string closes with 3 quotes\n"
    );
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
        // String literals nested in each other's interpolations, each one
        // level above the last: the lexer recurses through them too.
        let literals = depth - 1;
        let nested_strings = source(&format!(
            "{}1{}",
            "\"{".repeat(literals),
            "}\"".repeat(literals)
        ));
        // An `else if` chain whose every condition is false, so that
        // evaluation walks all of it. The innermost `if` is one above its
        // blocks, which are one above their literals, so N `if`s are N + 2
        // levels.
        let ifs = depth - 2;
        let else_ifs = source(&format!("{}{{ 7 }}", "if false { 0 } else ".repeat(ifs)));
        // Left-nested chains of the other types, each evaluated by its own
        // walker.
        let floats = source(&format!("0.5{}", " + 0.5".repeat(depth - 1)));
        let floats_value = Value::Float(0.5 * f64::from(u32::try_from(depth).unwrap()));
        let bools = source(&format!("true{}", " and true".repeat(depth - 1)));
        let concatenation = source(&format!("\"a\"{}", " + \"a\"".repeat(depth - 1)));
        let concatenation_value = Value::Str("a".repeat(depth).into());
        // The longest dotted path, two levels a segment: built from nothing,
        // then updated all the way down, which reads into each record
        // before it, then read back.
        let path = format!("a{}", ".b".repeat((depth - 1) / 2));
        let dotted = source(&format!("let {path} = 1\nlet {path} = 2\n{path}"));
        // Calls nested in each other's arguments: N calls around a literal
        // are N + 1 levels, and each is made from its caller's frame, so
        // they nest in Rust but not in GCSL.
        let calls = source(&format!(
            "fn id(x: Int): Int {{ x }}\n{}1{}",
            "id(".repeat(depth - 1),
            ")".repeat(depth - 1)
        ));
        // Function expressions nested in each other's bodies, two levels
        // each, the innermost using a variable of the top level, which every
        // one of them therefore captures.
        let functions_deep = (depth - 1) / 2;
        let functions = source(&format!(
            "let v = 7\nlet f = {}fn(): Int {{ v }}{}\nf()",
            "fn(): Int { let g = ".repeat(functions_deep - 1),
            "\n1 }".repeat(functions_deep - 1)
        ));
        // The printers recurse too, but printing a tree this deep takes
        // seconds, since each line is indented by its depth. They recurse
        // one frame per level from a node straight to its children,
        // whatever the node, except through a block's items, so the trees
        // printed here already cover every way they recurse; printing the
        // others would only add time.
        for (file, expected, print) in [
            (&chain, chain_value, true),
            (&nested, nested_value, true),
            (&nested_blocks, Value::Int(1), true),
            (&mixed, mixed_value, true),
            (&nested_strings, Value::Str("1".into()), false),
            (&else_ifs, Value::Int(7), false),
            (&floats, floats_value, false),
            (&bools, Value::Bool(true), false),
            (&concatenation, concatenation_value, false),
            (&dotted, Value::Int(2), false),
            // Printing a call recurses through its arguments; a function's
            // body prints as the blocks above do.
            (&calls, Value::Int(1), true),
            (&functions, Value::Int(1), false),
        ] {
            let (tokens, names, strings) = gcsl::lexer::tokenize(file).unwrap();
            let ast = gcsl::parser::parse(&tokens, names, strings, file).unwrap();
            let mut types = TypeTable::new();
            let (checked, _) = gcsl::check::check(&ast, &mut types).unwrap();
            let ir = Rc::new(gcsl::lower::lower(&ast, &checked, &types, 0).unwrap());
            assert_eq!(gcsl::eval::eval(&ir).unwrap(), expected);
            if print {
                let annotation = checked.types.annotation(&types, ast.names());
                assert!(!ast.pretty().to_string().is_empty());
                assert!(!ast.pretty_with(&annotation).to_string().is_empty());
                assert!(!ir.pretty(ast.names()).to_string().is_empty());
            }
        }
    });
}

/// Records nest as deeply as the parser allows, through every stage: as
/// record blocks, as chains of field accesses on them, and as record types.
#[test]
fn every_stage_handles_maximum_record_depth() {
    on_recommended_stack(|| {
        // Each nested record block is two levels of parser recursion (its
        // body and its field's value), but one level of height, and each
        // field access one more level of height.
        let levels = (gcsl::MAX_DEPTH - 1) / 2;
        let blocks = format!("{}1{}", "{ a = ".repeat(levels), " }".repeat(levels));
        let chain = source(&format!("{blocks}{}", ".a".repeat(levels)));
        // The same blocks checked against a record type as deep, which the
        // checker resolves and checks each block against, level by level.
        let ty = format!("{}Int{}", "{a: ".repeat(levels), "}".repeat(levels));
        let annotated = source(&format!("let r: {ty} = {blocks}\nr == r"));
        for (file, expected) in [(&chain, Value::Int(1)), (&annotated, Value::Bool(true))] {
            let (tokens, names, strings) = gcsl::lexer::tokenize(file).unwrap();
            let ast = gcsl::parser::parse(&tokens, names, strings, file).unwrap();
            let mut types = TypeTable::new();
            let output = gcsl::check::Checker::new(&ast, &mut types).check().unwrap();
            let ir = Rc::new(gcsl::lower::lower(&ast, &output.checked, &types, 0).unwrap());
            assert_eq!(gcsl::eval::eval(&ir).unwrap(), expected);
            assert!(!ast.pretty().to_string().is_empty());
            assert!(!ir.pretty(ast.names()).to_string().is_empty());
        }
        // Showing the deepest record, and its type. (Annotating every node
        // of these trees with its type would show types of every depth, in
        // time quadratic in the depth, so only the deepest is shown.)
        let deepest = source(&blocks);
        let mut session = Session::new();
        let value = session.eval(&deepest).unwrap().value;
        let shown = value.to_string();
        assert!(shown.starts_with("{a = {a = ") && shown.ends_with("}}"));
        let ty = session
            .types()
            .display(session.value_type(), session.names())
            .to_string();
        assert!(ty.starts_with("{a: {a: ") && ty.ends_with("}}"));
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
        // String literals nested in each other's interpolations: the
        // lexer's limit.
        let text = format!("{}1{}", "\"{".repeat(200_000), "}\"".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        let text = format!("{}{{}}", "if true {} else ".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        // Record types and chains of field accesses.
        let text = format!("1 : {}Int{}", "{a: ".repeat(200_000), "}".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        let text = format!("x{}", ".a".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        // Dotted paths.
        let text = format!("let x{} = 1", ".a".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        let text = format!("x{} = 1", ".a".repeat(200_000));
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics[0].message, "expression nested too deeply");
        // Calls, function expressions and function types.
        for text in [
            format!("{}1{}", "f(".repeat(200_000), ")".repeat(200_000)),
            format!("{}1{}", "f { ".repeat(200_000), " }".repeat(200_000)),
            format!(
                "{}1{}",
                "fn(): Int { ".repeat(200_000),
                " }".repeat(200_000)
            ),
            format!("let f: {}Int = g", "fn(): ".repeat(200_000)),
            format!(
                "let f: fn({}Int{}): Int = g",
                "fn(".repeat(200_000),
                "): Int".repeat(200_000)
            ),
        ] {
            let diagnostics = run(&text).unwrap_err();
            assert_eq!(diagnostics[0].message, "expression nested too deeply");
        }
    });
}

/// `STACK_SIZE` covers `MAX_CALL_DEPTH` calls in progress with an
/// expression as deep as the parser accepts evaluated in the deepest of
/// them, whatever kind of nesting it is, unoptimised.
#[test]
fn the_deepest_calls_with_the_deepest_expression() {
    on_recommended_stack(|| {
        // The function around the expression takes four levels: itself,
        // its body, the `if` and its block.
        let levels = gcsl::MAX_DEPTH - 4;
        let calls = gcsl::MAX_CALL_DEPTH - 1;
        let recurse = |ret: &str, deepest: &str| {
            source(&format!(
                "fn f(n: Int): {ret} {{ if n == 0 {{ {deepest} }} else {{ f(n - 1) }} }}\n\
                 f({calls})"
            ))
        };
        let terms = i64::try_from(levels).unwrap();
        let chain = recurse("Int", &format!("1{}", " + 1".repeat(levels - 1)));
        let pairs = (levels - 1) / 2;
        let negations = recurse(
            "Int",
            &format!("{}1{}", "-(".repeat(pairs), ")".repeat(pairs)),
        );
        let literals = levels - 1;
        let strings = recurse(
            "String",
            &format!("{}n{}", "\"{".repeat(literals), "}\"".repeat(literals)),
        );
        let blocks = (levels - 1) / 2;
        let nested_blocks = recurse(
            "Int",
            &format!("{}n{}", "{".repeat(blocks), "}".repeat(blocks)),
        );
        let outer = 2_000;
        let mixed = recurse(
            "Int",
            &format!(
                "{}n{}{}",
                "{ let v = 1\n".repeat(outer),
                " + v".repeat(levels - outer - 1),
                "\n}".repeat(outer)
            ),
        );
        for (file, expected) in [
            (&chain, Value::Int(terms)),
            (
                &negations,
                Value::Int(if pairs.is_multiple_of(2) { 1 } else { -1 }),
            ),
            (&strings, Value::Str("0".into())),
            (&nested_blocks, Value::Int(0)),
            (
                &mixed,
                Value::Int(i64::try_from(levels - outer - 1).unwrap()),
            ),
        ] {
            assert_eq!(run(file.text()).unwrap(), expected);
        }
    });
}

#[test]
fn programs_with_functions() {
    let cases = [
        (
            "fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\nfact(10)",
            "3628800 : Int",
        ),
        (
            "fn apply(f: fn(Int): Int, v: Int): Int { f(v) }\n\
             apply(fn(n: Int): Int {\n\
             \x20 let doubled = n * 2\n\
             \x20 doubled + 1\n\
             }, 20)",
            "41 : Int",
        ),
        (
            "fn greet(name: String, times: Int): String { \"hello {name} x{times}\" }\n\
             greet(times = 2, name = \"jack\")",
            "\"hello jack x2\" : String",
        ),
        (
            "fn service(name: String, config: {port: Int, image: String}): {name: String, url: String} {\n\
             \x20 name = name\n\
             \x20 url = \"{config.image}:{config.port}\"\n\
             }\n\
             service(\"web\") { port = 80; image = \"nginx\" }",
            "{name = \"web\"; url = \"nginx:80\"} : {name: String, url: String}",
        ),
        (
            "let f = fn(x: Int): Int { x }\nf",
            "fn(x: Int): Int : fn(x: Int): Int",
        ),
        (
            "fn run(body: {steps: Int}) -> Int { body.steps }\nrun { steps = 3 }",
            "3 : Int",
        ),
        ("let x = 1\nlet f = fn(): Int { x }\nx := 2\nf()", "1 : Int"),
        (
            "let math = { square = fn(n: Int): Int { n * n } }\nmath.square(4)",
            "16 : Int",
        ),
        (
            "fn compose(f: fn(Int): Int, g: fn(Int): Int): fn(Int): Int { fn(x: Int): Int { g(f(x)) } }\n\
             compose(fn(x: Int): Int { x + 1 }, fn(x: Int): Int { x * 10 })(4)",
            "50 : Int",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(show(text), expected, "{text}");
    }
}

#[test]
fn an_impure_call_error_renders_exactly() {
    assert_eq!(
        rendered("fn f() -> Int { 1 }\nfn g(): Int { f() }\n"),
        "error: cannot call an impure function from a pure function\n\
         \x20--> calc.gcsl:2:15\n\
         \x20 |\n\
         2 | fn g(): Int { f() }\n\
         \x20 |       -       -^^\n\
         \x20 |       |       |\n\
         \x20 |       |       this is fn() -> Int\n\
         \x20 |       this function is pure, declared with `:`\n\
         \x20 |\n\
         \x20 = note: a pure function, declared with `:`, may call only pure functions; declare it \
         with `->` to make it impure\n"
    );
}

#[test]
fn a_call_error_renders_exactly() {
    assert_eq!(
        rendered("fn add(a: Int, b: Int): Int { a + b }\nadd(1, 2, 3)\n"),
        "error: expected 2 arguments, found 3\n\
         \x20--> calc.gcsl:2:1\n\
         \x20 |\n\
         2 | add(1, 2, 3)\n\
         \x20 | ---^^^^^^^-^\n\
         \x20 | |         |\n\
         \x20 | |         unexpected argument\n\
         \x20 | the function is fn(a: Int, b: Int): Int\n"
    );
}

#[test]
fn stages_can_be_run_individually() {
    let file = source("let x = 1 : Float\nx + 2.5");
    let (tokens, names, strings) = gcsl::lexer::tokenize(&file).unwrap();
    assert_eq!(tokens.len(), 11);
    let ast = gcsl::parser::parse(&tokens, names, strings, &file).unwrap();
    let mut types = TypeTable::new();
    let (checked, warnings) = gcsl::check::check(&ast, &mut types).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(checked.resolution.frame_size(), 1);
    let ir = Rc::new(gcsl::lower::lower(&ast, &checked, &types, 0).unwrap());
    assert_eq!(ir.kind_of(ir.root()), ValueKind::Float);
    assert_eq!(gcsl::eval::eval(&ir).unwrap(), Value::Float(3.5));
}

#[test]
fn stages_report_every_new_kind_of_error() {
    let message = |text: &str| run(text).unwrap_err()[0].message.clone();
    assert_eq!(message("1-2"), "`-` here is ambiguous");
    assert_eq!(message("{ 1"), "expected `}`, found end of input");
    assert_eq!(message("1 + 2\n3"), "unused value of type Int");
    assert_eq!(message("x"), "undefined variable `x`");
    assert_eq!(message("\"abc"), "unterminated string literal");
    assert_eq!(message("\"\\q\""), "unknown escape `\\q`");
    assert_eq!(message("\"a } b\""), "unmatched `}` in string");
    assert_eq!(message("\"{x"), "unterminated interpolation");
    assert_eq!(message("!x"), "unexpected character `!`");
    assert_eq!(
        message("1 < 2 < 3"),
        "comparison operators cannot be chained"
    );
    assert_eq!(message("if true 1"), "expected `{` after `if` condition");
    assert_eq!(message("1 == \"a\""), "cannot compare Int and String");
    assert_eq!(message("true < false"), "cannot order Bool");
    assert_eq!(message("if 1 { 2 } else { 3 }"), "mismatched types");
    assert_eq!(message("if true { 1 }"), "`if` without `else` must be `{}`");
    assert_eq!(message("let x: Unit = {}"), "unknown type `Unit`");
    assert_eq!(message("x.1"), "expected a field name after `.`");
    assert_eq!(
        message("r\"x\" = 1"),
        "a field name must be a plain string literal"
    );
    assert_eq!(message("let \"x\" = 1"), "variables need plain names");
    assert_eq!(message("1 : {x Int}"), "expected `:` after field name");
    assert_eq!(
        message("{ x = 1; = 2 }"),
        "a block with fields cannot have a result"
    );
    assert_eq!(message("{ x = 1; x = 2 }"), "field `x` is defined twice");
    assert_eq!(
        message("1 : {x: Int, x: Int}"),
        "field `x` appears twice in this type"
    );
    assert_eq!(message("1.x"), "cannot access field `x` of Int");
    assert_eq!(message("{}.x"), "no field `x` in `{}`");
    assert_eq!(
        message("{ x = 1 } == { y = 1 }"),
        "cannot compare `{x: Int}` and `{y: Int}`"
    );
    assert_eq!(message("{ x = 1 } < { x = 1 }"), "cannot order `{x: Int}`");
    assert_eq!(message("a.b + 1 = 2"), "expected end of item, found `=`");
    assert_eq!(message("\"a\".b := 1"), "variables need plain names");
    assert_eq!(
        message("{ a = 1; a.b = 2 }"),
        "field `a` is already defined"
    );
    assert_eq!(
        message("{ a.b = 1; a.b.c = 2 }"),
        "cannot define `a.b.c`: `a.b` is Int, not a record"
    );
    assert_eq!(
        message("{ a.b = 1; a.b = 2 }"),
        "field `a.b` is defined twice"
    );
    assert_eq!(
        message("let a = 1\nlet a.b = 2"),
        "cannot update `a.b`: `a` is Int, not a record"
    );
    assert_eq!(
        message("{ x = 1 } === { y = 1 }"),
        "cannot compare `{x: Int}` and `{y: Int}`"
    );
}

/// Runs `text`, which must succeed, and renders its warnings as the CLI
/// prints them.
fn rendered_warnings(text: &str) -> String {
    let file = source(text);
    let outcome = gcsl::run(&file).unwrap();
    outcome
        .warnings
        .iter()
        .map(|diagnostic| diagnostic.render(&file))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn an_excess_field_warning_renders_exactly() {
    let text = "let point: {x: Int} = { x = 1; z = 2 }\npoint";
    let outcome = gcsl::run(&source(text)).unwrap();
    assert_eq!(outcome.value.to_string(), "{x = 1; z = 2}");
    assert_eq!(outcome.warnings.len(), 1);
    assert_eq!(outcome.warnings[0].severity, Severity::Warning);
    assert_eq!(
        rendered_warnings(text),
        "warning: field `z` is not in `{x: Int}`\n\
         \x20--> calc.gcsl:1:32\n\
         \x20 |\n\
         1 | let point: {x: Int} = { x = 1; z = 2 }\n\
         \x20 |            --------            ^\n\
         \x20 |            |\n\
         \x20 |            expected because of this annotation\n\
         \x20 |\n\
         \x20 = note: it is allowed, but nothing with this type can see it\n"
    );
}

#[test]
fn a_missing_field_error_renders_exactly() {
    assert_eq!(
        rendered("let point: {x: Int, y: Int} = {\n  x = 1\n}\n"),
        "error: missing field `y` in record\n\
         \x20--> calc.gcsl:3:1\n\
         \x20 |\n\
         1 | let point: {x: Int, y: Int} = {\n\
         \x20 |            ---------------- expected because of this annotation\n\
         2 |   x = 1\n\
         3 | }\n\
         \x20 | ^\n\
         \x20 |\n\
         \x20 = note: the type `{x: Int, y: Int}` needs it\n"
    );
}

#[test]
fn rebinding_a_field_renders_exactly() {
    assert_eq!(
        rendered("total = 0\ntotal := 1\n"),
        "error: cannot rebind `total`: it is a field, not a variable\n\
         \x20--> calc.gcsl:2:1\n\
         \x20 |\n\
         1 | total = 0\n\
         \x20 | --------- declared here\n\
         2 | total := 1\n\
         \x20 | ^^^^^\n\
         \x20 |\n\
         \x20 = note: rebind with `total = ...` is not allowed either: a field is defined once\n"
    );
}

#[test]
fn warnings_and_errors_render_together_in_source_order() {
    let text = "let p: {x: Int} = { x = 1; z = 2 }\n= p.z";
    let file = source(text);
    let diagnostics = gcsl::run(&file).unwrap_err();
    let severities: Vec<Severity> = diagnostics.iter().map(|d| d.severity).collect();
    assert_eq!(severities, vec![Severity::Warning, Severity::Error]);
    assert!(rendered(text).starts_with("warning: field `z` is not in `{x: Int}`\n"));
    assert!(rendered(text).contains("\nerror: no field `z` in `{x: Int}`\n"));
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
