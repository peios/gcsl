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
        // are N + 1 levels of the syntax tree, the deepest the parser
        // accepts, and each is made from its caller's frame. In the IR each
        // call is two levels, the call and its argument, so the calls
        // deeper than the nesting budget allows fail at runtime.
        let nested_calls = |calls: usize| {
            source(&format!(
                "fn id(x: Int): Int {{ x }}\n{}1{}",
                "id(".repeat(calls),
                ")".repeat(calls)
            ))
        };
        let calls = nested_calls(depth - 1);
        // As many as the budget allows: the innermost call, at depth
        // 2N - 1 in the file's body, enters its frame at 2N - 1 +
        // CALL_LEVELS levels, and `id`'s body, a block around a load, is 2
        // tall, so 2N - 1 + CALL_LEVELS + 2 <= MAX_DEPTH.
        let budgeted = (depth - gcsl::CALL_LEVELS - 1) / 2;
        let calls_within_budget = nested_calls(budgeted);
        let calls_past_budget = nested_calls(budgeted + 1);
        // Function expressions nested in each other's bodies, two levels
        // each, the innermost using a variable of the top level, which every
        // one of them therefore captures.
        let functions_deep = (depth - 1) / 2;
        let functions = source(&format!(
            "let v = 7\nlet f = {}fn(): Int {{ v }}{}\nf()",
            "fn(): Int { let g = ".repeat(functions_deep - 1),
            "\n1 }".repeat(functions_deep - 1)
        ));
        // Lazies nested in each other, each a function of its own, awaited
        // fully by one await, which runs them one after the other.
        let lazies = source(&format!("await {}1", "lazy ".repeat(depth - 2)));
        // Awaits nested in lazies nested in awaits: each await runs the
        // future inside it from inside the one around it, so they nest at
        // runtime too, and the budget stops them.
        let pairs = (depth - 1) / 2;
        let awaits = source(&format!("{}1", "await lazy ".repeat(pairs)));
        // The printers recurse too, but printing a tree this deep takes
        // seconds, since each line is indented by its depth. They recurse
        // one frame per level from a node straight to its children,
        // whatever the node, except through a block's items, so the trees
        // printed here already cover every way they recurse; printing the
        // others would only add time.
        // Each program's value, or the message of the runtime error it
        // fails with.
        for (file, expected, print) in [
            (&chain, Ok(chain_value), true),
            (&nested, Ok(nested_value), true),
            (&nested_blocks, Ok(Value::Int(1)), true),
            (&mixed, Ok(mixed_value), true),
            (&nested_strings, Ok(Value::Str("1".into())), false),
            (&else_ifs, Ok(Value::Int(7)), false),
            (&floats, Ok(floats_value), false),
            (&bools, Ok(Value::Bool(true)), false),
            (&concatenation, Ok(concatenation_value), false),
            (&dotted, Ok(Value::Int(2)), false),
            // Printing a call recurses through its arguments; a function's
            // body prints as the blocks above do.
            (&calls, Err(BUDGET_ERROR), true),
            (&calls_within_budget, Ok(Value::Int(1)), false),
            (&calls_past_budget, Err(BUDGET_ERROR), false),
            (&functions, Ok(Value::Int(1)), false),
            (&lazies, Ok(Value::Int(1)), false),
            (&awaits, Err(BUDGET_ERROR), false),
        ] {
            let (tokens, names, strings) = gcsl::lexer::tokenize(file).unwrap();
            let ast = gcsl::parser::parse(&tokens, names, strings, file).unwrap();
            let mut types = TypeTable::new();
            let (checked, _) = gcsl::check::check(&ast, &mut types).unwrap();
            let ir = Rc::new(gcsl::lower::lower(&ast, &checked, &types, 0).unwrap());
            let value = gcsl::eval::eval(&ir).map_err(|diagnostic| diagnostic.message);
            assert_eq!(value, expected.map_err(str::to_owned));
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
            // Futures and their types.
            format!("{}1", "lazy ".repeat(200_000)),
            format!("{}1", "await ".repeat(200_000)),
            format!(
                "let x: {}Int{} = 1",
                "Future<".repeat(200_000),
                ">".repeat(200_000)
            ),
            format!("let x: Int{} = 1", " | Int".repeat(200_000)),
        ] {
            let diagnostics = run(&text).unwrap_err();
            assert_eq!(diagnostics[0].message, "expression nested too deeply");
        }
    });
}

/// The error of a call past the nesting budget.
const BUDGET_ERROR: &str =
    "call nested too deeply: the limit is 10000 levels of nesting, calls included";

/// The note of a call past the nesting budget.
const BUDGET_NOTE: &str = "every level of expression nesting around a call counts, and so does \
                           the body of the function called; GCSL does not eliminate tail calls, \
                           so recursion this deep is not supported";

/// A recursion that used to overflow the stack of an unoptimised build, when
/// only the number of calls was limited, now fails with a diagnostic.
#[test]
fn a_recursion_past_the_nesting_budget_is_a_diagnostic_not_a_crash() {
    on_recommended_stack(|| {
        // 999 calls, each made 60 levels deep in its function's body.
        let text = format!(
            "fn f(n: Int): Int {{ if n == 0 {{ 0 }} else {{ {}f(n - 1){} }} }}\nf(998)",
            "(".repeat(60),
            " + 0)".repeat(60)
        );
        let diagnostics = run(&text).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, BUDGET_ERROR);
        assert_eq!(diagnostics[0].notes, vec![BUDGET_NOTE]);
        // At the recursive call that would have exceeded the budget.
        let start = u32::try_from(text.find("f(n - 1)").unwrap()).unwrap();
        assert_eq!(diagnostics[0].span, gcsl::Span::new(start, start + 8));
    });
}

/// A kind of nesting to wrap a recursive call in.
struct Nesting {
    /// What it is, for failure messages.
    name: &'static str,
    /// The return type of the function whose call it wraps.
    ret: &'static str,
    /// Wraps an expression in `WRAP` levels of the nesting.
    wrap: fn(&str) -> String,
    /// The value of the recursion that makes the number of calls given.
    value: fn(i64) -> Value,
}

/// The levels of nesting that
/// `the_nesting_budget_fits_the_stack_for_every_kind_of_nesting` wraps each
/// recursive call in.
const WRAP: usize = 60;

/// `STACK_SIZE` covers the nesting budget, unoptimised, whatever kind of
/// nesting the calls are made from: for each costly kind, a recursion whose
/// recursive call is wrapped in that nesting, and whose deepest frame is as
/// tall, makes as many calls as the budget allows, which takes its nesting
/// to exactly `MAX_DEPTH` levels, and one more call fails.
#[test]
fn the_nesting_budget_fits_the_stack_for_every_kind_of_nesting() {
    on_recommended_stack(|| {
        let kinds = [
            Nesting {
                name: "additions",
                ret: "Int",
                wrap: |inner| format!("{inner}{}", " + 1".repeat(WRAP)),
                // `f(0)` is `-1 + WRAP`, and each call adds `WRAP`.
                value: |calls| Value::Int(calls * i64::try_from(WRAP).unwrap() - 1),
            },
            Nesting {
                name: "negations",
                ret: "Int",
                wrap: |inner| format!("{}{inner}{}", "-(".repeat(WRAP), ")".repeat(WRAP)),
                // An even number of negations.
                value: |_| Value::Int(-1),
            },
            Nesting {
                name: "interpolated strings",
                ret: "String",
                wrap: |inner| format!("{}{inner}{}", "\"{".repeat(WRAP), "}\"".repeat(WRAP)),
                value: |_| Value::Str("-1".into()),
            },
            Nesting {
                name: "nested blocks",
                ret: "Int",
                wrap: |inner| format!("{}{inner}{}", "{ ".repeat(WRAP), " }".repeat(WRAP)),
                value: |_| Value::Int(-1),
            },
            Nesting {
                name: "blocks with a declaration each",
                ret: "Int",
                // Half the levels are blocks, and half additions of the
                // variables they declare.
                wrap: |inner| {
                    format!(
                        "{}{inner}{}{}",
                        "{ let v = 1\n".repeat(WRAP / 2),
                        " + v".repeat(WRAP / 2),
                        "\n}".repeat(WRAP / 2)
                    )
                },
                value: |calls| Value::Int(calls * i64::try_from(WRAP / 2).unwrap() - 1),
            },
        ];
        // In the function's body, the block (0), the `if` (1) and the
        // branch's block (2) are around the wrapping, so the recursive call
        // is at depth `WRAP + 3`, and it is as tall as its argument, its
        // subtraction and its load: the body is `WRAP + 7` tall, and so is
        // the other branch, whose negations and subtraction stand in for
        // the call.
        let depth = WRAP + 3;
        let height = WRAP + 7;
        let step = depth + gcsl::CALL_LEVELS;
        // The first call, `padding + 1` deep in the file's body, enters its
        // frame at `padding + 1 + CALL_LEVELS` levels, and each nested call
        // `step` further in. With the padding making up the remainder, the
        // deepest frame's body reaches exactly `MAX_DEPTH`.
        let room = gcsl::MAX_DEPTH - height - gcsl::CALL_LEVELS - 1;
        let padding = room % step;
        let calls = 1 + room / step;
        assert!(calls > 100, "{calls}");
        for Nesting {
            name: kind,
            ret,
            wrap,
            value,
        } in kinds
        {
            // `f(n)` makes n + 1 calls.
            let program = |calls: usize, padding: usize| {
                format!(
                    "fn f(n: Int): {ret} {{ if n == 0 {{ {} }} else {{ {} }} }}\n{}f({}){}",
                    wrap("-(-(n - 1))"),
                    wrap("f(n - 1)"),
                    "{ ".repeat(padding),
                    calls - 1,
                    " }".repeat(padding)
                )
            };
            let text = program(calls, padding);
            let file = source(&text);
            let (tokens, names, strings) = gcsl::lexer::tokenize(&file).unwrap();
            let ast = gcsl::parser::parse(&tokens, names, strings, &file).unwrap();
            let mut types = TypeTable::new();
            let (checked, _) = gcsl::check::check(&ast, &mut types).unwrap();
            let ir = Rc::new(gcsl::lower::lower(&ast, &checked, &types, 0).unwrap());
            assert_eq!(ir.function_count(), 1, "{kind}");
            let gcsl::ir::IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                panic!("{kind}: the root is not a block");
            };
            let gcsl::ir::IrKind::Store(_, closure) = ir.get(ir.items(items)[0]).kind else {
                panic!("{kind}: the first item is not a store");
            };
            let gcsl::ir::IrKind::MakeClosure { function, .. } = ir.get(closure).kind else {
                panic!("{kind}: the function item is not a closure");
            };
            assert_eq!(ir.function(function).height as usize, height, "{kind}");
            let expected = value(i64::try_from(calls).unwrap());
            assert_eq!(gcsl::eval::eval(&ir), Ok(expected), "{kind}");
            // One more call is past the budget, and so is one more level of
            // padding: the budget is used exactly.
            for past in [program(calls + 1, padding), program(calls, padding + 1)] {
                let diagnostics = run(&past).unwrap_err();
                assert_eq!(diagnostics[0].message, BUDGET_ERROR, "{kind}");
            }
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
            // An impure function's bare result may be a future.
            "3 : Int | Future<Int>",
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
        rendered("fn f() -> Present<Int> { 1 }\nfn g(): Int { f() }\n"),
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

/// A build script: a pure plan whose steps run when an impure wrapper
/// awaits them, and a pure function passing a step through, whose bare
/// parameter is inferred to be `String | Future<String>`, since its body
/// never measures it.
const BUILD_SCRIPT: &str = "\
fn compile(target: String) -> String { \"compiled {target}\" }
fn plan(name: String): {name: String, build: Future<String>, check: Future<{}>} {
  name = name
  build = lazy compile(name)
  check = lazy { }
}
fn build(package: fn(String): {name: String, build: Future<String>, check: Future<{}>}, name: String) -> {name: String, log: String} {
  let conf = package(name)
  await conf.check
  name = conf.name
  log = await conf.build
}
fn keep(step: String): {step: String | Future<String>} { step = step }
let planned = plan(\"gcsl\")
let built = build(plan, \"gcsl\")
name = built.name
log = built.log
later = keep(planned.build).step
";

#[test]
fn a_build_script_plans_purely_and_builds_on_await() {
    assert_eq!(
        run(BUILD_SCRIPT).unwrap().to_string(),
        "{name = \"gcsl\"; log = \"compiled gcsl\"; later = Future<String>}"
    );
    assert_eq!(
        show(BUILD_SCRIPT),
        "{name = \"gcsl\"; log = \"compiled gcsl\"; later = Future<String>} : \
         {later: String | Future<String>, log: String, name: String}"
    );
    // The planned step, passed through, runs where it is awaited.
    assert_eq!(
        run(&format!("{BUILD_SCRIPT}ran = await later"))
            .unwrap()
            .to_string(),
        "{name = \"gcsl\"; log = \"compiled gcsl\"; later = Future<String>; \
         ran = \"compiled gcsl\"}"
    );
    // `keep`'s parameter shows as inferred.
    let mut session = Session::new();
    session.eval(&source(BUILD_SCRIPT)).unwrap();
    assert_eq!(
        session.eval(&source("keep")).unwrap().value.to_string(),
        "fn(step: String | Future<String>): {step: String | Future<String>}"
    );
}

#[test]
fn a_future_passed_through_a_pure_function_runs_when_awaited_after() {
    let keep = "fn keep(x: Int): {v: Int | Future<Int>} { v = x }\n";
    // Passed through unawaited: a failing future is not run by `keep`.
    assert_eq!(
        run(&format!("{keep}let r = keep(lazy 1 // 0)\n= 7")).unwrap(),
        Value::Int(7)
    );
    // And runs where it is awaited after.
    assert_eq!(
        run(&format!("{keep}let r = keep(lazy 20 + 1)\n= await r.v")).unwrap(),
        Value::Int(21)
    );
    let diagnostics = run(&format!("{keep}let r = keep(lazy 1 // 0)\n= await r.v")).unwrap_err();
    assert_eq!(diagnostics[0].message, "division by zero");
    // A present value passes through as it is.
    assert_eq!(run(&format!("{keep}keep(5).v + 1")).unwrap(), Value::Int(6));
    // A function that measures its parameter makes it present, and an
    // impure caller awaits the argument at the call.
    assert_eq!(
        run("fn double(x: Int): Int { x * 2 }\nlet f = lazy 21\ndouble(f)").unwrap(),
        Value::Int(42)
    );
}

#[test]
fn an_argument_awaited_inside_a_pure_function_renders_exactly() {
    assert_eq!(
        rendered(
            "fn my_pure_func(x: Int): Int {\n  let y = x + 5\n  y\n}\n\
             fn caller(my_future: Future<Int>): Int {\n  my_pure_func(my_future)\n}\n"
        ),
        "error: argument `my_future` would be awaited inside `my_pure_func`, which is pure\n\
         \x20--> calc.gcsl:6:16\n\
         \x20 |\n\
         2 |   let y = x + 5\n\
         \x20 |           - `x` is measured here\n\
         ...\n\
         5 | fn caller(my_future: Future<Int>): Int {\n\
         \x20 |                                  - this function is pure, declared with `:`\n\
         6 |   my_pure_func(my_future)\n\
         \x20 |                ^^^^^^^^^ this is Future<Int>\n\
         \x20 |\n\
         \x20 = note: a pure function cannot await; await the argument before the call, in an \
         impure function: `my_pure_func(await my_future)`\n"
    );
}

#[test]
fn an_await_in_a_pure_function_renders_exactly() {
    assert_eq!(
        rendered("fn plan(step: Future<{}>): {} {\n  await step\n  = {}\n}\n"),
        "error: cannot await in a pure function\n\
         \x20--> calc.gcsl:2:3\n\
         \x20 |\n\
         1 | fn plan(step: Future<{}>): {} {\n\
         \x20 |                          - this function is pure, declared with `:`\n\
         2 |   await step\n\
         \x20 |   ^^^^^\n\
         \x20 |\n\
         \x20 = note: awaiting runs a deferred expression, which may have effects; declare the \
         function with `->` to make it impure\n"
    );
    assert_eq!(
        rendered("fn total(x: Int | Future<Int>): Int { x + 1 }\n"),
        "error: cannot use a value that may be a future in a pure function\n\
         \x20--> calc.gcsl:1:39\n\
         \x20 |\n\
         1 | fn total(x: Int | Future<Int>): Int { x + 1 }\n\
         \x20 |                               -       ^ this is Int | Future<Int>\n\
         \x20 |                               |\n\
         \x20 |                               this function is pure, declared with `:`\n\
         \x20 |\n\
         \x20 = note: using the value here would await it, which may run effects; a pure \
         function may hold and pass on a future but not await it. Declare the function with \
         `->` to make it impure\n"
    );
}

#[test]
fn a_captured_value_that_may_be_a_future_renders_exactly() {
    let outer = "fn outer(n: Int): Future<Int> {\n  lazy {\n    let g = fn(): Int { n + 1 }\n    = g()\n  }\n}\n";
    assert_eq!(
        rendered(outer),
        "error: cannot use a value that may be a future in a pure function\n\
         \x20--> calc.gcsl:3:25\n\
         \x20 |\n\
         3 |     let g = fn(): Int { n + 1 }\n\
         \x20 |                 -       ^ this is Int | Future<Int>\n\
         \x20 |                 |\n\
         \x20 |                 this function is pure, declared with `:`\n\
         \x20 |\n\
         \x20 = note: a pure function cannot await, and `n` may be a future; await it before \
         the function is created, in the enclosing impure code: `let n = await n`\n"
    );
    // The suggested fix, applied.
    let fixed = "fn outer(n: Int): Future<Int> {\n  lazy {\n    let n = await n\n    let g = fn(): Int { n + 1 }\n    = g()\n  }\n}\n";
    assert_eq!(
        run(&format!("{fixed}await outer(41)")).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        run(&format!("{fixed}await outer(lazy 41)")).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn a_record_that_may_be_a_future_can_be_updated() {
    // A result written as the either shape, which is not inferred.
    let get = "fn get() -> {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}> { a = 1 }\n\
               let r = get()\n";
    assert_eq!(
        run(&format!("{get}r.a := 2\nr")).unwrap().to_string(),
        "{a = 2}"
    );
    assert_eq!(
        show(&format!("{get}r.a := 2\nr")),
        "{a = 2} : {a: Int | Future<Int>}"
    );
    assert_eq!(show(&format!("{get}let r.a = 2\nr")), "{a = 2} : {a: Int}");
    // Through a `lazy`, and along a path of records that may be futures.
    assert_eq!(
        show("let r = lazy { a = 1; b = 2 }\nlet r.a = 5\nr"),
        "{a = 5; b = 2} : {a: Int, b: Int}"
    );
    let f = "fn f(conf: {a: {b: Int}}) -> {a: {b: Int}} {\n  let conf.a.b = 2\n  conf\n}\n";
    // The updated record is present, so the result is inferred to be.
    assert_eq!(
        run(&format!("{f}f(lazy {{ a = lazy {{ b = 1 }} }})"))
            .unwrap()
            .to_string(),
        "{a = {b = 2}}"
    );
    let f = "fn f(conf: {a: {b: Int}}) -> {a: {b: Int}} {\n  conf.a.b := 2\n  conf\n}\n";
    assert_eq!(
        run(&format!("{f}f(lazy {{ a = lazy {{ b = 1 }} }})"))
            .unwrap()
            .to_string(),
        "{a = {b = 2}}"
    );
}

#[test]
fn a_callback_that_cannot_take_futures_renders_exactly() {
    assert_eq!(
        rendered(
            "fn double(x: Int): Int { x * 2 }\nfn run(f: fn(Int) -> Int, j: Int) -> Int { f(j) }\nrun(double, 1)\n"
        ),
        "error: mismatched types\n\
         \x20--> calc.gcsl:3:5\n\
         \x20 |\n\
         2 | fn run(f: fn(Int) -> Int, j: Int) -> Int { f(j) }\n\
         \x20 |                                              - `f` is given a value that may be a \
         future here\n\
         3 | run(double, 1)\n\
         \x20 | --- ^^^^^^ expected fn(Int | Future<Int>) -> Int | Future<Int>, found fn(x: Int): Int\n\
         \x20 | |\n\
         \x20 | the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | \
         Future<Int>) -> Int | Future<Int>\n"
    );
    // Used as a value.
    assert_eq!(
        rendered(
            "fn double(x: Int): Int { x * 2 }\nfn run(f: fn(Int) -> Int) -> Int {\n  let g = f\n  g(1)\n}\nrun(double)\n"
        ),
        "error: mismatched types\n\
         \x20--> calc.gcsl:6:5\n\
         \x20 |\n\
         3 |   let g = f\n\
         \x20 |           - `f` is used as a value here, so it may be given a future\n\
         ...\n\
         6 | run(double)\n\
         \x20 | --- ^^^^^^ expected fn(Int | Future<Int>) -> Int | Future<Int>, found fn(x: Int): Int\n\
         \x20 | |\n\
         \x20 | the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>) -> Int | \
         Future<Int>\n"
    );
}

#[test]
fn a_callback_given_only_present_values_takes_a_pure_function() {
    let double = "fn double(x: Int): Int { x * 2 }\n";
    let run_fn = "fn run(f: fn(Int) -> Int) -> Int { f(1) }\n";
    assert_eq!(
        run(&format!("{double}{run_fn}run(double)")).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        show(&format!("{double}{run_fn}run")),
        "fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int> : fn(f: fn(Int) -> Int | \
         Future<Int>) -> Int | Future<Int>"
    );
    // An argument awaited before the call, a record, and a call in a
    // `lazy` awaited after.
    let text = "fn sum(r: {a: Int, b: Int}): Int { r.a + r.b }\n\
                fn run(f: fn({a: Int, b: Int}) -> Int, j: Int) -> Future<Int> {\n\
                \x20 let k = await j\n\
                \x20 lazy f({ a = k; b = 2 })\n\
                }\n\
                await run(sum, lazy 40)";
    assert_eq!(run(text).unwrap(), Value::Int(42));
}

/// A build script in the style of `BUILD_SCRIPT`, whose impure wrapper
/// takes a pure step function and gives it only present values.
const STEP_SCRIPT: &str = "\
fn compile(target: String): String { \"compiled {target}\" }
fn build(step: fn(String) -> String, name: String) -> {name: String, log: String} {
  let ready = await name
  name = ready
  log = step(ready)
}
let built = build(compile, lazy \"gcsl\")
name = built.name
log = built.log
";

#[test]
fn a_wrapper_takes_a_pure_step_function() {
    assert_eq!(
        show(STEP_SCRIPT),
        "{name = \"gcsl\"; log = \"compiled gcsl\"} : {log: String | Future<String>, name: \
         String}"
    );
    let mut session = Session::new();
    session.eval(&source(STEP_SCRIPT)).unwrap();
    assert_eq!(
        session.eval(&source("build")).unwrap().value.to_string(),
        "fn(step: fn(String) -> String | Future<String>, name: String | Future<String>) -> \
         {log: String | Future<String>, name: String}"
    );
    // Giving the step the name as it was given, which may be a future, it
    // takes only a step that can take one.
    let text = STEP_SCRIPT.replace("step(ready)", "step(name)");
    let diagnostics = run(&text).unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "mismatched types");
    assert_eq!(
        diagnostics[0].labels[1].text,
        "`step` is given a value that may be a future here"
    );
}

#[test]
fn a_narrowed_callback_is_kept_by_a_session() {
    let mut session = Session::new();
    session
        .eval(&source(
            "fn double(x: Int): Int { x * 2 }\n\
             fn run(f: fn(Int) -> Int) -> Int { f(1) }\n\
             fn loose(f: fn(Int) -> Int, j: Int) -> Int { f(j) }",
        ))
        .unwrap();
    assert_eq!(
        session.eval(&source("run(double)")).unwrap().value,
        Value::Int(2)
    );
    // The callee is of an earlier input, whose source is gone: no label.
    let diagnostics = session.eval(&source("loose(double, 1)")).unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "mismatched types");
    assert_eq!(diagnostics[0].labels.len(), 1, "{:?}", diagnostics[0]);
    assert_eq!(
        diagnostics[0].labels[0].text,
        "the function is fn(f: fn(Int | Future<Int>) -> Int | Future<Int>, j: Int | \
         Future<Int>) -> Int | Future<Int>"
    );
}

#[test]
fn a_runtime_error_in_a_future_renders_where_it_happens() {
    assert_eq!(
        rendered("let x = lazy 1 // 0\nawait x\n"),
        "error: division by zero\n\
         \x20--> calc.gcsl:1:16\n\
         \x20 |\n\
         1 | let x = lazy 1 // 0\n\
         \x20 |              - ^^ - 0\n\
         \x20 |              |\n\
         \x20 |              1\n\
         2 | await x\n\
         \x20 | ------- awaited here\n"
    );
}

/// An impure function whose body returns a present record of present
/// fields.
const GET: &str = "fn get() -> {a: Int, b: String} { a = 1; b = \"x\" }\n";

#[test]
fn an_impure_functions_inferred_result_is_what_it_returns() {
    assert_eq!(
        show(&format!("{GET}get()")),
        "{a = 1; b = \"x\"} : {a: Int, b: String}"
    );
    // A future of it satisfies a written future of a present record.
    assert_eq!(
        run(&format!(
            "{GET}let f: Future<{{a: Int}}> = lazy get()\n(await f).a"
        ))
        .unwrap(),
        Value::Int(1)
    );
    // A field written present is awaited, and one written bare keeps the
    // future; both read back give the value.
    let m =
        "fn m(x: Future<Int>) -> {a: Present<Int>, b: Int} { a = x; b = x }\nlet r = m(lazy 7)\n";
    assert_eq!(run(&format!("{m}r.a")).unwrap(), Value::Int(7));
    assert_eq!(run(&format!("{m}await r.b")).unwrap(), Value::Int(7));
    assert_eq!(run(&format!("{m}r.a + r.b")).unwrap(), Value::Int(14));
    // A body awaited deeply is present at every position.
    assert_eq!(
        show("fn f() -> {a: Present<Int>, b: Int} { = { a = lazy 1; b = lazy 2 } }\nf()"),
        "{a = 1; b = 2} : {a: Int, b: Int}"
    );
    // Nested records, and recursion.
    assert_eq!(
        run("fn n() -> {a: {b: Int}} { a = { b = 1 } }\nn().a.b + 1").unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        show("fn sum(n: Int) -> Int { if n == 0 { 0 } else { sum(n - 1) + n } }\nsum(4)"),
        "10 : Int"
    );
    assert_eq!(
        show("fn count(n: Int) -> Int { if n == 0 { 0 } else { count(n - 1) } }\ncount(4)"),
        "0 : Int | Future<Int>"
    );
    // A statement whose value is a future of `{}` is still awaited, and
    // its effects are not lost.
    assert_eq!(
        run("fn tock() -> {} { lazy { let x = 1 // 0 } }\ntock()\n1").unwrap_err()[0].message,
        "division by zero"
    );
    assert_eq!(
        run("fn tick() -> {} { }\ntick()\n1").unwrap(),
        Value::Int(1)
    );
}

#[test]
fn dotted_fields_are_read_back_through_an_inferred_result() {
    let n = "fn n() -> {a: {b: Int}} { a.b = 1 }\n";
    assert_eq!(show(&format!("{n}n()")), "{a = {b = 1}} : {a: {b: Int}}");
    // Present, so reading it awaits nothing; a leaf that is a future is
    // awaited.
    let f = "fn f() -> {a: {b: Int, c: Int}} { a.b = lazy 5; a.c = 1 }\n";
    for (text, awaits, expected) in [
        (format!("{n}n().a.b + 1"), false, 2),
        (format!("{f}f().a.b + 1"), true, 6),
    ] {
        let file = source(&text);
        let (tokens, names, strings) = gcsl::lexer::tokenize(&file).unwrap();
        let ast = gcsl::parser::parse(&tokens, names, strings, &file).unwrap();
        let mut types = TypeTable::new();
        let (checked, _) = gcsl::check::check(&ast, &mut types).unwrap();
        let ir = Rc::new(gcsl::lower::lower(&ast, &checked, &types, 0).unwrap());
        let printed = ir.pretty(ast.names()).to_string();
        assert_eq!(printed.contains("await"), awaits, "{text}: {printed}");
        assert_eq!(gcsl::eval::eval(&ir).unwrap(), Value::Int(expected));
    }
    // A leaf that may be a future, and one that is present.
    let m = "fn m(c: Int) -> {a: {b: Int, d: Int}} { a.b = c; a.d = 2 }\n";
    assert_eq!(
        run(&format!("{m}m(lazy 4).a.b + m(3).a.d")).unwrap(),
        Value::Int(6)
    );
    // A leaf that is a future, read with and without `await`.
    let f = format!("{f}let r = f()\n");
    assert_eq!(run(&format!("{f}await r.a.b")).unwrap(), Value::Int(5));
    assert_eq!(run(&format!("{f}r.a.b + r.a.c")).unwrap(), Value::Int(6));
    assert_eq!(
        run(&format!("{f}let x: Int = r.a.b\nx")).unwrap(),
        Value::Int(5)
    );
    // A leaf written present, given a future, is awaited where it is set.
    assert_eq!(
        run("fn p() -> {a: {b: Present<Int>}} { let x = lazy 7\na.b = x }\np().a.b + 1").unwrap(),
        Value::Int(8)
    );
    // Three levels, whole and dotted fields together, in a memoised
    // function.
    let g = "fn g(x: Int) ->> {a: {b: {c: Int}, d: Int}, e: Int} { a.b.c = x + 1; e = 3; a.d = lazy x }\n";
    assert_eq!(show(&format!("{g}g(1).a.b")), "{c = 2} : {c: Int}");
    assert_eq!(
        run(&format!("{g}g(1).a.b.c * g(1).e + g(1).a.d")).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn an_inferred_result_is_kept_by_a_session() {
    let mut session = Session::new();
    session.eval(&source(GET)).unwrap();
    assert_eq!(
        session.eval(&source("get")).unwrap().value.to_string(),
        "fn() -> {a: Int, b: String}"
    );
    // The scope holds the inferred type, which a tighter written type
    // accepts.
    assert!(
        session
            .eval(&source("let g: fn() -> Present<{a: Int}> = get"))
            .is_ok()
    );
}

#[test]
fn a_mismatch_shows_an_inferred_result() {
    assert_eq!(
        rendered(&format!("{GET}let r = get()\nr : Int\n")),
        "error: mismatched types\n\
         \x20--> calc.gcsl:3:1\n\
         \x20 |\n\
         3 | r : Int\n\
         \x20 | ^   --- ascribed here\n\
         \x20 | |\n\
         \x20 | expected Int, found {a: Int, b: String}\n"
    );
}
