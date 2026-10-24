//! Tests of the `gcsl` command: warnings and errors in source order, dumps
//! before errors, and syntax and scan errors.

use super::*;

/// The warning `tests/fixtures/warning.gcsl` produces, as rendered.
pub(super) const FIXTURE_WARNING: &str = "warning: field `z` is not in `{x: Int}`\n\
     \x20--> tests/fixtures/warning.gcsl:1:32\n\
     \x20 |\n\
     1 | let point: {x: Int} = { x = 1; z = 2 }\n\
     \x20 |            --------            ^\n\
     \x20 |            |\n\
     \x20 |            expected because of this annotation\n\
     \x20 |\n\
     \x20 = note: it is allowed, but nothing with this type can see it\n";

#[test]
fn warnings_go_to_stderr_and_the_value_still_prints() {
    let output = gcsl(&["run", "tests/fixtures/warning.gcsl"]);
    assert_eq!(stdout(&output), "1\n");
    assert_eq!(stderr(&output), FIXTURE_WARNING);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn with_errors_warnings_go_to_stderr_with_them_in_source_order() {
    let output = gcsl(&["run", "tests/fixtures/warning_and_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "warning: field `z` is not in `{x: Int}`\n\
         \x20--> tests/fixtures/warning_and_error.gcsl:1:32\n\
         \x20 |\n\
         1 | let point: {x: Int} = { x = 1; z = 2 }\n\
         \x20 |            --------            ^\n\
         \x20 |            |\n\
         \x20 |            expected because of this annotation\n\
         \x20 |\n\
         \x20 = note: it is allowed, but nothing with this type can see it\n\
         \n\
         error: no field `z` in `{x: Int}`\n\
         \x20--> tests/fixtures/warning_and_error.gcsl:2:8\n\
         \x20 |\n\
         2 | = point.z\n\
         \x20 |        ^^\n\
         \x20 |\n\
         \x20 = note: the only field is `x`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn dumps_show_record_types_and_record_ir() {
    let output = gcsl(&["run", "tests/fixtures/warning.gcsl", "--dump-types"]);
    assert_eq!(
        stdout(&output),
        "(file\n\
         \x20 (let point : {x: Int} @0..38\n\
         \x20   (block : {x: Int, z: Int} @22..38\n\
         \x20     (field x @24..29\n\
         \x20       (int 1 : Int @28..29))\n\
         \x20     (field z @31..36\n\
         \x20       (int 2 : Int @35..36))))\n\
         \x20 (result @39..48\n\
         \x20   (. x : Int @41..48\n\
         \x20     (name point : {x: Int} @41..46))))\n"
    );
    // A run that stops after a dump still reports its warnings.
    assert_eq!(stderr(&output), FIXTURE_WARNING);
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl(&["run", "tests/fixtures/record.gcsl", "--dump-ir"]);
    assert_eq!(
        stdout(&output),
        "(block @32..45\n\
         \x20 (store x#77 @32..37\n\
         \x20   (int 1 @36..37))\n\
         \x20 (store y#78 @38..45\n\
         \x20   (str \"a\" @42..45))\n\
         \x20 (make-record [x y] @32..45\n\
         \x20   (load x#77 @32..33)\n\
         \x20   (load y#78 @38..39)))\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dumps_show_names_and_slots() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/empty.gcsl",
        "--dump-tokens",
        "--dump-ir",
    ]);
    assert_eq!(
        stdout(&output),
        "Let @0..3\n\
         Ident(count) @4..9\n\
         Eq @10..11\n\
         Int(1) @12..13\n\
         Newline @13..14\n\
         Ident(count) @14..19\n\
         ColonEq @20..22\n\
         Ident(count) @23..28\n\
         Plus @29..30\n\
         Int(1) @31..32\n\
         Newline @32..33\n\
         Eof @32..32\n\
         (block @0..32\n\
         \x20 (store count#119 @0..13\n\
         \x20   (int 1 @12..13))\n\
         \x20 (store count#120 @14..32\n\
         \x20   (int-add @23..32\n\
         \x20     (load count#119 @23..28)\n\
         \x20     (int 1 @31..32)))\n\
         \x20 (empty @0..32))\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_stops_before_a_runtime_error() {
    let output = gcsl(&["run", "tests/fixtures/runtime_error.gcsl", "--dump-ir"]);
    assert!(stdout(&output).starts_with("(block @0..36\n  (int-add @0..36\n"));
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn dump_then_error_prints_dumped_stages_and_the_error() {
    let output = gcsl(&[
        "run",
        "tests/fixtures/type_error.gcsl",
        "--dump-ast",
        "--dump-ir",
    ]);
    assert!(stdout(&output).starts_with("(file\n  (expr @0..16\n    (+ @0..16\n"));
    assert!(!stdout(&output).contains("float-"));
    assert!(stderr(&output).starts_with("error: cannot floor-divide Float\n"));
    assert_eq!(output.status.code(), Some(1));
}

/// The binary runs the pipeline on a thread with enough stack for the
/// deepest accepted expression, whatever the main thread's stack is.
#[test]
fn deepest_accepted_expression_runs() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("deep.gcsl");
    std::fs::write(&path, format!("1{}", " + 1".repeat(gcsl::MAX_DEPTH - 1))).unwrap();
    let output = gcsl(&["run", path.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(stderr(&output), "");
    assert_eq!(stdout(&output), format!("{}\n", gcsl::MAX_DEPTH));
    assert_eq!(output.status.code(), Some(0));
}

/// A syntax error in one file of a workspace, or in the file itself, runs
/// nothing: neither a file's top-level `sh` nor the file asked for. Every
/// syntax error of the workspace is rendered, and a type error of a file
/// that parses with them.
#[test]
fn a_syntax_error_anywhere_runs_nothing() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-syntax-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let marker = |name: &str| dir.join(name).to_str().unwrap().to_owned();
    let files = [
        ("workspace.gcsl", String::new()),
        (
            "top.gcsl",
            format!("z = sh(\"touch {}\")\nw: Int = \"s\"", marker("TOP")),
        ),
        (
            "bad.gcsl",
            format!("x = sh(\"touch {}\")\ny = (1 2)\nv = [3 3]", marker("BAD")),
        ),
    ];
    for (path, text) in &files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    let outputs: Vec<Output> = ["top.gcsl", "bad.gcsl"]
        .iter()
        .map(|file| gcsl(&["run", dir.join(file).to_str().unwrap()]))
        .collect();
    let markers = [dir.join("TOP").exists(), dir.join("BAD").exists()];
    // Without the errors, `top.gcsl` runs.
    std::fs::write(dir.join("bad.gcsl"), "x = 1").unwrap();
    std::fs::write(
        dir.join("top.gcsl"),
        &files[1].1[..files[1].1.find('\n').unwrap()],
    )
    .unwrap();
    let fixed = gcsl(&["run", dir.join("top.gcsl").to_str().unwrap()]);
    let ran = dir.join("TOP").exists();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(markers, [false, false]);
    assert_eq!(stderr(&fixed), "");
    assert!(ran);
    for output in &outputs {
        assert_eq!(stdout(output), "");
        let errors: Vec<&str> = stderr(output)
            .lines()
            .filter(|line| line.starts_with("error"))
            .collect();
        assert_eq!(
            errors,
            [
                "error: expected `)`, found `2`",
                "error: expected `,` or `]`, found `3`",
                "error: mismatched types",
            ],
            "{}",
            stderr(output)
        );
        assert_eq!(output.status.code(), Some(1));
    }
}

/// An error of the scan of a workspace (a field defined in two files, an
/// import of no module, files that need each other) runs nothing: neither a
/// file's top-level `sh`, in a file with the error or in another, nor the
/// file asked for. Every error of the scan is rendered, and a type error of
/// another file.
#[test]
fn a_scan_error_anywhere_runs_nothing() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("q")).unwrap();
    std::fs::create_dir_all(dir.join("cy")).unwrap();
    let marker = |name: &str| dir.join(name).to_str().unwrap().to_owned();
    let files = [
        ("workspace.gcsl", String::new()),
        (
            "top.gcsl",
            format!("z = sh(\"touch {}\")\nw: Int = \"s\"", marker("TOP")),
        ),
        ("q/a.gcsl", format!("x = sh(\"touch {}\")", marker("A"))),
        (
            "q/b.gcsl",
            format!("import nope\nx = sh(\"touch {}\")", marker("B")),
        ),
        (
            "cy/c.gcsl",
            format!("c = d\ne = sh(\"touch {}\")", marker("C")),
        ),
        ("cy/d.gcsl", "d = c".to_owned()),
    ];
    for (path, text) in &files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    let outputs: Vec<Output> = ["top.gcsl", "q/a.gcsl", "q/b.gcsl", "cy/c.gcsl"]
        .iter()
        .map(|file| gcsl(&["run", dir.join(file).to_str().unwrap()]))
        .collect();
    let markers = ["TOP", "A", "B", "C"].map(|name| dir.join(name).exists());
    // Without the errors, `top.gcsl` runs.
    std::fs::write(dir.join("q/b.gcsl"), "y = 1").unwrap();
    std::fs::write(dir.join("cy/d.gcsl"), "d = 1").unwrap();
    std::fs::write(
        dir.join("top.gcsl"),
        &files[1].1[..files[1].1.find('\n').unwrap()],
    )
    .unwrap();
    let fixed = gcsl(&["run", dir.join("top.gcsl").to_str().unwrap()]);
    let ran = dir.join("TOP").exists();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(markers, [false; 4]);
    assert_eq!(stderr(&fixed), "");
    assert!(ran);
    for output in &outputs {
        assert_eq!(stdout(output), "");
        let errors: Vec<&str> = stderr(output)
            .lines()
            .filter(|line| line.starts_with("error"))
            .collect();
        assert_eq!(
            errors,
            [
                "error: `cy/c.gcsl` and `cy/d.gcsl` need each other",
                "error: no module `nope`",
                "error: field `x` is defined in two files",
                "error: mismatched types",
            ],
            "{}",
            stderr(output)
        );
        assert_eq!(output.status.code(), Some(1));
    }
}

/// `--dump-ast` of a file with syntax errors prints nothing and shows them
/// all.
#[test]
fn dump_ast_shows_every_syntax_error() {
    let dir = std::env::temp_dir().join(format!("gcsl-cli-dump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("two.gcsl");
    std::fs::write(&path, "x = (1 2)\ny = 3\nz = [4 4]").unwrap();
    let output = gcsl(&["run", path.to_str().unwrap(), "--dump-ast"]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(stdout(&output), "");
    let errors: Vec<&str> = stderr(&output)
        .lines()
        .filter(|line| line.starts_with("error"))
        .collect();
    assert_eq!(
        errors,
        [
            "error: expected `)`, found `2`",
            "error: expected `,` or `]`, found `4`"
        ]
    );
    assert_eq!(output.status.code(), Some(1));
}
