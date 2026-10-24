//! Tests of workspaces: the arguments of an invocation, their types and
//! defaults, and what an invocation runs.

use super::*;

/// A def-type of one invocation parameter, [`BUILD`]'s `build`, one of
/// two, `deploy`, and one of none, `package`, with a definition of
/// each.
pub(super) const ARGUMENTS: [(&str, &str); 3] = [
    BUILD,
    ("app.gcsl", "def build x(t) { t }"),
    (
        "lib/defs.gcsl",
        "#[def_type]\n\
             def deploy(name: String, f: fn(): String, target: String, mode: String): String {\n\
             \x20   \"{name} {f()} {target} {mode}\"\n\
             }\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
             def deploy site() { \"s\" }\n\
             def package p() { \"p\" }",
    ),
];

#[test]
fn invoking_gives_its_arguments_as_strings() {
    // One argument, two, and none, each by its parameter's name, in any
    // order.
    let files = ARGUMENTS;
    assert_eq!(
        invoked_text_with(&files, ("build", "x"), &[("mode", "debug")]),
        "x: debug"
    );
    assert_eq!(
        invoked_text_with(
            &files,
            ("deploy", "site"),
            &[("target", "prod"), ("mode", "fast")]
        ),
        "site s prod fast"
    );
    assert_eq!(
        invoked_text_with(
            &files,
            ("deploy", "site"),
            &[("mode", "fast"), ("target", "prod")]
        ),
        "site s prod fast"
    );
    assert_eq!(invoked_text_with(&files, ("package", "p"), &[]), "p p");
    // Each a string literal as it is, nothing in it read as GCSL.
    assert_eq!(
        invoked_text_with(&files, ("build", "x"), &[("mode", "{mode} \\ \" -1")]),
        "x: {mode} \\ \" -1"
    );
    assert_eq!(
        invoked_text_with(&files, ("build", "x"), &[("mode", "")]),
        "x: "
    );
    // Listed, the def-types' invocation parameters, as declared.
    let mut workspace = rooted(&files).unwrap();
    let parameters: Vec<(String, Vec<String>)> = workspace
        .def_types()
        .unwrap()
        .into_iter()
        .map(|def_type| (def_type.path, def_type.parameters))
        .collect();
    let owned = |names: &[&str]| names.iter().map(|&name| name.to_owned()).collect();
    assert_eq!(
        parameters,
        [
            ("deploy".to_owned(), owned(&["target", "mode"])),
            ("package".to_owned(), owned(&[])),
            ("build".to_owned(), owned(&["mode"])),
        ]
    );
    // Invoked again on one workspace, with other arguments.
    for mode in ["a", "b"] {
        let value = workspace.invoke("build", "x", &[string("mode", mode)]);
        assert_eq!(value.unwrap().value.to_string(), format!("x: {mode}"));
    }
}

#[test]
fn the_arguments_must_name_each_required_parameter_once() {
    let files = ARGUMENTS;
    let error = |def_type: &str, path: &str, args: &[(&str, &str)]| {
        request_error_with(&files, (def_type, path), args)
    };
    // Missing, every one, by name.
    assert_eq!(
        error("build", "x", &[]),
        "`build` needs `mode`, which was not given"
    );
    assert_eq!(
        error("deploy", "site", &[("target", "a")]),
        "`deploy` needs `mode`, which was not given"
    );
    assert_eq!(
        error("deploy", "site", &[]),
        "`deploy` needs `target` and `mode`, which were not given"
    );
    // Twice.
    assert_eq!(
        error("build", "x", &[("mode", "a"), ("mode", "b")]),
        "`build`'s parameter `mode` is given more than once"
    );
    // No such parameter, those there are noted.
    assert_eq!(
        error(
            "deploy",
            "site",
            &[("target", "a"), ("mode", "b"), ("extra", "c")]
        ),
        "`deploy` has no invocation parameter `extra`; note: its invocation parameters are \
             `target`, `mode`"
    );
    assert_eq!(
        error("build", "x", &[("mdoe", "a")]),
        "`build` has no invocation parameter `mdoe`; note: its only invocation parameter is \
             `mode`"
    );
    assert_eq!(
        error("package", "p", &[("a", "a")]),
        "`package` has no invocation parameter `a`; note: it has no invocation parameters"
    );
    assert_eq!(
        error("package", "p", &[("a", "a"), ("b", "b")]),
        "`package` has no invocation parameter `a`; note: it has no invocation parameters"
    );
    // Argument by argument, in order, before what is missing.
    assert_eq!(
        error(
            "deploy",
            "site",
            &[("x", "a"), ("target", "a"), ("target", "b")]
        ),
        "`deploy` has no invocation parameter `x`; note: its invocation parameters are \
             `target`, `mode`"
    );
    assert_eq!(
        error(
            "deploy",
            "site",
            &[("target", "a"), ("target", "b"), ("x", "a")]
        ),
        "`deploy`'s parameter `target` is given more than once"
    );
    // Before the arguments, the path.
    assert_eq!(
        error("build", "y", &[("mode", "a"), ("mode", "b")]),
        "`build` has no definition `y`; note: its only definition is `x`"
    );
}

#[test]
fn only_six_types_of_parameter_can_be_given() {
    let files = [
        (
            "workspace.gcsl",
            "type Version = { version: String }\n\
                 type Name = String\n\
                 #[def_type]\n\
                 def count(name: String, f: fn(): String, n: Int): String { f() }\n\
                 #[def_type]\n\
                 def mixed(name: String, f: fn(): String, a: String, b: List<String>, c: Int): String { f() }\n\
                 #[def_type]\n\
                 def versioned(name: String, f: fn(): String, v: Version): String { f() }\n\
                 #[def_type]\n\
                 def later(name: String, f: fn(): String, s: Future<String>) -> String { f() }\n\
                 #[def_type]\n\
                 def fits(name: String, f: fn(): String, n: Name, s: String | Int): String {\n\
                 \x20   \"{f()} {n} {s}\"\n\
                 }",
        ),
        (
            "defs.gcsl",
            "def count x() { \"x\" }\n\
                 def mixed x() { \"x\" }\n\
                 def versioned x() { \"x\" }\n\
                 def later x() { \"x\" }\n\
                 def fits x() { \"x\" }",
        ),
    ];
    let error =
        |def_type: &str, args: &[(&str, &str)]| request_error_with(&files, (def_type, "x"), args);
    // An `Int` can be given now; not given, it is missing.
    assert_eq!(
        invoked_text_with(&files, ("count", "x"), &[("n", "1")]),
        "x"
    );
    assert_eq!(
        error("count", &[]),
        "`count` needs `n`, which was not given"
    );
    // One that has no default is reported whatever is given, before
    // anything given, the first, its type as it is written.
    let mixed = "`mixed`'s parameter `b` is `List<String>`, but only `String`, `Int`, \
                     `Float`, `Bool`, `Duration` and `Size` parameters can be given here yet";
    assert_eq!(error("mixed", &[("a", "a"), ("b", "b"), ("c", "1")]), mixed);
    assert_eq!(error("mixed", &[("a", "a"), ("c", "1")]), mixed);
    assert_eq!(error("mixed", &[("nope", "a")]), mixed);
    assert_eq!(
        error("versioned", &[("v", "1")]),
        "`versioned`'s parameter `v` is `Version`, but only `String`, `Int`, `Float`, \
             `Bool`, `Duration` and `Size` parameters can be given here yet"
    );
    // A present value fits no future.
    assert_eq!(
        error("later", &[("s", "1")]),
        "`later`'s parameter `s` is `Future<String>`, but only `String`, `Int`, `Float`, \
             `Bool`, `Duration` and `Size` parameters can be given here yet"
    );
    // An alias of `String` can be given; a union with it cannot.
    assert_eq!(
        error("fits", &[("n", "n"), ("s", "s")]),
        "`fits`'s parameter `s` is `String | Int`, but only `String`, `Int`, `Float`, \
             `Bool`, `Duration` and `Size` parameters can be given here yet"
    );
}

#[test]
fn a_bare_string_parameter_of_an_impure_def_type_is_given() {
    // `version: String` of an impure def-type is `String |
    // Future<String>`, which a string fits; of a memoised one, `String`.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def run(name: String, f: fn(v: String) -> String, version: String) -> String {\n\
                 \x20   f(version)\n\
                 }\n\
                 #[def_type]\n\
                 def once(name: String, f: fn(v: String) -> String, version: String) ->> String {\n\
                 \x20   f(version)\n\
                 }",
        ),
        (
            "pkg/p.gcsl",
            "def run p(v) { \"run p at {v}\" }\ndef once p(v) { \"once p at {v}\" }",
        ),
    ];
    assert_eq!(
        invoked_text_with(&files, ("run", "p"), &[("version", "0.1.0")]),
        "run p at 0.1.0"
    );
    assert_eq!(
        invoked_text_with(&files, ("once", "p"), &[("version", "0.2.0")]),
        "once p at 0.2.0"
    );
    let workspace = rooted(&files).unwrap();
    let shown: Vec<String> = workspace
        .found_def_types()
        .iter()
        .map(|found| {
            let (_, ty) = workspace.leaf_cell(found.listed.file, found.item).unwrap();
            let sig = workspace.types.fn_sig(ty).unwrap();
            let version = workspace
                .types
                .display(sig.params[2].1, &workspace.scan.names);
            format!("{}: {version}", found.listed.path)
        })
        .collect();
    assert_eq!(shown, ["run: String | Future<String>", "once: String"]);
}

/// A workspace whose def-types have optional invocation parameters (see
/// the checker's "Calls"): Jack's `lock` from his newpkgs, in the shape
/// its TODO asks for, and others with optional parameters after, around
/// and before required ones.
pub(super) const OPTIONAL: [(&str, &str); 2] = [
    (
        "workspace.gcsl",
        "type Version = { version: String }\n\
             #[def_type]\n\
             def lock(name: String, func: fn(version: String) -> List<Version>, version: String = \"\") -> String {\n\
             \x20   let found = func(version)\n\
             \x20   \"{name} [{version}] {len(found)}\"\n\
             }\n\
             #[def_type]\n\
             def three(name: String, f: fn(): String, a: String, b: String = \"B\", c: String = \"C\") -> String {\n\
             \x20   \"{name} {a} {b} {c}\"\n\
             }\n\
             #[def_type]\n\
             def two(name: String, f: fn(): String, a: String = \"A\", b: String = \"B\") -> String {\n\
             \x20   \"{name} {a} {b}\"\n\
             }\n\
             #[def_type]\n\
             def first(name: String, f: fn(): String, a: String = \"A\", b: String) -> String {\n\
             \x20   \"{name} {a} {b}\"\n\
             }",
    ),
    (
        "org.figlet.figlet/lock.gcsl",
        "def lock org.figlet.figlet(version) { [{ version = version }] }\n\
             def three x() { \"x\" }\n\
             def two x() { \"x\" }\n\
             def first x() { \"x\" }",
    ),
];

#[test]
fn an_optional_invocation_parameter_may_be_left_out() {
    let files = OPTIONAL;
    // Jack's `pekit lock org.figlet.figlet`, and with the version.
    assert_eq!(
        invoked_text_with(&files, ("lock", "org.figlet.figlet"), &[]),
        "org.figlet.figlet [] 1"
    );
    assert_eq!(
        invoked_text_with(
            &files,
            ("lock", "org.figlet.figlet"),
            &[("version", "2.2.6")]
        ),
        "org.figlet.figlet [2.2.6] 1"
    );
    // By name, each left out given its default, whichever others are
    // given.
    let three = |args: &[(&str, &str)]| invoked_text_with(&files, ("three", "x"), args);
    assert_eq!(three(&[("a", "a")]), "x a B C");
    assert_eq!(three(&[("a", "a"), ("b", "b")]), "x a b C");
    assert_eq!(three(&[("a", "a"), ("b", "b"), ("c", "c")]), "x a b c");
    assert_eq!(three(&[("c", "c"), ("a", "a")]), "x a B c");
    assert_eq!(invoked_text_with(&files, ("two", "x"), &[]), "x A B");
    assert_eq!(
        invoked_text_with(&files, ("two", "x"), &[("a", "a")]),
        "x a B"
    );
    assert_eq!(
        invoked_text_with(&files, ("two", "x"), &[("b", "b")]),
        "x A b"
    );
    // A required one after an optional one: the optional one may be
    // left out too.
    assert_eq!(
        invoked_text_with(&files, ("first", "x"), &[("a", "a"), ("b", "b")]),
        "x a b"
    );
    assert_eq!(
        invoked_text_with(&files, ("first", "x"), &[("b", "b")]),
        "x A b"
    );
    // Listed: which are optional, in the order of the parameters.
    let workspace = rooted(&files).unwrap();
    let optional: Vec<(String, Vec<String>, Vec<bool>)> = workspace
        .def_types()
        .unwrap()
        .into_iter()
        .map(|def_type| (def_type.path, def_type.parameters, def_type.optional))
        .collect();
    let owned = |names: &[&str]| names.iter().map(|&name| name.to_owned()).collect();
    assert_eq!(
        optional,
        [
            ("lock".to_owned(), owned(&["version"]), vec![true]),
            (
                "three".to_owned(),
                owned(&["a", "b", "c"]),
                vec![false, true, true]
            ),
            ("two".to_owned(), owned(&["a", "b"]), vec![true, true]),
            ("first".to_owned(), owned(&["a", "b"]), vec![true, false]),
        ]
    );
}

#[test]
fn the_arguments_must_fill_every_required_parameter() {
    let files = OPTIONAL;
    let error = |def_type: &str, path: &str, args: &[(&str, &str)]| {
        request_error_with(&files, (def_type, path), args)
    };
    assert_eq!(
        error(
            "lock",
            "org.figlet.figlet",
            &[("version", "a"), ("version", "b")]
        ),
        "`lock`'s parameter `version` is given more than once"
    );
    assert_eq!(
        error("three", "x", &[]),
        "`three` needs `a`, which was not given"
    );
    assert_eq!(
        error(
            "three",
            "x",
            &[("a", "a"), ("b", "b"), ("c", "c"), ("d", "d")]
        ),
        "`three` has no invocation parameter `d`; note: its invocation parameters are `a`, \
             `b`, `c`"
    );
    assert_eq!(
        error("two", "x", &[("a", "a"), ("b", "b"), ("c", "c")]),
        "`two` has no invocation parameter `c`; note: its invocation parameters are `a`, `b`"
    );
    // A required parameter after an optional one: it alone must be
    // given.
    assert_eq!(
        error("first", "x", &[("a", "a")]),
        "`first` needs `b`, which was not given"
    );
    assert_eq!(
        error("first", "x", &[]),
        "`first` needs `b`, which was not given"
    );
    // A parameter of one of the four types, with a default, left out.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def count(name: String, f: fn(): String, n: Int = 1): String { \"{f()} {n}\" }",
        ),
        ("defs.gcsl", "def count x() { \"x\" }"),
    ];
    assert_eq!(invoked_text_with(&files, ("count", "x"), &[]), "x 1");
    assert_eq!(
        invoked_text_with(&files, ("count", "x"), &[("n", "5")]),
        "x 5"
    );
}

/// A def-type with an invocation parameter of each of the four types
/// that can be given, each with a default, and one of another type with
/// a default; an impure one whose bare `Int` is either-shaped; and one
/// whose parameter is an alias of `Int`.
pub(super) const TYPED: [(&str, &str); 2] = [
    (
        "workspace.gcsl",
        "type Count = Int\n\
             #[def_type]\n\
             def typed(name: String, f: fn(): String, s: String = \"-\", i: Int = 0, x: Float = 0.5, \
             b: Bool = false, tags: List<String> = []) -> String {\n\
             \x20   \"{f()} {s} {i} {x} {b} {len(tags)}\"\n\
             }\n\
             #[def_type]\n\
             def later(name: String, f: fn(): String, n: Int) -> String { \"{f()} {n + 1}\" }\n\
             #[def_type]\n\
             def aliased(name: String, f: fn(): String, n: Count): String { \"{f()} {n}\" }",
    ),
    (
        "defs.gcsl",
        "def typed x() { \"x\" }\ndef later x() { \"x\" }\ndef aliased x() { \"x\" }",
    ),
];

#[test]
fn each_of_the_four_types_is_read_from_its_text() {
    let files = TYPED;
    let typed = |args: &[(&str, &str)]| invoked_text_with(&files, ("typed", "x"), args);
    assert_eq!(typed(&[]), "x - 0 0.5 false 0");
    // A `String` as it is, an empty one too.
    assert_eq!(typed(&[("s", "a b,c=d")]), "x a b,c=d 0 0.5 false 0");
    assert_eq!(typed(&[("s", "")]), "x  0 0.5 false 0");
    // An `Int` in decimal, `-` before it if negative, in range.
    for (text, shown) in [
        ("4", "4"),
        ("-4", "-4"),
        ("007", "7"),
        ("9223372036854775807", "9223372036854775807"),
        ("-9223372036854775808", "-9223372036854775808"),
    ] {
        assert_eq!(typed(&[("i", text)]), format!("x - {shown} 0.5 false 0"));
    }
    // A `Float` as Rust's `f64` reads it, finite.
    for (text, shown) in [
        ("1.5", "1.5"),
        ("2", "2.0"),
        ("-0.25", "-0.25"),
        ("1e3", "1000.0"),
    ] {
        assert_eq!(typed(&[("x", text)]), format!("x - 0 {shown} false 0"));
    }
    // A `Bool`, `true` or `false`.
    assert_eq!(typed(&[("b", "true")]), "x - 0 0.5 true 0");
    assert_eq!(typed(&[("b", "false")]), "x - 0 0.5 false 0");
    // All at once, in any order.
    assert_eq!(
        typed(&[("b", "true"), ("x", "3.25"), ("i", "-1"), ("s", "s")]),
        "x s -1 3.25 true 0"
    );
    // An impure def-type's bare `Int`, and an alias of `Int`.
    assert_eq!(
        invoked_text_with(&files, ("later", "x"), &[("n", "41")]),
        "x 42"
    );
    assert_eq!(
        invoked_text_with(&files, ("aliased", "x"), &[("n", "3")]),
        "x 3"
    );
}

#[test]
fn a_text_that_is_not_a_value_of_its_type_is_an_error() {
    let files = TYPED;
    let error =
        |name: &str, text: &str| request_error_with(&files, ("typed", "x"), &[(name, text)]);
    let int = "note: an `Int` is written as a decimal integer, with `-` before it if it is \
                   negative, from -9223372036854775808 to 9223372036854775807";
    for text in [
        "9223372036854775808",
        "-9223372036854775809",
        "1.5",
        "+1",
        "",
        "-",
        " 1",
        "1_0",
        "0x10",
    ] {
        assert_eq!(
            error("i", text),
            format!("`{text}` is not an `Int`, the type of `i` of `typed x`; {int}")
        );
    }
    let float = "note: a `Float` is written as a decimal number, `1.5` or `2`, and must be \
                     finite";
    for text in ["nan", "NaN", "inf", "-inf", "infinity", "1e400", "x", ""] {
        assert_eq!(
            error("x", text),
            format!("`{text}` is not a `Float`, the type of `x` of `typed x`; {float}")
        );
    }
    let bool = "note: a `Bool` is written `true` or `false`";
    for text in ["True", "FALSE", "1", "yes", ""] {
        assert_eq!(
            error("b", text),
            format!("`{text}` is not a `Bool`, the type of `b` of `typed x`; {bool}")
        );
    }
    // A parameter of another type, with a default: left out, fine;
    // given, it cannot be.
    assert_eq!(
        error("tags", "a"),
        "`typed`'s parameter `tags` is `List<String>`, but only `String`, `Int`, `Float`, \
             `Bool`, `Duration` and `Size` parameters can be given here yet"
    );
    // An alias, by the type it names.
    assert_eq!(
        request_error_with(&files, ("aliased", "x"), &[("n", "three")]),
        format!("`three` is not an `Int`, the type of `n` of `aliased x`; {int}")
    );
}

#[test]
fn a_duration_or_a_size_can_be_given_and_read_from_text() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def job(name: String, f: fn(): String, timeout: Duration = 30s, memory: Size) -> \
                 String {\n\
                 \x20   \"{f()} {timeout} {memory}\"\n\
                 }",
        ),
        ("defs.gcsl", "def job x() { \"x\" }"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let given = [
        (
            "timeout".to_owned(),
            Given::Quantity(Quantity::Duration, 90_000_000_000),
        ),
        (
            "memory".to_owned(),
            Given::Quantity(Quantity::Size, 2 << 30),
        ),
    ];
    let outcome = workspace.invoke("job", "x", &given).unwrap();
    assert_eq!(outcome.value.to_string(), "x 1m30s 2GiB");
    assert_eq!(given[0].1.to_string(), "1m30s");
    // From text, as a command line gives it: a literal, negated or not.
    let value = |args: &[(&str, &str)]| match invoke_with(&files, ("job", "x"), args) {
        Ok(value) => value.to_string(),
        Err(error) => panic!("{args:?}: {error:?}"),
    };
    assert_eq!(value(&[("memory", "512MiB")]), "x 30s 512MiB");
    assert_eq!(value(&[("timeout", "-1h"), ("memory", "1B")]), "x -1h 1B");
    // Not a literal of its type, or not a literal alone.
    let duration = "note: a `Duration` is written as a duration literal, `30s` or `1h30m`, with \
                    `-` before it if it is negative";
    for text in ["30", "2GiB", "30 s", "30sec", "1m1h", " 1s", ""] {
        assert_eq!(
            request_error_with(&files, ("job", "x"), &[("timeout", text), ("memory", "1B")]),
            format!("`{text}` is not a `Duration`, the type of `timeout` of `job x`; {duration}")
        );
    }
    let size = "note: a `Size` is written as a size literal, `512B` or `2GiB`, with `-` before \
                it if it is negative";
    assert_eq!(
        request_error_with(&files, ("job", "x"), &[("memory", "2gib")]),
        format!("`2gib` is not a `Size`, the type of `memory` of `job x`; {size}")
    );
}

#[test]
fn invoke_takes_typed_values_by_name() {
    let mut workspace = rooted(&TYPED).unwrap();
    let mut invoke = |args: &[(&str, Given)]| {
        let args: Vec<(String, Given)> = args
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect();
        workspace
            .invoke("typed", "x", &args)
            .map(|outcome| outcome.value.to_string())
    };
    assert_eq!(
        invoke(&[
            ("i", Given::Int(i64::MIN)),
            ("x", Given::Float(-1.5)),
            ("b", Given::Bool(true)),
            ("s", Given::Str("{s}".to_owned())),
        ]),
        Ok("x {s} -9223372036854775808 -1.5 true 0".to_owned())
    );
    // A value of another type than its parameter's, or not finite, as
    // the text it shows as.
    let error = |result: Result<String, InvokeError>| match result {
        Err(InvokeError::Request(error)) => error.message(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        error(invoke(&[("i", Given::Str("4".to_owned()))])),
        "`4` is not an `Int`, the type of `i` of `typed x`"
    );
    assert_eq!(
        error(invoke(&[("s", Given::Int(4))])),
        "`4` is not a `String`, the type of `s` of `typed x`"
    );
    assert_eq!(
        error(invoke(&[("x", Given::Float(f64::NAN))])),
        "`NaN` is not a `Float`, the type of `x` of `typed x`"
    );
    assert_eq!(
        error(invoke(&[("x", Given::Int(1))])),
        "`1` is not a `Float`, the type of `x` of `typed x`"
    );
    assert_eq!(
        error(invoke(&[("i", Given::Int(1)), ("i", Given::Int(2))])),
        "`typed`'s parameter `i` is given more than once"
    );
    assert_eq!(
        error(invoke(&[("n", Given::Int(1))])),
        "`typed` has no invocation parameter `n`"
    );
    // Shown as text.
    assert_eq!(Given::Str("a b".to_owned()).to_string(), "a b");
    assert_eq!(Given::Int(-3).to_string(), "-3");
    assert_eq!(Given::Float(2.0).to_string(), "2");
    assert_eq!(Given::Bool(false).to_string(), "false");
}

#[test]
fn arguments_are_checked_without_running_anything() {
    // The definition's file panics when it runs: reading the
    // arguments runs nothing; invoking runs it.
    let files = [
        TYPED[0],
        (
            "defs.gcsl",
            "def typed x() { \"x\" }\ndef later x() { \"x\" }\n\
                 def aliased x() { \"x\" }\nboom = panic(\"ran\")",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let read = workspace.arguments("typed", "x", &[("i", Some("4")), ("b", Some("true"))]);
    assert_eq!(
        read.unwrap(),
        [
            ("i".to_owned(), Given::Int(4)),
            ("b".to_owned(), Given::Bool(true))
        ]
    );
    // A name with no text is checked as given, and left out.
    let read = workspace.arguments("later", "x", &[("n", None)]);
    assert_eq!(read.unwrap(), []);
    let request = |result: Result<Vec<(String, Given)>, InvokeError>| match result {
        Err(InvokeError::Request(error)) => error.message(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        request(workspace.arguments("later", "x", &[])),
        "`later` needs `n`, which was not given"
    );
    assert_eq!(
        request(workspace.arguments("later", "x", &[("n", None), ("n", Some("1"))])),
        "`later`'s parameter `n` is given more than once"
    );
    assert_eq!(
        request(workspace.arguments("later", "x", &[("m", None)])),
        "`later` has no invocation parameter `m`"
    );
    assert_eq!(
        request(workspace.arguments("typed", "x", &[("tags", None)])),
        "`typed`'s parameter `tags` is `List<String>`, but only `String`, `Int`, `Float`, \
             `Bool`, `Duration` and `Size` parameters can be given here yet"
    );
    // What is asked for, before its arguments.
    assert_eq!(
        request(workspace.arguments("later", "y", &[("m", None)])),
        "`later` has no definition `y`"
    );
    assert_eq!(
        request(workspace.arguments("nope", "x", &[])),
        "no def-type `nope`"
    );
    // Nothing ran: invoking runs the file, which panics.
    let invoked = workspace.invoke("later", "x", &[("n".to_owned(), Given::Int(1))]);
    let Err(InvokeError::Diagnostics(errors)) = invoked else {
        panic!("{invoked:?}");
    };
    assert_eq!(errors[0].message, "ran");
}

#[test]
fn definitions_of_two_directories_invoked_on_one_workspace() {
    // Each directory shadows a `#[shadowable]` field with a file of its
    // own, checked before the workspace file but run, for the second
    // invocation, after it: `b/lock.gcsl` must neither cut the frame
    // back to its own, nor write its temporaries (its list's record's
    // fields) over the def-type's slot.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def run(name: String, f: fn() -> String) -> String {\n\
                 \x20   \"{name} {f()}\"\n\
                 }\n\
                 #[shadowable]\n\
                 x: List<{ v: String, d: String }> = []",
        ),
        ("a/a.gcsl", "def run a() { \"A {x[0].v}\" }"),
        (
            "a/lock.gcsl",
            "x: List<{ v: String, d: String }> = [{ v = \"xa\"; d = \"1\" }]",
        ),
        ("b/b.gcsl", "def run b() { \"B {x[0].v}\" }"),
        (
            "b/lock.gcsl",
            "x: List<{ v: String, d: String }> = [{ v = \"xb\"; d = \"2\" }]",
        ),
    ];
    for order in [["a", "b"], ["b", "a"]] {
        let mut workspace = rooted(&files).unwrap();
        for path in order {
            let outcome = workspace.invoke("run", path, &[]);
            let value = outcome.unwrap_or_else(|error| panic!("{order:?}: {error:?}"));
            let upper = path.to_uppercase();
            assert_eq!(value.value.to_string(), format!("{path} {upper} x{path}"));
        }
    }
}

#[test]
fn a_def_with_defaults_gives_them_from_another_file() {
    // Called by its name from a directory below, by its path from
    // another file, through a member import and through its module.
    let files = [
        (
            "workspace.gcsl",
            "def greet(name: String, greeting: String = \"hello\"): String {\n\
                 \x20   \"{greeting}, {name}\"\n\
                 }\n\
                 def dev.peios.pad(s: String, n: Int = 2, tail: List<String> = [\"!\"]): String {\n\
                 \x20   \"{s}{n}{tail[0]}\"\n\
                 }",
        ),
        (
            "lib/strings.gcsl",
            "def shout(s: String, mark: {m: String} = { m = \"!\" }): String { \"{s}{mark.m}\" }",
        ),
        (
            "app/main.gcsl",
            "import lib/strings\n\
                 import shout from lib/strings\n\
                 a = greet(\"jack\")\n\
                 b = dev.peios.pad(\"x\")\n\
                 c = dev.peios.pad(\"x\", tail = [\"?\"])\n\
                 d = strings.shout(\"hey\")\n\
                 e = shout(\"ho\")\n\
                 f = shout(\"ho\", { m = \"?\" })",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"hello, jack\"; b = \"x2!\"; c = \"x2?\"; d = \"hey!\"; e = \"ho!\"; \
             f = \"ho?\"}"
    );
    // Through a copy, from another file too, every parameter is filled.
    let files = [
        files[0],
        files[1],
        (
            "app/main.gcsl",
            "import shout from lib/strings\nlet s = shout\na = s(\"ho\")",
        ),
    ];
    assert_eq!(rooted_messages(&files), ["expected 2 arguments, found 1"]);
}

#[test]
fn a_def_s_default_that_is_no_constant_runs_for_another_file() {
    // The default reads its own file's `suffix`, which the calling file
    // cannot see, and the parameter before it; `fileroot.caller` in it is
    // the calling file's directory, as it would be written at the call.
    let files = [
        (
            "workspace.gcsl",
            "import fileroot\n\
                 let suffix = \"!\"\n\
                 def greet(name: String, line: String = \"hi {name}{suffix}\"): String { line }\n\
                 def from(dir: String = fileroot.caller): String { dir }",
        ),
        (
            "app/main.gcsl",
            "a = greet(\"jack\")\nb = greet(\"jack\", \"yo\")\nc = from()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"hi jack!\"; b = \"yo\"; c = \"<root>/app\"}"
    );
}

#[test]
fn a_declared_types_field_defaults_reach_every_file() {
    // A type's defaults are copied out of the file declaring it into
    // each file using it, by its name from a directory below and
    // through an import: an integer default of a `Float` field is a
    // `Float` there, and a record default is given the defaults of its
    // own type, as if written there. A record literal written where the
    // type is expected is given them.
    let files = [
        (
            "workspace.gcsl",
            "type Install = { from: String, to: String, exclude: List<String> = [] }\n\
                 def count(e: Install): Int { len(e.exclude) }",
        ),
        (
            "lib/types.gcsl",
            "type Opt = { level: Float = 1, inner: {b: Int, c: Int = 3} = { b = 2 }, name: String,\n\
                 \x20   flags: {debug: Bool = false} = {} }\n\
                 type Flags = { debug: Bool = false, level: Int = 1 }",
        ),
        (
            "app/main.gcsl",
            "import lib/types\n\
                 entries: List<Install> = [\n\
                 \x20   { from = \"a\"; to = \"b\" },\n\
                 \x20   { from = \"c\"; to = \"d\"; exclude = [\"x\"] },\n\
                 ]\n\
                 n = count({ from = \"a\"; to = \"b\" })\n\
                 o: types.Opt = { name = \"o\" }\n\
                 type Both = { ...types.Opt, extra: Bool = true }\n\
                 b: Both = { name = \"b\" }\n\
                 f: types.Flags = {}",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{entries = [{from = \"a\"; to = \"b\"; exclude = []}, \
             {from = \"c\"; to = \"d\"; exclude = [\"x\"]}]; n = 0; \
             o = {name = \"o\"; flags = {debug = false}; inner = {b = 2; c = 3}; level = 1.0}; \
             b = {name = \"b\"; extra = true; flags = {debug = false}; inner = {b = 2; c = 3}; \
             level = 1.0}; f = {debug = false; level = 1}}"
    );
    // What is wrong in a default is reported once, where it is written;
    // a file using the type fails quietly.
    let files = [
        ("workspace.gcsl", "type T = { a: Int = \"x\" }"),
        ("app/main.gcsl", "t: T = { a = 1 }"),
    ];
    assert_eq!(rooted_messages(&files), ["mismatched types"]);
    let files = [
        ("workspace.gcsl", "let k = 1\ntype T = { a: Int = k }"),
        ("app/main.gcsl", "t: T = { a = 1 }"),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["a field's default must be a constant"]
    );
}

#[test]
fn an_invoked_definition_is_called_as_from_its_directory() {
    // The def-type's caller is the definition's directory; without
    // `fileroot.helper()` in the def-type, the definition's is the
    // def-type's file's directory, as for any call.
    let files = [
        (
            "workspace.gcsl",
            "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn() -> String) -> { called: String, defined: String } {\n\
                 \x20   called = fileroot.caller\n\
                 \x20   defined = f()\n\
                 }",
        ),
        (
            "pkg/deep/p.gcsl",
            "import fileroot\ndef where x() { fileroot.caller }",
        ),
    ];
    assert_eq!(
        invoked_text(&files, "where", "x"),
        "{called = \"<root>/pkg/deep\"; defined = \"<root>\"}"
    );
}

#[test]
fn invoking_runs_the_files_once_and_the_call_each_time() {
    // A log the definition's file writes to as it runs, and each call of
    // the impure def-type, and of the memoised one, once for its
    // arguments.
    let dir = std::env::temp_dir().join(format!("gcsl-invoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log").display().to_string();
    let def_types = format!(
        "#[def_type]\n\
             def logged(name: String, f: fn(): String) -> String {{\n\
             \x20   let out = sh(\"echo {{name}} >> {log}\")\n\
             \x20   \"{{out}}{{f()}}\"\n\
             }}\n\
             #[def_type]\n\
             def cached(name: String, f: fn(): String) ->> String {{\n\
             \x20   let out = sh(\"echo cached {{name}} >> {log}\")\n\
             \x20   \"{{out}}{{f()}}\"\n\
             }}\n\
             #[def_type]\n\
             def pure(name: String, f: fn(): String): String {{ \"pure {{f()}}\" }}"
    );
    let package = format!(
        "x = sh(\"echo file >> {log}\")\n\
             def logged p() {{ \"l\" }}\n\
             def cached p() {{ \"c\" }}\n\
             def pure p() {{ \"p\" }}"
    );
    let files = [
        ("workspace.gcsl", def_types.as_str()),
        ("pkg/p.gcsl", package.as_str()),
    ];
    let mut workspace = rooted(&files).unwrap();
    let mut invoke = |def_type: &str| {
        workspace
            .invoke(def_type, "p", &[])
            .unwrap()
            .value
            .to_string()
    };
    for _ in 0..2 {
        assert_eq!(invoke("logged"), "l");
        assert_eq!(invoke("cached"), "c");
        assert_eq!(invoke("pure"), "pure p");
    }
    let written = std::fs::read_to_string(&log).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(written, "file\np\ncached p\np\n");
}

#[test]
fn a_runtime_error_of_an_invoked_definition_names_its_file() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }",
        ),
        ("pkg/p.gcsl", "def run p() { panic(\"boom\") }"),
    ];
    assert_eq!(invoke_errors(&files, "run", "p"), ["boom at pkg/p.gcsl"]);
    // A file that runs first fails as `run` reports it.
    let files = [
        files[0],
        ("pkg/p.gcsl", "def run p() { \"p\" }\nx = [1][3]"),
    ];
    assert_eq!(
        invoke_errors(&files, "run", "p"),
        ["index out of range at pkg/p.gcsl (the index is 3, and the list has 1 element)"]
    );
}

#[test]
fn an_error_of_the_call_itself_is_at_the_definition_s_path() {
    // A def-type whose body is nested so deeply that a call of it from
    // a top level, as the invocation is, takes the nesting past the
    // limit: "call nested too deeply", at the call, which has no source.
    let depth = crate::MAX_DEPTH - 4;
    let body = format!("1{}", " + 1".repeat(depth - 1));
    let def_type = format!("#[def_type]\ndef deep(name: String, f: fn(): Int): Int {{ {body} }}");
    let (shown, errors) = std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(move || {
            let files = [
                ("workspace.gcsl", def_type.as_str()),
                ("pkg/p.gcsl", "def deep p() { 1 }"),
            ];
            let shown = invoke_errors(&files, "deep", "p");
            let mut workspace = rooted(&files).unwrap();
            let Err(InvokeError::Diagnostics(errors)) = workspace.invoke("deep", "p", &[]) else {
                panic!("no error");
            };
            (shown, errors)
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        shown,
        [
            "call nested too deeply: the limit is 10000 levels of nesting, calls included \
                 at pkg/p.gcsl (invoked here); note: every level of expression nesting around \
                 a call counts, and so does the body of the function called; GCSL does not \
                 eliminate tail calls, so recursion this deep is not supported"
        ]
    );
    // At the definition's path, `p`, with nothing elsewhere.
    assert_eq!(errors[0].span, Span::new(9, 10));
    assert!(errors[0].labels.is_empty());
}

/// A def-type of the root of a workspace, for the metas' tests:
/// `tagged`, whose definitions give a string and carry a pure meta of a
/// tag and a number, read through functions that take less of it: one
/// whose meta has the tag alone, and one with no meta.
#[test]
fn a_field_of_a_definition_s_meta_is_read_by_name() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(v: String): String META-> {versions: List<String>}, \
                 version: String) -> String { f(version) }\n\
                 #[def_type]\n\
                 def plain(name: String, f: fn(): String): String { f() }",
        ),
        (
            "app/a.gcsl",
            "defmeta package dev.x { versions = [\"0.1.0\", \"0.2.0\"] }\n\
                 def package dev.x(v) { v }\n\
                 def plain y() { \"y\" }",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    // Read from the definition's directory, its `defmeta` run.
    let versions = workspace.meta_field("package", "dev.x", "versions");
    assert_eq!(
        versions.unwrap().unwrap().value.to_string(),
        "[\"0.1.0\", \"0.2.0\"]"
    );
    // No such field, and no meta: nothing to read.
    assert!(
        workspace
            .meta_field("package", "dev.x", "nope")
            .unwrap()
            .is_none()
    );
    assert!(
        workspace
            .meta_field("plain", "y", "versions")
            .unwrap()
            .is_none()
    );
    // What cannot be invoked cannot be read either.
    assert_eq!(
        workspace
            .meta_field("package", "dev.nope", "versions")
            .unwrap_err(),
        InvokeError::Request(RequestError::NoDefinition {
            def_type: "package".to_owned(),
            path: "dev.nope".to_owned(),
            definitions: vec!["dev.x".to_owned()],
        })
    );
    // Invoking is unchanged.
    let value = workspace.invoke("package", "dev.x", &[string("version", "0.1.0")]);
    assert_eq!(value.unwrap().value.to_string(), "0.1.0");
}
