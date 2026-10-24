//! Tests of workspaces: the summary, needs and merging across files, and
//! benchmarks of chains of files and imports.

use super::*;

#[test]
fn the_summary_runs_in_need_order() {
    let scan = ws(&SUMMARY).unwrap();
    let paths: Vec<&str> = scan.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "<stdlib>/prelude.gcsl",
            "app.gcsl",
            "common.gcsl",
            "lib.gcsl",
            "workspace.gcsl"
        ]
    );
    assert_eq!(scan.needs, [vec![], vec![2, 3], vec![], vec![2], vec![]]);
    assert_eq!(scan.order, [0, 2, 3, 1, 4]);
    assert_eq!(scan.needed(1), [0, 2, 3, 1]);
    assert_eq!(scan.needed(3), [0, 2, 3]);
    assert_eq!(scan.needed(4), [0, 4]);
}

#[test]
fn a_reference_means_its_prefix_in_the_merged_tree() {
    let scan = ws(&SUMMARY).unwrap();
    let app = &scan.files[1];
    // `com.x.lib` in app's second item is lib's interior node, not
    // app's own `com.x`.
    let foreign = &app.mentions.foreign[1];
    assert_eq!(foreign.len(), 1);
    let Need::Field(node) = foreign[0].0 else {
        panic!("{foreign:?}")
    };
    let merged = FileScope::new(&scan, app).merged(node).unwrap();
    assert_eq!(scan.show(merged), "com.x.lib");
    assert_eq!(scan.nodes[merged as usize].contributors, [3]);
}

#[test]
fn dotted_paths_merge_and_the_last_contributor_assembles() {
    let scan = ws(&[("a.gcsl", "com.a = 1"), ("b.gcsl", "all = com\ncom.b = 2")]).unwrap();
    assert_eq!(scan.order, [0, 1, 2]);
    let com = scan.by_name[&(0, ROOT, scan.files[1].mentions.tree.nodes[0].name)];
    assert_eq!(scan.nodes[com as usize].contributors, [1, 2]);
    assert_eq!(scan.computed_by(com), Some(2));
    // a leaves `com` out; b computes it before its first mention.
    assert!(!scan.files[1].schedule.order.contains(&0));
    let b = &scan.files[2];
    let local = b.global.iter().position(|&g| g == com).unwrap();
    assert!(b.schedule.order.contains(&count(local)));
    assert_eq!(b.schedule.point[local], 0);
}

#[test]
fn a_shared_node_read_whole_from_a_third_file() {
    let scan = ws(&[
        ("a.gcsl", "com.a = 1"),
        ("b.gcsl", "com.b = 2\nz = 3"),
        ("c.gcsl", "all = com"),
    ])
    .unwrap();
    assert_eq!(scan.needs, [vec![], vec![], vec![], vec![1, 2]]);
    // b, the last contributor, assembles `com` after its last child.
    let b = &scan.files[2];
    assert_eq!(b.schedule.point[0], 1);
    assert!(!scan.files[1].schedule.order.contains(&0));
}

#[test]
fn a_def_of_one_file_called_from_another_is_a_need() {
    let scan = ws(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "x = f()")]).unwrap();
    assert_eq!(scan.needs, [vec![], vec![], vec![1]]);
}

#[test]
fn siblings_and_subdirectories_are_hidden() {
    let scan = ws(&[("x/a.gcsl", "v = 1"), ("y/b.gcsl", "w = v")]).unwrap();
    assert_eq!(scan.needs, [vec![], vec![], vec![]]);
    let scan = ws(&[("a.gcsl", "w = v"), ("sub/b.gcsl", "v = 1")]).unwrap();
    assert_eq!(scan.needs, [vec![], vec![], vec![]]);
    let scan = ws(&[("a.gcsl", "v = 1"), ("sub/b.gcsl", "w = v")]).unwrap();
    assert_eq!(scan.needs, [vec![], vec![], vec![1]]);
}

#[test]
fn conflicts_between_files() {
    assert_eq!(
        error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x.y = 2")]),
        (
            "field `x.y` is defined in two files".to_owned(),
            Some(2),
            vec![Some(1)]
        )
    );
    assert_eq!(
        error(&[("a.gcsl", "x = 1"), ("b.gcsl", "x.y = 2")]).0,
        "field `x` is already defined"
    );
    assert_eq!(
        error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x = 2")]).0,
        "field `x` is already defined"
    );
    assert_eq!(
        error(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "f = 2")]).0,
        "field `f` is defined in two files"
    );
    // A root of a directory above merges; the same leaf in both is a
    // conflict, reported at the directory above.
    assert_eq!(
        error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
        (
            "field `version` is defined in two files".to_owned(),
            Some(1),
            vec![Some(2)]
        )
    );
}

#[test]
fn files_that_need_each_other() {
    let (message, input, labels) = error(&[("a.gcsl", "p = q"), ("b.gcsl", "q = p")]);
    assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
    assert_eq!((input, labels), (Some(1), vec![Some(2)]));
    let (message, _, labels) = error(&[
        ("a.gcsl", "p = q"),
        ("b.gcsl", "q = r"),
        ("c.gcsl", "r = p"),
    ]);
    assert_eq!(message, "`a.gcsl`, `b.gcsl` and `c.gcsl` need each other");
    assert_eq!(labels, [Some(2), Some(3)]);
}

#[test]
fn path_order_breaks_ties_and_errors_name_their_file() {
    let scan = ws(&[("b.gcsl", "x = 1"), ("a.gcsl", "y = 2")]).unwrap();
    assert_eq!(scan.order, [0, 1, 2]);
    assert_eq!(scan.files[1].path, "a.gcsl");
    let errors = ws(&[("a.gcsl", "x = 1"), ("b.gcsl", "y = (")]).unwrap_err();
    assert_eq!(errors[0].input, Some(2));
}

#[test]
fn the_summary_runs_each_file_to_its_own_value() {
    let mut files = SUMMARY.to_vec();
    // A file nothing needs is checked but never run: running it fails.
    files.push(("unused.gcsl", "def boom(): Int { [1][5] }\nnever = boom()"));
    assert_eq!(
        value(&files, "app.gcsl"),
        "{com = {x = {app = {name = \"app-1.2\"; deps = [{name = \"lib-1.2\"; \
             src = \"lib-1.2.tar\"}]}}}}"
    );
    assert_eq!(
        value(&files, "lib.gcsl"),
        "{com = {x = {lib = {name = \"lib-1.2\"; src = \"lib-1.2.tar\"}}}}"
    );
    // A `def` is a field of its file's record, as in one file.
    assert_eq!(
        value(&files, "common.gcsl"),
        "{version = \"1.2\"; tagged = fn(n: String): String}"
    );
    assert_eq!(value(&files, "workspace.gcsl"), "{}");
}

#[test]
fn dotted_paths_merge_in_either_path_order() {
    for (a, b) in [("a.gcsl", "b.gcsl"), ("b.gcsl", "a.gcsl")] {
        let files = [
            ("workspace.gcsl", ""),
            (a, "com.x.one = 1"),
            (b, "com.x.two = 2"),
            ("c.gcsl", "all = com\nx = com.x"),
        ];
        // The merged node's fields come in path order of their files.
        let x = if a == "a.gcsl" {
            "{one = 1; two = 2}"
        } else {
            "{two = 2; one = 1}"
        };
        assert_eq!(
            value(&files, "c.gcsl"),
            format!("{{all = {{x = {x}}}; x = {x}}}")
        );
        assert_eq!(value(&files, a), "{com = {x = {one = 1}}}");
    }
}

#[test]
fn defs_and_function_values_cross_files() {
    let files = [
        (
            "a.gcsl",
            "def twice(n: Int): Int { n * 2 }\nf = fn(n: Int): Int { n + 1 }",
        ),
        (
            "b.gcsl",
            "g = f\nh = twice(3)\nfn k(): Int { twice(4) }\nm = k()",
        ),
        ("c.gcsl", "r = g(10)"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "b.gcsl"),
        "{g = fn(n: Int): Int; h = 6; m = 8}"
    );
    assert_eq!(value(&files, "c.gcsl"), "{r = 11}");
}

#[test]
fn a_contributor_reads_a_shared_node_whole() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "com.one = 1"),
        ("b.gcsl", "com.two = 2\nall = com"),
    ];
    assert_eq!(
        value(&files, "b.gcsl"),
        "{com = {two = 2}; all = {one = 1; two = 2}}"
    );
}

#[test]
fn private_names_and_other_directories_are_not_shared() {
    let base = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "let p = 1\nfn q(): Int { 2 }\nx = 3\ndef d(): Int { 4 }\n...{ s = 5 }",
        ),
    ];
    for (text, message) in [
        ("y = p", "undefined variable `p`"),
        ("y = q()", "undefined variable `q`"),
        ("y = s", "undefined variable `s`"),
        ("let x = 1", "`x` is a field of this directory"),
        ("let mut x = 1", "`x` is a field of this directory"),
        ("x := 1", "`x` is a field of this directory"),
        ("let x.y = 1", "`x` is a field of this directory"),
        ("fn x(): Int { 1 }", "`x` is a field of this directory"),
        ("let d = 1", "`d` is a field of this directory"),
    ] {
        let mut files = base.to_vec();
        files.push(("b.gcsl", text));
        assert!(
            messages(&files, "b.gcsl").iter().any(|m| m == message),
            "{text}: {:?}",
            messages(&files, "b.gcsl")
        );
        // From a directory below, likewise.
        let mut files = base.to_vec();
        files.push(("sub/b.gcsl", text));
        assert!(
            messages(&files, "sub/b.gcsl").iter().any(|m| m == message),
            "sub: {text}"
        );
    }
    let mut files = base.to_vec();
    files.push(("sub/c.gcsl", "y = x + d()"));
    files.push(("other/d.gcsl", "z = 1"));
    assert_eq!(value(&files, "sub/c.gcsl"), "{y = 7}");
    files.push(("e.gcsl", "w = z"));
    assert_eq!(messages(&files, "e.gcsl"), ["undefined variable `z`"]);
    // The directory above does not see a subdirectory's declarations.
    files.pop();
    files.push(("e.gcsl", "w = y"));
    assert_eq!(messages(&files, "e.gcsl"), ["undefined variable `y`"]);
}

#[test]
fn a_clash_within_a_file_is_reported_as_in_one_file() {
    // A root of the file itself: one file's errors only, never also "of
    // this directory", though the root is a declaration of the directory.
    let files = [
        ("a.gcsl", "x = 1\nlet x = 2\ny.z = 3\nfn y(): Int { 1 }"),
        ("b.gcsl", "w = 1"),
    ];
    assert_eq!(
        messages(&files, "a.gcsl"),
        [
            "`x` is a field of this file",
            "`y` is a function of this file"
        ]
    );
}

#[test]
fn a_future_of_another_file_runs_once() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "p = await f\nq = await f"),
        ("b.gcsl", "r = { a = 20 }\nf = lazy { v = r.a + 1 }"),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    let index = workspace.file("a.gcsl").unwrap();
    let Value::Record(record) = workspace.run(index).unwrap().value else {
        panic!("expected a record");
    };
    // Both awaits give the one record its body built, reading the
    // field of `b.gcsl` from the frame.
    let (Some(Value::Record(p)), Some(Value::Record(q))) =
        (record.values.first(), record.values.get(1))
    else {
        panic!("expected records");
    };
    assert!(std::rc::Rc::ptr_eq(p, q));
    assert_eq!(Value::Record(p.clone()).to_string(), "{v = 21}");
}

#[test]
fn spreads_and_paths_read_other_files() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "com.lib.name = \"n\"\ncom.lib.other = [1][0]\nr = { k = 1 }",
        ),
        ("b.gcsl", "n = com.lib.name\n...r\n...{ z = com.lib.name }"),
        ("c.gcsl", "...com.lib\nall = com"),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{n = \"n\"; k = 1; z = \"n\"}");
    assert_eq!(
        value(&files, "c.gcsl"),
        "{name = \"n\"; other = 1; all = {lib = {name = \"n\"; other = 1}}}"
    );
}

#[test]
fn empty_and_value_files() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = 3"),
        ("b.gcsl", "= x + 1"),
        ("c.gcsl", "y = x"),
    ];
    assert_eq!(value(&files, "b.gcsl"), "4");
    assert_eq!(value(&files, "workspace.gcsl"), "{}");
    assert_eq!(value(&files, "c.gcsl"), "{y = 3}");
}

#[test]
fn a_needed_file_runs_whole_first_and_its_error_names_it() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "def boom(): Int { [1][5] }\nx = boom()"),
        ("b.gcsl", "y = x"),
    ];
    let errors = run_file(&files, "b.gcsl").unwrap_err();
    // `a.gcsl` is input 1, after the prelude: the error names it, and
    // `b.gcsl` never ran.
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].1, Some(1));
    let erroneous = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = 1"),
        ("z.gcsl", "y = 1 + \"\""),
    ];
    let errors = run_file(&erroneous, "a.gcsl").unwrap_err();
    // The error is `z.gcsl`'s, input 3.
    assert!(errors.iter().all(|error| error.1 == Some(3)), "{errors:?}");
    // Every failing file is reported, in path order; a file that needs
    // one that failed is not checked.
    let erroneous = [
        ("a.gcsl", "x = 1 + \"\""),
        ("b.gcsl", "y = x\nz = undefined"),
        ("c.gcsl", "w = 2 + \"\""),
        ("workspace.gcsl", ""),
    ];
    let errors = run_file(&erroneous, "workspace.gcsl").unwrap_err();
    let inputs: Vec<_> = errors.iter().map(|error| error.1).collect();
    assert_eq!(inputs, [Some(1), Some(3)], "{errors:?}");
    // A closure of another file names that file when it fails.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "def boom(): Int { [1][5] }"),
        ("b.gcsl", "y = boom()"),
    ];
    let errors = run_file(&files, "b.gcsl").unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].1, Some(1), "{errors:?}");
}

/// The time to check a workspace of `count` files of ten fields each,
/// every file reading a field of the one before, and to run its last
/// file, which needs every other: linear in `count`. Run with
/// `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_a_chain_of_files() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [50_u32, 100, 200, 400, 800] {
        let sources: Vec<SourceFile> = (0..count)
            .map(|file| {
                let mut text = if file == 0 {
                    String::from("f0_0 = 1\n")
                } else {
                    format!("f{file}_0 = f{}_9 + 1\n", file - 1)
                };
                for field in 1..10 {
                    writeln!(text, "f{file}_{field} = f{file}_{} + 1", field - 1).unwrap();
                }
                SourceFile::new(format!("f{file:04}.gcsl"), text).unwrap()
            })
            .collect();
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace.file(&format!("f{:04}.gcsl", count - 1)).unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        let last = format!("{}", outcome.value);
        assert!(last.contains(&format!("= {}", count * 10)), "{last}");
        writeln!(
            report,
            "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

/// The time to check and run a lone file of `count` fields, each reading
/// the one before, beside the standard library ([`check_lone`]), against
/// checking, lowering and running it alone as [`crate::check::check`]
/// does, with no standard library: what the prelude and the path of a
/// workspace cost a lone file. Run with `cargo test --release --
/// --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_a_lone_file_beside_the_prelude() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [1_u32, 1000, 2000, 4000, 8000] {
        let mut text = String::from("f0 = 1\n");
        for field in 1..count {
            writeln!(text, "f{field} = f{} + 1", field - 1).unwrap();
        }
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let start = std::time::Instant::now();
        let (tokens, names, strings) = lexer::tokenize(&source).unwrap();
        let ast = parser::parse(&tokens, names, strings, &source).unwrap();
        let mut types = TypeTable::new();
        let (checked, _) = crate::check::check(&ast, &mut types).unwrap();
        let ir = std::rc::Rc::new(crate::lower::lower(&ast, &checked, &types, 0).unwrap());
        let alone = crate::eval::eval(&ir).unwrap();
        let bare = start.elapsed();
        // Its scan and its check, apart.
        let start = std::time::Instant::now();
        let scanned = scan_into(
            (std::slice::from_ref(&source), Placing::Lone(&[], false)),
            Library::NEEDED,
            &mut Names::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let scanning = start.elapsed();
        let start = std::time::Instant::now();
        check_scan(scanned).unwrap();
        let checking = start.elapsed();
        let start = std::time::Instant::now();
        let lone = crate::run(&source).unwrap().value;
        let beside = start.elapsed();
        assert_eq!(alone.to_string(), lone.to_string());
        writeln!(
            report,
            "{count} fields: alone {bare:?}; beside the standard library {beside:?} \
                 (scan {scanning:?}, check {checking:?})"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

/// [`benchmark_a_chain_of_files`] through imports: each file in a
/// directory of its own, importing the previous file's directory and
/// reading one leaf through it. Linear in `count`, as the chain is.
/// Run with `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_a_chain_of_imports() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [50_u32, 100, 200, 400, 800] {
        let sources: Vec<SourceFile> = (0..count)
            .map(|file| {
                let mut text = if file == 0 {
                    String::from("f0_0 = 1\n")
                } else {
                    let before = file - 1;
                    format!("import d{before:04}\nf{file}_0 = d{before:04}.f{before}_9 + 1\n")
                };
                for field in 1..10 {
                    writeln!(text, "f{file}_{field} = f{file}_{} + 1", field - 1).unwrap();
                }
                SourceFile::new(format!("d{file:04}/f.gcsl"), text).unwrap()
            })
            .collect();
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace
            .file(&format!("d{:04}/f.gcsl", count - 1))
            .unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        let last = format!("{}", outcome.value);
        assert!(last.contains(&format!("= {}", count * 10)), "{last}");
        writeln!(
            report,
            "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}
