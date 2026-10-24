use super::*;
use crate::ir::IrKind;

/// Scans the workspace of `files`, each a path and a text.
fn ws(files: &[(&str, &str)]) -> Result<Scan, Vec<Diagnostic>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    scan(&sources)
}

/// The single error of scanning `files`: its message, input, and the
/// inputs of its labels.
fn error(files: &[(&str, &str)]) -> (String, Option<u32>, Vec<Option<u32>>) {
    let errors = ws(files).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let error = &errors[0];
    let labels = error.labels.iter().map(|l| l.input).collect();
    (error.message.clone(), error.input, labels)
}

/// The files of the summary, in path order: app, common, lib,
/// workspace.
const SUMMARY: [(&str, &str); 4] = [
    ("workspace.gcsl", ""),
    (
        "common.gcsl",
        "version = \"1.2\"\ndef tagged(n: String): String { \"{n}-{version}\" }",
    ),
    (
        "app.gcsl",
        "com.x.app.name = tagged(\"app\")\ncom.x.app.deps = [com.x.lib]",
    ),
    (
        "lib.gcsl",
        "com.x.lib.name = tagged(\"lib\")\ncom.x.lib.src = \"lib-{version}.tar\"",
    ),
];

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

/// Checks the workspace of `files` and runs the file at `path`,
/// returning its value shown, or the messages and inputs of its
/// diagnostics.
fn run_file(files: &[(&str, &str)], path: &str) -> Result<String, Vec<(String, Option<u32>)>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let shown = |errors: Vec<Diagnostic>| {
        errors
            .into_iter()
            .map(|error| (error.message, error.input))
            .collect::<Vec<_>>()
    };
    let mut workspace = check(&sources).map_err(shown)?;
    assert_imports_stored(&workspace);
    assert_assemblies_stored(&workspace);
    let index = workspace.file(path).unwrap();
    let outcome = workspace.run(index).map_err(shown)?;
    Ok(outcome.value.to_string())
}

/// Asserts that the driver wrote the cell of every built-in function
/// as it laid the frame out, before any file runs: a `Value::Builtin`,
/// never `Value::Empty`, which would read as a record-kinded `{}` that
/// nothing else can see. Returns their slots, which no file stores.
fn assert_builtins_written(workspace: &Workspace) -> Vec<Slot> {
    Builtin::ALL
        .into_iter()
        .zip(0u32..)
        .map(|(builtin, node)| {
            let slot = workspace.cell_slots[node as usize];
            assert_eq!(workspace.scan.builtin(node), Some(builtin));
            assert_eq!(
                workspace.frame.get(slot.index()),
                Some(&Value::Builtin(builtin)),
                "the cell of `{}` is not written by the driver",
                builtin.name()
            );
            slot
        })
        .collect()
}

/// Asserts that every cell an import of `workspace` reaches, directly
/// or as a field of a record read whole, is stored by a node of its
/// owner's top-level block: slice 24's schedule test sees one file
/// only, and a cell no owner writes would read as `{}`. A built-in
/// function's cell has no owner: the driver wrote it before any file
/// ran ([`assert_builtins_written`]).
fn assert_imports_stored(workspace: &Workspace) {
    let scan = &workspace.scan;
    let types = vec![TyId::ERROR; scan.cells()];
    let builtins = assert_builtins_written(workspace);
    for (index, file) in (0u32..).zip(&scan.files) {
        let reads = (&workspace.cell_slots[..], &[][..], &workspace.for_later);
        let context = Context::new(scan, index, &types, reads).unwrap();
        let mut pending: Vec<Target> = file
            .mentions
            .foreign
            .iter()
            .flatten()
            .filter_map(|&(need, _)| match need {
                Need::Field(n) => context.scope.target(n),
                Need::Function(_) => None,
            })
            .collect();
        while let Some(target) = pending.pop() {
            let cell = match target {
                Target::Node(node) => Some(context.cell_slot(node)),
                other => context.value_of(other).map(Slot::at),
            };
            let Some(slot) = cell else {
                pending.extend(context.target_fields(target).into_iter().map(|(_, t)| t));
                continue;
            };
            if builtins.contains(&slot) {
                continue;
            }
            let owners = scan.target_files(target);
            let stored = owners.iter().any(|&owner| {
                let Some(FileRun::Pending(ir)) = workspace.runs.get(owner as usize) else {
                    return false;
                };
                let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                    return false;
                };
                ir.items(items)
                    .iter()
                    .any(|&item| matches!(ir.get(item).kind, IrKind::Store(s, _) if s == slot))
            });
            assert!(
                stored,
                "{}: `{}` is not stored by {owners:?}",
                file.path,
                scan.show_target(target)
            );
        }
    }
}

/// The slots the top-level block of file `file` of `workspace` stores,
/// in order.
fn top_stores(workspace: &Workspace, file: u32) -> Vec<Slot> {
    let Some(FileRun::Pending(ir)) = workspace.runs.get(file as usize) else {
        return Vec::new();
    };
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        return Vec::new();
    };
    ir.items(items)
        .iter()
        .filter_map(|&item| match ir.get(item).kind {
            IrKind::Store(slot, _) => Some(slot),
            _ => None,
        })
        .collect()
}

/// Asserts that every record a file of `workspace` assembles of a node
/// of its view with a leaf of the file under it reads only cells stored
/// before it: the file's own by an earlier node of its top-level block,
/// every other one by the top-level block of a file it needs, which runs
/// whole before it. And that every record any file assembles at its top
/// level (an interior node's, a shared node's, an imported one) reads
/// only cells stored earlier in its block or by a file earlier in the
/// order: slice 25's shared nodes read the cells of the files that
/// contribute to them, which come earlier but need not be needed. A cell
/// nobody stored first would read as `{}`. A built-in function's cell
/// counts as stored before any file: the driver wrote it as it laid the
/// frame out ([`assert_builtins_written`]).
fn assert_assemblies_stored(workspace: &Workspace) {
    let scan = &workspace.scan;
    let builtins = assert_builtins_written(workspace);
    for (index, file) in (0u32..).zip(&scan.files) {
        let own = top_stores(workspace, index);
        let needed: Vec<Slot> = scan
            .needed(index)
            .into_iter()
            .filter(|&other| other != index)
            .flat_map(|other| top_stores(workspace, other))
            .chain(builtins.iter().copied())
            .collect();
        let rank = scan.order.iter().position(|&f| f == index).unwrap_or(0);
        let earlier: Vec<Slot> = scan.order[..rank]
            .iter()
            .flat_map(|&other| top_stores(workspace, other))
            .chain(builtins.iter().copied())
            .collect();
        let records = workspace.view_records.get(index as usize);
        for (slot, _, fields) in records.into_iter().flatten() {
            let Some(at) = own.iter().position(|s| s == slot) else {
                let path = &file.path;
                assert_eq!(*slot, Slot::NONE, "{path}: a view record is not stored");
                continue;
            };
            for &(name, cell, _) in fields {
                assert!(
                    own[..at].contains(&cell) || needed.contains(&cell),
                    "{}: the view record in {slot:?} reads `{}` from {cell:?}, stored by \
                         nothing before it",
                    file.path,
                    scan.names.get(name)
                );
            }
        }
        let Some(FileRun::Pending(ir)) = workspace.runs.get(index as usize) else {
            continue;
        };
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            continue;
        };
        let mut stored: Vec<Slot> = Vec::new();
        for &item in ir.items(items) {
            let IrKind::Store(slot, value) = ir.get(item).kind else {
                continue;
            };
            if let IrKind::MakeRecord { values, .. } = ir.get(value).kind {
                for &field in ir.items(values) {
                    if let IrKind::Load(cell, _) = ir.get(field).kind {
                        assert!(
                            stored.contains(&cell) || earlier.contains(&cell),
                            "{}: a record stored in {slot:?} reads {cell:?}, stored by \
                                 nothing before it",
                            file.path
                        );
                    }
                }
            }
            stored.push(slot);
        }
    }
}

/// The value of the file at `path` of the workspace of `files`.
fn value(files: &[(&str, &str)], path: &str) -> String {
    run_file(files, path).unwrap_or_else(|errors| panic!("{path}: {errors:?}"))
}

/// The messages of the diagnostics of running `path` in `files`.
fn messages(files: &[(&str, &str)], path: &str) -> Vec<String> {
    match run_file(files, path) {
        Ok(value) => panic!("{path}: unexpected value {value}"),
        Err(errors) => errors.into_iter().map(|(message, _)| message).collect(),
    }
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

/// The files of the imports summary, in path order.
const IMPORTS: [(&str, &str); 6] = [
    (
        "app/main.gcsl",
        "import lib\nimport tools/build as builder\nimport com as libs from lib\n\
             import lib/y\ncom.x.app.deps = [lib.com.x.lib, libs.y.lib]\n\
             com.x.app.tool = builder\ncom.x.app.all = y.com.y.lib.name",
    ),
    ("common.gcsl", "version = \"1.2\""),
    ("lib/x.gcsl", "com.x.lib.name = \"lib-{version}\""),
    ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
    ("tools/build.gcsl", "= \"build-{version}\""),
    ("workspace.gcsl", ""),
];

/// The scan of `files` told that `nested` are workspaces of their own.
fn ws_nested(files: &[(&str, &str)], nested: &[&str]) -> Result<Scan, Vec<Diagnostic>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let nested: Vec<String> = nested.iter().map(|&n| n.to_owned()).collect();
    scan_with(&sources, &nested)
}

/// The message and note of the single error of scanning `files`.
fn noted(files: &[(&str, &str)], nested: &[&str]) -> (String, Option<String>) {
    let errors = ws_nested(files, nested).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    (errors[0].message.clone(), errors[0].notes.first().cloned())
}

#[test]
fn imports_resolve_to_their_targets() {
    let scan = ws(&IMPORTS).unwrap();
    let main = &scan.files[1];
    // Its own imports; the implicit prelude's names follow them.
    let named: Vec<(&str, Target)> = main
        .aliases
        .iter()
        .filter(|a| !a.implicit)
        .map(|a| (scan.names.get(a.name), a.target))
        .collect();
    let lib = scan.dirs.iter().position(|d| d.path == "lib").unwrap();
    let com = scan.by_name[&(count(lib), ROOT, scan.names.lookup("com").unwrap())];
    assert_eq!(
        named,
        [
            ("lib", Target::Dir(count(lib))),
            ("builder", Target::File(5)),
            ("libs", Target::Node(com)),
            ("y", Target::File(4)),
        ]
    );
    // Needs per leaf: x, y and build, each through an import; `common`
    // only through them.
    assert_eq!(scan.needs[1], [3, 4, 5]);
    assert_eq!(scan.needed(1), [0, 2, 3, 4, 5, 1]);
    assert_eq!(scan.value_slot(5), Some(Slot::at(count(scan.nodes.len()))));
    assert_eq!(scan.value_slot(2), None);
}

#[test]
fn a_reference_through_an_import_needs_only_its_leaf() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib\nimport lib/y\nimport tools\na = lib.com.x.lib.name\nb = y";
    let scan = ws(&files).unwrap();
    // `lib.com.x.lib.name` is x's leaf; `y` whole is y; nothing reads
    // `tools`, so it needs nothing.
    assert_eq!(scan.needs[1], [3, 4]);
    files[0].1 = "import lib\na = lib";
    assert_eq!(ws(&files).unwrap().needs[1], [3, 4]);
    files[0].1 = "import lib\na = lib.com.y";
    assert_eq!(ws(&files).unwrap().needs[1], [4]);
}

#[test]
fn a_file_part_and_a_whole_node_are_told_apart() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib/y\na = y.com\nb = y.com.y";
    let scan = ws(&files).unwrap();
    let main = &scan.files[1];
    let scope = FileScope::new(&scan, main);
    let targets: Vec<Target> = main
        .mentions
        .foreign
        .iter()
        .flatten()
        .filter_map(|&(need, _)| match need {
            Need::Field(n) => scope.target(n),
            Need::Function(_) => None,
        })
        .collect();
    // `com` of `lib/` is shared with x: y's part of it. `com.y` is y's
    // alone: the merged node.
    assert!(
        matches!(targets[0], Target::Part { file: 4, .. }),
        "{targets:?}"
    );
    assert!(matches!(targets[1], Target::Node(_)), "{targets:?}");
}

#[test]
fn no_module_says_what_there_is() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import nope";
    let (message, note) = noted(&files, &["vendor"]);
    assert_eq!(message, "no module `nope`");
    assert_eq!(
        note.as_deref(),
        Some("the workspace has `app/`, `lib/`, `tools/`, `common`, `workspace`")
    );
    files[0].1 = "import lib/z";
    let (message, note) = noted(&files, &[]);
    assert_eq!(message, "no module `lib/z`");
    assert_eq!(note.as_deref(), Some("`lib/` has `x`, `y`"));
    files[0].1 = "import lib/x/z";
    assert_eq!(noted(&files, &[]).0, "no module `lib/x/z`");
    files[0].1 = "import vendor/pkg";
    let (_, note) = noted(&files, &["vendor"]);
    assert_eq!(note.as_deref(), Some("`vendor/` is a workspace of its own"));
}

/// A segment of a module path may have dots in it, as reverse-DNS names
/// do: it names a directory or a file whose name has them.
#[test]
fn a_module_path_segment_may_have_dots_in_it() {
    let files = [
        (
            "app.gcsl",
            "import pkgs/net.busybox.busybox as busybox
             import w from pkgs/dev.peios.live-root/root
= busybox.v + w",
        ),
        ("pkgs/net.busybox.busybox/pkg.gcsl", "v = 1"),
        ("pkgs/dev.peios.live-root/root.gcsl", "w = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(run_file(&files, "app.gcsl").unwrap(), "3");
}

/// A path in the old form, its segments separated by `.`, names no module;
/// when it would with each `.` a `/`, the error says so and suggests it.
#[test]
fn a_path_with_dots_for_slashes_is_suggested_the_slashes() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib.x as old";
    let errors = ws_nested(&files, &[]).unwrap_err();
    let [error] = errors.as_slice() else {
        panic!("expected one error: {errors:?}");
    };
    assert_eq!(error.message, "no module `lib.x`");
    assert_eq!(
        error.notes,
        ["module paths separate their segments with `/`: `lib/x` names a module"]
    );
    let suggestion = error.suggestion.as_deref().unwrap();
    assert_eq!(suggestion.replacement, "lib/x");
    assert_eq!(suggestion.span, Span::new(7, 12));
    // A name of dots that names nothing either way is no module, as any.
    files[0].1 = "import lib.q as old";
    let (message, note) = noted(&files, &[]);
    assert_eq!(message, "no module `lib.q`");
    assert!(note.unwrap().starts_with("the workspace has"));
}

/// An import that nothing mentions needs nothing, but is still resolved:
/// a broken one stops the run of every file, even one that does not
/// import anything.
#[test]
fn an_import_nothing_mentions_is_still_resolved() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib\nimport nope\na = 1";
    let errors = run_file(&files, "lib/x.gcsl").unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].0, "no module `nope`", "{errors:?}");
    files[0].1 = "import lib\na = 1";
    assert!(run_file(&files, "app/main.gcsl").is_ok());
}

#[test]
fn a_directory_and_a_file_of_one_name_are_reported_once() {
    let mut files = IMPORTS.to_vec();
    files[0].1 = "import lib\nimport lib as again";
    files.push(("lib.gcsl", "z = 1"));
    let (message, _) = noted(&files, &[]);
    assert_eq!(message, "`lib` is both a directory and a file");
    files[0].1 = "a = 1";
    assert!(ws(&files).is_ok());
}

#[test]
fn imported_names_clash() {
    let mut files = IMPORTS.to_vec();
    let imported = "is imported in this file";
    for (text, message) in [
        ("import lib\nimport tools as lib", imported),
        ("import lib\nfn lib(): Int { 1 }", imported),
        ("import lib\nlet lib = 1", imported),
        ("import lib\nlib.a = 1", imported),
        ("import tools as version", "is a field of this directory"),
        ("import x from lib/x", "has no `x`"),
    ] {
        files[0].1 = text;
        assert!(noted(&files, &[]).0.ends_with(message), "{text}");
    }
    // A member or a `*` root merges with the file's own root of its
    // name.
    for text in [
        "import com from lib\ncom.a = 1",
        "import * from lib\ncom.a = 1",
    ] {
        files[0].1 = text;
        assert!(ws(&files).is_ok(), "{text}");
    }
    files[0].1 = "import * from lib\na = com.x.lib.name";
    assert!(ws(&files).is_ok());
}

#[test]
fn reaching_into_your_own_file_through_an_import_needs_itself() {
    let mut files = IMPORTS.to_vec();
    files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nall = lib";
    let (message, _) = noted(&files, &[]);
    assert_eq!(message, "`lib/x.gcsl` needs itself");
    files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nother = lib.com.y.lib.name";
    assert!(ws(&files).is_ok());
}

#[test]
fn files_importing_each_other_are_a_cycle() {
    let (message, _) = noted(
        &[
            ("a.gcsl", "import b\nx = b.y"),
            ("b.gcsl", "import a\ny = a.x"),
            ("workspace.gcsl", ""),
        ],
        &[],
    );
    assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
    // A whole read of a directory one of whose files needs this one is
    // the one-file form, at the read, and no cycle of two files.
    let files = [
        ("app.gcsl", "import lib\nall = lib"),
        ("lib/y.gcsl", "import app\ny = app.all"),
        ("workspace.gcsl", ""),
    ];
    let errors = match ws(&files) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    };
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["`app.gcsl` needs itself"]);
}

#[test]
fn the_imports_summary_runs() {
    assert_eq!(
        value(&IMPORTS, "app/main.gcsl"),
        "{com = {x = {app = {deps = [{name = \"lib-1.2\"}, {name = \"ylib\"}]; \
             tool = \"build-1.2\"; all = \"ylib\"}}}}"
    );
}

/// A workspace of a library directory with a `def`, a leaf and an
/// interior node in one file, a file failing at runtime if it runs, and
/// `main` as given.
fn library(main: &str) -> Vec<(&str, &str)> {
    vec![
        ("app/main.gcsl", main),
        ("lib/bad.gcsl", "def bad(): Int { [1][5] }\nboom = bad()"),
        (
            "lib/f.gcsl",
            "def twice(n: Int): Int { n * 2 }\nleafy = 3\ncom.k.v = 1",
        ),
        ("lib/sub/s.gcsl", "import lib\nv = lib.leafy"),
        ("workspace.gcsl", ""),
    ]
}

#[test]
fn every_import_form_binds_its_names() {
    let files = library("import lib/f as lf\nimport lib/f\na = lf.leafy\nb = f.com.k.v");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3; b = 1}");
    let main = "import twice, leafy as l, com from lib\na = twice(l)\nb = com.k\nc = com.k.v";
    let files = library(main);
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{a = 6; b = {v = 1}; c = 1}"
    );
    let files = library("import * from lib/f\na = twice(leafy)\nb = com");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 6; b = {k = {v = 1}}}");
    // Each root of `*` is a member: `lib/bad.gcsl`'s `boom` is not read.
    let files = library("import * from lib\na = twice(leafy)");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 6}");
}

#[test]
fn a_def_is_read_from_its_functions_slot() {
    let files = library("import lib\nimport twice from lib/f\n= lib.twice(1) + twice(2)");
    assert_eq!(value(&files, "app/main.gcsl"), "6");
    let main = "import lib/f\nfn g(): Int { let r = f\nr.twice(5) }\nr = f.com\nn = g()";
    assert_eq!(
        value(&library(main), "app/main.gcsl"),
        "{r = {k = {v = 1}}; n = 10}"
    );
    let main = "import lib/f\nlet r = f\n= r.twice(4) + r.leafy";
    assert_eq!(value(&library(main), "app/main.gcsl"), "11");
}

/// A file mentioning a record-valued leaf through an import from every
/// kind of position, reading the directory whole and calling a `def`
/// out of the record, at the top level and inside functions, a nested
/// `def` and lazies.
const POSITIONS: &str = r#"import lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := lib.com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy lib.com.x.lib.r
let lw = lazy lib
let g = fn(): {a: Int} { lib.com.x.lib.r }
let gw = fn(): Int { let w = lib; = w.twice(lib.com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = lib
    = lib.com.x.lib.r.a + w.com.x.lib.r.a
  }
}
let w = lib
p01 = "{lib.com.x.lib.r}"
p02 = xs[lib.com.x.lib.r.a]
p03 = m
p04 = for v in [lib.com.x.lib.r] { v.a }
p05 = for v in [1] if lib.com.x.lib.r.a == 42 { v }
p06 = for let i = lib.com.x.lib.r.a; i < lib.com.x.lib.r.a + 1; i := i + lib.com.x.lib.r.a { i }
p07 = for k < lib.com.x.lib.r.a { k := k + 50; lib.com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < lib.com.x.lib.r.a)
p09 = true and lib.com.x.lib.r.a == 42
p10 = lib.com.x.lib.r is {a: Int}
p11 = (lib.com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue lib.com.x.lib.r }; break lib.com.x.lib.r }
p13 = get(config = lib.com.x.lib.r)
p14 = wrap(lib.com.x.lib.r) { ...lib.com.x.lib.r }
p15 = { ...lib.com.x.lib.r }
p16 = len(0..lib.com.x.lib.r.a)
p17 = [lib.com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).com.x.lib.r.a
p20 = g()
p21 = { let q = lib.com.x.lib.r; = q }
p22 = w.twice(w.com.x.lib.r.a)
p23 = gw()
p24 = w.com"#;

#[test]
fn records_read_whole_have_the_types_of_their_roots() {
    let files = [
        ("app/main.gcsl", "import lib\nimport lib/x\nd = lib\nf = x"),
        (
            "lib/x.gcsl",
            "com.x.lib.n = 1\ndef twice(n: Int): Int { n * 2 }",
        ),
        ("lib/y.gcsl", "top = \"y\""),
        ("workspace.gcsl", ""),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    let index = workspace.file("app/main.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    // A directory's record has every file's roots, a `def`'s its
    // function's type; a file's, its own roots.
    assert!(
        dump.contains(
            "(name lib : {com: {x: {lib: {n: Int}}}, top: String, twice: fn(n: Int): Int}"
        )
    );
    assert!(dump.contains("(name x : {com: {x: {lib: {n: Int}}}, twice: fn(n: Int): Int}"));
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{d = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int; top = \"y\"}; \
             f = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int}}"
    );
}

#[test]
fn a_call_of_an_imported_def_is_of_a_known_callee() {
    // The call-site form of the implicit-await error, as for a `def` of
    // the directory, through a member and through a path.
    let files = [
        (
            "app/main.gcsl",
            "import inc from lib\nimport lib\n\
                 fn p(f: Future<Int>): Int { inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.inc(f) }",
        ),
        ("lib/x.gcsl", "def inc(n: Int): Int { n + 1 }"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        [
            "argument `f` would be awaited inside `inc`, which is pure",
            "argument `f` would be awaited inside `lib.inc`, which is pure",
        ]
    );
}

#[test]
fn an_import_is_read_from_every_position() {
    let files = [
        ("app/main.gcsl", POSITIONS),
        (
            "lib/x.gcsl",
            "let base = 40\ncom.x.lib.r = { a = base + 2 }\ndef twice(n: Int): Int { n * 2 }",
        ),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{outer = fn(): {inner: fn(): Int}; p01 = \"{{a = 42}}\"; p02 = 42; \
             p03 = {a = 42}; p04 = [42]; p05 = [1]; p06 = [42]; p07 = [{a = 42}]; \
             p08 = 42; p09 = true; p10 = true; p11 = 42; p12 = [{a = 42}, {a = 42}]; \
             p13 = 42; p14 = 84; p15 = {a = 42}; p16 = 42; p17 = [{a = 42}]; p18 = 84; \
             p19 = 84; p20 = {a = 42}; p21 = {a = 42}; p22 = 84; p23 = 84; \
             p24 = {x = {lib = {r = {a = 42}}}}}"
    );
}

#[test]
fn needs_through_an_import_are_per_leaf() {
    let files = library("import lib\na = lib.leafy");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
    let files = library("import lib\na = lib.com.k");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = {v = 1}}");
    let files = library("import lib\nfn g(): Int { lib.leafy }\na = g()");
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
    let main = "import lib/f\ndef d(): Int { f.com.k.v + (fn(): Int { f.twice(1) })() }\na = d()";
    assert_eq!(
        value(&library(main), "app/main.gcsl"),
        "{d = fn(): Int; a = 3}"
    );
    // Read whole, every file of `lib/` runs, `lib/bad.gcsl` too.
    let messages = messages(
        &library("import lib\nlet r = lib\na = r.leafy"),
        "app/main.gcsl",
    );
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(value(&library(""), "lib/sub/s.gcsl"), "{v = 3}");
}

#[test]
fn a_value_file_is_imported_by_its_value() {
    let files = [
        ("a.gcsl", "import b\nimport c\nx = b + 1\ny = c.z"),
        ("b.gcsl", "= 41"),
        ("c.gcsl", "z = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a.gcsl"), "{x = 42; y = 2}");
    assert_eq!(value(&files, "b.gcsl"), "41");
}

#[test]
fn imported_names_are_the_files_own() {
    let main = "a = leafy + g(10)\nfn g(leafy: Int): Int { leafy + 1 }\nimport leafy from lib/f";
    assert_eq!(value(&library(main), "app/main.gcsl"), "{a = 14}");
    let mut files = library("import leafy from lib/f\na = leafy");
    files.push(("app/other.gcsl", "b = leafy"));
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        ["undefined variable `leafy`"]
    );
    files.pop();
    assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
}

#[test]
fn an_importing_files_items_are_in_any_order() {
    let lines = [
        "import lib/f",
        "a = f.leafy + b",
        "b = twice(2)",
        "import twice from lib/f",
    ];
    // Every rotation, and (as slice 23's helper in lowering, which runs
    // a file alone, compares) the reverse order.
    let reversed: Vec<&str> = lines.iter().rev().copied().collect();
    let orders = (0..lines.len())
        .map(|start| {
            let mut rotated = lines.to_vec();
            rotated.rotate_left(start);
            rotated
        })
        .chain([reversed]);
    for order in orders {
        let main = order.join("\n");
        let mut record: Vec<String> = value(&library(&main), "app/main.gcsl")
            .trim_matches(['{', '}'])
            .split("; ")
            .map(str::to_owned)
            .collect();
        record.sort();
        assert_eq!(record, ["a = 7", "b = 4"], "{main}");
    }
}

/// The files of the merging summary (slice 27), in path order: `pkgs/`
/// is the root.
const MERGE: [(&str, &str); 5] = [
    (
        "app/main.gcsl",
        "import * from lib\ncom.x.app.deps = [com.x.lib, com.y.lib]\n\
             com.x.app.base = com.base.name",
    ),
    ("app/vendor/v.gcsl", "com.vendor.thing = 1\nall = com"),
    ("lib/x.gcsl", "com.x.lib.name = \"lib\""),
    ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
    ("workspace.gcsl", "com.base.name = \"base\""),
];

/// The single error of scanning `files`, shown: its message and file,
/// its label, each other label's file and text, and its notes.
fn shown_error(files: &[(&str, &str)]) -> String {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut loaded = Vec::new();
    let placed = (&sources[..], Placing::Workspace(&[]));
    // The scan recovers from its errors (see "Scan errors").
    let errors = match scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded) {
        Ok(scan) => scan.errors(),
        Err(errors) => errors,
    };
    assert_eq!(errors.len(), 1, "{errors:?}");
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    // The standard files the scan loaded take the first input numbers.
    let standard = Library::NEEDED.sources(&loaded);
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    all.extend(paths);
    let path = |input: Option<u32>| {
        input
            .and_then(|i| all.get(i as usize))
            .copied()
            .unwrap_or("?")
    };
    let error = &errors[0];
    let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
    if let Some(label) = &error.label {
        parts[0] = format!("{} ({label})", parts[0]);
    }
    for label in &error.labels {
        let file = path(label.input.or(error.input));
        parts.push(format!("{file}: {}", label.text));
    }
    for note in &error.notes {
        parts.push(format!("note: {note}"));
    }
    parts.join("; ")
}

/// The checked workspace of `files`.
fn checked(files: &[(&str, &str)]) -> Workspace {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    check(&sources).unwrap()
}

/// Whether the first item of the top-level block of the file at `path`
/// of `workspace` stores a record: one assembled as the file starts.
fn assembled_first(workspace: &Workspace, path: &str) -> bool {
    let index = workspace.file(path).unwrap();
    let FileRun::Pending(ir) = &workspace.runs[index as usize] else {
        panic!("expected pending IR for file {index}");
    };
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("expected a block")
    };
    let first = ir.items(items)[0];
    matches!(ir.get(first).kind, IrKind::Store(_, value)
            if matches!(ir.get(value).kind, IrKind::MakeRecord { .. }))
}

#[test]
fn the_merging_summary_runs_both_files() {
    assert_eq!(
        value(&MERGE, "app/main.gcsl"),
        "{com = {x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; \
             base = \"base\"}}}}"
    );
    // `all` has the root's part, `app/`'s and v's own, root directory
    // first, but not `lib/`'s, which v does not import.
    assert_eq!(
        value(&MERGE, "app/vendor/v.gcsl"),
        "{com = {vendor = {thing = 1}}; all = {base = {name = \"base\"}; \
             x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; base = \"base\"}}; \
             vendor = {thing = 1}}}"
    );
    // v owns a leaf of `com`: its view's `com` is a node of its own
    // schedule, after its own child and before `all`.
    let scan = ws(&MERGE).unwrap();
    let v = &scan.files[2];
    assert_eq!((v.view.own, v.view.end - v.view.start), (1, 1));
    let number = count(v.global.len());
    assert!(v.schedule.order.contains(&number));
    assert_eq!(v.schedule.point[number as usize], 1);
    // main reads no node of its view whole: nothing is assembled.
    let main = &scan.files[1];
    let own = count(main.global.len());
    assert!(main.schedule.order.iter().all(|&n| n < own));
    // Its `com` is the root's, `app/`'s and `lib/`'s, root first.
    let com = &scan.views[main.view.start as usize];
    let sources: Vec<u32> = com.parts.iter().map(|p| p.source).collect();
    assert_eq!(sources, [0, 1, 2]);
    // v's type of `all` is the record of the merged children.
    let mut workspace = checked(&MERGE);
    let dump = workspace.dump_types(2).unwrap();
    assert!(
        dump.contains(
            "(name com : {base: {name: String}, vendor: {thing: Int}, x: {app: {base: \
                 String, deps: List<{name: String}>}}}"
        ),
        "{dump}"
    );
}

/// [`POSITIONS`] through a merged node: `com` is the file's own and
/// `lib/`'s, imported with `*`, and `com.x.lib.r` is `lib/`'s leaf, read
/// from every kind of position; `com` is read whole too (a node of the
/// file's schedule, the file owning `com.x.app.name`), and a function
/// out of that record called. A `def` is a root, never a leaf under
/// `com`: `com.x.lib.f` is a field holding one, and `twice` itself is
/// called through the `*`.
const MERGED_POSITIONS: &str = r#"import * from lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy com.x.lib.r
let lw = lazy com
let g = fn(): {a: Int} { com.x.lib.r }
let gw = fn(): Int { let w = com; = w.x.lib.f(com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = com
    = com.x.lib.r.a + w.x.lib.r.a
  }
}
let w = com
com.x.app.name = "app"
p01 = "{com.x.lib.r}"
p02 = xs[com.x.lib.r.a]
p03 = m
p04 = for v in [com.x.lib.r] { v.a }
p05 = for v in [1] if com.x.lib.r.a == 42 { v }
p06 = for let i = com.x.lib.r.a; i < com.x.lib.r.a + 1; i := i + com.x.lib.r.a { i }
p07 = for k < com.x.lib.r.a { k := k + 50; com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < com.x.lib.r.a)
p09 = true and com.x.lib.r.a == 42
p10 = com.x.lib.r is {a: Int}
p11 = (com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue com.x.lib.r }; break com.x.lib.r }
p13 = get(config = com.x.lib.r)
p14 = wrap(com.x.lib.r) { ...com.x.lib.r }
p15 = { ...com.x.lib.r }
p16 = len(0..com.x.lib.r.a)
p17 = [com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).x.lib.r.a
p20 = g()
p21 = { let q = com.x.lib.r; = q }
p22 = w.x.lib.f(w.x.lib.r.a)
p23 = gw()
p24 = twice(21)
p25 = com
p26 = com.x"#;

#[test]
fn a_merged_node_is_read_from_every_position() {
    let files = [
        ("app/main.gcsl", MERGED_POSITIONS),
        (
            "lib/x.gcsl",
            "let base = 40\ncom.x.lib.r = { a = base + 2 }\ncom.x.lib.f = twice\n\
                 def twice(n: Int): Int { n * 2 }",
        ),
        ("workspace.gcsl", ""),
    ];
    let lib = "lib = {r = {a = 42}; f = fn(n: Int): Int}";
    assert_eq!(
        value(&files, "app/main.gcsl"),
        format!(
            "{{outer = fn(): {{inner: fn(): Int}}; com = {{x = {{app = {{name = \"app\"}}}}}}; \
                 p01 = \"{{{{a = 42}}}}\"; p02 = 42; p03 = {{a = 42}}; p04 = [42]; p05 = [1]; \
                 p06 = [42]; p07 = [{{a = 42}}]; p08 = 42; p09 = true; p10 = true; p11 = 42; \
                 p12 = [{{a = 42}}, {{a = 42}}]; p13 = 42; p14 = 84; p15 = {{a = 42}}; \
                 p16 = 42; p17 = [{{a = 42}}]; p18 = 84; p19 = 84; p20 = {{a = 42}}; \
                 p21 = {{a = 42}}; p22 = 84; p23 = 84; p24 = 42; \
                 p25 = {{x = {{app = {{name = \"app\"}}; {lib}}}}}; \
                 p26 = {{app = {{name = \"app\"}}; {lib}}}}}"
        )
    );
}

#[test]
fn star_and_member_imports_merge_with_the_files_own_root() {
    let lib = [
        ("lib/x.gcsl", "com.x.lib.name = \"x\""),
        ("lib/y.gcsl", "com.y.lib.name = \"y\""),
        ("workspace.gcsl", ""),
    ];
    // A directory's tree, a file's own tree, and a member, each with an
    // own `com`: the import side's children after the file's own.
    for (main, all) in [
        (
            "import * from lib\ncom.app = 1\nall = com",
            "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
        ),
        (
            "import * from lib/y\ncom.app = 1\nall = com",
            "{app = 1; y = {lib = {name = \"y\"}}}",
        ),
        (
            "import com from lib\ncom.app = 1\nall = com",
            "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
        ),
    ] {
        let mut files = lib.to_vec();
        files.push(("app/main.gcsl", main));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            format!("{{com = {{app = 1}}; all = {all}}}"),
            "{main}"
        );
    }
    // A member that is a leaf, and one that is a `def`.
    let mut files = lib.to_vec();
    files.push(("lib/z.gcsl", "n = 1\ndef twice(k: Int): Int { k * 2 }"));
    files.push((
        "app/main.gcsl",
        "import n, twice from lib\nm = twice(n + 1)",
    ));
    assert_eq!(value(&files, "app/main.gcsl"), "{m = 4}");
}

#[test]
fn a_subdirectory_merges_under_a_root_above_two_levels_deep() {
    let files = [
        ("workspace.gcsl", "com.a = 1"),
        ("x/b.gcsl", "com.b = 2"),
        ("x/y/c.gcsl", "com.c = 3\nall = com\nb = com.b"),
    ];
    assert_eq!(
        value(&files, "x/y/c.gcsl"),
        "{com = {c = 3}; all = {a = 1; b = 2; c = 3}; b = 2}"
    );
    assert_eq!(value(&files, "x/b.gcsl"), "{com = {b = 2}}");
}

/// A whole read of each kind of node of the view: who assembles it, and
/// when, with its contents and their order.
#[test]
fn a_view_node_read_whole_of_each_kind() {
    // One directory's tree, without an import: slice 25's rule, the
    // last contributor's schedule; the view has no node of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "com.one = 1"),
        ("b.gcsl", "com.two = 2\nall = com"),
    ];
    let scan = ws(&files).unwrap();
    assert!(scan.views.is_empty());
    assert_eq!(
        value(&files, "b.gcsl"),
        "{com = {two = 2}; all = {one = 1; two = 2}}"
    );
    // Across directories, with no leaf of the reading file: assembled
    // as the file starts, the root directory's part first.
    let files = [
        ("workspace.gcsl", "com.a = 1"),
        ("sub/b.gcsl", "com.b = 2"),
        ("sub/c.gcsl", "x = 0\nall = com"),
    ];
    assert_eq!(value(&files, "sub/c.gcsl"), "{x = 0; all = {a = 1; b = 2}}");
    assert!(assembled_first(&checked(&files), "sub/c.gcsl"));
    // Across directories, with a leaf of the reading file: a node of
    // its schedule, after its own leaf, so not first.
    let files = [
        ("workspace.gcsl", "com.a = 1"),
        ("sub/b.gcsl", "com.b = 2\nall = com"),
    ];
    assert_eq!(
        value(&files, "sub/b.gcsl"),
        "{com = {b = 2}; all = {a = 1; b = 2}}"
    );
    assert!(!assembled_first(&checked(&files), "sub/b.gcsl"));
    // Across imports only, with no leaf of the reading file: as the
    // file starts, in the imports' source order.
    let files = [
        (
            "app/main.gcsl",
            "import * from tools\nimport * from lib\nall = com",
        ),
        ("lib/x.gcsl", "com.x = 1"),
        ("tools/t.gcsl", "com.t = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{all = {t = 2; x = 1}}");
    assert!(assembled_first(&checked(&files), "app/main.gcsl"));
    // Mixed: a directory above, the file's own directory, an import and
    // a leaf of the file, two levels of the view merged; `com.x` is
    // read whole too, and a part of it alone.
    let files = [
        (
            "app/main.gcsl",
            "import * from lib\ncom.x.app = 1\nall = com\nx = com.x",
        ),
        ("app/other.gcsl", "com.other = 2"),
        ("lib/x.gcsl", "com.x.lib = 3"),
        ("workspace.gcsl", "com.x.root = 4\ncom.base = 5"),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{com = {x = {app = 1}}; all = {x = {root = 4; app = 1; lib = 3}; base = 5; \
             other = 2}; x = {root = 4; app = 1; lib = 3}}"
    );
    let scan = ws(&files).unwrap();
    let main = &scan.files[1];
    // `com` and `com.x`, both with a leaf of main under them.
    assert_eq!((main.view.own, main.view.end - main.view.start), (2, 2));
}

#[test]
fn the_needs_of_a_view_node_are_per_leaf() {
    let base = [
        ("workspace.gcsl", ""),
        ("bad.gcsl", "def bad(): Int { [1][5] }\ncom.bad.v = bad()"),
        ("ok.gcsl", "com.ok.v = 1"),
    ];
    // A leaf read alone needs its own file only: `bad.gcsl` never runs.
    let mut files = base.to_vec();
    files.push(("app/main.gcsl", "com.app.v = 2\na = com.ok.v"));
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{com = {app = {v = 2}}; a = 1}"
    );
    // Read whole, every owner of every leaf runs, `bad.gcsl` too.
    files.pop();
    files.push(("app/main.gcsl", "com.app.v = 2\na = com"));
    assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
    files.pop();
    files.push(("app/main.gcsl", "a = com"));
    assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
}

#[test]
fn conflicts_across_directories() {
    let above = "one is in a directory above";
    // Two leaves, at the directory above, whichever path is first.
    assert_eq!(
        shown_error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
        format!(
            "field `version` is defined in two files at a.gcsl (defined here); \
                 sub/b.gcsl: defined here; note: {above}"
        )
    );
    assert_eq!(
        shown_error(&[("z.gcsl", "version = 1"), ("a/b.gcsl", "version = 2")]),
        format!(
            "field `version` is defined in two files at z.gcsl (defined here); \
                 a/b.gcsl: defined here; note: {above}"
        )
    );
    // A whole value extended by a path below, and the other way.
    let defined = format!(
        "field `com` is already defined at a.gcsl; sub/b.gcsl: defined here; note: {above}"
    );
    assert_eq!(
        shown_error(&[("a.gcsl", "com = 1"), ("sub/b.gcsl", "com.x = 2")]),
        defined
    );
    assert_eq!(
        shown_error(&[("a.gcsl", "com.x = 1"), ("sub/b.gcsl", "com = 2")]),
        defined
    );
    // A `def` is a leaf: with a field, either way, and with a `def`.
    for (a, b) in [
        ("def f(): Int { 1 }", "f = 2"),
        ("f = 2", "def f(): Int { 1 }"),
        ("def f(): Int { 1 }", "def f(): Int { 2 }"),
    ] {
        assert_eq!(
            shown_error(&[("a.gcsl", a), ("sub/b.gcsl", b)]),
            format!(
                "field `f` is defined in two files at a.gcsl (defined here); \
                     sub/b.gcsl: defined here; note: {above}"
            ),
            "{a} / {b}"
        );
    }
    // Once per pair, however many files' views have it.
    assert_eq!(
        ws(&[
            ("a.gcsl", "version = 1"),
            ("sub/b.gcsl", "version = 2"),
            ("sub/c.gcsl", "x = 1"),
            ("sub/d/e.gcsl", "y = 1"),
        ])
        .unwrap_err()
        .len(),
        1
    );
}

#[test]
fn conflicts_through_imports() {
    let lib = [
        ("lib/x.gcsl", "com.x.lib.name = \"x\""),
        ("workspace.gcsl", ""),
    ];
    let with = |main: &'static str, more: &[(&'static str, &'static str)]| {
        let mut files = lib.to_vec();
        files.push(("app/main.gcsl", main));
        files.extend_from_slice(more);
        shown_error(&files)
    };
    // Two leaves, an own leaf and an imported interior node, and an own
    // interior node and an imported leaf: at the import.
    assert_eq!(
        with("import * from lib\ncom.x.lib.name = \"own\"", &[]),
        "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
    );
    assert_eq!(
        with("import * from lib\ncom.x.lib = 1", &[]),
        "field `com.x.lib` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
    );
    assert_eq!(
        with("import * from lib\ncom.x.lib.name.deep = 1", &[]),
        "field `com.x.lib.name` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
    );
    // A member whose bound name is a plain root of the file, a leaf and
    // an interior node of it.
    let version = [("lib/v.gcsl", "version = 1")];
    assert_eq!(
        with("import version from lib\nversion = 2", &version),
        "field `version` is defined in two files at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
    );
    assert_eq!(
        with("import version from lib\nversion.minor = 2", &version),
        "field `version` is already defined at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
    );
    // Against a directory above the file (not above `lib/`), and
    // between two imports, at the later.
    let mut files = lib.to_vec();
    files.push(("app/sub/main.gcsl", "import * from lib"));
    files.push(("app/up.gcsl", "com.x.lib.name = \"up\""));
    assert_eq!(
        shown_error(&files),
        "field `com.x.lib.name` is defined in two files at app/sub/main.gcsl (imported \
             here); lib/x.gcsl: defined here; app/up.gcsl: defined here; note: one is in a \
             directory above"
    );
    assert_eq!(
        with(
            "import * from lib\nimport * from tools",
            &[("tools/t.gcsl", "com.x.lib.name = \"t\"")]
        ),
        "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             tools/t.gcsl: defined here; app/main.gcsl: imported here; lib/x.gcsl: defined here"
    );
}

#[test]
fn a_view_is_the_files_own() {
    // b does not see what a imported with `*`, nor its own directory's
    // tree `lib/`'s part.
    let mut files = vec![
        ("app/a.gcsl", "import * from lib\nx = com.lib.v"),
        ("lib/l.gcsl", "com.lib.v = 1"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "app/a.gcsl"), "{x = 1}");
    files.push(("app/b.gcsl", "y = com.lib.v"));
    assert_eq!(messages(&files, "app/b.gcsl"), ["undefined variable `com`"]);
    files.pop();
    files.push(("app/b.gcsl", "com.app.v = 2\ny = com.lib.v"));
    assert_eq!(
        messages(&files, "app/b.gcsl"),
        ["no field `lib` in `{app: {v: Int}}`"]
    );
}

#[test]
fn star_from_a_value_file_is_an_error() {
    let files = [
        ("app/main.gcsl", "import * from tools/build"),
        ("tools/build.gcsl", "= 1"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "a value file has no members to import at app/main.gcsl"
    );
}

#[test]
fn a_whole_import_still_clashes_with_a_root_of_the_view() {
    let files = |main: &'static str| {
        vec![
            ("app/main.gcsl", main),
            ("lib/x.gcsl", "com.x = 1"),
            ("tools/t.gcsl", "t = 1"),
            ("workspace.gcsl", ""),
        ]
    };
    for main in [
        "import * from lib\nimport tools as com",
        "import tools as com\nimport * from lib",
        "import tools as com\ncom.y = 1",
    ] {
        assert!(
            shown_error(&files(main)).starts_with("`com` is imported in this file"),
            "{main}"
        );
    }
}

#[test]
fn merging_the_same_cell_twice_is_idempotent() {
    let lib = [("lib/x.gcsl", "com.x.lib.name = 1"), ("workspace.gcsl", "")];
    // `*` from one directory twice.
    let mut files = lib.to_vec();
    files.push((
        "app/main.gcsl",
        "import * from lib\nimport * from lib\na = com.x.lib.name\nall = com",
    ));
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{a = 1; all = {x = {lib = {name = 1}}}}"
    );
    assert!(ws(&files).unwrap().views.is_empty());
    // `*` from the file's own directory (`app` inside `app/`), from a
    // file of it, and from a directory above, with its own leaves.
    for main in [
        "import * from app\ncom.main.v = 1\nall = com",
        "import * from app/other\ncom.main.v = 1\nall = com",
        "import * from app/main\ncom.main.v = 1\nall = com",
        "import com from app\ncom.main.v = 1\nall = com",
    ] {
        let mut files = lib.to_vec();
        files.push(("app/main.gcsl", main));
        files.push(("app/other.gcsl", "com.other.v = 2"));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{com = {main = {v = 1}}; all = {main = {v = 1}; other = {v = 2}}}",
            "{main}"
        );
        assert!(ws(&files).unwrap().views.is_empty(), "{main}");
    }
    let mut files = lib.to_vec();
    files.push((
        "app/sub/main.gcsl",
        "import * from app\ncom.sub = 1\nall = com",
    ));
    files.push(("app/other.gcsl", "com.other = 2"));
    assert_eq!(
        value(&files, "app/sub/main.gcsl"),
        "{com = {sub = 1}; all = {other = 2; sub = 1}}"
    );
}

/// A member of the file's own directory bound by another name is not
/// the same cell at the same path: reaching the file's own nodes through
/// it is still the one-file cycle, as through a whole import.
#[test]
fn a_renamed_member_reaching_the_files_own_nodes_needs_itself() {
    let files = [
        (
            "app/main.gcsl",
            "import com as c from app\ncom.x = 1\ny = c.x",
        ),
        ("workspace.gcsl", ""),
    ];
    assert!(shown_error(&files).starts_with(
        "`app/main.gcsl` needs itself at app/main.gcsl (`com.x` is computed when this \
             file runs)"
    ));
}

#[test]
fn an_importing_files_items_merge_in_any_order() {
    let lines = [
        "import * from lib",
        "com.x.app.a = com.x.lib.name + b",
        "b = 2",
        "all = com.x",
    ];
    let reversed: Vec<&str> = lines.iter().rev().copied().collect();
    let orders = (0..lines.len())
        .map(|start| {
            let mut rotated = lines.to_vec();
            rotated.rotate_left(start);
            rotated
        })
        .chain([reversed]);
    for order in orders {
        let main = order.join("\n");
        let files = [
            ("app/main.gcsl", main.as_str()),
            ("lib/x.gcsl", "com.x.lib.name = 1"),
            ("workspace.gcsl", ""),
        ];
        let shown = value(&files, "app/main.gcsl");
        for field in [
            "com = {x = {app = {a = 3}}}",
            "b = 2",
            "all = {app = {a = 3}; lib = {name = 1}}",
        ] {
            assert!(shown.contains(field), "{main}: {shown}");
        }
    }
}

/// The time to check a workspace of `count` sibling directories of the
/// root, each with one file of ten leaves under the root's `com`, each
/// reading one leaf of the root's file through the merged `com`, and to
/// run the last: linear in `count`. Run with `cargo test --release --
/// --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_sibling_directories_merging() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [50_u32, 100, 200, 400, 800] {
        let mut sources = vec![
            SourceFile::new(
                "workspace.gcsl",
                (0..10).fold(String::new(), |mut text, leaf| {
                    writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                    text
                }),
            )
            .unwrap(),
        ];
        for dir in 0..count {
            let mut text = format!("com.d{dir}.l0 = com.base.l3 + 1\n");
            for leaf in 1..10 {
                writeln!(text, "com.d{dir}.l{leaf} = com.d{dir}.l{} + 1", leaf - 1).unwrap();
            }
            sources.push(SourceFile::new(format!("d{dir:04}/f.gcsl"), text).unwrap());
        }
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
        assert!(format!("{}", outcome.value).contains("l9 = 13"));
        writeln!(
            report,
            "{count} directories: scan {scanned:?}; check {checked:?}, check and run {total:?}"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

/// [`benchmark_sibling_directories_merging`] with every file in one
/// directory below the root: each file's view merges the root's `com`
/// with the directory's, which has every file's part, so a file's view
/// must not cost the size of its directory's tree. Linear in `count`.
/// Run with `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_one_directory_merging_with_the_root() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for count in [50_u32, 100, 200, 400, 800] {
        let mut sources = vec![
            SourceFile::new(
                "workspace.gcsl",
                (0..10).fold(String::new(), |mut text, leaf| {
                    writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                    text
                }),
            )
            .unwrap(),
        ];
        for file in 0..count {
            let mut text = format!("com.f{file}.l0 = com.base.l3 + 1\n");
            for leaf in 1..10 {
                let before = leaf - 1;
                writeln!(text, "com.f{file}.l{leaf} = com.f{file}.l{before} + 1").unwrap();
            }
            sources.push(SourceFile::new(format!("app/f{file:04}.gcsl"), text).unwrap());
        }
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace
            .file(&format!("app/f{:04}.gcsl", count - 1))
            .unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        assert!(format!("{}", outcome.value).contains("l9 = 13"));
        writeln!(
            report,
            "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

/// The cost of a file's view in a chain of `depth` directories, each
/// below the one before and with one file of ten leaves under `com`,
/// each reading a leaf of the directory above through the merged `com`:
/// a file's view merges every directory above it, so the time per file
/// grows with its depth. Run with `cargo test --release -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn benchmark_a_deep_chain_of_directories() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    let mut report = String::new();
    for depth in [10_u32, 25, 50, 100] {
        let mut path = String::new();
        let mut sources = Vec::new();
        for level in 0..depth {
            let mut text = if level == 0 {
                String::from("com.d0.l0 = 0\n")
            } else {
                format!("com.d{level}.l0 = com.d{}.l9 + 1\n", level - 1)
            };
            for leaf in 1..10 {
                let before = leaf - 1;
                writeln!(text, "com.d{level}.l{leaf} = com.d{level}.l{before} + 1").unwrap();
            }
            let name = if level == 0 {
                String::from("workspace.gcsl")
            } else {
                write!(path, "d{level}/").unwrap();
                format!("{path}f.gcsl")
            };
            sources.push(SourceFile::new(name, text).unwrap());
        }
        let start = std::time::Instant::now();
        scan(&sources).unwrap();
        let scanned = start.elapsed();
        let start = std::time::Instant::now();
        let mut workspace = check(&sources).unwrap();
        let checked = start.elapsed();
        let last = workspace.file(&format!("{path}f.gcsl")).unwrap();
        let outcome = workspace.run(last).unwrap();
        let total = start.elapsed();
        assert!(format!("{}", outcome.value).contains(&format!("l9 = {}", depth * 10 - 1)));
        let per_file = total / depth;
        writeln!(
            report,
            "{depth} deep: scan {scanned:?}; check {checked:?}, check and run {total:?}, \
                 {per_file:?} a file"
        )
        .unwrap();
    }
    std::io::stdout().write_all(report.as_bytes()).unwrap();
}

// ----- The standard library -----

/// The summary of slice 28: the prelude's names, `builtins` and the
/// prelude imported by name.
const STANDARD_SUMMARY: &str = "xs = [1, 2, 3]\n\
                                    n = len(xs)\n\
                                    h = half(n)\n\
                                    import builtins\n\
                                    same = builtins.len(xs) == n\n\
                                    import len as count from prelude\n\
                                    c = count(xs)";

/// The value of `text` run as a lone file, beside the standard library,
/// or the messages of its diagnostics.
fn lone(text: &str) -> Result<String, Vec<String>> {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    crate::run(&source)
        .map(|outcome| outcome.value.to_string())
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect())
}

#[test]
fn the_standard_summary_runs_in_a_workspace_and_alone() {
    let expected = "{xs = [1, 2, 3]; n = 3; h = 1.5; same = true; c = 3}";
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", STANDARD_SUMMARY)];
    assert_eq!(value(&files, "app/main.gcsl"), expected);
    assert_eq!(lone(STANDARD_SUMMARY).unwrap(), expected);
    // The standard files are the first files, by input number, in a
    // directory of their own.
    let scan = ws(&files).unwrap();
    assert_eq!(scan.standard(), 1);
    assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
    assert_eq!(scan.dirs[scan.files[0].dir as usize].parent, None);
    assert_eq!(scan.order[0], 0);
    // The built-in functions' cells are the first.
    let builtins: Vec<Option<Builtin>> = (0..30).map(|node| scan.builtin(node)).collect();
    assert_eq!(
        builtins,
        [
            Some(Builtin::Len),
            Some(Builtin::Int),
            Some(Builtin::Float),
            Some(Builtin::Exec),
            Some(Builtin::Panic),
            Some(Builtin::Split),
            Some(Builtin::Join),
            Some(Builtin::StartsWith),
            Some(Builtin::EndsWith),
            Some(Builtin::Trim),
            Some(Builtin::StripPrefix),
            Some(Builtin::StripSuffix),
            Some(Builtin::Lines),
            Some(Builtin::Fields),
            Some(Builtin::CompareVersions),
            Some(Builtin::FsExists),
            Some(Builtin::FsRead),
            Some(Builtin::FsMkdir),
            Some(Builtin::FsCopy),
            Some(Builtin::FsTempDir),
            Some(Builtin::FsWrite),
            Some(Builtin::SortVersions),
            Some(Builtin::Show),
            Some(Builtin::ExecLive),
            Some(Builtin::VariantMatches),
            Some(Builtin::VariantPayload),
            Some(Builtin::PartialMatches),
            Some(Builtin::Secret),
            Some(Builtin::SecretValue),
            None
        ]
    );
}

#[test]
fn len_is_reached_by_every_road() {
    for text in [
        "n = len([1, 2])",
        "import len from builtins\nn = len([1, 2])",
        "import builtins\nn = builtins.len([1, 2])",
        "import * from builtins\nn = len([1, 2])",
        "import prelude\nn = prelude.len([1, 2])",
        "let l = len\nn = l([1, 2])",
        "let l = len\nlet m = l\nn = m([1, 2])",
        "l = len\nn = l([1, 2])",
        "import len as count from prelude\nlet l = count\nn = l([1, 2])",
        "import builtins\nl = builtins.len\nn = l([1, 2])",
    ] {
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        let shown = value(&files, "main.gcsl");
        assert!(shown.contains("n = 2"), "{text}: {shown}");
        assert!(lone(text).unwrap().contains("n = 2"), "{text}");
    }
    // An alias of `len` is of its opaque type; the callee of a call of
    // it has the function type of the call.
    let mut workspace = checked(&[("workspace.gcsl", ""), ("main.gcsl", "l = len\nn = l([1])")]);
    let index = workspace.file("main.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(name len : builtin len @4..7)"), "{dump}");
    assert!(
        dump.contains("(name l : fn(List<Int>): Int @12..13)"),
        "{dump}"
    );
}

#[test]
fn an_alias_of_len_crosses_files_directories_and_imports() {
    // A field of the root's, a member import's alias of an alias in
    // another directory, and a `let` of that in a third file.
    let files = [
        ("workspace.gcsl", "measure = len"),
        (
            "lib/a.gcsl",
            "import len as count from prelude\nsize = count",
        ),
        (
            "app/main.gcsl",
            "import size from lib\nlet l = size\nn = l([1, 2]) + measure([1])\n\
                 def twice(v: Int[]): Int { l(v) + size(v) }",
        ),
        ("app/other.gcsl", "m = measure([1, 2, 3]) + twice([1])"),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{n = 3; twice = fn(v: List<Int>): Int}"
    );
    assert_eq!(value(&files, "app/other.gcsl"), "{m = 5}");
    assert_eq!(value(&files, "lib/a.gcsl"), "{size = builtin len}");
    // Read whole, a record holding one is a use of it as a value.
    let mut whole = files.to_vec();
    whole.push(("app/whole.gcsl", "import lib\nall = lib"));
    assert_eq!(
        messages(&whole, "app/whole.gcsl"),
        ["`len` is a built-in function and cannot be used as a value yet"]
    );
}

#[test]
fn int_and_float_are_values_of_their_function_types() {
    for (text, expected) in [
        ("= int(2.7)", "2"),
        ("= int(-2.7)", "-2"),
        ("= int(x = 9.99)", "9"),
        ("= float(3)", "3.0"),
        ("= float(-3)", "-3.0"),
        ("= int", "fn(x: Float): Int"),
        ("import builtins\n= builtins.float", "fn(x: Int): Float"),
        ("import int from builtins\n= int(1.5)", "1"),
        // Passed, stored and called through a variable.
        ("let i = int\n= i(-0.5)", "0"),
        (
            "fn apply(f: fn(x: Float): Int, v: Float): Int { f(v) }\n= apply(int, 7.5)",
            "7",
        ),
        ("let fs = [float, float]\n= fs[1](2)", "2.0"),
        ("let r = { f = float }\n= r.f(4)", "4.0"),
        ("let mut f = int\nf := int\n= f(3.5)", "3"),
        ("= half(3)", "1.5"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // Truncation toward zero fails when the result is no `Int`.
    for (text, message) in [
        ("= int(1e300)", "integer overflow"),
        ("= int(-1e300)", "integer overflow"),
        ("= int(1.0 / 0.0)", "integer overflow"),
        ("= int(0.0 / 0.0)", "cannot truncate NaN to an Int"),
        ("let i = int\n= i(1e300)", "integer overflow"),
    ] {
        assert_eq!(lone(text).unwrap_err(), [message], "{text}");
    }
    assert_eq!(
        lone("= int(9223372036854774784.0)").unwrap(),
        "9223372036854774784"
    );
    assert_eq!(
        lone("= int(-9223372036854775808.0)").unwrap(),
        "-9223372036854775808"
    );
    // Functions have no equality, built-in ones included.
    assert_eq!(
        lone("= int === int").unwrap_err(),
        ["cannot compare functions"]
    );
}

#[test]
fn a_file_s_own_names_shadow_the_prelude_s_silently() {
    // A field, a `def`, a `let` and a `fn`: no clash, and the built-in
    // is gone there.
    for (text, expected) in [
        ("len = 1\nn = len + 1", "{len = 1; n = 2}"),
        (
            "def len(v: Int[]): Int { v[0] + 41 }\nn = len([1])",
            "{len = fn(v: List<Int>): Int; n = 42}",
        ),
        ("let len = 5\nn = len", "{n = 5}"),
        ("fn len(): Int { 7 }\nn = len()", "{n = 7}"),
        ("let int = 2\nn = int * 3", "{n = 6}"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // A `let` shadows from its item on, as it always has.
    assert_eq!(
        lone("n = len([1])\nlet len = 5\nm = len").unwrap(),
        "{n = 1; m = 5}"
    );
    // A field of the directory, or of a directory above, shadows it in
    // every file of the directory, and an explicit import does too.
    let files = [
        ("workspace.gcsl", "len = 3"),
        ("sub/a.gcsl", "n = len + 1"),
        ("sub/b.gcsl", "import int from builtins\nm = len + int(1.5)"),
    ];
    assert_eq!(value(&files, "sub/a.gcsl"), "{n = 4}");
    assert_eq!(value(&files, "sub/b.gcsl"), "{m = 4}");
    // Nothing of the implicit import is an alias the scan knows of.
    let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "len = 1")]).unwrap();
    let names: Vec<&str> = scan.files[2]
        .aliases
        .iter()
        .map(|alias| scan.names.get(alias.name))
        .collect();
    assert_eq!(
        names,
        [
            "show",
            "int",
            "float",
            "half",
            "exec",
            "exec_live",
            "panic",
            "split",
            "join",
            "starts_with",
            "ends_with",
            "trim",
            "strip_prefix",
            "strip_suffix",
            "lines",
            "fields",
            "compare_versions",
            "sort_versions",
            "sh",
            "sh_live",
            "secret"
        ]
    );
    assert!(scan.files[2].aliases.iter().all(|alias| alias.implicit));
    assert!(scan.files[0].aliases.iter().all(|alias| !alias.implicit));
}

#[test]
fn an_explicit_star_from_the_prelude_conflicts_as_any() {
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import * from prelude\nlen = 3"),
    ];
    assert_eq!(
        shown_error(&files),
        "field `len` is defined in two files at main.gcsl (imported here); \
             <stdlib>/prelude.gcsl: defined here; main.gcsl: defined here"
    );
    // The same for the `builtins` module, whose side has no definition.
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import * from builtins\nlen = 3"),
    ];
    assert_eq!(
        shown_error(&files),
        "field `len` is defined in two files at main.gcsl (imported here); \
             main.gcsl: defined here; note: `len` is a built-in function"
    );
    // An explicit import of the prelude's name and the implicit one are
    // one cell: no conflict.
    assert_eq!(
        lone("import * from prelude\nn = len([1, 2])").unwrap(),
        "{n = 2}"
    );
    // A `let` of an imported name is the clash it always was.
    assert_eq!(
        lone("import len from prelude\nlet len = 1").unwrap_err(),
        ["`len` is imported in this file"]
    );
}

#[test]
fn the_standard_modules_names_are_reserved() {
    for (files, message, input) in [
        (
            &[("workspace.gcsl", ""), ("prelude.gcsl", "x = 1")][..],
            "`prelude` is the name of a standard module",
            Some(1),
        ),
        (
            &[("builtins/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
            "`builtins` is the name of a built-in module",
            Some(1),
        ),
        (
            &[("builtins.gcsl", ""), ("workspace.gcsl", "")][..],
            "`builtins` is the name of a built-in module",
            Some(1),
        ),
    ] {
        assert_eq!(
            error(files),
            (message.to_owned(), input, vec![]),
            "{files:?}"
        );
    }
    // Below the root, a name of a path of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/prelude.gcsl", "x = 1"),
        ("main.gcsl", "import lib/prelude\nn = prelude.x + len([1])"),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{n = 2}");
    // A path of more than one segment never names a standard module.
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import prelude/len")];
    assert_eq!(error(&files).0, "no module `prelude/len`");
}

#[test]
fn a_lone_file_imports_only_the_standard_library() {
    assert_eq!(
        lone("import nothing").unwrap_err(),
        [
            "`nothing` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
        ]
    );
    assert_eq!(
        lone("x = 1 + true\nimport a from c/d").unwrap_err(),
        [
            "cannot add Int and Bool",
            "`c/d` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
        ]
    );
    assert_eq!(lone("import builtins\n= builtins.len([1])").unwrap(), "1");
    assert_eq!(lone("import prelude\n= prelude.half(1)").unwrap(), "0.5");
    // Its diagnostics name no input; a standard file's names its own.
    let source = SourceFile::new("lone.gcsl", "x = 1 + true").unwrap();
    assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
    let source = SourceFile::new("lone.gcsl", "import * from prelude\nlen = 1").unwrap();
    let error = &crate::run(&source).unwrap_err()[0];
    assert_eq!(error.input, None);
    let inputs: Vec<Option<u32>> = error.labels.iter().map(|l| l.input).collect();
    assert_eq!(inputs, [Some(0), None]);
}

#[test]
fn the_table_of_standard_files_is_the_stdlib_directory() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib");
    let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "gcsl"))
        .map(|path| {
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            (stem, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort();
    let table: Vec<(String, String)> = crate::STANDARD
        .iter()
        .map(|&(stem, text)| (stem.to_owned(), text.to_owned()))
        .collect();
    assert_eq!(table, found);
    // Each is a standard file, in path order, named under `<stdlib>/`.
    let names: Vec<String> = standard_sources()
        .iter()
        .map(|source| source.name().to_owned())
        .collect();
    let expected: Vec<String> = found
        .iter()
        .map(|(stem, _)| format!("<stdlib>/{stem}.gcsl"))
        .collect();
    assert_eq!(names, expected);
}

// ----- `exec`, `panic` and `sh` -----

/// The summary of slice 29, its `ls` of a directory every machine has.
const SHELL_SUMMARY: &str = "r = exec([\"ls\", \"-1d\", \"/\"])\n\
                                 lines = r.stdout\n\
                                 ok = r.status == 0\n\
                                 out = sh(\"echo hi\")\n\
                                 fn pkg(name: String): { build: Future<String> } {\n\
                                     { build = lazy { sh(\"echo make {name}\") } }\n\
                                 }\n\
                                 built = await pkg(\"lib\").build\n\
                                 fn guard(n: Int): Int {\n\
                                     if n < 0 { panic(\"negative size {n}\") }\n\
                                     n\n\
                                 }\n\
                                 size = guard(3)";

/// The one diagnostic of running `text` as a lone file.
fn lone_error(text: &str) -> Diagnostic {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let mut errors = crate::run(&source).unwrap_err();
    assert_eq!(errors.len(), 1, "{text}: {errors:?}");
    errors.remove(0)
}

#[test]
fn the_shell_summary_runs_in_a_workspace_and_alone() {
    let expected = "{r = {status = 0; stdout = \"/\\n\"; stderr = \"\"}; lines = \"/\\n\"; \
                        ok = true; out = \"hi\\n\"; built = \"make lib\\n\"; size = 3}";
    assert_eq!(lone(SHELL_SUMMARY).unwrap(), expected);
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", SHELL_SUMMARY)];
    assert_eq!(value(&files, "app/main.gcsl"), expected);
    assert_eq!(
        lone_error(
            "fn guard(n: Int): Int {\nif n < 0 { panic(\"negative size {n}\") }\nn\n}\n= guard(-2)"
        )
        .message,
        "negative size -2"
    );
}

#[test]
fn exec_reports_what_its_program_did() {
    for (text, expected) in [
        (
            "= exec([\"true\"])",
            "{status = 0; stdout = \"\"; stderr = \"\"}",
        ),
        (
            "= exec([\"false\"])",
            "{status = 1; stdout = \"\"; stderr = \"\"}",
        ),
        (
            "= exec([\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"])",
            "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
        ),
        // A non-zero status is data, never an error.
        ("= exec([\"false\"]).status != 0", "true"),
        // Killed by a signal: 128 plus its number.
        ("= exec([\"sh\", \"-c\", \"kill -9 $$\"]).status", "137"),
        // Its standard input is the null device: `cat` ends at once.
        (
            "= exec([\"cat\"])",
            "{status = 0; stdout = \"\"; stderr = \"\"}",
        ),
        // Invalid UTF-8 is replaced.
        (
            "= exec([\"sh\", \"-c\", \"printf '\\\\377ok'\"]).stdout",
            "\u{FFFD}ok",
        ),
        // The arguments are passed as they are, with no shell.
        (
            "= exec([\"printf\", \"%s|\", \"a b\", \"$HOME\", \"*\"]).stdout",
            "a b|$HOME|*|",
        ),
        // Named, through a variable, a field, a `->` function.
        ("= exec(argv = [\"echo\", \"n\"]).stdout", "n\n"),
        ("let e = exec\n= e([\"echo\", \"v\"]).stdout", "v\n"),
        (
            "let r = { run = exec }\n= r.run([\"echo\", \"f\"]).stdout",
            "f\n",
        ),
        (
            "fn run(a: String) -> String { exec([\"echo\", a]).stdout }\n= run(\"i\")",
            "i\n",
        ),
        (
            "fn pick(f: fn(argv: Present<List<String>>) -> {status: Int}): \
                 fn(Present<List<String>>) -> {status: Int} { f }\n\
                 = pick(exec)([\"false\"]).status",
            "1",
        ),
        (
            "import exec as run from builtins\n= run([\"true\"]).status",
            "0",
        ),
        // A future argument is awaited, as any call's.
        (
            "let xs = lazy [\"echo\", \"later\"]\n= exec(xs).stdout",
            "later\n",
        ),
        (
            "let xs = [lazy \"echo\", \"deep\"]\n= exec(xs).stdout",
            "deep\n",
        ),
        // The value shows as its type.
        (
            "= exec",
            "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}",
        ),
        ("= panic", "fn(message: String): Never"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
}

#[test]
fn exec_fails_only_when_its_program_cannot_run() {
    let text = "let none: List<String> = []\nr = exec(none)";
    let error = lone_error(text);
    assert_eq!(error.message, "`exec` needs a program to run");
    assert_eq!(error.span, Span::new(32, 42));
    assert!(error.notes.is_empty());
    let text = "r = exec([\"gcsl-no-such-program-slice-29\", \"x\"])";
    let error = lone_error(text);
    assert_eq!(error.message, "cannot run `gcsl-no-such-program-slice-29`");
    assert_eq!(error.span, Span::new(4, 48));
    assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
    assert_eq!(error.input, None);
    // A directory is not a program.
    let error = lone_error("r = exec([\"/\"])");
    assert_eq!(error.message, "cannot run `/`");
    assert_eq!(error.notes, ["Permission denied (os error 13)"]);
}

#[test]
fn panic_ends_the_run_with_its_message_at_the_call() {
    let text = "x = 1\ny = panic(\"x\")";
    let error = lone_error(text);
    assert_eq!(error.message, "x");
    assert_eq!(error.span, Span::new(10, 20));
    assert!(error.labels.is_empty() && error.notes.is_empty());
    assert_eq!(lone_error("panic(\"one\\ntwo\")").message, "one\ntwo");
    // Wherever it stands, whichever walker meets it: a statement, a
    // `let`, a branch, an operand of each kind, an element, a field, an
    // argument, a callee, an interpolation, a function's result, a
    // `lazy`, a loop.
    for text in [
        "panic(\"p\")",
        "let x = panic(\"p\")",
        "let c = true\nx = if c { panic(\"p\") } else { 1 }",
        "x = 1 + panic(\"p\")",
        "x = 1.5 * panic(\"p\")",
        "x = true and panic(\"p\")",
        "x = \"a\" + panic(\"p\")",
        "x = \"a{panic(\"p\")}\"",
        "x = [1, panic(\"p\")]",
        "x = len(panic(\"p\"))",
        "x = { a = panic(\"p\") }",
        "let r: {a: Int} = panic(\"p\")",
        "x = float(panic(\"p\"))",
        "x = panic(\"p\")(1)",
        "fn f(n: Int): Int { if n > 0 { panic(\"p\") }\nn }\nx = f(1)",
        "fn f(): String { panic(\"p\") }\nx = f() + \"!\"",
        "x = await lazy panic(\"p\")",
        "x = for i in [1] { panic(\"p\") }",
        "def f(n: Int) ->> Int { panic(\"p\") }\nx = f(1)",
        // As a value of a parameter of type `fn(String): Int`.
        "fn apply(f: fn(String): Int): Int { f(\"p\") }\nx = apply(panic)",
        "let p = panic\nx = p(\"p\")",
    ] {
        assert_eq!(lone_error(text).message, "p", "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(messages(&files, "main.gcsl"), ["p"], "{text}");
    }
    // What follows a panic never runs.
    assert_eq!(
        lone_error("let c = true\nif c { panic(\"first\") }\n= 1 // 0").message,
        "first"
    );
    // A message of the program's own wording, built at runtime.
    assert_eq!(
        lone_error("let n = 3\n= panic(\"{n} is {n * 2 // 2}\")").message,
        "3 is 3"
    );
}

#[test]
fn sh_returns_its_output_or_panics_in_the_prelude() {
    assert_eq!(lone("= sh(\"echo hi\")").unwrap(), "hi\n");
    assert_eq!(lone("= sh(\"printf hi\")").unwrap(), "hi");
    // The panic is the prelude's, located in it: input 0.
    let error = lone_error("x = sh(\"echo nope >&2; exit 2\")");
    assert_eq!(
        error.message,
        "`echo nope >&2; exit 2` exited with status 2: nope"
    );
    assert_eq!(error.input, Some(0));
    let prelude = Library::NEEDED.find("prelude").unwrap();
    let prelude = crate::STANDARD[prelude].1;
    let at = u32::try_from(prelude.find("panic(\"`").unwrap()).unwrap();
    assert_eq!(error.span.start, at);
    // With no standard error, the message ends in ": ".
    assert_eq!(
        lone_error("x = sh(\"exit 2\")").message,
        "`exit 2` exited with status 2: "
    );
    let files = [("workspace.gcsl", ""), ("main.gcsl", "x = sh(\"exit 2\")")];
    assert_eq!(
        run_file(&files, "main.gcsl").unwrap_err(),
        [("`exit 2` exited with status 2: ".to_owned(), Some(0))]
    );
}

#[test]
fn a_memoised_function_around_exec_runs_its_program_once() {
    let path = std::env::temp_dir().join(format!("gcsl-slice-29-{}", std::process::id()));
    let path = path.to_str().unwrap();
    let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
    let _ = std::fs::remove_file(path);
    let memo = format!(
        "fn mark(p: String) ->> Int {{ exec([\"sh\", \"-c\", \"echo x >> \" + p]).status }}\n\
             = [mark(\"{path}\"), mark(\"{path}\")]"
    );
    assert_eq!(lone(&memo).unwrap(), "[0, 0]");
    assert_eq!(count(), 1);
    // An impure function that is not memoised runs it each time.
    let impure = memo.replace("->>", "->");
    assert_eq!(lone(&impure).unwrap(), "[0, 0]");
    assert_eq!(count(), 3);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn exec_panic_and_sh_are_shadowed_silently() {
    for (text, expected) in [
        ("exec = 1\nn = exec + 1", "{exec = 1; n = 2}"),
        ("let panic = \"no\"\nn = panic", "{n = \"no\"}"),
        (
            "def sh(n: Int): Int { n * 2 }\nn = sh(2)",
            "{sh = fn(n: Int): Int; n = 4}",
        ),
        (
            "fn panic(m: String): Int { 7 }\nn = panic(\"x\")",
            "{n = 7}",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
}

#[test]
fn every_record_exec_builds_shares_the_runs_one_layout() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "ra = exec([\"true\"])\n\
                 def run() -> {status: Int, stdout: String, stderr: String} { exec([\"true\"]) }",
        ),
        (
            "b.gcsl",
            "rb = exec([\"false\"])\nlet e = exec\ns = e([\"true\"])\nt = run()",
        ),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    // The driver wrote the twenty-nine built-in functions' cells.
    assert_eq!(assert_builtins_written(&workspace).len(), 29);
    let a = workspace.file("a.gcsl").unwrap();
    let b = workspace.file("b.gcsl").unwrap();
    let first = workspace.run(a).unwrap().value;
    let second = workspace.run(b).unwrap().value;
    let layout = |value: &Value, field: &str| {
        let Value::Record(record) = value else {
            panic!("not a record: {value}");
        };
        let name = workspace.scan.names.lookup(field).unwrap();
        let Some(Value::Record(result)) = record.get(name) else {
            panic!("no record {field} in {value}");
        };
        Rc::clone(&result.layout)
    };
    for (value, field) in [
        (&first, "ra"),
        (&second, "rb"),
        (&second, "s"),
        (&second, "t"),
    ] {
        assert!(
            Rc::ptr_eq(&layout(value, field), &workspace.exec_result),
            "{field} of {value}"
        );
    }
    assert_eq!(
        &*workspace.exec_result.texts,
        ["status", "stdout", "stderr"].map(Box::from)
    );
}

// ----- `exec_live` and `sh_live` -----
//
// What `exec_live` copies goes to this process's standard error, which
// the test harness does not capture: these programs say little. The
// copy itself, and output larger than a pipe holds, are the CLI's tests.

#[test]
fn exec_live_gives_the_record_exec_gives() {
    for (argv, expected) in [
        ("[\"true\"]", "{status = 0; stdout = \"\"; stderr = \"\"}"),
        ("[\"false\"]", "{status = 1; stdout = \"\"; stderr = \"\"}"),
        (
            "[\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"]",
            "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
        ),
        // Killed by a signal: 128 plus its number.
        (
            "[\"sh\", \"-c\", \"kill -9 $$\"]",
            "{status = 137; stdout = \"\"; stderr = \"\"}",
        ),
        // Its standard input is the null device: `cat` ends at once.
        ("[\"cat\"]", "{status = 0; stdout = \"\"; stderr = \"\"}"),
        // Invalid UTF-8 is replaced in the record, on either stream.
        (
            "[\"sh\", \"-c\", \"printf '\\\\377ok'; printf 'e\\\\376' >&2\"]",
            "{status = 0; stdout = \"\u{FFFD}ok\"; stderr = \"e\u{FFFD}\"}",
        ),
        // A last line without a newline is captured as it is.
        (
            "[\"sh\", \"-c\", \"printf 'a\\\\nb'\"]",
            "{status = 0; stdout = \"a\\nb\"; stderr = \"\"}",
        ),
        // The arguments are passed as they are, with no shell.
        (
            "[\"printf\", \"%s|\", \"a b\", \"$HOME\", \"*\"]",
            "{status = 0; stdout = \"a b|$HOME|*|\"; stderr = \"\"}",
        ),
    ] {
        for builtin in ["exec", "exec_live"] {
            let text = format!("= {builtin}({argv})");
            assert_eq!(lone(&text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text.as_str())];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }
    // Named, through a variable, a field, an impure function, an import,
    // and a future argument, as `exec`.
    for (text, expected) in [
        ("= exec_live(argv = [\"echo\", \"n\"]).stdout", "n\n"),
        ("let e = exec_live\n= e([\"echo\", \"v\"]).stdout", "v\n"),
        (
            "fn run(a: String) -> String { exec_live([\"echo\", a]).stdout }\n= run(\"i\")",
            "i\n",
        ),
        (
            "import exec_live as run from builtins\n= run([\"true\"]).status",
            "0",
        ),
        (
            "let xs = lazy [\"echo\", \"later\"]\n= exec_live(xs).stdout",
            "later\n",
        ),
        // The value shows as its type, `exec`'s.
        (
            "= exec_live",
            "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
    }
}

#[test]
fn exec_live_fails_as_exec_does_naming_itself() {
    let text = "let none: List<String> = []\nr = exec_live(none)";
    let error = lone_error(text);
    assert_eq!(error.message, "`exec_live` needs a program to run");
    assert_eq!(error.span, Span::new(32, 47));
    assert!(error.notes.is_empty());
    let text = "r = exec_live([\"gcsl-no-such-program-slice-46\", \"x\"])";
    let error = lone_error(text);
    assert_eq!(error.message, "cannot run `gcsl-no-such-program-slice-46`");
    assert_eq!(error.span, Span::new(4, 53));
    assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
    let error = lone_error("r = exec_live([\"/\"])");
    assert_eq!(error.message, "cannot run `/`");
    assert_eq!(error.notes, ["Permission denied (os error 13)"]);
    // `exec`'s own message is unchanged.
    assert_eq!(
        lone_error("let none: List<String> = []\nr = exec(none)").message,
        "`exec` needs a program to run"
    );
}

#[test]
fn every_record_exec_live_builds_shares_the_runs_one_layout() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "ra = exec_live([\"true\"])\nlet e = exec_live\ns = e([\"false\"])\nt = exec([\"true\"])",
        ),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    let a = workspace.file("a.gcsl").unwrap();
    let Value::Record(record) = workspace.run(a).unwrap().value else {
        panic!("not a record");
    };
    for field in ["ra", "s", "t"] {
        let name = workspace.scan.names.lookup(field).unwrap();
        let Some(Value::Record(result)) = record.get(name) else {
            panic!("no record {field}");
        };
        assert!(
            Rc::ptr_eq(&result.layout, &workspace.exec_result),
            "{field}"
        );
    }
}

#[test]
fn sh_live_returns_its_output_or_panics_without_its_standard_error() {
    assert_eq!(lone("= sh_live(\"echo hi\")").unwrap(), "hi\n");
    assert_eq!(lone("= sh_live(\"printf hi\")").unwrap(), "hi");
    // The panic is the prelude's, located in it, at `sh_live`'s own
    // `panic`, and has no standard error in it.
    let error = lone_error("x = sh_live(\"echo nope >&2; exit 2\")");
    assert_eq!(
        error.message,
        "`echo nope >&2; exit 2` exited with status 2"
    );
    assert_eq!(error.input, Some(0));
    let prelude = Library::NEEDED.find("prelude").unwrap();
    let prelude = crate::STANDARD[prelude].1;
    let def = prelude.find("def sh_live(").unwrap();
    let at = def + prelude[def..].find("panic(\"`").unwrap();
    assert_eq!(error.span.start, u32::try_from(at).unwrap());
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "x = sh_live(\"exit 2\")"),
    ];
    assert_eq!(
        run_file(&files, "main.gcsl").unwrap_err(),
        [("`exit 2` exited with status 2".to_owned(), Some(0))]
    );
    // `sh`'s message is unchanged.
    assert_eq!(
        lone_error("x = sh(\"echo nope >&2; exit 2\")").message,
        "`echo nope >&2; exit 2` exited with status 2: nope"
    );
    // Impure, as `sh`: from a pure function, the existing error.
    let found = lone("fn f(): String { sh_live(\"true\") }\n= f()").unwrap_err();
    assert_eq!(
        found,
        ["cannot call an impure function from a pure function"]
    );
}

// ----- Strings, `path`, and standard modules as scopes of their own -----

/// The summary of slice 30, its `sh` a command every machine has.
const STRINGS_SUMMARY: &str = "parts = split(\"a/b/c\", \"/\")\n\
                                   s = join(parts, \"-\")\n\
                                   yes = starts_with(\"/usr\", \"/\")\n\
                                   gz = ends_with(\"x.tar.gz\", \".gz\")\n\
                                   hash = trim(sh(\"echo ' abc '\"))\n\
                                   import path\n\
                                   p = path.join(\"/src/pkg\", \"../lib\")\n\
                                   q = path.join(\"/src/pkg\", \"/etc\")\n\
                                   d = path.dirname(\"/src/pkg/a.c\")\n\
                                   b = path.basename(\"/src/pkg/a.c\")\n\
                                   n = path.normalize(\"a/./b/../c/\")\n\
                                   abs = path.is_absolute(\"a\")";

/// The prelude's entry of the table of standard files, for a test's
/// own table.
fn prelude_entry() -> (&'static str, &'static str) {
    crate::STANDARD[Library::NEEDED.find("prelude").unwrap()]
}

/// The workspace of `files` checked beside the standard library of
/// `table`, which it loads as a workspace does, with the names of the
/// standard files it loaded.
fn checked_beside(
    table: &[(&str, &str)],
    files: &[(&str, &str)],
) -> (Vec<String>, Result<Workspace, Vec<Diagnostic>>) {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let library = Library {
        table,
        every: false,
    };
    let (standard, checked) = check_in((&sources, Placing::Workspace(&[])), library, "/");
    let names = standard.iter().map(|s| s.name().to_owned()).collect();
    (names, checked)
}

/// The value of the file at `path` of the workspace of `files`, checked
/// beside the standard library of `table`, with the names of the
/// standard files it loaded.
fn value_beside(
    table: &[(&str, &str)],
    files: &[(&str, &str)],
    path: &str,
) -> (Vec<String>, String) {
    let (standard, checked) = checked_beside(table, files);
    let mut workspace = checked.unwrap();
    let index = workspace.file(path).unwrap();
    (standard, workspace.run(index).unwrap().value.to_string())
}

#[test]
fn the_strings_summary_runs_in_a_workspace_and_alone() {
    let expected = "{parts = [\"a\", \"b\", \"c\"]; s = \"a-b-c\"; yes = true; gz = true; \
                        hash = \"abc\"; p = \"/src/lib\"; q = \"/etc\"; d = \"/src/pkg\"; \
                        b = \"a.c\"; n = \"a/c\"; abs = false}";
    assert_eq!(lone(STRINGS_SUMMARY).unwrap(), expected);
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", STRINGS_SUMMARY)];
    assert_eq!(value(&files, "app/main.gcsl"), expected);
    // `path` is loaded, and takes the first input number, before the
    // prelude, in path order; each is a directory of its own.
    let scan = ws(&files).unwrap();
    assert_eq!(scan.standard(), 2);
    assert_eq!(scan.files[0].path, "<stdlib>/path.gcsl");
    assert_eq!(scan.files[1].path, "<stdlib>/prelude.gcsl");
    assert_ne!(scan.files[0].dir, scan.files[1].dir);
    for file in &scan.files[..2] {
        assert_eq!(scan.dirs[file.dir as usize].parent, None);
    }
    // It needs the prelude, whose `split` it calls: the prelude runs
    // first.
    assert_eq!(scan.needs[0], [1]);
    assert_eq!(&scan.order[..2], [1, 0]);
}

#[test]
fn split_gives_the_pieces_between_its_separator() {
    for (text, expected) in [
        ("= split(\"a/b/c\", \"/\")", "[\"a\", \"b\", \"c\"]"),
        ("= split(\"/a/b\", \"/\")", "[\"\", \"a\", \"b\"]"),
        ("= split(\"\", \"/\")", "[\"\"]"),
        ("= split(\"a//b\", \"/\")", "[\"a\", \"\", \"b\"]"),
        ("= split(\"a/\", \"/\")", "[\"a\", \"\"]"),
        ("= split(\"/\", \"/\")", "[\"\", \"\"]"),
        ("= split(\"abc\", \"/\")", "[\"abc\"]"),
        // A separator longer than one byte, and longer than the text.
        ("= split(\"a::b::\", \"::\")", "[\"a\", \"b\", \"\"]"),
        ("= split(\"a:::b\", \"::\")", "[\"a\", \":b\"]"),
        ("= split(\"ab\", \"abc\")", "[\"ab\"]"),
        // Text of several bytes a character, either side.
        (
            "= split(\"h\u{e9}llo w\u{f6}rld\", \"\u{f6}\")",
            "[\"h\u{e9}llo w\", \"rld\"]",
        ),
        (
            "= split(\"\u{65e5}\u{672c}\u{8a9e}\", \"\u{672c}\")",
            "[\"\u{65e5}\", \"\u{8a9e}\"]",
        ),
        ("= split(\"a\u{2192}b\", \"\u{2192}\")", "[\"a\", \"b\"]"),
        // Through a variable, by name.
        (
            "let f = split\n= f(sep = \",\", s = \"1,2\")",
            "[\"1\", \"2\"]",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // An empty separator is a runtime error at the call.
    let text = "x = 1\ny = split(\"abc\", \"\")";
    let error = lone_error(text);
    assert_eq!(error.message, "`split` needs a separator that is not empty");
    assert_eq!(error.span, Span::new(10, 26));
    assert_eq!(error.input, None);
    let error = lone_error("let sep = \"\"\nlet f = split\nx = f(\"a\", sep)");
    assert_eq!(error.message, "`split` needs a separator that is not empty");
}

#[test]
fn join_starts_with_ends_with_and_trim_work_on_strings() {
    for (text, expected) in [
        ("= join([\"a\", \"b\", \"c\"], \"-\")", "a-b-c"),
        ("let none: List<String> = []\n= join(none, \",\")", ""),
        ("= join([], \",\")", ""),
        ("= join([\"a\"], \",\")", "a"),
        ("= join([\"\", \"\"], \"/\")", "/"),
        ("= join([\"a\", \"b\"], \"\")", "ab"),
        ("= join([\"x\", \"y\"], \" \u{2192} \")", "x \u{2192} y"),
        ("= join(split(\"/a//b/\", \"/\"), \"/\")", "/a//b/"),
        ("= starts_with(\"/usr\", \"/\")", "true"),
        ("= starts_with(\"usr\", \"/\")", "false"),
        ("= starts_with(\"\", \"\")", "true"),
        ("= starts_with(\"a\", \"\")", "true"),
        ("= starts_with(\"\", \"a\")", "false"),
        ("= starts_with(\"/\", \"/usr\")", "false"),
        ("= ends_with(\"x.tar.gz\", \".gz\")", "true"),
        ("= ends_with(\"gz\", \".gz\")", "false"),
        ("= ends_with(\"a\", \"\")", "true"),
        // Byte-wise: a precomposed `é` is not an `e` with a combining
        // accent, though they look alike.
        ("= starts_with(\"\u{e9}t\u{e9}\", \"\u{e9}\")", "true"),
        ("= starts_with(\"e\\u{301}t\u{e9}\", \"\u{e9}\")", "false"),
        ("= ends_with(\"caf\u{e9}\", \"\u{e9}\")", "true"),
        ("= ends_with(\"cafe\\u{301}\", \"\u{e9}\")", "false"),
        ("= trim(\"  a b \\n\\t\")", "a b"),
        ("= trim(\"x\")", "x"),
        ("= trim(\"\")", ""),
        // Whitespace alone is nothing; Unicode's whitespace is trimmed
        // too, a no-break space and an ideographic space among it.
        ("= trim(\" \\n\\t\\r \")", ""),
        ("= trim(\"\\u{3000}x\\u{a0}\")", "x"),
        ("= trim(\"\\u{3000}\")", ""),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // Their values show as their types.
    for (text, expected) in [
        ("= split", "fn(s: String, sep: String): List<String>"),
        ("= join", "fn(parts: List<String>, sep: String): String"),
        ("= starts_with", "fn(s: String, prefix: String): Bool"),
        ("= ends_with", "fn(s: String, suffix: String): Bool"),
        ("= trim", "fn(s: String): String"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
    }
}

#[test]
fn path_handles_paths_lexically() {
    for (call, expected) in [
        // `normalize`, on the spec's examples and the edges.
        ("normalize(\"a/./b/../c/\")", "a/c"),
        ("normalize(\"\")", "."),
        ("normalize(\".\")", "."),
        ("normalize(\"./\")", "."),
        ("normalize(\"/\")", "/"),
        ("normalize(\"//\")", "/"),
        ("normalize(\"/../a\")", "/a"),
        ("normalize(\"../../a/..\")", "../.."),
        ("normalize(\"a/b/\")", "a/b"),
        ("normalize(\"a/b//\")", "a/b"),
        ("normalize(\"/a/b/\")", "/a/b"),
        ("normalize(\"a/../..\")", ".."),
        ("normalize(\"/a/../..\")", "/"),
        ("normalize(\"..\")", ".."),
        ("normalize(\"a//b\")", "a/b"),
        // `join`.
        ("join(\"/src/pkg\", \"../lib\")", "/src/lib"),
        ("join(\"/src/pkg\", \"/etc\")", "/etc"),
        ("join(\"/x\", \"\")", "/x"),
        ("join(\"\", \"a/b/\")", "a/b"),
        ("join(\"\", \"\")", "."),
        ("join(\"a\", \"b\")", "a/b"),
        ("join(\"a/\", \"../..\")", ".."),
        ("join(\"/\", \"..\")", "/"),
        ("join(\".\", \".\")", "."),
        // `dirname`.
        ("dirname(\"/src/pkg/a.c\")", "/src/pkg"),
        ("dirname(\"a\")", "."),
        ("dirname(\"/a\")", "/"),
        ("dirname(\"/\")", "/"),
        ("dirname(\"..\")", "."),
        ("dirname(\".\")", "."),
        ("dirname(\"\")", "."),
        ("dirname(\"//\")", "/"),
        ("dirname(\"a/../..\")", "."),
        ("dirname(\"/a/../..\")", "/"),
        ("dirname(\"a/b/\")", "a"),
        ("dirname(\"../../a\")", "../.."),
        // `basename`.
        ("basename(\"/src/pkg/a.c\")", "a.c"),
        ("basename(\"/\")", "/"),
        ("basename(\".\")", "."),
        ("basename(\"\")", "."),
        ("basename(\"//\")", "/"),
        ("basename(\"a/b/\")", "b"),
        ("basename(\"a/../..\")", ".."),
        ("basename(\"/a/../..\")", "/"),
        ("basename(\"a\")", "a"),
        // `is_absolute`.
        ("is_absolute(\"a\")", "false"),
        ("is_absolute(\"/\")", "true"),
        ("is_absolute(\"\")", "false"),
        ("is_absolute(\"//x\")", "true"),
    ] {
        let text = format!("import path\n= path.{call}");
        assert_eq!(lone(&text).unwrap(), expected, "{call}");
    }
    // In a workspace, as a member.
    let files = [
        ("workspace.gcsl", ""),
        (
            "main.gcsl",
            "import dirname from path\nd = dirname(\"a/b/c\")",
        ),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{d = \"a/b\"}");
}

#[test]
fn path_is_not_loaded_when_nobody_imports_it() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/a.gcsl", "x = [1]\ny = len(x)"),
        ("main.gcsl", "import lib/a\nz = a.y + 1"),
    ];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
    let mut workspace = checked.unwrap();
    assert!(
        workspace
            .scan
            .files
            .iter()
            .all(|f| f.path != "<stdlib>/path.gcsl")
    );
    // Its input numbers and slots are those of a run with no `path` at
    // all, beside the prelude alone.
    let (alone, bare) = checked_beside(&[prelude_entry()], &files);
    assert_eq!(alone, standard);
    let mut bare = bare.unwrap();
    let paths =
        |w: &Workspace| -> Vec<String> { w.scan.files.iter().map(|f| f.path.clone()).collect() };
    assert_eq!(paths(&workspace), paths(&bare));
    assert_eq!(workspace.scan.cells(), bare.scan.cells());
    assert_eq!(workspace.slot_names, bare.slot_names);
    let main = workspace.file("main.gcsl").unwrap();
    assert_eq!(main, 2);
    assert_eq!(workspace.dump_ir(main), bare.dump_ir(main));
    assert_eq!(
        workspace.run(main).unwrap().value,
        bare.run(main).unwrap().value
    );
    assert_eq!(workspace.frame.len(), bare.frame.len());
    // A lone file too: the prelude is input 0, the file the next.
    let source = SourceFile::new("lone.gcsl", "n = len([1])").unwrap();
    let (standard, checked) = check_lone_with_standard(&source, "/");
    assert_eq!(standard.len(), 1);
    assert_eq!(checked.unwrap().file("lone.gcsl"), Some(1));
}

#[test]
fn path_is_loaded_by_any_form_of_import() {
    for text in [
        "import path\nv = path.join(\"a\", \"b\")",
        "import path as p\nv = p.join(\"a\", \"b\")",
        "import join from path\nv = join(\"a\", \"b\")",
        "import join as pj from path\nv = pj(\"a\", \"b\")",
        "import normalize, join from path\nv = join(\"a\", \"b\")",
        // `*` binds its `join`, to which the prelude's yields.
        "import * from path\nv = join(\"a\", \"b\")",
    ] {
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
        assert_eq!(standard, ["<stdlib>/path.gcsl", "<stdlib>/prelude.gcsl"]);
        assert_eq!(value, "{v = \"a/b\"}", "{text}");
        assert_eq!(lone(text).unwrap(), "{v = \"a/b\"}", "{text}");
    }
    // By any scanned file, even one the file run does not need: every
    // file is checked, and the standard files loaded run first.
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "v = 1"),
        ("other.gcsl", "import path\nw = path.normalize(\"a/.\")"),
    ];
    let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
    assert_eq!(standard.len(), 2);
    assert_eq!(value, "{v = 1}");
    let workspace = checked(&files);
    assert_eq!(workspace.file("main.gcsl"), Some(2));
    assert_eq!(workspace.scan.needed(2), [1, 0, 2]);
    // An explicit `*` from both is the conflict of any two.
    let files = [
        ("workspace.gcsl", ""),
        (
            "main.gcsl",
            "import * from path\nimport * from prelude\nv = 1",
        ),
    ];
    assert!(
        shown_error(&files).starts_with("field `join` is defined in two files at main.gcsl"),
        "{}",
        shown_error(&files)
    );
}

#[test]
fn its_input_numbers_follow_the_standard_files_loaded() {
    // A syntax error is numbered after the standard files the files
    // that parsed need.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "import path"),
        ("b.gcsl", "y = ("),
    ];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard.len(), 2);
    assert_eq!(checked.unwrap_err()[0].input, Some(3));
    // A file with a syntax error imports what its recovered tree does.
    let files = [("workspace.gcsl", ""), ("a.gcsl", "import path\ny = (")];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard, ["<stdlib>/path.gcsl", "<stdlib>/prelude.gcsl"]);
    assert_eq!(checked.unwrap_err()[0].input, Some(2));
    // Run alone with `path`, the prelude is input 1, and the file's
    // diagnostics name no input.
    let error = lone_error("import path\nx = sh(\"exit 2\")");
    assert_eq!(error.input, Some(1));
    let source = SourceFile::new("lone.gcsl", "import path\nx = 1 + true").unwrap();
    assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
}

#[test]
fn a_standard_module_only_another_imports_is_loaded() {
    // A table of two modules, `a` importing `b`, beside the prelude.
    let table = [
        ("a", "import b\nx = b.y + 1"),
        ("b", "y = len([1, 2])"),
        prelude_entry(),
    ];
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import a\nv = a.x")];
    let (standard, value) = value_beside(&table, &files, "main.gcsl");
    assert_eq!(
        standard,
        [
            "<stdlib>/a.gcsl",
            "<stdlib>/b.gcsl",
            "<stdlib>/prelude.gcsl"
        ]
    );
    assert_eq!(value, "{v = 3}");
    // `b` alone; and neither when nobody imports them.
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import y from b\nv = y"),
    ];
    let (standard, value) = value_beside(&table, &files, "main.gcsl");
    assert_eq!(standard, ["<stdlib>/b.gcsl", "<stdlib>/prelude.gcsl"]);
    assert_eq!(value, "{v = 2}");
    let files = [("workspace.gcsl", ""), ("main.gcsl", "v = 1")];
    let (standard, _) = value_beside(&table, &files, "main.gcsl");
    assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
    // Standard modules importing each other are the usual error.
    let table = [
        ("a", "import b\nx = b.y"),
        ("b", "import a\ny = a.x"),
        prelude_entry(),
    ];
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import a\nv = a.x")];
    let (_, checked) = checked_beside(&table, &files);
    let messages: Vec<String> = checked
        .unwrap_err()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(
        messages,
        ["`<stdlib>/a.gcsl` and `<stdlib>/b.gcsl` need each other"]
    );
}

#[test]
fn standard_modules_are_scopes_of_their_own() {
    // Two may declare one top-level name: the prelude's string `join`
    // and `path`'s, and two of a test's table.
    let text = "import path\nv = [join([\"a\", \"b\"], \"-\"), path.join(\"a\", \"b\")]";
    assert_eq!(lone(text).unwrap(), "{v = [\"a-b\", \"a/b\"]}");
    let table = [("a", "x = 1"), ("b", "x = 2"), prelude_entry()];
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import a\nimport b\nv = a.x * 10 + b.x"),
    ];
    assert_eq!(value_beside(&table, &files, "main.gcsl").1, "{v = 12}");
    // One sees nothing of another it does not import, though both are
    // loaded; it sees the prelude's names.
    let table = [("a", "x = 1"), ("b", "y = x"), prelude_entry()];
    let files = [
        ("workspace.gcsl", ""),
        ("main.gcsl", "import a\nimport b\nv = b.y"),
    ];
    let (standard, checked) = checked_beside(&table, &files);
    assert_eq!(standard.len(), 3);
    let errors = checked.unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message, "undefined variable `x`");
    assert_eq!(errors[0].input, Some(1));
    let table = [("a", "x = 1"), ("b", "y = half(2)"), prelude_entry()];
    let files = [("workspace.gcsl", ""), ("main.gcsl", "import b\nv = b.y")];
    assert_eq!(value_beside(&table, &files, "main.gcsl").1, "{v = 1.0}");
    // Nor anything of the workspace.
    let table = [("a", "y = shared"), prelude_entry()];
    let files = [
        ("workspace.gcsl", "shared = 1"),
        ("main.gcsl", "import a\nv = a.y"),
    ];
    let (_, checked) = checked_beside(&table, &files);
    assert_eq!(
        checked.unwrap_err()[0].message,
        "undefined variable `shared`"
    );
}

#[test]
fn path_is_a_reserved_name() {
    for files in [
        &[("workspace.gcsl", ""), ("path.gcsl", "x = 1")][..],
        &[("path/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
    ] {
        // Reserved whether or not a file imports it.
        assert_eq!(
            error(files),
            (
                "`path` is the name of a standard module".to_owned(),
                Some(1),
                vec![]
            ),
            "{files:?}"
        );
    }
    // Below the root, a name of a path of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/path.gcsl", "x = 1"),
        ("main.gcsl", "import lib/path\nn = path.x"),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{n = 1}");
}

#[test]
fn a_file_s_own_join_shadows_the_prelude_s() {
    for (text, expected) in [
        (
            "join = \"mine\"\nv = join",
            "{join = \"mine\"; v = \"mine\"}",
        ),
        (
            "def join(a: Int, b: Int): Int { a + b }\nv = join(1, 2)",
            "{join = fn(a: Int, b: Int): Int; v = 3}",
        ),
        (
            "import path\njoin = 1\nv = path.join(\"a\", \"b\")",
            "{join = 1; v = \"a/b\"}",
        ),
        ("let trim = 2\nv = trim", "{v = 2}"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
}

#[test]
fn sh_s_message_has_its_standard_error_trimmed() {
    let error = lone_error("x = sh(\"printf ' a\\\\n\\\\n  ' >&2; exit 1\")");
    assert_eq!(
        error.message,
        "`printf ' a\\n\\n  ' >&2; exit 1` exited with status 1: a"
    );
    // Its result is still the raw standard output.
    assert_eq!(lone("= sh(\"printf ' a\\\\n'\")").unwrap(), " a\n");
}

// ----- More string built-ins, and `compare_versions` -----

/// The summary of slice 41.
const MORE_STRINGS_SUMMARY: &str = "a = strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")  # \"6.6\"\n\
         b = strip_prefix(\"x\", \"y\")                                     # \"x\"\n\
         c = strip_suffix(\"a.tar.gz\", \".gz\")                            # \"a.tar\"\n\
         d = lines(\"a\\nb\\n\")                                            # [\"a\", \"b\"]\n\
         e = fields(\"  tcp\\t6  TCP \")                                   # [\"tcp\", \"6\", \"TCP\"]\n\
         f = compare_versions(\"6.10\", \"6.6\")                            # 1";

/// `text` run as a lone file and as a file of a workspace, each giving
/// `expected`.
fn runs_to(text: &str, expected: &str) {
    assert_eq!(lone(text).unwrap(), expected, "{text}");
    let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
    assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
}

#[test]
fn the_more_strings_summary_runs_in_a_workspace_and_alone() {
    let expected = "{a = \"6.6\"; b = \"x\"; c = \"a.tar\"; d = [\"a\", \"b\"]; \
                        e = [\"tcp\", \"6\", \"TCP\"]; f = 1}";
    runs_to(MORE_STRINGS_SUMMARY, expected);
    // Nothing but the prelude is loaded for them.
    let files = [("workspace.gcsl", ""), ("main.gcsl", MORE_STRINGS_SUMMARY)];
    let scan = ws(&files).unwrap();
    assert_eq!(scan.standard(), 1);
    assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
}

#[test]
fn strip_prefix_and_strip_suffix_remove_what_is_there_once() {
    for (text, expected) in [
        (
            "= strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")",
            "6.6",
        ),
        ("= strip_prefix(\"x\", \"y\")", "x"),
        ("= strip_prefix(\"aab\", \"a\")", "ab"),
        ("= strip_prefix(\"abc\", \"abc\")", ""),
        ("= strip_prefix(\"ab\", \"abc\")", "ab"),
        ("= strip_prefix(\"abc\", \"\")", "abc"),
        ("= strip_prefix(\"\", \"\")", ""),
        ("= strip_prefix(\"\", \"a\")", ""),
        ("= strip_prefix(\"ba\", \"a\")", "ba"),
        ("= strip_suffix(\"a.tar.gz\", \".gz\")", "a.tar"),
        ("= strip_suffix(\"a.gz.gz\", \".gz\")", "a.gz"),
        ("= strip_suffix(\"a.tar\", \".gz\")", "a.tar"),
        ("= strip_suffix(\"abc\", \"\")", "abc"),
        ("= strip_suffix(\"\", \"\")", ""),
        ("= strip_suffix(\"gz\", \".gz\")", "gz"),
        ("= strip_suffix(\".gz\", \".gz\")", ""),
        // Byte-wise: a precomposed `é` is not an `e` with a combining
        // accent, and a prefix of `e` leaves the accent, a character of
        // its own, while it is not the start of a precomposed `é`.
        ("= strip_prefix(\"\u{e9}t\u{e9}\", \"\u{e9}\")", "t\u{e9}"),
        (
            "= strip_prefix(\"e\\u{301}t\u{e9}\", \"\u{e9}\")",
            "e\u{301}t\u{e9}",
        ),
        ("= strip_prefix(\"e\\u{301}x\", \"e\")", "\u{301}x"),
        ("= strip_prefix(\"\u{e9}t\u{e9}\", \"e\")", "\u{e9}t\u{e9}"),
        ("= strip_suffix(\"caf\u{e9}\", \"\u{e9}\")", "caf"),
        (
            "= strip_suffix(\"cafe\\u{301}\", \"\u{e9}\")",
            "cafe\u{301}",
        ),
        (
            "= strip_prefix(\"\u{65e5}\u{672c}\", \"\u{65e5}\")",
            "\u{672c}",
        ),
        // By name, and through a variable.
        ("= strip_prefix(prefix = \"v\", s = \"v1.2\")", "1.2"),
        ("let f = strip_suffix\n= f(\"x.c\", \".c\")", "x"),
    ] {
        runs_to(text, expected);
    }
}

#[test]
fn lines_and_fields_split_strings() {
    for (text, expected) in [
        ("= lines(\"a\\nb\\n\")", "[\"a\", \"b\"]"),
        ("= lines(\"\")", "[]"),
        ("= lines(\"\\n\")", "[\"\"]"),
        ("= lines(\"\\n\\n\")", "[\"\", \"\"]"),
        ("= lines(\"a\")", "[\"a\"]"),
        ("= lines(\"a\\n\\nb\")", "[\"a\", \"\", \"b\"]"),
        ("= lines(\"a\\r\\nb\\r\\n\")", "[\"a\", \"b\"]"),
        ("= lines(\"a\\r\\n\\r\\nb\")", "[\"a\", \"\", \"b\"]"),
        // A `\r` alone ends no line, and stays, as Rust's `lines` has
        // it.
        ("= lines(\"a\\rb\")", "[\"a\\rb\"]"),
        ("= lines(\"a\\r\")", "[\"a\\r\"]"),
        ("= lines(\" a \\n\\tb\")", "[\" a \", \"\\tb\"]"),
        (
            "= lines(\"h\u{e9}\\n\u{65e5}\")",
            "[\"h\u{e9}\", \"\u{65e5}\"]",
        ),
        ("= fields(\"  tcp\\t6  TCP \")", "[\"tcp\", \"6\", \"TCP\"]"),
        ("= fields(\"\")", "[]"),
        ("= fields(\"  \")", "[]"),
        ("= fields(\" \\n\\t\\r \")", "[]"),
        ("= fields(\"a\")", "[\"a\"]"),
        (
            "= fields(\"a b\\nc\\r\\nd\")",
            "[\"a\", \"b\", \"c\", \"d\"]",
        ),
        // Unicode's whitespace, as `trim`'s: a no-break space and an
        // ideographic space separate fields too.
        ("= fields(\"a\\u{a0}b\")", "[\"a\", \"b\"]"),
        (
            "= fields(\"\\u{3000}x\\u{3000}\\u{3000}y\")",
            "[\"x\", \"y\"]",
        ),
        (
            "= fields(\"h\u{e9} \u{65e5}\")",
            "[\"h\u{e9}\", \"\u{65e5}\"]",
        ),
        // Together, as a table is read.
        (
            "= for line in lines(\"tcp 6 TCP\\nudp\\t17 UDP\\n\") { fields(line)[1] }",
            "[\"6\", \"17\"]",
        ),
        ("let f = fields\n= f(s = \"x y\")", "[\"x\", \"y\"]"),
    ] {
        runs_to(text, expected);
    }
}

#[test]
fn compare_versions_orders_versions() {
    // Each pair in order, `a` before `b`: the other way round is `1`.
    let long = "1234567890123456789012345678901234567890";
    let longer = "12345678901234567890123456789012345678901";
    for (a, b) in [
        ("6.6", "6.10"),
        ("6.6", "6.6.1"),
        ("6.3", "6.3+nmu1"),
        ("6.3+nmu1", "6.4"),
        ("1.0~rc1", "1.0"),
        ("1.0~rc1", "1.0~rc2"),
        ("1.0~~", "1.0~"),
        ("1.0", "1.0a"),
        ("1.0", "1.0.0"),
        ("2", "10"),
        ("", "0"),
        ("", "a"),
        ("~", ""),
        ("1.01", "1.1"),
        ("01", "1"),
        ("a", "b"),
        ("Z", "a"),
        ("a", "."),
        ("a", "+"),
        ("1a", "1."),
        ("1.2.3", "1.2.10"),
        ("1:1.0", "1:1.1"),
        ("2.30-1", "2.30-2"),
        (long, longer),
        (
            "1.1234567890123456789012345678901234567890",
            "1.1234567890123456789012345678901234567891",
        ),
        // Bytes, not characters: a letter before any other byte, a
        // multi-byte character's bytes among them.
        ("1.a", "1.\u{e9}"),
        ("\u{e9}", "\u{f6}"),
    ] {
        let before = format!("= compare_versions(\"{a}\", \"{b}\")");
        runs_to(&before, "-1");
        let after = format!("= compare_versions(\"{b}\", \"{a}\")");
        runs_to(&after, "1");
    }
    // Equal strings, and only those, are `0`.
    for a in ["", "0", "6.6", "1.0~rc1", long, "\u{e9}"] {
        runs_to(&format!("= compare_versions(\"{a}\", \"{a}\")"), "0");
    }
    // By name, through `builtins`, and in a pure function.
    runs_to("= compare_versions(b = \"6.6\", a = \"6.10\")", "1");
    runs_to(
        "import builtins\n= builtins.compare_versions(\"1\", \"2\")",
        "-1",
    );
    runs_to(
        "fn newer(a: String, b: String): Bool { compare_versions(a, b) > 0 }\n\
             = [newer(\"6.10\", \"6.6\"), newer(\"1.0~rc1\", \"1.0\")]",
        "[true, false]",
    );
}

#[test]
fn the_more_string_builtins_are_values_of_their_types() {
    for (text, expected) in [
        ("= strip_prefix", "fn(s: String, prefix: String): String"),
        ("= strip_suffix", "fn(s: String, suffix: String): String"),
        ("= lines", "fn(s: String): List<String>"),
        ("= fields", "fn(s: String): List<String>"),
        ("= compare_versions", "fn(a: String, b: String): Int"),
        // Through `builtins`.
        (
            "import builtins\n= builtins.fields(\" a  b \")",
            "[\"a\", \"b\"]",
        ),
        (
            "import builtins\n= builtins.lines",
            "fn(s: String): List<String>",
        ),
        ("import lines from builtins\n= lines(\"x\\n\")", "[\"x\"]"),
    ] {
        runs_to(text, expected);
    }
    // A file's own declaration of one of their names shadows the
    // prelude's silently, and loses the built-in there.
    for (text, expected) in [
        (
            "fields = \"mine\"\nv = fields",
            "{fields = \"mine\"; v = \"mine\"}",
        ),
        ("let lines = 2\nv = lines", "{v = 2}"),
        (
            "def compare_versions(a: Int): Int { a }\nv = compare_versions(3)",
            "{compare_versions = fn(a: Int): Int; v = 3}",
        ),
        (
            "fields.x = 1\nv = trim(\" a \")",
            "{fields = {x = 1}; v = \"a\"}",
        ),
    ] {
        runs_to(text, expected);
    }
}

// ----- The `fs` module -----

/// A new empty directory for the `fs` test `name` to work in, in the
/// system's temporary directory, never the repository's: its absolute
/// path, canonical.
fn fs_scratch(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("gcsl-slice-42-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// The summary of slice 42, which copies `x.txt` from its own directory.
const FS_SUMMARY: &str = "import fs\n\
                              import fileroot\n\
                              import path\n\
                              let dir = fs.temp_dir()\n\
                              fs.mkdir(path.join(dir, \"a/b\"))\n\
                              fs.copy(path.join(fileroot.this, \"x.txt\"), path.join(dir, \"a/b/x.txt\"))\n\
                              there = fs.exists(path.join(dir, \"a/b/x.txt\"))\n\
                              text = fs.read(path.join(dir, \"a/b/x.txt\"))";

/// Runs `text` as the lone file `lone.gcsl` of the directory `dir`,
/// returning its value shown, or its diagnostics, which name the file by
/// no input, as [`crate::run`]'s do; the workspace it ran in, and its
/// temporary directories, dropped before this returns.
fn lone_in(dir: &str, text: &str) -> Result<String, Vec<Diagnostic>> {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, dir);
    let index = count(loaded.len());
    let being_run = |diagnostics| being_run(diagnostics, index);
    let mut workspace = checked.map_err(being_run)?;
    workspace
        .run(index)
        .map(|outcome| outcome.value.to_string())
        .map_err(being_run)
}

/// The one diagnostic of running `text` as a lone file of `dir`.
fn lone_error_in(dir: &str, text: &str) -> Diagnostic {
    let mut errors = lone_in(dir, text).unwrap_err();
    assert_eq!(errors.len(), 1, "{text}: {errors:?}");
    errors.remove(0)
}

#[test]
fn the_fs_summary_runs_in_a_workspace_and_alone() {
    let dir = fs_scratch("summary");
    std::fs::write(format!("{dir}/x.txt"), "hello, fs\n").unwrap();
    let expected = "{there = true; text = \"hello, fs\\n\"}";
    assert_eq!(lone_in(&dir, FS_SUMMARY).unwrap(), expected);
    // In a workspace, from a file of a directory of its own.
    std::fs::create_dir(format!("{dir}/app")).unwrap();
    std::fs::write(format!("{dir}/app/x.txt"), "hello, fs\n").unwrap();
    let sources = [
        SourceFile::new("workspace.gcsl", "").unwrap(),
        SourceFile::new("app/main.gcsl", FS_SUMMARY).unwrap(),
    ];
    let (standard, checked) = check_with_standard(&sources, &[], &dir);
    // `fs` and `path` are loaded, before the prelude, in path order.
    let names: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    assert_eq!(
        names,
        [
            "<stdlib>/fs.gcsl",
            "<stdlib>/path.gcsl",
            "<stdlib>/prelude.gcsl"
        ]
    );
    let mut workspace = checked.unwrap();
    let main = workspace.file("app/main.gcsl").unwrap();
    assert_eq!(main, 3);
    assert_eq!(workspace.run(main).unwrap().value.to_string(), expected);
    drop(workspace);
    // A relative path is an error at the call, before anything is
    // touched.
    let text = "import fs\ntext = fs.read(\"x.txt\")";
    let error = lone_error_in(&dir, text);
    assert_eq!(
        error.message,
        "`fs.read` needs an absolute path, found `x.txt`"
    );
    assert_eq!(error.span, Span::new(17, 33));
    assert_eq!(error.input, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn each_member_of_fs_does_what_it_says() {
    let dir = fs_scratch("members");
    std::fs::write(format!("{dir}/x.txt"), "h\u{e9}llo\n").unwrap();
    std::os::unix::fs::symlink(format!("{dir}/nothing"), format!("{dir}/dangling")).unwrap();
    std::os::unix::fs::symlink(format!("{dir}/x.txt"), format!("{dir}/link")).unwrap();
    for (text, expected) in [
        // `exists`: a file, a directory, nothing, links followed.
        (format!("= fs.exists(\"{dir}/x.txt\")"), "true"),
        (format!("= fs.exists(\"{dir}\")"), "true"),
        (format!("= fs.exists(\"{dir}/missing\")"), "false"),
        (format!("= fs.exists(\"{dir}/link\")"), "true"),
        (format!("= fs.exists(\"{dir}/dangling\")"), "false"),
        ("= fs.exists(\"/\")".to_owned(), "true"),
        // `read`: the text, UTF-8, through a link too.
        (format!("= fs.read(\"{dir}/x.txt\")"), "h\u{e9}llo\n"),
        (format!("= fs.read(p = \"{dir}/link\")"), "h\u{e9}llo\n"),
        // `mkdir`: nested, `{}`, and again, fine.
        (
            format!(
                "fs.mkdir(\"{dir}/m/n/o\")\nfs.mkdir(\"{dir}/m/n/o\")\nfs.mkdir(\"{dir}/m\")\n\
                     = fs.exists(\"{dir}/m/n/o\")"
            ),
            "true",
        ),
        (format!("= fs.mkdir(\"{dir}/m2\")"), "{}"),
        // `copy`: a new file, then over it; `{}`.
        (
            format!("fs.copy(\"{dir}/x.txt\", \"{dir}/y.txt\")\n= fs.read(\"{dir}/y.txt\")"),
            "h\u{e9}llo\n",
        ),
        (
            format!("= fs.copy(to = \"{dir}/y.txt\", from = \"{dir}/link\")"),
            "{}",
        ),
    ] {
        assert_eq!(
            lone_in(&dir, &format!("import fs\n{text}")).unwrap(),
            expected,
            "{text}"
        );
    }
    // `copy` replaces a file's contents, and copies its permissions.
    std::fs::write(format!("{dir}/z.txt"), "old").unwrap();
    std::fs::write(format!("{dir}/w.txt"), "new").unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o640);
    std::fs::set_permissions(format!("{dir}/w.txt"), mode).unwrap();
    let text = format!("import fs\n= fs.copy(\"{dir}/w.txt\", \"{dir}/z.txt\")");
    assert_eq!(lone_in(&dir, &text).unwrap(), "{}");
    assert_eq!(
        std::fs::read_to_string(format!("{dir}/z.txt")).unwrap(),
        "new"
    );
    let copied = std::fs::metadata(format!("{dir}/z.txt")).unwrap();
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&copied.permissions()) & 0o777,
        0o640
    );
    // Every form of import, and `builtins` itself.
    for text in [
        format!("import exists from fs\n= exists(\"{dir}\")"),
        format!("import exists as here from fs\n= here(\"{dir}\")"),
        format!("import fs as f\n= f.exists(\"{dir}\")"),
        format!("import * from fs\n= exists(\"{dir}\")"),
        format!("import builtins\n= builtins.fs_exists(\"{dir}\")"),
        format!("import fs\nlet e = fs.exists\n= e(\"{dir}\")"),
        format!("import fs\nfn f(p: String) -> Bool {{ fs.exists(p) }}\n= f(\"{dir}\")"),
        format!("import fs\n= await lazy fs.exists(\"{dir}\")"),
    ] {
        assert_eq!(lone_in(&dir, &text).unwrap(), "true", "{text}");
    }
    // The paths are used as given: not normalised, and so `..` is the
    // file system's own.
    let text = format!("import fs\n= fs.read(\"{dir}/m/../x.txt\")");
    assert_eq!(lone_in(&dir, &text).unwrap(), "h\u{e9}llo\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_path_given_to_fs_must_be_absolute() {
    let dir = fs_scratch("relative");
    // Named whatever it is imported as, at the call, the paths of each
    // member checked before anything is touched.
    let made = "gcsl-slice-42-never-made";
    for (text, message) in [
        (
            "import fs\n= fs.exists(\"a\")",
            "`fs.exists` needs an absolute path, found `a`",
        ),
        (
            "import fs\n= fs.read(\"x.txt\")",
            "`fs.read` needs an absolute path, found `x.txt`",
        ),
        (
            "import fs\n= fs.read(\"\")",
            "`fs.read` needs an absolute path, found ``",
        ),
        (
            "import fs\n= fs.mkdir(\"gcsl-slice-42-never-made/a\")",
            "`fs.mkdir` needs an absolute path, found `gcsl-slice-42-never-made/a`",
        ),
        (
            "import fs\n= fs.copy(\"./a\", \"/nowhere/b\")",
            "`fs.copy` needs an absolute path, found `./a`",
        ),
        (
            "import fs\n= fs.copy(\"/nowhere/a\", \"b\")",
            "`fs.copy` needs an absolute path, found `b`",
        ),
        (
            "import fs\n= fs.copy(\"a\", \"b\")",
            "`fs.copy` needs an absolute path, found `a`",
        ),
        (
            "import exists from fs\n= exists(\"x\")",
            "`fs.exists` needs an absolute path, found `x`",
        ),
        (
            "import fs as f\n= f.exists(\"x\")",
            "`fs.exists` needs an absolute path, found `x`",
        ),
        (
            "import read as slurp from fs\n= slurp(\"x\")",
            "`fs.read` needs an absolute path, found `x`",
        ),
        (
            "import builtins\n= builtins.fs_mkdir(\"x\")",
            "`fs.mkdir` needs an absolute path, found `x`",
        ),
    ] {
        let error = lone_error_in(&dir, text);
        assert_eq!(error.message, message, "{text}");
        // At the user's call, the whole of it, in the user's file.
        let call = &text[text.find("= ").unwrap() + 2..];
        assert_eq!(error.span.start as usize, text.len() - call.len(), "{text}");
        assert_eq!(error.span.end as usize, text.len(), "{text}");
        assert_eq!(error.input, None, "{text}");
        assert!(error.labels.is_empty() && error.notes.is_empty(), "{text}");
    }
    assert!(!std::path::Path::new(made).exists());
    // In a workspace, the error names the file that made the call, not
    // `fs.gcsl`.
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib/util.gcsl",
            "import fs\ndef load(p: String) -> String { fs.read(p) }",
        ),
        ("main.gcsl", "import lib/util\nv = util.load(\"x\")"),
    ];
    let error = run_file(&files, "main.gcsl").unwrap_err();
    let util = checked(&files).file("lib/util.gcsl");
    assert_eq!(
        error,
        [(
            "`fs.read` needs an absolute path, found `x`".to_owned(),
            util
        )]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failure_of_fs_names_the_member_the_paths_and_the_reason() {
    let dir = fs_scratch("failures");
    std::fs::write(format!("{dir}/x.txt"), "x").unwrap();
    std::fs::write(format!("{dir}/bad.bin"), [b'a', 0xff, 0xfe]).unwrap();
    let copy_dir = std::fs::copy(&dir, format!("{dir}/never"))
        .unwrap_err()
        .to_string();
    for (text, message) in [
        (
            format!("= fs.read(\"{dir}/missing\")"),
            format!(
                "`fs.read` could not read `{dir}/missing`: No such file or directory (os error 2)"
            ),
        ),
        (
            format!("= fs.read(\"{dir}\")"),
            format!("`fs.read` could not read `{dir}`: Is a directory (os error 21)"),
        ),
        (
            format!("= fs.read(\"{dir}/bad.bin\")"),
            format!("`fs.read`: `{dir}/bad.bin` is not UTF-8 text"),
        ),
        (
            format!("= fs.mkdir(\"{dir}/x.txt/sub\")"),
            format!("`fs.mkdir` could not create `{dir}/x.txt/sub`: Not a directory (os error 20)"),
        ),
        (
            format!("= fs.mkdir(\"{dir}/x.txt\")"),
            format!("`fs.mkdir` could not create `{dir}/x.txt`: File exists (os error 17)"),
        ),
        (
            format!("= fs.copy(\"{dir}\", \"{dir}/c\")"),
            format!("`fs.copy` could not copy `{dir}` to `{dir}/c`: {copy_dir}"),
        ),
        (
            format!("= fs.copy(\"{dir}/missing\", \"{dir}/c\")"),
            format!(
                "`fs.copy` could not copy `{dir}/missing` to `{dir}/c`: No such file or \
                     directory (os error 2)"
            ),
        ),
        (
            format!("= fs.copy(\"{dir}/x.txt\", \"{dir}/none/x.txt\")"),
            format!(
                "`fs.copy` could not copy `{dir}/x.txt` to `{dir}/none/x.txt`: No such file \
                     or directory (os error 2)"
            ),
        ),
    ] {
        let text = format!("import fs\n{text}");
        let error = lone_error_in(&dir, &text);
        assert_eq!(error.message, message, "{text}");
        assert_eq!(error.span.end as usize, text.len(), "{text}");
        assert_eq!(error.span.start, 12, "{text}");
        assert_eq!(error.input, None, "{text}");
    }
    assert!(!std::path::Path::new(&format!("{dir}/c")).exists());
    assert!(!std::path::Path::new(&format!("{dir}/never")).exists());
    // What cannot be checked: below a directory that cannot be searched,
    // unless the process may search anything.
    let locked = format!("{dir}/locked");
    std::fs::create_dir(&locked).unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o000);
    std::fs::set_permissions(&locked, mode).unwrap();
    let text = format!("import fs\n= fs.exists(\"{locked}/x\")");
    let result = lone_in(&dir, &text).map_err(|errors| errors[0].message.clone());
    let expected = match std::path::Path::new(&format!("{locked}/x")).try_exists() {
        Ok(exists) => Ok(exists.to_string()),
        Err(error) => Err(format!("`fs.exists` could not check `{locked}/x`: {error}")),
    };
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o700);
    std::fs::set_permissions(&locked, mode).unwrap();
    assert_eq!(result, expected);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The temporary directories a run gave as its value, a list of their
/// paths, once checked to be there, while the run's owner lives: each
/// absolute, a directory, empty, of mode `0700`, and another than those
/// before it, all children of one parent of this process's in the
/// system's temporary directory.
fn assert_fresh_temp_dirs(value: &Value) -> Vec<String> {
    let Value::List(list) = value else {
        panic!("not a list: {value}");
    };
    let dirs: Vec<String> = list
        .values()
        .iter()
        .map(|dir| match dir {
            Value::Str(dir) => dir.to_string(),
            other => panic!("not a string: {other}"),
        })
        .collect();
    let prefix = format!("gcsl-{}-", std::process::id());
    let parent = std::path::Path::new(&dirs[0]).parent().unwrap().to_owned();
    for (at, dir) in dirs.iter().enumerate() {
        let path = std::path::Path::new(dir);
        assert!(path.is_absolute(), "{dir}");
        assert!(path.is_dir(), "{dir}");
        assert_eq!(std::fs::read_dir(path).unwrap().count(), 0, "{dir}");
        let mode = std::fs::metadata(path).unwrap().permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o700
        );
        assert_eq!(path.parent().unwrap(), parent, "{dir}");
        assert!(!dirs[..at].contains(dir), "{dir} twice");
    }
    let name = parent.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with(&prefix), "{name}");
    assert_eq!(parent.parent().unwrap(), std::env::temp_dir());
    dirs
}

#[test]
fn temporary_directories_live_as_long_as_the_workspace() {
    let text = "import fs\n\
                    fn fresh() -> String { fs.temp_dir() }\n\
                    let a = fs.temp_dir()\n\
                    let b = fresh()\n\
                    = [a, b, fresh(), await lazy fs.temp_dir()]";
    // A workspace's: made by its runs, there until it is dropped.
    let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check(&sources).unwrap();
    let main = workspace.file("main.gcsl").unwrap();
    let dirs = assert_fresh_temp_dirs(&workspace.run(main).unwrap().value);
    assert_eq!(dirs.len(), 4);
    let parent = std::path::Path::new(&dirs[0]).parent().unwrap().to_owned();
    assert_eq!(std::path::Path::new(&dirs[0]).file_name().unwrap(), "1");
    // Removed with what was put in them.
    std::fs::write(format!("{}/kept", dirs[1]), "x").unwrap();
    drop(workspace);
    assert!(!parent.exists(), "{}", parent.display());
    // A lone file's: gone as its run is.
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap();
    let outcome = workspace.run(count(loaded.len())).unwrap();
    let dirs = assert_fresh_temp_dirs(&outcome.value);
    let parent = std::path::Path::new(&dirs[0]).parent().unwrap().to_owned();
    drop(workspace);
    assert!(!parent.exists(), "{}", parent.display());
    let Value::List(dirs) = crate::run(&source).unwrap().value else {
        panic!("not a list");
    };
    for dir in dirs.values() {
        assert!(!std::path::Path::new(&dir.to_string()).exists(), "{dir}");
    }
}

#[test]
fn temporary_directories_are_removed_after_a_run_that_failed() {
    // With what the run put in them.
    let text = "import fs\n\
                    let d = fs.temp_dir()\n\
                    fs.mkdir(\"{d}/a/b\")\n\
                    = panic(d)";
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap();
    let error = workspace.run(count(loaded.len())).unwrap_err().remove(0);
    let dir = std::path::Path::new(&error.message).to_owned();
    assert!(dir.join("a/b").is_dir());
    drop(workspace);
    assert!(!dir.exists());
    assert!(!dir.parent().unwrap().exists());
    let error = crate::run(&source).unwrap_err().remove(0);
    assert!(!std::path::Path::new(&error.message).exists());
    // A run that fails before making one: nothing to remove.
    let error = lone_error("import fs\n= fs.read(\"x\")");
    assert_eq!(error.message, "`fs.read` needs an absolute path, found `x`");
}

#[test]
fn each_call_of_temp_dir_is_its_own() {
    // A memoised function makes one for each list of arguments; an
    // impure one, one each call.
    let text = "import fs\n\
                    fn once(n: Int) ->> String { fs.temp_dir() }\n\
                    fn each(n: Int) -> String { fs.temp_dir() }\n\
                    = [once(1), once(1), once(2), each(1), each(1)]";
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap();
    let outcome = workspace.run(count(loaded.len())).unwrap();
    let Value::List(dirs) = outcome.value else {
        panic!("not a list");
    };
    let dirs: Vec<String> = dirs.values().iter().map(ToString::to_string).collect();
    assert_eq!(dirs[0], dirs[1]);
    let mut distinct = vec![&dirs[0], &dirs[2], &dirs[3], &dirs[4]];
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "{dirs:?}");
}

#[test]
fn temp_dir_in_a_run_without_an_owner_is_an_internal_error() {
    // The run of the file's IR alone, given no temporary directories,
    // once the standard files have run in the workspace.
    let source = SourceFile::new("lone.gcsl", "import fs\n= fs.temp_dir()").unwrap();
    let (loaded, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap();
    for file in 0..count(loaded.len()) {
        workspace.run(file).unwrap();
    }
    let FileRun::Pending(ir) = mem::replace(&mut workspace.runs[loaded.len()], FileRun::Succeeded)
    else {
        panic!("expected pending IR for the lone file");
    };
    let ir = Rc::new(*ir);
    let size = ir.frame_size() as usize;
    workspace
        .frame
        .resize(size.max(workspace.frame.len()), Value::Empty);
    let context = (
        Some(&workspace.exec_result),
        &workspace.scan.dir_texts[..],
        None,
    );
    let error = crate::eval::eval_with(&ir, &mut workspace.frame, &mut workspace.memo, context)
        .unwrap_err();
    assert!(
        error.message.starts_with("internal error"),
        "{}",
        error.message
    );
}

/// The workspace of a `workspace.gcsl` and two files, `one.gcsl` and
/// `two.gcsl`, each of whose value is a list of two new temporary
/// directories, checked.
fn temp_dir_workspace() -> Workspace {
    let text = "import fs\n= [fs.temp_dir(), fs.temp_dir()]";
    let files = [
        ("workspace.gcsl", ""),
        ("one.gcsl", text),
        ("two.gcsl", text),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    check(&sources).unwrap()
}

/// The paths of the temporary directories a run of `file` of
/// `workspace` gave, each checked to be a directory.
fn temp_dirs_of(workspace: &mut Workspace, file: &str) -> Vec<std::path::PathBuf> {
    let file = workspace.file(file).unwrap();
    let Value::List(list) = workspace.run(file).unwrap().value else {
        panic!("not a list");
    };
    list.values()
        .iter()
        .map(|dir| {
            let dir = std::path::PathBuf::from(dir.to_string());
            assert!(dir.is_dir(), "{}", dir.display());
            dir
        })
        .collect()
}

#[test]
fn temporary_directories_are_made_in_the_temp_root_set() {
    let scratch = fs_scratch("temp-root");
    // A missing root, nested, is made as the first is asked for; the
    // parent is in it, removed as the workspace is dropped; the root
    // stays.
    let root = std::path::Path::new(&scratch).join("a/b/root");
    let mut workspace = temp_dir_workspace();
    workspace.set_temp_root(root.clone());
    assert!(!std::path::Path::new(&scratch).join("a").exists());
    let dirs = temp_dirs_of(&mut workspace, "one.gcsl");
    let parent = dirs[0].parent().unwrap().to_owned();
    assert_eq!(parent.parent(), Some(root.as_path()));
    assert_eq!(dirs, [parent.join("1"), parent.join("2")]);
    let name = parent.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with(&format!("gcsl-{}-", std::process::id())),
        "{name}"
    );
    std::fs::write(dirs[1].join("kept"), "x").unwrap();
    drop(workspace);
    assert!(!parent.exists(), "{}", parent.display());
    assert!(root.is_dir());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    // An existing root is used as it is.
    let mut workspace = temp_dir_workspace();
    workspace.set_temp_root(root.clone());
    let dirs = temp_dirs_of(&mut workspace, "two.gcsl");
    let parent = dirs[0].parent().unwrap().to_owned();
    assert_eq!(parent.parent(), Some(root.as_path()));
    drop(workspace);
    assert!(!parent.exists(), "{}", parent.display());
    assert!(root.is_dir());
    // Set after a run made one: the parent made stays in use, in the
    // system's temporary directory.
    let mut workspace = temp_dir_workspace();
    let first = temp_dirs_of(&mut workspace, "one.gcsl");
    let parent = first[0].parent().unwrap().to_owned();
    assert_eq!(parent.parent(), Some(std::env::temp_dir().as_path()));
    workspace.set_temp_root(root.clone());
    let second = temp_dirs_of(&mut workspace, "two.gcsl");
    assert_eq!(second, [parent.join("3"), parent.join("4")]);
    drop(workspace);
    assert!(!parent.exists(), "{}", parent.display());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn a_temp_root_that_is_not_absolute_is_an_error() {
    let mut workspace = temp_dir_workspace();
    workspace.set_temp_root(std::path::PathBuf::from("rel/tmp"));
    let file = workspace.file("one.gcsl").unwrap();
    let mut errors = workspace.run(file).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let error = errors.remove(0);
    assert_eq!(
        error.message,
        "`fs.temp_dir` could not create a temporary directory: the temporary directory \
             root `rel/tmp` is not an absolute path"
    );
    assert_eq!(error.input, Some(file));
    assert!(!std::path::Path::new("rel").exists());
}

#[test]
fn fs_is_loaded_only_when_imported() {
    // Not imported: not loaded, and input numbers and slots are those of
    // a run beside the prelude alone.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/a.gcsl", "x = [1]\ny = len(x)"),
        ("main.gcsl", "import lib/a\nz = a.y + 1"),
    ];
    let (standard, checked) = checked_beside(&crate::STANDARD, &files);
    assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
    let mut workspace = checked.unwrap();
    let (_, bare) = checked_beside(&[prelude_entry()], &files);
    let mut bare = bare.unwrap();
    assert_eq!(workspace.scan.cells(), bare.scan.cells());
    assert_eq!(workspace.slot_names, bare.slot_names);
    let main = workspace.file("main.gcsl").unwrap();
    assert_eq!(main, 2);
    assert_eq!(workspace.dump_ir(main), bare.dump_ir(main));
    assert_eq!(
        workspace.run(main).unwrap().value,
        bare.run(main).unwrap().value
    );
    // Imported by any form, from any file scanned: loaded first.
    for text in [
        "import fs",
        "import fs as f",
        "import exists from fs",
        "import read as r from fs",
        "import * from fs",
    ] {
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "v = 1"),
            ("other.gcsl", text),
        ];
        let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
        assert_eq!(standard, ["<stdlib>/fs.gcsl", "<stdlib>/prelude.gcsl"]);
        assert_eq!(value, "{v = 1}");
    }
    // A lone file too.
    let source = SourceFile::new("lone.gcsl", "import fs").unwrap();
    let (standard, checked) = check_lone_with_standard(&source, "/");
    assert_eq!(standard.len(), 2);
    assert_eq!(checked.unwrap().file("lone.gcsl"), Some(2));
}

#[test]
fn fs_is_a_reserved_name() {
    for files in [
        &[("workspace.gcsl", ""), ("fs.gcsl", "x = 1")][..],
        &[("fs/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
    ] {
        // Reserved whether or not a file imports it.
        assert_eq!(
            error(files),
            (
                "`fs` is the name of a standard module".to_owned(),
                Some(1),
                vec![]
            ),
            "{files:?}"
        );
    }
    // Below the root, a name of a path of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/fs.gcsl", "x = 1"),
        ("main.gcsl", "import lib/fs\nn = fs.x"),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{n = 1}");
}

// ----- `fs.write`, `sort_versions` and `show` -----

/// The summary of slice 43, its lock file written to `dir`.
fn locks_summary(dir: &str) -> String {
    format!(
        "import fs\n\
             a = sort_versions([\"6.10\", \"6.6\", \"6.6.1\", \"1.0~rc1\", \"1.0\"])\n\
             b = show({{version = \"6.6\"; commit = \"3a8f\"}})\n\
             c = show(\"say \\\"hi\\\" {{{{x}}}}\\n\")\n\
             d = show([1, 2])\n\
             fs.write(\"{dir}/lock.gcsl\", \"locked = [{{b}}]\\n\")"
    )
}

/// The value of the summary of slice 43, shown.
const LOCKS_SUMMARY_VALUE: &str = r#"{a = ["1.0~rc1", "1.0", "6.6", "6.6.1", "6.10"]; b = "{{version = \"6.6\"; commit = \"3a8f\"}}"; c = "\"say \\\"hi\\\" {{{{x}}}}\\n\""; d = "[1, 2]"}"#;

/// The names of the entries of `dir` and of every directory below it
/// that are temporary files of `fs.write`.
fn temp_files_below(dir: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![std::path::PathBuf::from(dir)];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(".gcsl-tmp-") {
                found.push(name);
            }
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            }
        }
    }
    found
}

/// The permission bits of the file at `path`.
fn mode_of(path: &str) -> u32 {
    let permissions = std::fs::symlink_metadata(path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::mode(&permissions) & 0o7777
}

#[test]
fn the_locks_summary_runs_in_a_workspace_and_alone() {
    let dir = fs_scratch("locks-summary");
    assert_eq!(
        lone_in(&dir, &locks_summary(&dir)).unwrap(),
        LOCKS_SUMMARY_VALUE
    );
    let written = std::fs::read_to_string(format!("{dir}/lock.gcsl")).unwrap();
    assert_eq!(
        written,
        "locked = [{version = \"6.6\"; commit = \"3a8f\"}]\n"
    );
    // The lock it wrote reads back as the record it showed.
    assert_eq!(
        lone(&written).unwrap(),
        "{locked = [{version = \"6.6\"; commit = \"3a8f\"}]}"
    );
    std::fs::remove_file(format!("{dir}/lock.gcsl")).unwrap();
    // In a workspace, the same.
    let sources = [
        SourceFile::new("workspace.gcsl", "").unwrap(),
        SourceFile::new("app/main.gcsl", locks_summary(&dir)).unwrap(),
    ];
    let (_, checked) = check_with_standard(&sources, &[], &dir);
    let mut workspace = checked.unwrap();
    let main = workspace.file("app/main.gcsl").unwrap();
    assert_eq!(
        workspace.run(main).unwrap().value.to_string(),
        LOCKS_SUMMARY_VALUE
    );
    drop(workspace);
    assert_eq!(
        std::fs::read_to_string(format!("{dir}/lock.gcsl")).unwrap(),
        written
    );
    assert!(temp_files_below(&dir).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn fs_write_makes_and_replaces_files_whole() {
    let dir = fs_scratch("write");
    for (text, expected) in [
        // A new file, its contents exactly as given.
        (
            format!(
                "import fs\nfs.write(\"{dir}/new.txt\", \"h\u{e9}llo\\n\")\n\
                     = fs.read(\"{dir}/new.txt\")"
            ),
            "h\u{e9}llo\n",
        ),
        (
            format!("import fs\n= fs.write(\"{dir}/empty.txt\", \"\")"),
            "{}",
        ),
        (format!("import fs\n= fs.read(\"{dir}/empty.txt\")"), ""),
        // Over a file, whole: a shorter text leaves nothing of the
        // longer, and a run reads what it wrote.
        (
            format!(
                "import fs\nfs.write(\"{dir}/new.txt\", \"a much longer text\")\n\
                     fs.write(\"{dir}/new.txt\", \"b\")\n= fs.read(\"{dir}/new.txt\")"
            ),
            "b",
        ),
        // By every road, and by its parameters' names.
        (
            format!("import write from fs\n= write(\"{dir}/w1\", \"1\")"),
            "{}",
        ),
        (
            format!("import write as put from fs\n= put(\"{dir}/w2\", \"2\")"),
            "{}",
        ),
        (
            format!("import fs as f\n= f.write(\"{dir}/w3\", \"3\")"),
            "{}",
        ),
        (
            format!("import builtins\n= builtins.fs_write(\"{dir}/w4\", \"4\")"),
            "{}",
        ),
        (
            format!("import fs\n= fs.write(contents = \"5\", p = \"{dir}/w5\")"),
            "{}",
        ),
        (
            format!(
                "import fs\nfn put(p: String) -> {{}} {{ fs.write(p, \"6\") }}\n= put(\"{dir}/w6\")"
            ),
            "{}",
        ),
        (
            format!("import fs\n= await lazy fs.write(\"{dir}/w7\", \"7\")"),
            "{}",
        ),
    ] {
        assert_eq!(lone_in(&dir, &text).unwrap(), expected, "{text}");
    }
    for number in 1..=7 {
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/w{number}")).unwrap(),
            number.to_string()
        );
    }
    // A file it replaces keeps neither its contents nor its permissions:
    // the new file has the mode any new file gets, never an executable
    // bit, whatever the umask.
    let kept = format!("{dir}/kept.sh");
    std::fs::write(&kept, "old").unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(&kept, mode).unwrap();
    let text = format!("import fs\n= fs.write(\"{kept}\", \"new\")");
    assert_eq!(lone_in(&dir, &text).unwrap(), "{}");
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "new");
    let fresh = format!("{dir}/fresh");
    std::fs::File::create(&fresh).unwrap();
    assert_eq!(mode_of(&kept), mode_of(&fresh));
    assert_eq!(mode_of(&kept) & 0o111, 0);
    // A symbolic link is replaced by the file, not followed.
    let target = format!("{dir}/target.txt");
    std::fs::write(&target, "target").unwrap();
    std::os::unix::fs::symlink(&target, format!("{dir}/link")).unwrap();
    let text = format!("import fs\n= fs.write(\"{dir}/link\", \"over\")");
    assert_eq!(lone_in(&dir, &text).unwrap(), "{}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "target");
    let link = std::fs::symlink_metadata(format!("{dir}/link")).unwrap();
    assert!(link.file_type().is_file());
    assert_eq!(
        std::fs::read_to_string(format!("{dir}/link")).unwrap(),
        "over"
    );
    // No temporary file is left behind.
    assert!(
        temp_files_below(&dir).is_empty(),
        "{:?}",
        temp_files_below(&dir)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failure_of_fs_write_names_the_path_and_leaves_nothing_behind() {
    let dir = fs_scratch("write-failures");
    std::fs::create_dir(format!("{dir}/full")).unwrap();
    std::fs::write(format!("{dir}/full/inner"), "inner").unwrap();
    std::fs::create_dir(format!("{dir}/empty")).unwrap();
    std::fs::write(format!("{dir}/x.txt"), "x").unwrap();
    for (text, message) in [
        // A relative path, before anything is touched, whatever name it
        // is called by.
        (
            "import fs\n= fs.write(\"x.txt\", \"a\")".to_owned(),
            "`fs.write` needs an absolute path, found `x.txt`".to_owned(),
        ),
        (
            "import fs\n= fs.write(\"\", \"a\")".to_owned(),
            "`fs.write` needs an absolute path, found ``".to_owned(),
        ),
        (
            "import write as put from fs\n= put(\"gcsl-slice-43-never/x\", \"a\")".to_owned(),
            "`fs.write` needs an absolute path, found `gcsl-slice-43-never/x`".to_owned(),
        ),
        // A directory that does not exist, or is a file.
        (
            format!("import fs\n= fs.write(\"{dir}/none/x.txt\", \"a\")"),
            format!(
                "`fs.write` could not write `{dir}/none/x.txt`: No such file or directory \
                     (os error 2)"
            ),
        ),
        (
            format!("import fs\n= fs.write(\"{dir}/x.txt/y\", \"a\")"),
            format!("`fs.write` could not write `{dir}/x.txt/y`: Not a directory (os error 20)"),
        ),
        // A directory, full or empty.
        (
            format!("import fs\n= fs.write(\"{dir}/full\", \"a\")"),
            format!("`fs.write` could not write `{dir}/full`: Is a directory (os error 21)"),
        ),
        (
            format!("import fs\n= fs.write(\"{dir}/empty\", \"a\")"),
            format!("`fs.write` could not write `{dir}/empty`: Is a directory (os error 21)"),
        ),
        // A path that names no file at all.
        (
            "import fs\n= fs.write(\"/\", \"a\")".to_owned(),
            "`fs.write` could not write `/`: the path names no file".to_owned(),
        ),
    ] {
        let error = lone_error_in(&dir, &text);
        assert_eq!(error.message, message, "{text}");
        // At the user's call, the whole of it.
        let call = &text[text.find("= ").unwrap() + 2..];
        assert_eq!(error.span.start as usize, text.len() - call.len(), "{text}");
        assert_eq!(error.span.end as usize, text.len(), "{text}");
        assert!(error.labels.is_empty() && error.notes.is_empty(), "{text}");
    }
    assert!(!std::path::Path::new("gcsl-slice-43-never").exists());
    // Nothing was changed, and no temporary file is left behind.
    assert_eq!(
        std::fs::read_to_string(format!("{dir}/full/inner")).unwrap(),
        "inner"
    );
    assert!(std::path::Path::new(&format!("{dir}/empty")).is_dir());
    assert_eq!(
        std::fs::read_to_string(format!("{dir}/x.txt")).unwrap(),
        "x"
    );
    assert!(!std::path::Path::new(&format!("{dir}/none")).exists());
    assert!(
        temp_files_below(&dir).is_empty(),
        "{:?}",
        temp_files_below(&dir)
    );
    // From a pure function, the purity error, before anything runs.
    let text = format!("import fs\nfn f(): {{}} {{ fs.write(\"{dir}/pure\", \"a\") }}\n= f()");
    assert_eq!(
        lone_error_in(&dir, &text).message,
        "cannot call an impure function from a pure function"
    );
    assert!(!std::path::Path::new(&format!("{dir}/pure")).exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sort_versions_sorts_in_version_order() {
    for (text, expected) in [
        (
            "= sort_versions([\"6.10\", \"6.6\", \"6.6.1\", \"1.0~rc1\", \"1.0\"])",
            "[\"1.0~rc1\", \"1.0\", \"6.6\", \"6.6.1\", \"6.10\"]",
        ),
        // A string there twice is there twice, side by side; strings
        // that differ but whose runs are equal are apart, by their bytes.
        (
            "= sort_versions([\"1.0\", \"0.9\", \"1.00\", \"1.0\", \"0.9\"])",
            "[\"0.9\", \"0.9\", \"1.0\", \"1.0\", \"1.00\"]",
        ),
        ("= sort_versions([])", "[]"),
        ("let none: List<String> = []\n= sort_versions(none)", "[]"),
        ("= sort_versions([\"6.6\"])", "[\"6.6\"]"),
        (
            "= sort_versions([\"1\", \"2\", \"10\"])",
            "[\"1\", \"2\", \"10\"]",
        ),
        (
            "= sort_versions([\"10\", \"2\", \"1\"])",
            "[\"1\", \"2\", \"10\"]",
        ),
        (
            "= sort_versions([\"1.0\", \"1.0~rc2\", \"1.0~rc1\", \"1.0a\"])",
            "[\"1.0~rc1\", \"1.0~rc2\", \"1.0\", \"1.0a\"]",
        ),
        // The list sorted is not changed.
        (
            "let vs = [\"2\", \"1\"]\nlet sorted = sort_versions(vs)\n= [vs, sorted]",
            "[[\"2\", \"1\"], [\"1\", \"2\"]]",
        ),
        // By name, through `builtins` and a variable, and in a pure
        // function.
        ("= sort_versions(vs = [\"b\", \"a\"])", "[\"a\", \"b\"]"),
        (
            "import builtins\n= builtins.sort_versions([\"2\", \"1\"])",
            "[\"1\", \"2\"]",
        ),
        (
            "let s = sort_versions\n= s([\"2\", \"1\"])",
            "[\"1\", \"2\"]",
        ),
        (
            "fn newest(vs: List<String>): String {\n\
                 let sorted = sort_versions(vs)\n\
                 sorted[len(sorted) - 1]\n\
                 }\n\
                 = newest([\"6.6\", \"6.10\", \"6.9\"])",
            "6.10",
        ),
        // As `git.update` uses it.
        (
            "let tags = [{version = \"6.6\"}, {version = \"6.10\"}, {version = \"5.0\"}]\n\
                 = sort_versions(for t in tags if compare_versions(t.version, \"6.0\") >= 0 { t.version })",
            "[\"6.6\", \"6.10\"]",
        ),
        ("= sort_versions", "fn(vs: List<String>): List<String>"),
        // A file's own declaration shadows the prelude's.
        (
            "sort_versions = 1\nv = sort_versions",
            "{sort_versions = 1; v = 1}",
        ),
    ] {
        runs_to(text, expected);
    }
    // Agrees with `compare_versions` on every pair of what it sorts.
    let versions = [
        "1.0", "1.0~rc1", "1.0a", "1.0.0", "", "0", "01", "1.01", "1.1", "a", ".", "~", "6.10",
        "6.6", "6.3+nmu1", "2:1", "10", "9",
    ];
    let list = versions.map(|v| format!("\"{v}\"")).join(", ");
    let text = format!(
        "let sorted = sort_versions([{list}])\n\
             = for i in 1..len(sorted) {{ compare_versions(sorted[i - 1], sorted[i]) }}"
    );
    // All different, so each strictly before the next.
    let expected = format!("[{}]", vec!["-1"; versions.len() - 1].join(", "));
    assert_eq!(lone(&text).unwrap(), expected);
}

#[test]
fn show_gives_the_source_of_a_value() {
    for (text, expected) in [
        // Every kind of value.
        ("= show(1)", "1"),
        ("= show(-5)", "-5"),
        ("= show(9223372036854775807)", "9223372036854775807"),
        ("= show(1.5)", "1.5"),
        ("= show(3.0)", "3.0"),
        ("= show(-0.0)", "-0.0"),
        ("= show(1e300)", "1e300"),
        ("= show(0.000000015)", "1.5e-8"),
        ("= show(true)", "true"),
        ("= show(false)", "false"),
        ("= show(\"a\")", "\"a\""),
        ("= show(\"\")", "\"\""),
        ("= show({})", "{}"),
        ("= show([1, 2])", "[1, 2]"),
        ("let none: List<Int> = []\n= show(none)", "[]"),
        // The lock entry's shape.
        (
            "= show({version = \"6.6\"; commit = \"3a8f\"})",
            "{version = \"6.6\"; commit = \"3a8f\"}",
        ),
        // Nested records and lists, fields in their order.
        (
            "= show({z = {b = [{c = [1.5, 2.0]}]}; a = [[\"x\"], [\"y\", \"z\"]]; e = {}})",
            "{z = {b = [{c = [1.5, 2.0]}]}; a = [[\"x\"], [\"y\", \"z\"]]; e = {}}",
        ),
        (
            "let xs: List<List<Int>> = [[1], []]\n= show(xs)",
            "[[1], []]",
        ),
        // Field names quoted unless identifiers.
        (
            r#"= show({"a b" = 1; "if" = 2; "a.b" = 3; "{{x}}" = 4; "" = 5; ok-name = 6; "\n" = 7})"#,
            r#"{"a b" = 1; "if" = 2; "a.b" = 3; "{{x}}" = 4; "" = 5; ok-name = 6; "\n" = 7}"#,
        ),
        // Every escape, and braces, which are doubled; any other
        // character as it is, a control character too.
        (
            r#"= show("q\"b\\n\nt\tr\r{{x}} }}{{")"#,
            r#""q\"b\\n\nt\tr\r{{x}} }}{{""#,
        ),
        (
            "= show(\"\\u{0}\\u{1b}\\u{7f}\\u{85}\\u{2028}\u{e9}\u{1f600}\")",
            "\"\u{0}\u{1b}\u{7f}\u{85}\u{2028}\u{e9}\u{1f600}\"",
        ),
        // A list of several kinds, and a union.
        (
            "= show([1, \"x\", 2.5, true, {}])",
            "[1, \"x\", 2.5, true, {}]",
        ),
        ("let u: Int | String = \"s\"\n= show(u)", "\"s\""),
        // By name, through `builtins`, an alias, a member import.
        ("= show(value = 1)", "1"),
        ("import builtins\n= builtins.show(true)", "true"),
        ("let s = show\n= s(1)", "1"),
        ("s = show\nv = s([2])", "{s = builtin show; v = \"[2]\"}"),
        ("import show as render from prelude\n= render([1])", "[1]"),
        // A future argument, or one holding futures, awaited.
        ("fn f() -> Int { 3 }\n= show(f())", "3"),
        ("fn f() -> {a: Int} { {a = 1} }\n= show(f())", "{a = 1}"),
        ("= show(lazy {a = lazy 1})", "{a = 1}"),
        ("= show([lazy 1, lazy 2])", "[1, 2]"),
        (
            "let r: {a: Future<Int>} = {a = lazy 7}\n= show(r)",
            "{a = 7}",
        ),
        // In a pure function, of a present value.
        (
            "fn entry(p: {version: String, commit: String}): String { \"    {show(p)},\" }\n\
                 = entry({version = \"1\"; commit = \"c\"})",
            "    {version = \"1\"; commit = \"c\"},",
        ),
        // A file's own declaration shadows the prelude's.
        (
            "def show(n: Int): Int { n }\nv = show(3)",
            "{show = fn(n: Int): Int; v = 3}",
        ),
    ] {
        runs_to(text, expected);
    }
    // An alias of it crosses the boundary of a file, as `len`'s does.
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib/render.gcsl",
            "import show as r from prelude\nlet s = r\nas_text = s",
        ),
        (
            "main.gcsl",
            "import lib/render\nv = render.as_text({n = [1]})",
        ),
    ];
    assert_eq!(value(&files, "main.gcsl"), "{v = \"{{n = [1]}}\"}");
    // `Int`'s least value has no literal: its source does not read back.
    runs_to("= show(-9223372036854775807 - 1)", "-9223372036854775808");
    assert_eq!(
        lone("= -9223372036854775808").unwrap_err(),
        ["integer literal out of range for Int"]
    );
}

#[test]
fn show_cannot_render_nan_or_an_infinity() {
    for (text, shown) in [
        ("= show(0.0 / 0.0)", "nan"),
        ("= show(1.0 / 0.0)", "inf"),
        ("= show(-1.0 / 0.0)", "-inf"),
        ("= show({a = [1.0, 1.0 / 0.0]})", "inf"),
        // The first in the order it is shown.
        ("= show([[2.0], [0.0 / 0.0, 1.0 / 0.0]])", "nan"),
        ("= show({b = -1.0 / 0.0; a = 0.0 / 0.0})", "-inf"),
    ] {
        let error = lone_error(text);
        assert_eq!(
            error.message,
            format!("`show` cannot render `{shown}` as source"),
            "{text}"
        );
        assert_eq!(error.span, Span::new(2, u32::try_from(text.len()).unwrap()));
        assert!(error.labels.is_empty() && error.notes.is_empty(), "{text}");
    }
}

/// A deterministic generator of pseudo-random numbers, xorshift64*, for
/// the round-trip test of `show`.
struct Rng(u64);

impl Rng {
    /// The next number.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n`.
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
    }

    /// One of `choices`.
    fn pick<T: Copy>(&mut self, choices: &[T]) -> T {
        choices[self.below(choices.len())]
    }
}

/// The type of a value the round-trip test of `show` generates.
enum Shape {
    /// `Int`.
    Int,
    /// `Float`.
    Float,
    /// `Bool`.
    Bool,
    /// `String`.
    Str,
    /// A record type of these fields, in order.
    Record(Vec<(String, Shape)>),
    /// A list type of this element type.
    List(Box<Shape>),
}

impl Shape {
    /// A random type, nested at most `depth` deep.
    fn random(rng: &mut Rng, depth: u32) -> Shape {
        match rng.below(if depth == 0 { 4 } else { 6 }) {
            0 => Shape::Int,
            1 => Shape::Float,
            2 => Shape::Bool,
            3 => Shape::Str,
            4 => {
                let mut fields: Vec<(String, Shape)> = Vec::new();
                for _ in 0..rng.below(4) {
                    let name = random_name(rng);
                    if fields.iter().all(|(other, _)| *other != name) {
                        fields.push((name, Shape::random(rng, depth - 1)));
                    }
                }
                Shape::Record(fields)
            }
            _ => Shape::List(Box::new(Shape::random(rng, depth - 1))),
        }
    }

    /// The type as GCSL writes it, field names as [`FieldName`] shows
    /// them.
    fn text(&self) -> String {
        match self {
            Shape::Int => "Int".to_owned(),
            Shape::Float => "Float".to_owned(),
            Shape::Bool => "Bool".to_owned(),
            Shape::Str => "String".to_owned(),
            Shape::Record(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, shape)| {
                        format!("{}: {}", crate::strings::FieldName(name), shape.text())
                    })
                    .collect();
                format!("{{{}}}", fields.join(", "))
            }
            Shape::List(element) => format!("List<{}>", element.text()),
        }
    }

    /// A random value of the type as a GCSL literal, written another way
    /// than `show` writes it: every character of a string or a field
    /// name as a `\u{...}` escape, every `Float` in exponent form. Also
    /// whether it holds an empty list, which has no element type to
    /// infer where no type is expected.
    fn literal(&self, rng: &mut Rng) -> (String, bool) {
        match self {
            Shape::Int => {
                let value = match rng.below(3) {
                    0 => rng.pick(&[0, 1, -1, i64::MAX, i64::MIN + 1, 42, -7]),
                    1 => i64::try_from(rng.below(1000)).unwrap() - 500,
                    // Any but `i64::MIN`, which has no literal.
                    _ => i64::from_ne_bytes(rng.next().to_ne_bytes()).max(i64::MIN + 1),
                };
                (value.to_string(), false)
            }
            Shape::Float => {
                let value = match rng.below(2) {
                    0 => rng.pick(&[
                        0.0,
                        -0.0,
                        1.0,
                        -1.5,
                        0.1 + 0.2,
                        1e300,
                        1e16,
                        1e15,
                        1.5e-7,
                        5e-324,
                        f64::MAX,
                        f64::MIN,
                        f64::MIN_POSITIVE,
                        f64::EPSILON,
                    ]),
                    _ => loop {
                        let value = f64::from_bits(rng.next());
                        if value.is_finite() {
                            break value;
                        }
                    },
                };
                let sign = if value.is_sign_negative() { "-" } else { "" };
                (format!("{sign}{:e}", value.abs()), false)
            }
            Shape::Bool => (rng.pick(&["true", "false"]).to_owned(), false),
            Shape::Str => (escaped(&random_text(rng)), false),
            Shape::Record(fields) => {
                let mut empty = false;
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, shape)| {
                        let (value, holds) = shape.literal(rng);
                        empty |= holds;
                        format!("{} = {value}", escaped(name))
                    })
                    .collect();
                (format!("{{{}}}", fields.join("; ")), empty)
            }
            Shape::List(element) => {
                let count = rng.below(4);
                let mut empty = count == 0;
                let elements: Vec<String> = (0..count)
                    .map(|_| {
                        let (value, holds) = element.literal(rng);
                        empty |= holds;
                        value
                    })
                    .collect();
                (format!("[{}]", elements.join(", ")), empty)
            }
        }
    }
}

/// The characters the round-trip test of `show` builds strings of: what
/// is escaped, braces, control characters, and others.
const TEXT_CHARS: &str = "aZ0 \"\\{}\n\t\r\0\u{1}\u{1b}\u{7f}\u{80}\u{85}\u{a0}\u{e9}\u{2028}\
                              \u{feff}\u{1f600}#$'-";

/// A random string, of up to seven characters.
fn random_text(rng: &mut Rng) -> String {
    let chars: Vec<char> = TEXT_CHARS.chars().collect();
    (0..rng.below(8))
        .map(|_| {
            if rng.below(8) == 0 {
                // Any character at all.
                loop {
                    let code = u32::try_from(rng.below(0x11_0000)).unwrap();
                    if let Some(ch) = char::from_u32(code) {
                        break ch;
                    }
                }
            } else {
                rng.pick(&chars)
            }
        })
        .collect()
}

/// A random field name: an identifier, a keyword, or any text.
fn random_name(rng: &mut Rng) -> String {
    match rng.below(3) {
        0 => rng
            .pick(&["a", "b", "x1", "_", "ok-name", "version", "commit"])
            .to_owned(),
        1 => rng
            .pick(&["if", "let", "true", "fn", "a b", "", "1x", "a.b", "a-"])
            .to_owned(),
        _ => random_text(rng),
    }
}

/// `text` as a GCSL string literal of `\u{...}` escapes alone.
fn escaped(text: &str) -> String {
    let mut literal = String::from("\"");
    for ch in text.chars() {
        std::fmt::Write::write_fmt(&mut literal, format_args!("\\u{{{:x}}}", u32::from(ch)))
            .unwrap();
    }
    literal.push('"');
    literal
}

/// The fields of the record that `text`, run as a lone file, evaluates
/// to, by name.
fn lone_fields(text: &str) -> Vec<(String, Value)> {
    let source = SourceFile::new("lone.gcsl", text).unwrap();
    let outcome = crate::run(&source).unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let Value::Record(record) = outcome.value else {
        panic!("{text}: not a record");
    };
    record
        .layout
        .texts
        .iter()
        .map(ToString::to_string)
        .zip(record.values.iter().cloned())
        .collect()
}

/// The text of the string `value`.
fn text_of(value: &Value) -> String {
    let Value::Str(text) = value else {
        panic!("not a string: {value}");
    };
    text.to_string()
}

#[test]
fn what_show_gives_reads_back_as_an_equal_value() {
    let mut rng = Rng(0x5eed_0043_1ab5_c0de);
    // How many values held a record, a string, and no empty list: that
    // the generator is not degenerate.
    let (mut records, mut strings, mut bare) = (0, 0, 0);
    for _ in 0..300 {
        let shape = Shape::random(&mut rng, 3);
        let ty = shape.text();
        let (literal, holds_empty) = shape.literal(&mut rng);
        records += usize::from(ty.contains('{'));
        strings += usize::from(literal.contains("\\u{"));
        bare += usize::from(!holds_empty);
        // The value, written without `show`'s help, and its source.
        let fields = lone_fields(&format!("v: {ty} = {literal}\ns = show(v)"));
        let source = text_of(&fields[1].1);
        // Read back where its type is expected, it is an equal value,
        // whose source is the same text, `-0.0` and every digit of a
        // `Float` included.
        let text = format!(
            "a: {ty} = {literal}\n\
                 b: {ty} = {source}\n\
                 same = a == b\n\
                 again = show(b)"
        );
        let back = lone_fields(&text);
        assert_eq!(back[2].1, Value::Bool(true), "{text}");
        assert_eq!(text_of(&back[3].1), source, "{text}");
        // And where none is, unless an empty list's element type is left
        // to infer.
        if !holds_empty {
            let text = format!("a: {ty} = {literal}\nc = {source}\nsame = a == c");
            let back = lone_fields(&text);
            assert_eq!(back[2].1, Value::Bool(true), "{text}");
        }
    }
    assert!(
        records > 50 && strings > 50 && bare > 100,
        "{records} {strings} {bare}"
    );
}

/// The files of the summary of slice 31, in path order.
const DEFAULTS: [(&str, &str); 3] = [
    (
        "_lib/defaults.gcsl",
        "#[shadowable]\narch = \"x86_64\"\n#[shadowable] jobs = 4",
    ),
    (
        "pkg/build.gcsl",
        "import * from _lib/defaults\narch = \"aarch64\"          # shadows the default\n\
             j = jobs                   # 4",
    ),
    ("workspace.gcsl", ""),
];

#[test]
fn the_shadowable_summary_runs() {
    assert_eq!(
        value(&DEFAULTS, "pkg/build.gcsl"),
        "{arch = \"aarch64\"; j = 4}"
    );
    assert_eq!(
        value(&DEFAULTS, "_lib/defaults.gcsl"),
        "{arch = \"x86_64\"; jobs = 4}"
    );
    // The implicit prelude is the file's too, and a name of it is
    // shadowed by a declaration as before.
    let mut files = DEFAULTS.to_vec();
    files[1].1 = "import * from _lib/defaults\narch = \"aarch64\"\nn = len([jobs])\n\
                      trim = 3";
    assert_eq!(
        value(&files, "pkg/build.gcsl"),
        "{arch = \"aarch64\"; n = 1; trim = 3}"
    );
    // Without its mark, the import's `arch` conflicts, as it always has.
    files[0].1 = "arch = \"x86_64\"\n#[shadowable] jobs = 4";
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at pkg/build.gcsl (imported here); \
             _lib/defaults.gcsl: defined here; pkg/build.gcsl: defined here"
    );
}

#[test]
fn within_one_file_the_last_declaration_wins_for_the_whole_file() {
    for (text, expected) in [
        // A read above it sees it too.
        (
            "x = a\n#[shadowable] a = 1\na = 2\ny = a",
            "{x = 2; a = 2; y = 2}",
        ),
        // The last of several shadowable ones.
        (
            "#[shadowable] a = 1\n#[shadowable] a = 2\nx = a\n#[shadowable] a = 3",
            "{x = 3; a = 3}",
        ),
        // A `def` by a field, and a field by a `def`: the winner's type.
        (
            "#[shadowable] def f(): Int { 1 }\nf = 2\nx = f + 1",
            "{f = 2; x = 3}",
        ),
        (
            "#[shadowable] f = 2\ndef f(n: Int): Int { n + 1 }\nx = f(1)",
            "{f = fn(n: Int): Int; x = 2}",
        ),
        // A function reads the winner too.
        (
            "fn get(): Int { a }\n#[shadowable] a = 1\na = 5\nx = get()",
            "{a = 5; x = 5}",
        ),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // A shadowing declaration cannot read the value it shadows.
    assert_eq!(
        lone("#[shadowable] a = 1\na = a + 1").unwrap_err(),
        ["`a` depends on itself"]
    );
    // A plain one is shadowed by nothing, and a plain one followed by a
    // shadowable one is the conflict it always was.
    for text in ["a = 1\na = 2", "a = 1\n#[shadowable] a = 2"] {
        assert_eq!(
            lone(text).unwrap_err(),
            ["field `a` is defined twice"],
            "{text}"
        );
    }
}

#[test]
fn a_shadowed_declaration_is_checked_but_never_run() {
    // Its own errors are reported.
    assert_eq!(
        lone("#[shadowable] a = 1 + \"x\"\na = 2").unwrap_err(),
        ["cannot add Int and String"]
    );
    // It runs nothing.
    assert_eq!(
        lone("#[shadowable] a = panic(\"never\")\na = 2").unwrap(),
        "{a = 2}"
    );
    // It needs nothing: here it alone reads `lib/x.gcsl`, which is not
    // run, and whose cell it sees as erroneous, without an error.
    let files = [
        (
            "a.gcsl",
            "import * from lib/x\n#[shadowable] arch = thing + 1\narch = 2",
        ),
        ("lib/x.gcsl", "thing = panic(\"never run\")"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a.gcsl"), "{arch = 2}");
    let scan = ws(&files).unwrap();
    let a = scan.files.iter().position(|f| f.path == "a.gcsl").unwrap();
    assert!(scan.needs[a].is_empty(), "{:?}", scan.needs[a]);
    // Nor is it in a cycle.
    assert_eq!(
        lone("#[shadowable] a = b\nb = a\na = 1").unwrap(),
        "{b = 1; a = 1}"
    );
}

#[test]
fn across_the_files_of_a_directory_in_path_order() {
    let files = [
        ("a.gcsl", "#[shadowable] arch = \"x86_64\"\nmine = arch"),
        ("b.gcsl", "arch = \"aarch64\""),
        ("c.gcsl", "got = arch"),
        ("workspace.gcsl", ""),
    ];
    // Every file of the directory agrees, the shadowed one included,
    // whose record no longer has it.
    assert_eq!(value(&files, "c.gcsl"), "{got = \"aarch64\"}");
    assert_eq!(value(&files, "a.gcsl"), "{mine = \"aarch64\"}");
    assert_eq!(value(&files, "b.gcsl"), "{arch = \"aarch64\"}");
    // The reverse conflicts.
    let files = [
        ("a.gcsl", "arch = \"x86_64\""),
        ("b.gcsl", "#[shadowable] arch = \"aarch64\""),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at b.gcsl (defined here); \
             a.gcsl: defined here"
    );
}

#[test]
fn an_inner_directory_shadows_an_outer_one() {
    let files = [
        ("other.gcsl", "got = arch"),
        ("sub/s.gcsl", "arch = \"aarch64\"\nmine = arch"),
        ("sub/t.gcsl", "seen = arch"),
        ("workspace.gcsl", "#[shadowable] arch = \"x86_64\""),
    ];
    assert_eq!(value(&files, "other.gcsl"), "{got = \"x86_64\"}");
    assert_eq!(
        value(&files, "sub/s.gcsl"),
        "{arch = \"aarch64\"; mine = \"aarch64\"}"
    );
    assert_eq!(value(&files, "sub/t.gcsl"), "{seen = \"aarch64\"}");
    // The reverse conflicts, with its note, reported as ever at the
    // later of the two in the order of conflicts (the file's own
    // directory first, then upward).
    let files = [
        ("sub/s.gcsl", "#[shadowable] arch = \"aarch64\""),
        ("workspace.gcsl", "arch = \"x86_64\""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at workspace.gcsl (defined here); \
             sub/s.gcsl: defined here; note: one is in a directory above"
    );
}

#[test]
fn an_import_and_a_field_shadow_in_their_order() {
    // A field after an import of a shadowable leaf: the field wins.
    let mut files = DEFAULTS.to_vec();
    files[1].1 = "import arch from _lib/defaults\narch = \"own\"\ngot = arch";
    assert_eq!(
        value(&files, "pkg/build.gcsl"),
        "{arch = \"own\"; got = \"own\"}"
    );
    // An import after a shadowable field: the import wins in this
    // file's view, which is its own; the field is still its
    // directory's, which the other files of the directory see.
    let files = [
        ("lib/defs.gcsl", "arch = \"lib\""),
        (
            "pkg/a.gcsl",
            "#[shadowable] arch = \"own\"\nimport arch from lib/defs\ngot = arch",
        ),
        ("pkg/b.gcsl", "seen = arch"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "pkg/a.gcsl"),
        "{arch = \"own\"; got = \"lib\"}"
    );
    assert_eq!(value(&files, "pkg/b.gcsl"), "{seen = \"own\"}");
    // A plain field before the import conflicts, as it always has.
    let files = [
        ("lib/defs.gcsl", "arch = \"lib\""),
        ("pkg/a.gcsl", "arch = \"own\"\nimport arch from lib/defs"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at pkg/a.gcsl (imported here); \
             lib/defs.gcsl: defined here; pkg/a.gcsl: defined here"
    );
}

#[test]
fn a_marked_import_brings_shadowable_leaves() {
    let files = [
        (
            "a/a.gcsl",
            "#[shadowable] import * from lib/defs\narch = \"own\"\nj = jobs",
        ),
        (
            "b/b.gcsl",
            "#[shadowable] import arch, jobs as n from lib/defs\narch = 1\nn = 3",
        ),
        ("lib/defs.gcsl", "arch = \"lib\"\njobs = 2"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a/a.gcsl"), "{arch = \"own\"; j = 2}");
    assert_eq!(value(&files, "b/b.gcsl"), "{arch = 1; n = 3}");
    // Unmarked, the same is the conflict it always was.
    let files = [
        ("a/a.gcsl", "import * from lib/defs\narch = \"own\""),
        ("lib/defs.gcsl", "arch = \"lib\""),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        shown_error(&files),
        "field `arch` is defined in two files at a/a.gcsl (imported here); \
             lib/defs.gcsl: defined here; a/a.gcsl: defined here"
    );
    // A `let` or a `fn` of its name shadows it silently, from its item
    // on, or in the whole file.
    let files = [
        ("lib/defs.gcsl", "arch = \"lib\"\njobs = 2"),
        (
            "pkg/a.gcsl",
            "#[shadowable] import * from lib/defs\nbefore = arch\nlet arch = 1\n\
                 after = arch\nfn jobs(): Int { 9 }\nj = jobs()",
        ),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "pkg/a.gcsl"),
        "{before = \"lib\"; after = 1; j = 9}"
    );
}

#[test]
fn shadowability_travels_with_a_leaf() {
    // Through `*` from a file, a member, a directory's merged tree and
    // a directory above.
    let files = [
        ("_lib/defaults.gcsl", "#[shadowable] arch = \"x86_64\""),
        ("_lib/more.gcsl", "jobs = 4"),
        ("a/a.gcsl", "import * from _lib/defaults\narch = \"a\""),
        ("b/b.gcsl", "import arch from _lib/defaults\narch = \"b\""),
        ("c/c.gcsl", "import * from _lib\narch = \"c\"\nj = jobs"),
        ("d/d.gcsl", "import _lib\narch = \"d\"\nx = _lib.arch"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a/a.gcsl"), "{arch = \"a\"}");
    assert_eq!(value(&files, "b/b.gcsl"), "{arch = \"b\"}");
    assert_eq!(value(&files, "c/c.gcsl"), "{arch = \"c\"; j = 4}");
    assert_eq!(value(&files, "d/d.gcsl"), "{arch = \"d\"; x = \"x86_64\"}");
    // And to the directories below its own, which see it in the
    // directory's merged tree.
    let files = [
        ("e/e.gcsl", "#[shadowable] arch = \"e\"\ny = arch"),
        ("e/f/f.gcsl", "arch = \"f\"\nz = arch"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "e/f/f.gcsl"), "{arch = \"f\"; z = \"f\"}");
    assert_eq!(value(&files, "e/e.gcsl"), "{arch = \"e\"; y = \"e\"}");
}

#[test]
fn dotted_leaves_are_shadowed_leaf_by_leaf() {
    // Two files of one directory: the later leaf wins, the others of
    // the node merge as before.
    let files = [
        ("a.gcsl", "#[shadowable] com.x.port = 1\ncom.x.host = \"h\""),
        ("b.gcsl", "com.x.port = 2"),
        ("c.gcsl", "all = com"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "c.gcsl"),
        "{all = {x = {host = \"h\"; port = 2}}}"
    );
    // Through an import.
    let files = [
        (
            "lib/defs.gcsl",
            "#[shadowable] com.x.port = 1\ncom.x.host = \"h\"",
        ),
        (
            "pkg/a.gcsl",
            "import * from lib/defs\ncom.x.port = 2\nall = com",
        ),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(
        value(&files, "pkg/a.gcsl"),
        "{com = {x = {port = 2}}; all = {x = {port = 2; host = \"h\"}}}"
    );
}

#[test]
fn a_declaration_overlapping_shadowable_ones_shadows_them_all() {
    for (text, expected) in [
        // At, above and below its path.
        ("#[shadowable] a.x = 1\na = {z = 3}", "{a = {z = 3}}"),
        ("#[shadowable] d = {j = 4; k = 5}\nd.j = 8", "{d = {j = 8}}"),
        (
            "#[shadowable] a.x.y = 1\n#[shadowable] a.w = 2\na.x = 3",
            "{a = {w = 2; x = 3}}",
        ),
        // Not overlapping: they merge as before.
        ("#[shadowable] a.x = 1\na.y = 2", "{a = {x = 1; y = 2}}"),
    ] {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }
    // If one it overlaps is not shadowable, the conflict as today.
    assert_eq!(
        lone("#[shadowable] a.x = 1\na.y = 2\na = {}").unwrap_err(),
        ["field `a` is defined twice"]
    );
    assert_eq!(
        lone("d = {j = 4}\nd.j = 8").unwrap_err(),
        ["field `d` is already defined"]
    );
    // Across files and directories, as within one.
    let files = [
        ("a.gcsl", "#[shadowable] d = {j = 4; k = 5}"),
        ("b.gcsl", "d.j = 8"),
        ("c.gcsl", "got = d"),
        ("sub/s.gcsl", "a = 1\nseen = d"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "c.gcsl"), "{got = {j = 8}}");
    assert_eq!(value(&files, "sub/s.gcsl"), "{a = 1; seen = {j = 8}}");
    let files = [
        ("sub/s.gcsl", "a = {z = 3}\ngot = a"),
        ("workspace.gcsl", "#[shadowable] a.x = 1\nseen = a"),
    ];
    assert_eq!(value(&files, "sub/s.gcsl"), "{a = {z = 3}; got = {z = 3}}");
    assert_eq!(
        value(&files, "workspace.gcsl"),
        "{a = {x = 1}; seen = {x = 1}}"
    );
}

#[test]
fn a_dotted_root_shadows_the_prelude_s_name_as_before() {
    // The five cases the old rule's drop by root name shadowed: each
    // is its result before this slice.
    assert_eq!(
        lone("len.x = 1\ny = len").unwrap(),
        "{len = {x = 1}; y = {x = 1}}"
    );
    let files = [
        ("app/main.gcsl", "import * from lib/a\ny = len"),
        ("lib/a.gcsl", "len.x = 1"),
        ("workspace.gcsl", "x = 1"),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{y = {x = 1}}");
    let files = [
        ("app/main.gcsl", "s = join\nn = len([1, 2])"),
        ("workspace.gcsl", "join.sep = \",\""),
    ];
    assert_eq!(value(&files, "app/main.gcsl"), "{s = {sep = \",\"}; n = 2}");
    let files = [
        ("a.gcsl", "len.x = 1"),
        ("b.gcsl", "y = len\nparts = split(\"a,b\", \",\")"),
        ("workspace.gcsl", "x = 1"),
    ];
    assert_eq!(
        value(&files, "b.gcsl"),
        "{y = {x = 1}; parts = [\"a\", \"b\"]}"
    );
    // (The REPL's is `crate::session`'s.) An explicit import is plain,
    // and still conflicts.
    assert_eq!(
        lone("import * from prelude\nlen.x = 1").unwrap_err(),
        ["field `len` is already defined"]
    );
}

#[test]
fn no_prelude_turns_the_implicit_import_off() {
    assert_eq!(
        lone("#![no_prelude]\nn = len([1])").unwrap_err(),
        ["undefined variable `len`"]
    );
    assert_eq!(
        lone("# a comment\n#![no_prelude]\nimport len from builtins\nn = len([1])").unwrap(),
        "{n = 1}"
    );
    let files = [
        ("main.gcsl", "#![no_prelude]\nn = trim(\" a \")"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(messages(&files, "main.gcsl"), ["undefined variable `trim`"]);
    // The prelude has it, and so no import of its own; every other
    // file, the standard ones included, has the implicit one.
    let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "import path")]).unwrap();
    let prelude = scan.standard_file(PRELUDE).unwrap();
    let path = scan.standard_file("path").unwrap();
    let files = &scan.files;
    assert!(files[prelude as usize].ast.no_prelude().is_some());
    assert!(files[prelude as usize].ast.implicit_import().is_none());
    assert!(files[prelude as usize].aliases.iter().all(|a| !a.implicit));
    for file in [path, 2, 3] {
        let file = &files[file as usize];
        assert!(file.ast.implicit_import().is_some(), "{}", file.path);
        assert!(file.aliases.iter().any(|a| a.implicit), "{}", file.path);
    }
}

#[test]
fn the_clash_family_ignores_shadowable_leaves() {
    // A `let`, a `fn`, a `def`, a field or a whole import of the name
    // of a shadowable field of a directory above shadows it silently.
    let files = [
        ("a/a.gcsl", "let port = 5\nn = port"),
        ("b/b.gcsl", "fn port(): Int { 7 }\nn = port()"),
        ("c/c.gcsl", "def port(): Int { 8 }\nn = port()"),
        ("d/d.gcsl", "import lib/port\nn = port.x"),
        ("e/e.gcsl", "port = 6\nn = port"),
        ("lib/port.gcsl", "x = 9"),
        ("workspace.gcsl", "#[shadowable] port = 1"),
    ];
    assert_eq!(value(&files, "a/a.gcsl"), "{n = 5}");
    assert_eq!(value(&files, "b/b.gcsl"), "{n = 7}");
    assert_eq!(value(&files, "c/c.gcsl"), "{port = fn(): Int; n = 8}");
    assert_eq!(value(&files, "d/d.gcsl"), "{n = 9}");
    assert_eq!(value(&files, "e/e.gcsl"), "{port = 6; n = 6}");
    // Within one file, a `let` from its item on and a `fn` in the whole
    // file; the field is still the file's.
    assert_eq!(
        lone("#[shadowable] a = 1\nx = a\nlet a = 2\ny = a").unwrap(),
        "{a = 1; x = 1; y = 2}"
    );
    assert_eq!(
        lone("x = a()\n#[shadowable] a = 1\nfn a(): Int { 3 }").unwrap(),
        "{x = 3; a = 1}"
    );
    // Plain, they clash as they always have.
    let files = [
        ("sub/a.gcsl", "let port = 5"),
        ("workspace.gcsl", "port = 1"),
    ];
    assert_eq!(
        shown_checked_error(&files),
        "`port` is a field of this directory"
    );
    assert_eq!(
        lone("a = 1\nlet a = 2").unwrap_err(),
        ["`a` is a field of this file"]
    );
}

#[test]
fn a_declaration_a_later_file_shadows_is_still_checked_and_its_file_s() {
    // Its own error, against its own file.
    let sources: Vec<SourceFile> = [
        ("a.gcsl", "#[shadowable] arch = 1 + \"x\""),
        ("b.gcsl", "arch = 2"),
        ("workspace.gcsl", ""),
    ]
    .iter()
    .map(|&(path, text)| SourceFile::new(path, text).unwrap())
    .collect();
    let errors = check(&sources).unwrap_err();
    let scan = ws(&[("a.gcsl", ""), ("workspace.gcsl", "")]).unwrap();
    let a = scan.files.iter().position(|f| f.path == "a.gcsl").unwrap();
    let shown: Vec<(&str, Option<u32>)> = errors
        .iter()
        .map(|e| (e.message.as_str(), e.input))
        .collect();
    assert_eq!(shown, [("cannot add Int and String", Some(count(a)))]);
    // A `let` that shadows a file's own shadowable field is the file's
    // alone: the directory has the field.
    let files = [
        ("a.gcsl", "#[shadowable] a = 1\nlet a = 2\nmine = a"),
        ("b.gcsl", "x = a"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{x = 1}");
    assert_eq!(value(&files, "a.gcsl"), "{a = 1; mine = 2}");
    // A `fn` shadows a dotted shadowable root of its file in the whole
    // file; the root is still its field, and its directory's.
    let files = [
        (
            "a.gcsl",
            "#[shadowable] a.x = 1\nfn a(): Int { 3 }\nb = a()",
        ),
        ("c.gcsl", "y = a.x"),
        ("workspace.gcsl", ""),
    ];
    assert_eq!(value(&files, "a.gcsl"), "{a = {x = 1}; b = 3}");
    assert_eq!(value(&files, "c.gcsl"), "{y = 1}");
}

/// The messages of the errors of checking `files`, joined.
fn shown_checked_error(files: &[(&str, &str)]) -> String {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let errors = check(&sources).unwrap_err();
    let shown: Vec<String> = errors.into_iter().map(|e| e.message).collect();
    shown.join("; ")
}

#[test]
fn an_input_cannot_turn_the_prelude_off() {
    let started = start("/").unwrap();
    let (mut names, mut types) = (started.names, started.types);
    let source = SourceFile::new("input", "#![no_prelude]\nx = 1").unwrap();
    let earlier = Earlier {
        imports: &[],
        types: &[],
        store: &mut TypeStore::new(),
        def_types: &[],
        variants: &[],
        contexts: &[],
    };
    let errors = check_input(
        &started.standard,
        (&source, earlier),
        (&mut names, &mut types),
        Scope::of_slots(started.slot_names),
        2,
    )
    .unwrap_err();
    let shown: Vec<(String, Span, Option<u32>)> = errors
        .into_iter()
        .map(|e| (e.message, e.span, e.input))
        .collect();
    assert_eq!(
        shown,
        [(
            "`#![no_prelude]` applies only to a file".to_owned(),
            Span::new(0, 14),
            None
        )]
    );
}

/// The root the `fileroot` tests' workspaces are located at: a path in
/// the scratch directory, which nothing reads, since the library reads
/// no file.
const SCRATCH: &str = "/scratch/fileroot";

/// The library of the `fileroot` summary, `_lib/ppk.gcsl`, with what the
/// other tests call besides: `commit`, `wrapped` (a helper), `twice`
/// (a helper of a helper), `here` (`this` in a `def`), `caller` (the
/// caller of a `def` of the library), `mid` (`helper()` between two
/// calls), closures made in the library, and a step made in it.
const PPK: &str = "import path\nimport fileroot\n\
        def commit(content: String, output: String) -> String {\n\
            let content = path.join(fileroot.caller, content)\n\
            let output = path.join(fileroot.workspace, output)\n\
            \"{content} {output}\"\n\
        }\n\
        def wrapped(content: String) -> String {\n\
            fileroot.helper()\n\
            commit(content, \"dist\")\n\
        }\n\
        def twice(content: String) -> String {\n\
            fileroot.helper()\n\
            wrapped(content)\n\
        }\n\
        def here(): String { fileroot.this }\n\
        def caller(): String { fileroot.caller }\n\
        def mid(): {before: String, after: String} {\n\
            let first = caller()\n\
            fileroot.helper()\n\
            let second = caller()\n\
            { before = first; after = second }\n\
        }\n\
        def reader(): fn(): String { fn(): String { fileroot.caller } }\n\
        def calling(): fn(): String { fn(): String { caller() } }\n\
        def call(f: fn(): String): String { f() }\n\
        def step(): {s: Future<String>} { s = lazy { fileroot.caller } }\n\
        def force(f: Future<String>) -> String { await f }";

/// The workspace of `files`, each a path and a text, located at
/// [`SCRATCH`], checked.
fn rooted(files: &[(&str, &str)]) -> Result<Workspace, Vec<Diagnostic>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    check_with_standard(&sources, &[], SCRATCH).1
}

/// The value of the file at `path` of the workspace of `files` located
/// at [`SCRATCH`].
fn rooted_value(files: &[(&str, &str)], path: &str) -> Value {
    let mut workspace = rooted(files).unwrap_or_else(|errors| panic!("{path}: {errors:?}"));
    assert_imports_stored(&workspace);
    assert_assemblies_stored(&workspace);
    let index = workspace.file(path).unwrap();
    match workspace.run(index) {
        Ok(outcome) => outcome.value,
        Err(errors) => panic!("{path}: {errors:?}"),
    }
}

/// [`rooted_value`] shown, [`SCRATCH`] shown as `<root>`.
fn rooted_text(files: &[(&str, &str)], path: &str) -> String {
    rooted_value(files, path)
        .to_string()
        .replace(SCRATCH, "<root>")
}

/// The messages of the diagnostics of checking the workspace of `files`
/// located at [`SCRATCH`].
fn rooted_messages(files: &[(&str, &str)]) -> Vec<String> {
    match rooted(files) {
        Ok(_) => panic!("{files:?}: no error"),
        Err(errors) => errors.into_iter().map(|error| error.message).collect(),
    }
}

#[test]
fn the_fileroot_summary_runs_in_a_workspace() {
    let files = [
        ("workspace.gcsl", ""),
        ("_lib/ppk.gcsl", PPK),
        (
            "pkg/test.gcsl",
            "import _lib/ppk\nimport fileroot\n\
                 a = ppk.commit(\"usr\", \"dist\")\n\
                 b = ppk.wrapped(\"usr\")\n\
                 here = fileroot.this",
        ),
    ];
    // The paths are absolute, from the root the driver was given.
    assert_eq!(
        rooted_value(&files, "pkg/test.gcsl").to_string(),
        "{a = \"/scratch/fileroot/pkg/usr /scratch/fileroot/dist\"; \
             b = \"/scratch/fileroot/pkg/usr /scratch/fileroot/dist\"; \
             here = \"/scratch/fileroot/pkg\"}"
    );
    // `this` and `workspace` are constants of the file, `caller` and
    // `helper()` nodes of their own, and each file's code carries its
    // directory, by its index in the run's table, the root's first.
    let mut workspace = rooted(&files).unwrap();
    let (test, lib) = (
        workspace.file("pkg/test.gcsl").unwrap(),
        workspace.file("_lib/ppk.gcsl").unwrap(),
    );
    let test_ir = workspace.dump_ir(test).unwrap();
    assert!(
        test_ir.contains("(store here#") && test_ir.contains("(str \"/scratch/fileroot/pkg\""),
        "{test_ir}"
    );
    let lib_ir = workspace.dump_ir(lib).unwrap();
    assert!(lib_ir.contains("(caller @"), "{lib_ir}");
    assert!(lib_ir.contains("(helper @"), "{lib_ir}");
    assert!(lib_ir.contains("(str \"/scratch/fileroot\" @"), "{lib_ir}");
    let dirs: Vec<&str> = workspace.scan.dir_texts.iter().map(|d| &**d).collect();
    assert_eq!(dirs[0], SCRATCH);
    for (file, dir) in [
        (test, "/scratch/fileroot/pkg"),
        (lib, "/scratch/fileroot/_lib"),
    ] {
        let FileRun::Pending(ir) = &workspace.runs[file as usize] else {
            panic!("expected pending IR for file {file}");
        };
        assert_eq!(dirs[ir.dir() as usize], dir);
    }
    // The same, run from a file of the root itself.
    let files = [
        ("workspace.gcsl", ""),
        ("_lib/ppk.gcsl", PPK),
        (
            "top.gcsl",
            "import _lib/ppk\na = ppk.commit(\"usr\", \"dist\")",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "top.gcsl"),
        "{a = \"<root>/usr <root>/dist\"}"
    );
}

#[test]
fn this_and_caller_through_calls_and_helpers() {
    let files = [
        ("workspace.gcsl", ""),
        ("_lib/ppk.gcsl", PPK),
        (
            "pkg/test.gcsl",
            "import _lib/ppk\nimport fileroot\n\
                 top = fileroot.caller\n\
                 this = fileroot.this\n\
                 lib = ppk.here()\n\
                 deep = ppk.caller()\n\
                 one = ppk.wrapped(\"usr\")\n\
                 two = ppk.twice(\"usr\")\n\
                 mid = ppk.mid()\n\
                 fn mine(): String { fileroot.caller }\n\
                 own = mine()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "pkg/test.gcsl"),
        "{top = \"<root>/pkg\"; this = \"<root>/pkg\"; lib = \"<root>/_lib\"; \
             deep = \"<root>/pkg\"; one = \"<root>/pkg/usr <root>/dist\"; \
             two = \"<root>/pkg/usr <root>/dist\"; \
             mid = {before = \"<root>/_lib\"; after = \"<root>/pkg\"}; own = \"<root>/pkg\"}"
    );
}

#[test]
fn closures_and_lazies_carry_their_files() {
    let files = [
        ("workspace.gcsl", ""),
        ("_lib/ppk.gcsl", PPK),
        (
            "pkg/make.gcsl",
            "import _lib/ppk\nimport fileroot\n\
                 fn mine(): String { fileroot.caller }\n\
                 reads = ppk.reader()()\n\
                 calls = ppk.calling()()\n\
                 passed = ppk.call(mine)\n\
                 made = ppk.step()\n\
                 written = lazy { ppk.caller() }",
        ),
        (
            "other/await.gcsl",
            "import pkg/make\nimport _lib/ppk\n\
                 made = await make.made.s\n\
                 written = await make.written\n\
                 forced = ppk.force(ppk.step().s)",
        ),
    ];
    // A closure of the library called from `pkg` has `pkg` as its
    // caller, and calls with `_lib`, its code's; a closure of `pkg`
    // called by the library has `_lib`.
    let made = rooted_text(&files, "pkg/make.gcsl");
    assert!(
        made.starts_with(
            "{reads = \"<root>/pkg\"; calls = \"<root>/_lib\"; passed = \"<root>/_lib\""
        ),
        "{made}"
    );
    // A `lazy` runs as if where it was written, with the caller of the
    // frame that made it, whoever awaits it.
    assert_eq!(
        rooted_text(&files, "other/await.gcsl"),
        "{made = \"<root>/pkg\"; written = \"<root>/pkg\"; forced = \"<root>/other\"}"
    );
}

#[test]
fn the_workspace_is_the_root_wherever_the_file_is() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "a/b/c.gcsl",
            "import fileroot\nws = fileroot.workspace\nhere = fileroot.this",
        ),
    ];
    assert_eq!(
        rooted_value(&files, "a/b/c.gcsl").to_string(),
        "{ws = \"/scratch/fileroot\"; here = \"/scratch/fileroot/a/b\"}"
    );
    // A lone file is the workspace of its own directory, as the driver
    // was told it.
    let source = SourceFile::new(
        "lone.gcsl",
        "import fileroot as fr\nws = fr.workspace\nhere = fr.this\ntop = fr.caller",
    )
    .unwrap();
    let (_, checked) = check_lone_with_standard(&source, "/scratch/alone");
    let mut workspace = checked.unwrap();
    let index = workspace.file("lone.gcsl").unwrap();
    assert_eq!(
        workspace.run(index).unwrap().value.to_string(),
        "{ws = \"/scratch/alone\"; here = \"/scratch/alone\"; top = \"/scratch/alone\"}"
    );
    // A root of `/` takes no second `/`.
    let source = SourceFile::new("lone.gcsl", "import fileroot\nws = fileroot.workspace").unwrap();
    let (_, checked) = check_lone_with_standard(&source, "/");
    let mut workspace = checked.unwrap();
    let index = workspace.file("lone.gcsl").unwrap();
    assert_eq!(
        workspace.run(index).unwrap().value.to_string(),
        "{ws = \"/\"}"
    );
    let files = [
        ("workspace.gcsl", ""),
        ("sub/a.gcsl", "import fileroot\nh = fileroot.this"),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let mut workspace = check_with_standard(&sources, &[], "/").1.unwrap();
    let index = workspace.file("sub/a.gcsl").unwrap();
    assert_eq!(
        workspace.run(index).unwrap().value.to_string(),
        "{h = \"/sub\"}"
    );
}

#[test]
fn this_in_a_standard_file_is_the_standard_directory() {
    let table = [
        (
            "here",
            "import fileroot\nwhere = fileroot.this\ndef call(): String { fileroot.caller }",
        ),
        prelude_entry(),
    ];
    let files = [
        ("workspace.gcsl", ""),
        (
            "app/main.gcsl",
            "import here\nw = here.where\nc = here.call()",
        ),
    ];
    let (_, value) = value_beside(&table, &files, "app/main.gcsl");
    assert_eq!(value, "{w = \"<stdlib>\"; c = \"/app\"}");
}

#[test]
fn fileroot_is_read_only_as_its_members() {
    let with = |text: &str| rooted_messages(&[("workspace.gcsl", ""), ("a.gcsl", text)]);
    let read_as = |member: &str| format!("`fileroot`'s members are read as `fileroot.{member}`");
    assert_eq!(with("import caller from fileroot"), [read_as("caller")]);
    assert_eq!(with("import this as t from fileroot"), [read_as("this")]);
    assert_eq!(with("import * from fileroot"), [read_as("caller")]);
    assert_eq!(
        with("import nope from fileroot"),
        ["`fileroot` has no `nope`"]
    );
    assert_eq!(with("import fileroot\nx = fileroot"), [read_as("caller")]);
    assert_eq!(
        with("import fileroot as fr\nx = fr"),
        ["`fr`'s members are read as `fr.caller`"]
    );
    assert_eq!(
        with("import fileroot\nx = (fileroot).this"),
        [read_as("caller")]
    );
    assert_eq!(
        with("import fileroot\nx = fileroot.helper"),
        ["`fileroot.helper` is not a value"]
    );
    assert_eq!(
        with("import fileroot\nx = (fileroot.helper)()"),
        ["`fileroot.helper` is not a value"]
    );
    assert_eq!(
        with("import fileroot\nx = fileroot.helper(1)"),
        ["expected 0 arguments, found 1"]
    );
    assert_eq!(
        with("import fileroot\nx = fileroot.this(1)"),
        ["cannot call a value of type String"]
    );
    assert_eq!(
        with("import fileroot\nx = fileroot.nope"),
        ["`fileroot` has no `nope`"]
    );
    assert_eq!(
        with("import fileroot\nx = fileroot.caller.x"),
        ["cannot access field `x` of String"]
    );
    // Not imported, it is no name; a nearer binding shadows it.
    assert_eq!(with("x = fileroot.this"), ["undefined variable `fileroot`"]);
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "import fileroot\nfn f(fileroot: {this: Int}): Int { fileroot.this }\nx = f({ this = 1 })",
        ),
    ];
    assert_eq!(rooted_text(&files, "a.gcsl"), "{x = 1}");
    // `helper()` is pure, and does nothing at the top level.
    let files = [
        ("workspace.gcsl", ""),
        ("_lib/ppk.gcsl", PPK),
        (
            "a.gcsl",
            "import fileroot\nimport _lib/ppk\nfileroot.helper()\nx = ppk.caller()\n\
                 fn pure(): String { fileroot.helper()\nfileroot.caller }\ny = pure()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "a.gcsl"),
        "{x = \"<root>\"; y = \"<root>\"}"
    );
}

#[test]
fn fileroot_is_a_reserved_name() {
    for files in [
        &[("workspace.gcsl", ""), ("fileroot.gcsl", "x = 1")][..],
        &[("fileroot/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
    ] {
        assert_eq!(
            error(files),
            (
                "`fileroot` is the name of a built-in module".to_owned(),
                Some(1),
                vec![]
            ),
            "{files:?}"
        );
    }
}

/// The library of the memoisation tests: `reads`, which reads its
/// caller; `plain`, which does not; `outer`, a helper whose callee reads
/// the caller it passes on; `outer_hit`, a helper whose memoised callee,
/// `reads`, may be found; `awaits`, which reads its caller in a `lazy`
/// it awaits while it runs; `again`, which calls itself with its own
/// arguments, from its own file, once it has read a caller of another;
/// and four that return a step: `step`, whose `lazy` reads its caller,
/// `plain_step`, whose does not, `helped`, whose is made after
/// `fileroot.helper()` and calls `inner`, `nested`, whose makes a `lazy`
/// that reads it, and `fn_step`, whose makes a function that reads its
/// own.
const MEMO_LIB: &str = "import fileroot\n\
        def reads(x: Int) ->> {c: String} { c = fileroot.caller }\n\
        def plain(x: Int) ->> {v: Int} { v = x }\n\
        def inner(): String { fileroot.caller }\n\
        def outer(x: Int) ->> {c: String} {\n\
            fileroot.helper()\n\
            c = inner()\n\
        }\n\
        def outer_hit(x: Int) ->> {c: String} {\n\
            fileroot.helper()\n\
            reads(x)\n\
        }\n\
        def awaits(x: Int) ->> {c: String} {\n\
            let made = lazy { fileroot.caller }\n\
            c = await made\n\
        }\n\
        def again(x: Int) ->> String {\n\
            let c = fileroot.caller\n\
            if c == fileroot.this { c } else { again(x) + \" \" + c }\n\
        }\n\
        def step(x: Int) ->> {s: Future<String>} { s = lazy { fileroot.caller } }\n\
        def plain_step(x: Int) ->> {s: Future<Int>} { s = lazy { x + 1 } }\n\
        def helped(x: Int) ->> {s: Future<String>} {\n\
            fileroot.helper()\n\
            s = lazy { inner() }\n\
        }\n\
        def nested(x: Int) ->> {s: Future<String>} { s = lazy { await lazy { fileroot.caller } } }\n\
        def fn_step(x: Int) ->> {s: Future<fn(): String>} { s = lazy { fn(): String { fileroot.caller } } }";

/// Whether the values of the fields of `record` at `a` and `b` are one
/// record: one run of the memoised function that made both.
fn one_run(record: &Value, (a, b): (usize, usize)) -> bool {
    let Value::Record(record) = record else {
        panic!("expected a record, got {record}");
    };
    match (record.values.get(a), record.values.get(b)) {
        (Some(Value::Record(a)), Some(Value::Record(b))) => Rc::ptr_eq(a, b),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn a_memoised_call_depends_on_the_caller_it_reads() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        (
            "p/a.gcsl",
            "import lib/m\nr = m.reads(1)\nv = m.plain(1)\no = m.outer(1)\nw = m.awaits(1)",
        ),
        (
            "p/b.gcsl",
            "import lib/m\nrb = m.reads(1)\nvb = m.plain(1)\nob = m.outer(1)\nwb = m.awaits(1)",
        ),
        (
            "q/c.gcsl",
            "import lib/m\nr = m.reads(1)\nv = m.plain(1)\no = m.outer(1)\nw = m.awaits(1)",
        ),
        (
            "z/all.gcsl",
            "import p/a\nimport p/b\nimport q/c\n\
                 ra = a.r\nrb = b.rb\nrc = c.r\n\
                 va = a.v\nvb = b.vb\nvc = c.v\n\
                 oa = a.o\nob = b.ob\noc = c.o\n\
                 wa = a.w\nwb = b.wb\nwc = c.w",
        ),
    ];
    let all = rooted_value(&files, "z/all.gcsl");
    assert_eq!(
        all.to_string().replace(SCRATCH, "<root>"),
        "{ra = {c = \"<root>/p\"}; rb = {c = \"<root>/p\"}; rc = {c = \"<root>/q\"}; \
             va = {v = 1}; vb = {v = 1}; vc = {v = 1}; \
             oa = {c = \"<root>/p\"}; ob = {c = \"<root>/p\"}; oc = {c = \"<root>/q\"}; \
             wa = {c = \"<root>/p\"}; wb = {c = \"<root>/p\"}; wc = {c = \"<root>/q\"}}"
    );
    // A `lazy` it awaits while it runs reads its caller as it would.
    assert!(one_run(&all, (9, 10)));
    assert!(!one_run(&all, (9, 11)));
    // Reading the caller: one run for two files of one directory, whose
    // callers are equal, and another for another directory.
    assert!(one_run(&all, (0, 1)));
    assert!(!one_run(&all, (0, 2)));
    // Not reading it: one run from every directory.
    assert!(one_run(&all, (3, 4)));
    assert!(one_run(&all, (3, 5)));
    // A helper whose callee reads the caller it passed on depends on it.
    assert!(one_run(&all, (6, 7)));
    assert!(!one_run(&all, (6, 8)));
}

#[test]
fn a_memoised_call_found_passes_its_dependencies_on() {
    // In `p`, `outer_hit(1)` finds `reads(1)`, which read `p`, as `first`
    // ran it: `outer_hit`'s entry depends on the caller through the hit,
    // so `q` runs it again rather than finding `p`'s.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        (
            "p/a.gcsl",
            "import lib/m\nfirst = m.reads(1)\nhit = m.outer_hit(len([first]))",
        ),
        ("q/c.gcsl", "import lib/m\nhit = m.outer_hit(1)"),
        (
            "z/all.gcsl",
            "import p/a\nimport q/c\nfirst = a.first\nhit = a.hit\nother = c.hit",
        ),
    ];
    let all = rooted_value(&files, "z/all.gcsl");
    assert_eq!(
        all.to_string().replace(SCRATCH, "<root>"),
        "{first = {c = \"<root>/p\"}; hit = {c = \"<root>/p\"}; other = {c = \"<root>/q\"}}"
    );
    // `p`'s `outer_hit` found the entry `first` made.
    assert!(one_run(&all, (0, 1)));
    assert!(!one_run(&all, (1, 2)));
}

#[test]
fn a_step_that_can_read_its_caller_is_a_dependency_of_its_maker() {
    // Each step is awaited after the memoised call that made it has
    // returned, in a third directory.
    let made = "import lib/m\nst = m.step(1)\nps = m.plain_step(1)\n\
                    hs = m.helped(1)\nns = m.nested(1)\nfs = m.fn_step(1)";
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        ("p/a.gcsl", made),
        ("q/c.gcsl", made),
        (
            "z/all.gcsl",
            "import p/a\nimport q/c\n\
                 sa = await a.st.s\nsc = await c.st.s\n\
                 pa = a.ps\npc = c.ps\n\
                 ha = await a.hs.s\nhc = await c.hs.s\n\
                 na = await a.ns.s\nnc = await c.ns.s\n\
                 fa = a.fs\nfc = c.fs",
        ),
    ];
    let all = rooted_value(&files, "z/all.gcsl");
    let Value::Record(record) = &all else {
        panic!("expected a record, got {all}");
    };
    let shown: Vec<String> = record
        .values
        .iter()
        .map(|value| value.to_string().replace(SCRATCH, "<root>"))
        .collect();
    // A step reading its caller: one run of its maker for each
    // directory, each step reading its own, as the maker's caller.
    assert_eq!(shown[0], "<root>/p");
    assert_eq!(shown[1], "<root>/q");
    // A step that cannot read it: one run from every directory.
    assert!(one_run(&all, (2, 3)));
    // Made after `fileroot.helper()`, its calls pass the caller on.
    assert_eq!(shown[4], "<root>/p");
    assert_eq!(shown[5], "<root>/q");
    // A step making a step that reads it.
    assert_eq!(shown[6], "<root>/p");
    assert_eq!(shown[7], "<root>/q");
    // A function a step makes reads its own caller, not the step's: one
    // run from every directory.
    assert!(one_run(&all, (8, 9)));
}

#[test]
fn a_running_memoised_call_is_found_as_a_lookup_finds_it() {
    // `again(1)` from `p` has read `p` when it calls `again(1)` from
    // `lib`: the running entry depends on `p`, so the call from `lib` is
    // not the call depending on its own result, and runs.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        ("p/a.gcsl", "import lib/m\nr = m.again(1)"),
    ];
    assert_eq!(
        rooted_text(&files, "p/a.gcsl"),
        "{r = \"<root>/lib <root>/p\"}"
    );
}

/// Every diagnostic of checking the workspace of `files`, in order,
/// each shown as `MESSAGE at FILE (LABEL); FILE: LABEL; note: NOTE`,
/// its file and each label's by path.
fn check_errors(files: &[(&str, &str)]) -> Vec<String> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], "/");
    let Err(errors) = checked else {
        panic!("{files:?}: checked without an error");
    };
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    all.extend(paths);
    let path = |input: Option<u32>| {
        input
            .and_then(|i| all.get(i as usize))
            .copied()
            .unwrap_or("?")
    };
    errors
        .iter()
        .map(|error| {
            let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
            if let Some(label) = &error.label {
                parts[0] = format!("{} ({label})", parts[0]);
            }
            for label in &error.labels {
                let file = path(label.input.or(error.input));
                parts.push(format!("{file}: {}", label.text));
            }
            for note in &error.notes {
                parts.push(format!("note: {note}"));
            }
            parts.join("; ")
        })
        .collect()
}

/// The files of the slice 33 summary: named record types, one built
/// from another by spread, an annotated field, a qualified name and a
/// file below reading a type it imports.
const TYPES: [(&str, &str); 3] = [
    (
        "_lib/types.gcsl",
        "type PackageDef = {\n\
             \x20   version: String,\n\
             \x20   arch: String,\n\
             \x20   content: String,\n\
             }\n\
             type Package = { ...PackageDef, name: String }",
    ),
    (
        "workspace.gcsl",
        "import _lib/types as t\n\
             type Steps = List<fn() -> String>\n\
             def make(name: String, f: fn(): t.PackageDef): { pkg: t.Package } {\n\
             \x20   pkg: t.Package = { ...f(); name = name }\n\
             }\n\
             built = make(\"hello\", fn(): t.PackageDef { version = \"1\"; arch = \"x\"; content = \"usr\" })",
    ),
    (
        "pkg/app.gcsl",
        "import Package from _lib/types\n\
             p: Package = { version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\" }\n\
             steps: Steps = []\n\
             n = built.pkg.name",
    ),
];

#[test]
fn the_types_summary_runs() {
    assert_eq!(
        value(&TYPES, "pkg/app.gcsl"),
        "{p = {version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\"}; \
             steps = []; n = \"hello\"}"
    );
    // A type is shown as what it stands for, never by its name.
    assert_eq!(
        value(&TYPES, "workspace.gcsl"),
        "{make = fn(name: String, f: fn(): {arch: String, content: String, version: \
             String}): {pkg: {arch: String, content: String, name: String, version: String}}; \
             built = {pkg = {arch = \"x\"; content = \"usr\"; version = \"1\"; name = \"hello\"}}}"
    );
}

#[test]
fn jacks_workspace_checks_with_spread_for_intersection() {
    // `/home/jack/projects/peios/newpkgs/workspace.gcsl`, with `&`
    // written as a spread and `#[def_type]` removed, and `ppk.commit`
    // stubbed by a module of the test's own.
    let files = [
        (
            "workspace.gcsl",
            "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
                 import _lib/ppk\n\
                 import fileroot\n\
                 \n\
                 type PackageDef = {\n\
                 \x20 version: String,\n\
                 \x20 arch: String,\n\
                 \x20 content: String,\n\
                 }\n\
                 \n\
                 type Package = { ...PackageDef, name: String }\n\
                 \n\
                 \n\
                 def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
                 \x20   fileroot.helper()\n\
                 \x20   pkg: Package = { ...func(); name = name }\n\
                 \n\
                 \x20   manifest = ppk.commit(\n\
                 \x20       name = pkg.name,\n\
                 \x20       version = pkg.version,\n\
                 \x20       arch = pkg.arch,\n\
                 \x20       content = pkg.content,\n\
                 \x20   )\n\
                 }\n",
        ),
        (
            "_lib/ppk.gcsl",
            "def commit(name: String, version: String, arch: String, content: String) -> String {\n\
                 \x20   \"{name} {version} {arch} {content}\"\n\
                 }",
        ),
        (
            "dev.peios.testpkg/test.gcsl",
            "built = package(\"dev.peios.testpkg\", fn(): PackageDef {\n\
                 \x20   version = \"0.1.0\"\n\
                 \x20   arch = \"x86_64\"\n\
                 \x20   content = \"usr\"\n\
                 })",
        ),
    ];
    checked(&files);
    assert_eq!(
        value(&files, "dev.peios.testpkg/test.gcsl"),
        "{built = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}"
    );
}

/// The files of the slice 34 summary as it is written: `hello` in
/// `a.gcsl` reads `name` of `b.gcsl`, which reads `hello`.
const DOTTED_DEFS: [(&str, &str); 3] = [
    ("workspace.gcsl", ""),
    (
        "pkgs/a.gcsl",
        "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.count(n: Int): Int {\n\
             \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
             }",
    ),
    (
        "pkgs/b.gcsl",
        "def dev.peios.name(): String { \"world\" }\n\
             dev.peios.version = \"1\"\n\
             greeting = dev.peios.hello()     # \"hello world\"\n\
             all = dev.peios                  # record of hello, count, name, version",
    ),
];

#[test]
fn the_dotted_defs_summary_is_two_files_that_need_each_other() {
    // As written, the two files need each other, as they do with every
    // `def` named by one name: a file runs as a whole before a file
    // that needs it.
    let (message, input, labels) = error(&DOTTED_DEFS);
    assert_eq!(message, "`pkgs/a.gcsl` and `pkgs/b.gcsl` need each other");
    assert_eq!((input, labels), (Some(1), vec![Some(2)]));
    let plain = [
        ("workspace.gcsl", ""),
        ("pkgs/a.gcsl", "def hello(): String { \"hello {name()}\" }"),
        (
            "pkgs/b.gcsl",
            "def name(): String { \"world\" }\ngreeting = hello()",
        ),
    ];
    assert_eq!(error(&plain).0, message);
    // With `name` beside `hello`, it runs: each file to its own value,
    // and `all` the record of the directory's leaves under the prefix,
    // the functions among them.
    let files = [
        ("workspace.gcsl", ""),
        (
            "pkgs/a.gcsl",
            "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
                 def dev.peios.count(n: Int): Int {\n\
                 \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
                 }\n\
                 def dev.peios.name(): String { \"world\" }",
        ),
        (
            "pkgs/b.gcsl",
            "dev.peios.version = \"1\"\n\
                 greeting = dev.peios.hello()\n\
                 all = dev.peios\n\
                 counted = dev.peios.count(3)",
        ),
    ];
    assert_eq!(
        value(&files, "pkgs/b.gcsl"),
        "{dev = {peios = {version = \"1\"}}; greeting = \"hello world\"; \
             all = {hello = fn(): String; count = fn(n: Int): Int; name = fn(): String; \
             version = \"1\"}; counted = 3}"
    );
    assert_eq!(
        value(&files, "pkgs/a.gcsl"),
        "{dev = {peios = {hello = fn(): String; count = fn(n: Int): Int; \
             name = fn(): String}}}"
    );
}

#[test]
fn dotted_defs_across_the_files_of_a_directory() {
    // Called by path from another file of the directory, and from a
    // directory below.
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def dev.odd(n: Int): Bool { if n == 0 { false } else { dev.even(n - 1) } }\n\
                 def dev.even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
        ),
        ("b.gcsl", "x = dev.even(10)"),
        ("sub/c.gcsl", "y = dev.odd(3)\nall = dev"),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{x = true}");
    assert_eq!(
        value(&files, "sub/c.gcsl"),
        "{y = true; all = {odd = fn(n: Int): Bool; even = fn(n: Int): Bool}}"
    );
    // Two in two files that call each other need each other, as two
    // `def`s of one name do.
    let mutual = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def dev.even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
        ),
        (
            "b.gcsl",
            "def dev.odd(n: Int): Bool { if n == 0 { false } else { dev.even(n - 1) } }",
        ),
    ];
    let plain = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }",
        ),
        (
            "b.gcsl",
            "def odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }",
        ),
    ];
    assert_eq!(error(&mutual), error(&plain));
    assert_eq!(error(&mutual).0, "`a.gcsl` and `b.gcsl` need each other");
    // A leaf conflict across files, as any leaf's.
    let conflict = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "def dev.x(): Int { 1 }"),
        ("b.gcsl", "dev.x = 2"),
    ];
    assert_eq!(error(&conflict).0, "field `dev.x` is defined in two files");
    // Shadowable across the files of a directory, in path order.
    let shadowed = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "#[shadowable] def dev.x(): Int { 1 }"),
        ("b.gcsl", "def dev.x(): Int { 2 }\ny = dev.x()"),
        ("c.gcsl", "z = dev.x()"),
    ];
    assert_eq!(value(&shadowed, "c.gcsl"), "{z = 2}");
    // A call from another file is of a known callee, as a `def` of one
    // name is: the call-site form of the implicit-await error.
    let known = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "def dev.inc(n: Int): Int { n + 1 }\ndef inc(n: Int): Int { n + 1 }",
        ),
        (
            "b.gcsl",
            "fn p(f: Future<Int>): Int { dev.inc(f) }\n\
                 fn q(f: Future<Int>): Int { inc(f) }",
        ),
    ];
    assert_eq!(
        messages(&known, "b.gcsl"),
        [
            "argument `f` would be awaited inside `dev.inc`, which is pure",
            "argument `f` would be awaited inside `inc`, which is pure",
        ]
    );
}

#[test]
fn dotted_defs_through_each_form_of_import() {
    let lib = (
        "pkgs/a.gcsl",
        "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.name(): String { \"world\" }",
    );
    for (text, expected) in [
        (
            "import dev from pkgs/a\nh = dev.peios.hello()",
            "{h = \"hello world\"}",
        ),
        (
            "import dev as d from pkgs/a\nh = d.peios.name()",
            "{h = \"world\"}",
        ),
        (
            "import * from pkgs/a\ndef dev.own(): String { dev.peios.name() }\n\
                 h = dev.peios.hello()\no = dev.own()\nall = dev",
            "{dev = {own = fn(): String}; h = \"hello world\"; o = \"world\"; \
                 all = {own = fn(): String; peios = {hello = fn(): String; \
                 name = fn(): String}}}",
        ),
        (
            "import pkgs/a\nh = a.dev.peios.hello()\nw = a",
            "{h = \"hello world\"; w = {dev = {peios = {hello = fn(): String; \
                 name = fn(): String}}}}",
        ),
        ("import pkgs\nh = pkgs.dev.peios.name()", "{h = \"world\"}"),
    ] {
        let files = [("workspace.gcsl", ""), lib, ("app/main.gcsl", text)];
        assert_eq!(value(&files, "app/main.gcsl"), expected, "{text}");
    }
    // Through an import, a call is of a known callee.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/x.gcsl", "def dev.inc(n: Int): Int { n + 1 }"),
        (
            "app/main.gcsl",
            "import dev from lib/x\nimport lib\n\
                 fn p(f: Future<Int>): Int { dev.inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.dev.inc(f) }",
        ),
    ];
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        [
            "argument `f` would be awaited inside `dev.inc`, which is pure",
            "argument `f` would be awaited inside `lib.dev.inc`, which is pure",
        ]
    );
}

#[test]
fn a_view_read_whole_has_its_dotted_defs_resolved() {
    // `dev` merges the root's, the import's and the file's own leaves:
    // read whole above the `def`, it holds the type its body gives it,
    // in either order of the file's items.
    let resolved = "{base: Int, lib: Int, own: {get: fn(n: Int | Future<Int>) -> Int}}";
    for text in [
        "import * from lib\nwhole = dev\ndef dev.own.get(n: Int) -> Int { n + 1 }\n\
             got = whole.own.get(1)",
        "import * from lib\ndef dev.own.get(n: Int) -> Int { n + 1 }\nwhole = dev\n\
             got = whole.own.get(1)",
    ] {
        let files = [
            ("workspace.gcsl", "dev.base = 1"),
            ("lib/l.gcsl", "dev.lib = 2"),
            ("app/main.gcsl", text),
        ];
        let mut workspace = checked(&files);
        let index = workspace.file("app/main.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(
            dump.contains(&format!("(name dev : {resolved}")),
            "{text}\n{dump}"
        );
        assert!(
            value(&files, "app/main.gcsl").ends_with("got = 2}"),
            "{text}"
        );
    }
    // And one that cannot be had there yet is the error at the read.
    let files = [
        ("workspace.gcsl", "dev.base = 1"),
        ("lib/l.gcsl", "dev.lib = 2"),
        (
            "app/main.gcsl",
            "import * from lib\nwhole = dev\nlet k = 1\ndef dev.own(): Int { k }",
        ),
    ];
    assert_eq!(
        messages(&files, "app/main.gcsl"),
        ["`dev.own` cannot be used here yet"]
    );
}

#[test]
fn jacks_package_is_a_dotted_def_given_to_package() {
    // `/home/jack/projects/peios/newpkgs`, with `&` written as a spread,
    // `#[def_type]` removed and `ppk.commit` stubbed as in the slice 33
    // test, and the package written as a `def` named by its dotted path
    // passed to `package`.
    let files = [
        (
            "workspace.gcsl",
            "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
                 import _lib/ppk\n\
                 import fileroot\n\
                 \n\
                 type PackageDef = {\n\
                 \x20 version: String,\n\
                 \x20 arch: String,\n\
                 \x20 content: String,\n\
                 }\n\
                 \n\
                 type Package = { ...PackageDef, name: String }\n\
                 \n\
                 \n\
                 def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
                 \x20   fileroot.helper()\n\
                 \x20   pkg: Package = { ...func(); name = name }\n\
                 \n\
                 \x20   manifest = ppk.commit(\n\
                 \x20       name = pkg.name,\n\
                 \x20       version = pkg.version,\n\
                 \x20       arch = pkg.arch,\n\
                 \x20       content = pkg.content,\n\
                 \x20   )\n\
                 }\n",
        ),
        (
            "_lib/ppk.gcsl",
            "def commit(name: String, version: String, arch: String, content: String) -> String {\n\
                 \x20   \"{name} {version} {arch} {content}\"\n\
                 }",
        ),
        (
            "dev.peios.testpkg/test.gcsl",
            "def dev.peios.testpkg(): PackageDef {\n\
                 \x20   version = \"0.1.0\"\n\
                 \x20   arch = \"x86_64\"\n\
                 \x20   content = \"usr\"\n\
                 }\n\
                 \n\
                 result = package(\"dev.peios.testpkg\", dev.peios.testpkg)",
        ),
    ];
    checked(&files);
    assert_eq!(
        value(&files, "dev.peios.testpkg/test.gcsl"),
        "{dev = {peios = {testpkg = fn(): {arch: String, content: String, \
             version: String}}}; result = {pkg = {arch = \"x86_64\"; content = \"usr\"; \
             version = \"0.1.0\"; name = \"dev.peios.testpkg\"}; \
             manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}"
    );
}

#[test]
fn a_type_is_used_above_its_declaration_and_across_files() {
    // In its own file above its line, from another file of its
    // directory, and from a directory below, through the chain.
    let root = "a: Point = { x = 1; y = 2 }\ntype Point = {x: Int, y: Int}";
    let files = [
        ("workspace.gcsl", root),
        ("other.gcsl", "b: Point = { x = 3; y = 4 }"),
        ("sub/deep.gcsl", "c: Point = { x = 5; y = 6 }"),
    ];
    assert_eq!(value(&files, "workspace.gcsl"), "{a = {x = 1; y = 2}}");
    assert_eq!(value(&files, "other.gcsl"), "{b = {x = 3; y = 4}}");
    assert_eq!(value(&files, "sub/deep.gcsl"), "{c = {x = 5; y = 6}}");
    // A sibling directory does not see them.
    let hidden = [
        ("workspace.gcsl", ""),
        ("a/types.gcsl", "type Point = {x: Int}"),
        ("b/use.gcsl", "p: Point = { x = 1 }"),
    ];
    assert_eq!(messages(&hidden, "b/use.gcsl"), ["unknown type `Point`"]);
}

/// A library of types for the import tests: `lib/types.gcsl` with a
/// record type and a type also a value's name, and `lib/more.gcsl`.
const TYPE_LIB: [(&str, &str); 3] = [
    ("workspace.gcsl", ""),
    (
        "lib/types.gcsl",
        "type P = {x: Int}\ntype Both = Int\nBoth = 7",
    ),
    ("lib/more.gcsl", "type Q = {...P, y: String}"),
];

#[test]
fn types_are_reached_through_every_import_form() {
    let with = |main: &str| {
        let mut files = TYPE_LIB.to_vec();
        files.push(("app/main.gcsl", main));
        value(&files, "app/main.gcsl")
    };
    // Whole, by its path's last segment and `as`.
    assert_eq!(
        with("import lib/types\nv: types.P = { x = 1 }"),
        "{v = {x = 1}}"
    );
    assert_eq!(
        with("import lib/types as t\nv: t.P = { x = 1 }"),
        "{v = {x = 1}}"
    );
    // A directory's merged tree has its files' types.
    assert_eq!(
        with("import lib\nv: lib.Q = { x = 1; y = \"a\" }"),
        "{v = {x = 1; y = \"a\"}}"
    );
    // A member, bare or renamed.
    assert_eq!(
        with("import P from lib/types\nv: P = { x = 2 }"),
        "{v = {x = 2}}"
    );
    assert_eq!(
        with("import P as R from lib/types\nv: R = { x = 2 }"),
        "{v = {x = 2}}"
    );
    // `*`, with the fields.
    assert_eq!(
        with("import * from lib/types\nv: P = { x = Both }"),
        "{v = {x = 7}}"
    );
    // A member that is both a type and a value binds both.
    assert_eq!(
        with("import Both from lib/types\nv: Both = Both + 1"),
        "{v = 8}"
    );
}

#[test]
fn qualified_names_go_two_deep() {
    // A deep module path, and a type whose definition names another
    // module's type qualified, used qualified from a third.
    let files = [
        ("workspace.gcsl", ""),
        ("a/b/inner.gcsl", "type Leaf = {v: Int}"),
        (
            "a/b/types.gcsl",
            "import a/b/inner as i\ntype Node = {leaf: i.Leaf, name: String}",
        ),
        (
            "app/main.gcsl",
            "import a/b/types as t\nn: t.Node = { leaf = { v = 1 }; name = \"n\" }\nw = n.leaf.v",
        ),
    ];
    assert_eq!(
        value(&files, "app/main.gcsl"),
        "{n = {leaf = {v = 1}; name = \"n\"}; w = 1}"
    );
}

#[test]
fn a_type_and_a_value_of_one_name_coexist() {
    let files = [(
        "workspace.gcsl",
        "type Foo = {a: Int}\nFoo = 3\nlet x: Foo = { a = Foo }\nfn bar(Foo: Foo): Int { Foo.a }\ny = bar(x)",
    )];
    assert_eq!(value(&files, "workspace.gcsl"), "{Foo = 3; y = 3}");
    // `type` is still a name, as it was, for a variable and a field.
    let files = [(
        "workspace.gcsl",
        "type = 1\nlet t = type\nx = { type = t }.type\ny: Int = type",
    )];
    assert_eq!(value(&files, "workspace.gcsl"), "{type = 1; x = 1; y = 1}");
}

#[test]
fn spread_in_a_record_type_overrides_as_value_spread_does() {
    let run = |types: &str, value_text: &str| {
        let prefix = "type A = {a: Int, b: Int}\ntype B = {b: String}";
        let text = format!("{prefix}\n{types}\n{value_text}");
        value(&[("workspace.gcsl", &text)], "workspace.gcsl")
    };
    // A later field overrides a spread's, its type differing.
    assert_eq!(
        run("type T = {...A, b: String}", "v: T = { a = 1; b = \"x\" }"),
        "{v = {a = 1; b = \"x\"}}"
    );
    // A later spread overrides an earlier one's.
    assert_eq!(
        run("type T = {...A, ...B}", "v: T = { a = 1; b = \"y\" }"),
        "{v = {a = 1; b = \"y\"}}"
    );
    assert_eq!(
        run("type T = {...B, ...A}", "v: T = { a = 1; b = 2 }"),
        "{v = {a = 1; b = 2}}"
    );
    // The same spread twice is idempotent.
    assert_eq!(
        run("type T = {...A, ...A}", "v: T = { a = 1; b = 2 }"),
        "{v = {a = 1; b = 2}}"
    );
    // A spread of a type built by a spread.
    assert_eq!(
        run(
            "type T = {...A, c: Bool}\ntype U = {...T, d: Int}",
            "v: U = { a = 1; b = 2; c = true; d = 4 }"
        ),
        "{v = {a = 1; b = 2; c = true; d = 4}}"
    );
    // A record type written in place spread.
    assert_eq!(
        run(
            "type T = {...{z: Int}, ...A}",
            "v: T = { z = 0; a = 1; b = 2 }"
        ),
        "{v = {z = 0; a = 1; b = 2}}"
    );
}

#[test]
fn spread_problems_in_record_types() {
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    let twice = "field `b` is defined twice at workspace.gcsl (`b` is a field of this, of \
                     type Int); workspace.gcsl: first defined here; note: a spread cannot \
                     replace a field defined before it; put the spread first";
    let text = "type A = {b: Int}\ntype T = {b: String, ...A}";
    assert_eq!(one(text), [twice]);
    let text = "type A = {b: Int, c: Int}\nx: {b: String, c: Bool, ...A} = { b = \"x\" }";
    let others = format!("{twice} (it also has 1 other field defined before it)");
    assert_eq!(one(text), [others]);
    assert_eq!(
        one("type T = {...Int, a: Int}"),
        ["cannot spread type Int: it is not a record type at workspace.gcsl"]
    );
    assert_eq!(
        one("type N = List<Int>\ntype T = {...N}"),
        ["cannot spread type `List<Int>`: it is not a record type at workspace.gcsl"]
    );
    // Only a record type, written so or named, is spread: the type is
    // shown as what it stands for.
    assert_eq!(
        one("x: {...Future<{a: Int}>} = { a = 1 }"),
        ["cannot spread type `Future<{a: Int}>`: it is not a record type at workspace.gcsl"]
    );
    let union = "cannot spread type `{a: Int} | {b: Int}`: it is not a record type at \
                     workspace.gcsl";
    assert_eq!(
        one("type A = {a: Int}\ntype B = {b: Int}\nx: {...A | B} = { a = 1 }"),
        [union]
    );
    // A record type written in place with a name twice, spread, is the
    // repeated-name error it is anywhere else, at the same place, in a
    // `type` and in a field's annotation alike.
    let repeated = |text: &str| {
        let source = SourceFile::new("workspace.gcsl", text).unwrap();
        let errors = check(std::slice::from_ref(&source)).unwrap_err();
        assert_eq!(errors.len(), 1, "{text}: {errors:?}");
        let at = u32::try_from(text.rfind("a: String").unwrap()).unwrap();
        assert_eq!(errors[0].span, Span::new(at, at + 1), "{text}");
        errors[0].message.clone()
    };
    let twice = "field `a` appears twice in this type";
    for text in [
        "type R = { a: Int, a: String }",
        "type R = { ...{ a: Int, a: String } }",
        "x: { a: Int, a: String } = { a = 1 }",
        "x: { ...{ a: Int, a: String } } = { a = 1 }",
        "type B = {b: Int}\nx: { ...B, ...{ a: Int, a: String } } = { a = 1; b = 2 }",
    ] {
        assert_eq!(repeated(text), twice, "{text}");
    }
    // And a type so defined is erroneous wherever it is used, reported
    // once, where it is written.
    assert_eq!(
        check_errors(&[
            ("workspace.gcsl", "type R = { ...{ a: Int, a: String } }"),
            ("use.gcsl", "x: R = { a = 1 }"),
        ]),
        [format!("{twice} at workspace.gcsl")]
    );
    // An erroneous type spread is reported where it is written, once.
    assert_eq!(
        one("type A = {a: Strng}\ntype T = {...A}\nx: T = { a = 1 }"),
        ["unknown type `Strng` at workspace.gcsl"]
    );
}

#[test]
fn a_spread_of_a_record_type_carries_its_rest_field() {
    let run = |text: &str| value(&[("workspace.gcsl", text)], "workspace.gcsl");
    // A spread's rest-field joins the record type, softly, as its fields
    // do: a later one replaces it.
    assert_eq!(
        run("type M = {*: Int}\ntype T = {...M, to: String}\nv: T = { to = \"a\"; x = 1 }"),
        "{v = {to = \"a\"; x = 1}}"
    );
    assert_eq!(
        run("type M = {*: Int}\ntype T = {...M, *: String}\nv: T = { x = \"y\" }"),
        "{v = {x = \"y\"}}"
    );
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    // One written before the spread is not replaced.
    assert_eq!(
        one("type M = {*: Int}\ntype T = {*: String, ...M}"),
        [
            "the rest-field is defined twice at workspace.gcsl (this has a rest-field, of \
             type Int); workspace.gcsl: first defined here; note: a spread cannot replace a \
             field defined before it; put the spread first"
        ]
    );
    // Two in a record type written in place and spread.
    assert_eq!(
        one("type T = {...{*: Int, *: String}}"),
        ["the rest-field appears twice in this type at workspace.gcsl"]
    );
}

#[test]
fn a_def_type_s_definitions_can_return_a_record_of_any_fields() {
    // pekit's packages map paths to paths: `files` is a map, which each
    // definition writes as a record literal.
    let types = "type Package = {build: String, files: {*: String}}\n\
                 #[def_type]\n\
                 def package(name: String, func: fn(): Package) -> {*: String} {\n\
                 \x20   func().files\n\
                 }\n";
    let busybox = "def package net.busybox.busybox() {\n\
                   \x20   build = \"busybox\"\n\
                   \x20   files = {\n\
                   \x20       \"bin/*\" = \"bin/*\"\n\
                   \x20       \"sbin/*\" = \"sbin/*\"\n\
                   \x20   }\n\
                   }\n\
                   files = package net.busybox.busybox()\n";
    let files = [
        ("workspace.gcsl", types),
        ("net.busybox.busybox/pekit.gcsl", busybox),
    ];
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let checked = check(&sources).unwrap();
    assert_eq!(checked.warnings(), []);
    assert_eq!(
        value(&files, "net.busybox.busybox/pekit.gcsl"),
        "{files = {\"bin/*\" = \"bin/*\"; \"sbin/*\" = \"sbin/*\"}}"
    );
}

#[test]
fn type_declaration_errors() {
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    assert_eq!(
        one("type Int = String"),
        ["`Int` is a built-in type at workspace.gcsl"]
    );
    assert_eq!(
        one("type List = {a: Int}\ntype Future = Int"),
        [
            "`List` is a built-in type at workspace.gcsl",
            "`Future` is a built-in type at workspace.gcsl"
        ]
    );
    assert_eq!(
        one("type A = Int\nx = 1\ntype A = String"),
        ["type `A` is declared twice at workspace.gcsl; workspace.gcsl: first declared here"]
    );
    assert_eq!(
        one("type A = {a: Nope}\nx: A = { a = 1 }\ny: A = { a = 2 }"),
        ["unknown type `Nope` at workspace.gcsl"]
    );
    assert_eq!(
        one("type A = Int\nx: A<Int> = 1"),
        ["type `A` takes no type argument at workspace.gcsl"]
    );
    let mismatch = "mismatched types at workspace.gcsl (expected Int, found String); \
                        workspace.gcsl: expected because of this annotation";
    assert_eq!(one("type A = Int\nx: A = \"s\""), [mismatch]);
}

#[test]
fn an_erroneous_type_is_reported_once_whichever_file_is_checked_first() {
    // `a.gcsl` is checked before `z.gcsl`, whose type it uses; and
    // `n.gcsl`, which needs `z.gcsl`, after it.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x: Bad = { b = 1 }"),
        ("n.gcsl", "y = zed + 1"),
        ("z.gcsl", "type Bad = {b: Strng}\nzed = 1"),
    ];
    assert_eq!(check_errors(&files), ["unknown type `Strng` at z.gcsl"]);
}

#[test]
fn type_conflicts_are_the_conflicts_of_fields() {
    let errors = |files: &[(&str, &str)]| check_errors(files);
    // Two files of one directory.
    let two = "type `Foo` is declared in two files at b.gcsl (declared here); \
                   a.gcsl: declared here";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", ""),
            ("a.gcsl", "type Foo = Int"),
            ("b.gcsl", "type Foo = String"),
        ]),
        [two]
    );
    // An inner directory's and an outer one's.
    let above = "type `Foo` is declared in two files at workspace.gcsl (declared here); \
                     sub/a.gcsl: declared here; note: one is in a directory above";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", "type Foo = Int"),
            ("sub/a.gcsl", "type Foo = String"),
        ]),
        [above]
    );
    // An imported one and an own one.
    let imported = "type `Foo` is declared in two files at app/m.gcsl (imported here); \
                        lib/t.gcsl: declared here; app/m.gcsl: declared here";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", ""),
            ("lib/t.gcsl", "type Foo = Int"),
            ("app/m.gcsl", "import Foo from lib/t\ntype Foo = String"),
        ]),
        [imported]
    );
    // A `#[shadowable]` import brings types as plain ones, which an own
    // type of their name conflicts with.
    let star = "type `Foo` is declared in two files at app/m.gcsl (imported here); \
                    lib/t.gcsl: declared here; app/m.gcsl: declared here";
    assert_eq!(
        errors(&[
            ("workspace.gcsl", ""),
            ("lib/t.gcsl", "type Foo = Int"),
            (
                "app/m.gcsl",
                "#[shadowable] import * from lib/t\ntype Foo = String"
            ),
        ]),
        [star]
    );
    // One type reached twice is one type.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/t.gcsl", "type Foo = Int"),
        (
            "lib/m.gcsl",
            "import * from lib\nimport Foo from lib/t\nx: Foo = 1",
        ),
    ];
    assert_eq!(value(&files, "lib/m.gcsl"), "{x = 1}");
}

#[test]
fn qualified_names_that_name_no_type() {
    let with = |main: &str| {
        let mut files = TYPE_LIB.to_vec();
        files.push(("app/main.gcsl", main));
        check_errors(&files)
    };
    assert_eq!(
        with("import lib/types as t\nx: t.Nope = 1"),
        ["`t` has no type `Nope` at app/main.gcsl; note: `t` has the types `P`, `Both`"]
    );
    assert_eq!(
        with("import lib/more as m\nimport builtins\nx: builtins.Int = 1"),
        ["`builtins` has no type `Int` at app/main.gcsl; note: `builtins` has no types"]
    );
    assert_eq!(
        with("import lib/types as t\nx: t = 1"),
        ["`t` is a module, not a type at app/main.gcsl"]
    );
    assert_eq!(
        with("import lib/types as t\nx: t.P.x = 1"),
        ["`t.P` is a type, not a module at app/main.gcsl"]
    );
    assert_eq!(
        with("x: nothing.P = 1"),
        ["unknown type `nothing.P` at app/main.gcsl"]
    );
}

#[test]
fn cycles_of_types() {
    let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    assert_eq!(
        one("type A = {a: A}"),
        ["type `A` refers to itself at workspace.gcsl; workspace.gcsl: `A` uses `A` here"]
    );
    assert_eq!(
        one("type A = B\ntype B = {a: A}"),
        ["type `A` refers to itself at workspace.gcsl; \
              workspace.gcsl: `A` uses `B` here; workspace.gcsl: `B` uses `A` here"]
    );
    // Across files, at the first in path then source order.
    let three = "type `A` refers to itself at a.gcsl; a.gcsl: `A` uses `B` here; \
                     b.gcsl: `B` uses `C` here; x.gcsl: `C` uses `A` here";
    assert_eq!(
        check_errors(&[
            ("workspace.gcsl", ""),
            ("x.gcsl", "type C = {c: A}"),
            ("a.gcsl", "type A = List<B>"),
            ("b.gcsl", "type B = {...C}"),
        ]),
        [three]
    );
}

#[test]
fn annotated_fields_have_their_annotations_types() {
    // Top-level, in an inner block, dotted and quoted, read from another
    // file, whose readers see the annotation's type, not the value's.
    let files = [
        ("workspace.gcsl", ""),
        (
            "a.gcsl",
            "x: Int | String = 1\n\
                 conf.port: Int | String = 80\n\
                 \"a b\": Float = 2\n\
                 inner = { y: Int | String = \"s\"; z = y }",
        ),
        (
            "b.gcsl",
            "import a as m\n\
                 kind = if x is Int { \"int\" } else { \"string\" }\n\
                 port = conf.port\n\
                 f = m.\"a b\" + 0.5",
        ),
    ];
    assert_eq!(
        value(&files, "a.gcsl"),
        "{x = 1; conf = {port = 80}; \"a b\" = 2.0; inner = {y = \"s\"; z = \"s\"}}"
    );
    assert_eq!(
        value(&files, "b.gcsl"),
        "{kind = \"int\"; port = 80; f = 2.5}"
    );
    let wide = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x: Int | String = 1"),
        ("b.gcsl", "y = x + 1"),
    ];
    let add = "cannot add `Int | String` and Int at b.gcsl; b.gcsl: Int | String; \
                   b.gcsl: Int; note: tell them apart first, as in `if x is Int { ... }`";
    assert_eq!(check_errors(&wide), [add]);
}

#[test]
fn annotated_fields_check_as_annotated_lets_do() {
    let one = |text: &str| value(&[("workspace.gcsl", text)], "workspace.gcsl");
    let errors = |text: &str| check_errors(&[("workspace.gcsl", text)]);
    // A value that fits once awaited is awaited, a literal adapts, and a
    // pure function's parameter the annotation needs present is
    // measured, present.
    assert_eq!(
        one("x: Int = lazy 1\ny: Float = 2\nf = fn(n: Int): {v: Int} { v: Int = n }"),
        "{x = 1; y = 2.0; f = fn(n: Int): {v: Int}}"
    );
    // In a record block checked against an expected record type.
    assert_eq!(
        one("let r: {a: Int | String} = { a: Int = 1 }\nv = r.a"),
        "{v = 1}"
    );
    let mismatch = "mismatched types at workspace.gcsl (expected Int, found Int | String); \
                        workspace.gcsl: expected because of this annotation";
    assert_eq!(
        errors("let r: {a: Int} = { a: Int | String = 1 }"),
        [mismatch]
    );
    // TEMPORARY (alias rule): an annotated field is no alias of a
    // built-in function, as an annotated `let` is none.
    let value_yet = "`len` is a built-in function and cannot be used as a value yet at \
                         workspace.gcsl; note: a built-in function with a type for any list \
                         cannot be written as a type yet; call it, or bind it to a name";
    assert_eq!(errors("l: fn(xs: List<Int>): Int = len"), [value_yet]);
}

#[test]
fn an_alias_is_bare_where_it_is_used() {
    // As if `Int` were written at each use: the either shape in an
    // impure function's bare parameter, inferred in a pure one's.
    let files = [(
        "workspace.gcsl",
        "type N = Int\n\
             type F = Future<Int>\n\
             def impure(x: N) -> N { x }\n\
             def pure(x: N): Int { x }\n\
             def either(x: Int | F): Int { 1 }",
    )];
    assert_eq!(
        value(&files, "workspace.gcsl"),
        "{impure = fn(x: Int | Future<Int>) -> Int | Future<Int>; \
              pure = fn(x: Int): Int; either = fn(x: Int | Future<Int>): Int}"
    );
    assert_eq!(
        check_errors(&[(
            "workspace.gcsl",
            "type F = Future<Int>\nx: Int | F | String = 1"
        )]),
        ["a union with a `Future` member must be `T | Future<T>` at workspace.gcsl"]
    );
}

#[test]
fn a_type_expanded_too_deep_is_an_error_where_it_is_written() {
    // Each type nests the one before it a level deeper, so the chain
    // passes `MAX_DEPTH` though no type is written deeply: `T10000` is
    // one level too deep where it is written, and `T10001`, which uses
    // it, is erroneous wherever it is used.
    let chain: Vec<String> = (1..=10_001)
        .map(|at| format!("type T{at} = {{a: T{}}}", at - 1))
        .collect();
    let text = format!("type T0 = Int\n{}\nx: T10001 = 1", chain.join("\n"));
    let errors = std::thread::Builder::new()
        .stack_size(crate::STACK_SIZE)
        .spawn(move || check_errors(&[("workspace.gcsl", &text)]))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(errors, ["expression nested too deeply at workspace.gcsl"]);
}

#[test]
fn a_value_file_declares_types() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/v.gcsl", "type T = {a: Int}\n= 5"),
        (
            "app.gcsl",
            "import lib/v\nimport T from lib/v\nx: v.T = { a = v }\ny: T = x",
        ),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{x = {a = 5}; y = {a = 5}}");
}

#[test]
fn dump_types_shows_each_type_with_its_expansion() {
    let mut workspace = checked(&TYPES);
    let index = workspace.file("_lib/types.gcsl").unwrap();
    assert_eq!(
        workspace.dump_types(index).unwrap(),
        "(file\n\
             \x20 (type PackageDef {version: String, arch: String, content: String} = \
             {arch: String, content: String, version: String} @0..81)\n\
             \x20 (type Package {...PackageDef, name: String} = {arch: String, content: String, \
             name: String, version: String} @82..128))"
    );
    // A use prints as written.
    let index = workspace.file("pkg/app.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(field p : Package @"), "{dump}");
    let index = workspace.file("workspace.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(f fn(): t.PackageDef)"), "{dump}");
    assert!(
        dump.contains("(type Steps List<fn() -> String> = List<fn() -> String | Future<String>> @"),
        "{dump}"
    );
    // A type defined as another prints as written, and dumps as what
    // both stand for.
    let mut workspace = checked(&[("workspace.gcsl", "type A = Int\ntype B = A")]);
    let index = workspace.file("workspace.gcsl").unwrap();
    assert_eq!(
        workspace.dump_types(index).unwrap(),
        "(file\n  (type A Int = Int @0..12)\n  (type B A = Int @13..23))"
    );
}

/// Jack's `/home/jack/projects/peios/newpkgs` as it is written, with
/// `&` written as a spread and the commas of the package's block as
/// newlines, the bottom line `result = package dev.peios.testpkg()`,
/// and `ppk.commit` stubbed by a module of the test's own, which joins
/// the content to its caller's directory as the real one does.
const JACKS_PACKAGE: [(&str, &str); 3] = [
    (
        "workspace.gcsl",
        "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
             import _lib/ppk\n\
             import fileroot\n\
             \n\
             type PackageDef = {\n\
             \x20 version: String,\n\
             \x20 arch: String,\n\
             \x20 content: String,\n\
             }\n\
             \n\
             type Package = { ...PackageDef, name: String }\n\
             \n\
             \n\
             #[def_type]\n\
             def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
             \x20   fileroot.helper()\n\
             \x20   pkg: Package = { ...func(); name = name }\n\
             \n\
             \x20   manifest = ppk.commit(\n\
             \x20       name = pkg.name,\n\
             \x20       version = pkg.version,\n\
             \x20       arch = pkg.arch,\n\
             \x20       content = pkg.content,\n\
             \x20   )\n\
             }\n",
    ),
    (
        "_lib/ppk.gcsl",
        "import path\n\
             import fileroot\n\
             def commit(name: String, version: String, arch: String, content: String) -> String {\n\
             \x20   let content = path.join(fileroot.caller, content)\n\
             \x20   \"{name} {version} {arch} {content}\"\n\
             }",
    ),
    (
        "dev.peios.testpkg/test.gcsl",
        "def package dev.peios.testpkg() {\n\
             \x20   version = \"0.1.0\"\n\
             \x20   arch = \"x86_64\"\n\
             \x20   content = \"usr\"\n\
             }\n\
             \n\
             \n\
             result = package dev.peios.testpkg()",
    ),
];

#[test]
fn jacks_package_is_defined_and_called_with_its_def_type() {
    // The stub is given the package's directory as the content's base,
    // through the def-type's `fileroot.helper()`; the definition binds
    // nothing of the file's record.
    assert_eq!(
        rooted_text(&JACKS_PACKAGE, "dev.peios.testpkg/test.gcsl"),
        "{result = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
             <root>/dev.peios.testpkg/usr\"}}"
    );
}

/// A def-type of the root of a workspace, for the def-type tests:
/// `build`, whose definitions take a target and give a string, and
/// which takes a mode when it is invoked.
const BUILD: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\n\
         def build(name: String, f: fn(target: String): String, mode: String): String {\n\
         \x20   \"{name}: {f(mode)}\"\n\
         }",
);

#[test]
fn a_definition_takes_its_types_and_its_invocation_its_arguments() {
    // A definition with a parameter, used above it; invocation
    // parameters; a reference stored and called later; `def x()` beside
    // `def build x()`, a bare `x` seeing only the former.
    let files = [
        BUILD,
        (
            "app.gcsl",
            "y = build x(\"release\")\n\
                 def build x(target) { \"built {target}\" }\n\
                 def x(): Int { 7 }\n\
                 let later = build x\n\
                 z = later(mode = \"debug\")\n\
                 plain = x()",
        ),
    ];
    assert_eq!(
        value(&files, "app.gcsl"),
        "{y = \"x: built release\"; x = fn(): Int; z = \"x: built debug\"; plain = 7}"
    );
    // From another file of the directory and from a directory below.
    let files = [
        BUILD,
        ("a.gcsl", "def build lib.core(t) { \"core-{t}\" }"),
        ("b.gcsl", "b = build lib.core(\"b\")"),
        ("sub/c.gcsl", "c = build lib.core(\"c\")"),
    ];
    assert_eq!(value(&files, "b.gcsl"), "{b = \"lib.core: core-b\"}");
    assert_eq!(value(&files, "sub/c.gcsl"), "{c = \"lib.core: core-c\"}");
    assert_eq!(value(&files, "a.gcsl"), "{}");
}

#[test]
fn a_definition_is_a_leaf_under_its_namespace_s_key() {
    // The scan names the namespace, makes the definition a top-level
    // function and a leaf under its key, and a reference above it
    // mentions it, as a read of its path.
    let files = [
        BUILD,
        ("app.gcsl", "y = build x(\"m\")\ndef build x(t) { t }"),
    ];
    let scan = ws(&files).unwrap();
    let app = scan.files.iter().find(|f| f.path == "app.gcsl").unwrap();
    let workspace = scan
        .files
        .iter()
        .position(|f| f.path == "workspace.gcsl")
        .unwrap();
    let tree = &app.mentions.tree;
    // The roots: `y`, then the namespace, whose child `x` is the leaf.
    let root = &tree.nodes[tree.roots[1] as usize];
    // The `def`'s item, the second the parser made, after its body's.
    assert_eq!(scan.names.get(root.name), format!("<build#{workspace}.1>"));
    assert_eq!(root.children, [2]);
    assert_eq!(tree.path(2).len(), 2);
    assert_eq!(app.mentions.functions.len(), 1);
    assert_eq!(app.mentions.functions[0].name, None);
    assert_eq!(app.mentions.items[0], [0]);
    assert_eq!(app.mentions.fields[0], [2]);
    // The definition's item mentions its def-type, another file's `def`:
    // its node and its function.
    assert_eq!(app.mentions.foreign[1].len(), 2);
}

#[test]
fn each_def_type_has_a_namespace_of_its_own() {
    // Two def-types, each with a definition of one path, and a field of
    // that path too: none meets another.
    let files = [
        BUILD,
        (
            "app.gcsl",
            "#[def_type]\n\
                 def test(name: String, f: fn(target: String): String): String { f(name) }\n\
                 def build lib.core(t) { \"built {t}\" }\n\
                 def test lib.core(t) { \"tested {t}\" }\n\
                 lib.core = 1\n\
                 a = build lib.core(\"r\")\n\
                 b = test lib.core()",
        ),
    ];
    assert_eq!(
        value(&files, "app.gcsl"),
        "{test = fn(name: String, f: fn(target: String): String): String; \
             lib = {core = 1}; a = \"lib.core: built r\"; b = \"tested lib.core\"}"
    );
}

#[test]
fn definitions_conflict_and_shadow_as_dotted_defs_do() {
    let conflict = [
        BUILD,
        ("a.gcsl", "def build x(t) { t }"),
        ("b.gcsl", "def build x(t) { t }"),
    ];
    assert_eq!(
        error(&conflict).0,
        "field `build x` is defined in two files"
    );
    let under = [
        BUILD,
        ("a.gcsl", "def build x(t) { t }"),
        ("b.gcsl", "def build x.y(t) { t }"),
    ];
    assert_eq!(error(&under).0, "field `build x` is already defined");
    // In one file, as the dotted fields of a block.
    let files = [
        BUILD,
        (
            "a.gcsl",
            "def build x(t) { t }\ndef build x(t) { t }\ny = build x(\"m\")",
        ),
    ];
    assert_eq!(
        messages(&files, "a.gcsl"),
        ["field `build x` is defined twice"]
    );
    // Shadowable, across files in path order and within one file.
    let shadowed = [
        BUILD,
        ("a.gcsl", "#[shadowable] def build x(t) { \"a {t}\" }"),
        (
            "b.gcsl",
            "#[shadowable] def build x(t) { \"b {t}\" }\ndef build x(t) { \"c {t}\" }",
        ),
        ("d.gcsl", "y = build x(\"m\")"),
    ];
    assert_eq!(value(&shadowed, "d.gcsl"), "{y = \"x: c m\"}");
    let reversed = [
        BUILD,
        ("a.gcsl", "def build x(t) { t }"),
        ("b.gcsl", "#[shadowable] def build x(t) { t }"),
    ];
    assert_eq!(
        error(&reversed).0,
        "field `build x` is defined in two files"
    );
}

#[test]
fn a_def_type_is_reached_through_its_imports() {
    let lib = (
        "lib/types.gcsl",
        "#[def_type]\n\
             def build(name: String, f: fn(): String): String { \"{name} {f()}\" }",
    );
    // Through a renamed member, defining and invoking; through a whole
    // import, qualified; and a definition through one alias is in the
    // def-type's namespace whatever another file calls it.
    let files = [
        ("workspace.gcsl", ""),
        lib,
        (
            "app/a.gcsl",
            "import build as b from lib/types\n\
                 def b x() { \"via b\" }\n\
                 one = b x()",
        ),
        (
            "app/c.gcsl",
            "import lib/types as t\n\
                 two = t.build x()",
        ),
    ];
    assert_eq!(value(&files, "app/a.gcsl"), "{one = \"x via b\"}");
    assert_eq!(value(&files, "app/c.gcsl"), "{two = \"x via b\"}");
    // `*` brings a file's definitions; a whole import of it does not.
    let defs = (
        "lib/defs.gcsl",
        "import * from lib/types\ndef build x() { \"x\" }",
    );
    let files = [
        ("workspace.gcsl", ""),
        lib,
        defs,
        (
            "app/a.gcsl",
            "import * from lib/defs\nimport * from lib/types\na = build x()",
        ),
        ("app/c.gcsl", "import lib/defs\nc = defs"),
    ];
    assert_eq!(value(&files, "app/a.gcsl"), "{a = \"x x\"}");
    // Read whole, a file holds no namespace.
    assert_eq!(value(&files, "app/c.gcsl"), "{c = {}}");
    let files = [
        ("workspace.gcsl", ""),
        lib,
        defs,
        (
            "app/b.gcsl",
            "import lib/defs\nimport * from lib/types\nb = build x()",
        ),
    ];
    assert_eq!(
        messages(&files, "app/b.gcsl"),
        ["`build` has no definition `x`"]
    );
}

#[test]
fn a_def_type_s_caller_is_where_its_reference_is_called() {
    // The reference is made in `a/`, called in `b/`: the def-type runs
    // for `b/`, and the definition for the def-type's file, but through
    // its helper.
    let files = [
        (
            "workspace.gcsl",
            "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn() -> String) -> { called: String, defined: String } {\n\
                 \x20   called = fileroot.caller\n\
                 \x20   defined = f()\n\
                 }\n\
                 #[def_type]\n\
                 def helped(name: String, f: fn() -> String) -> String {\n\
                 \x20   fileroot.helper()\n\
                 \x20   f()\n\
                 }",
        ),
        (
            "a/a.gcsl",
            "import fileroot\n\
                 def where x() { fileroot.caller }\n\
                 def helped x() { fileroot.caller }\n\
                 def made(): fn() -> { called: String, defined: String } { where x }\n\
                 def made_helped(): fn() -> String { helped x }",
        ),
        (
            "b/b.gcsl",
            "import a/a\n\
                 there = a.made()()\n\
                 through = a.made_helped()()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "b/b.gcsl"),
        "{there = {called = \"<root>/b\"; defined = \"<root>\"}; through = \"<root>/b\"}"
    );
}

#[test]
fn a_pure_definition_cannot_call_an_impure_function() {
    let files = [
        BUILD,
        (
            "app.gcsl",
            "fn noisy() -> String { \"n\" }\n\
                 def build x(t) { noisy() }",
        ),
    ];
    assert_eq!(
        messages(&files, "app.gcsl"),
        ["cannot call an impure function from a pure function"]
    );
    // And an impure def-type's reference from a pure function.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn(): String) -> String { f() }",
        ),
        (
            "app.gcsl",
            "def run x() { \"x\" }\nfn p(): String { run x() }",
        ),
    ];
    assert_eq!(
        messages(&files, "app.gcsl"),
        ["cannot call an impure function from a pure function"]
    );
}

#[test]
fn def_type_errors() {
    // Each error with its labels and notes, joined by `; `, the errors
    // by newlines.
    let errors = |text: &str| {
        let files = [BUILD, ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    let signature = "a def-type takes the name as a `String` and then the definition as a \
                         function, as in `def bad(name: String, func: fn(): T)` at app.gcsl; \
                     note: or, for definitions made with a variant, the name, the variant and \
                     the definition, as in `def Toolchain<T>(name: String, kind: \
                     Variant<Toolchain, T>, func: fn(): T)`";
    assert_eq!(
        errors("#[def_type]\ndef bad(name: Int, f: fn(): Int): Int { 1 }"),
        signature
    );
    assert_eq!(errors("#[def_type]\ndef bad(): Int { 1 }"), signature);
    let not_one = "note: a def-type is a `def` marked `#[def_type]`";
    assert_eq!(
        errors("def plain(): Int { 1 }\ndef plain x() { 1 }"),
        format!("`plain` is not a def-type at app.gcsl; {not_one}")
    );
    assert_eq!(
        errors("def build x() { \"x\" }"),
        "`build` defines functions of 1 parameter, `target` at app.gcsl"
    );
    assert_eq!(
        errors(
            "#[def_type]\ndef two(name: String, f: fn(a: Int, b: Int): Int): Int { 1 }\n\
                 def two x(a) { a }"
        ),
        "`two` defines functions of 2 parameters, `a` and `b` at app.gcsl"
    );
    assert_eq!(
        errors(
            "#[def_type]\ndef none(name: String, f: fn(): Int): Int { 1 }\n\
                 def none x(a) { a }"
        ),
        "`none` defines functions of no parameters at app.gcsl"
    );
    assert_eq!(
        errors("def build a.b(t) { t }\nx = build a.c\ny = build a\nz = build q"),
        "`build` has no definition `a.c` at app.gcsl; \
             note: the only definition of `build` here is `a.b`\n\
             `a` is not a `build` definition at app.gcsl\n\
             `build` has no definition `q` at app.gcsl; \
             note: the only definition of `build` here is `a.b`"
    );
    assert_eq!(
        errors("def build a(t) { t }\ndef build b.c(t) { t }\nx = build d"),
        "`build` has no definition `d` at app.gcsl; \
             note: the definitions of `build` here are `a` and `b.c`"
    );
    assert_eq!(
        errors("x = build d"),
        "`build` has no definition `d` at app.gcsl; note: `build` has no definitions here"
    );
    // Two names in a row, the first no def-type, or undefined.
    let juxtaposed = "note: two names in a row are read as a def-type reference, as in \
                          `package dev.x()`; is an operator or a line break missing?";
    assert_eq!(
        errors("let a = 1\nx = a b"),
        format!("`a` is not a def-type at app.gcsl; {not_one}; {juxtaposed}")
    );
    assert!(
        errors("x = nope b").starts_with("undefined variable `nope` at app.gcsl")
            && errors("x = nope b").ends_with(juxtaposed),
        "{}",
        errors("x = nope b")
    );
    // A parameter of the def-type's name is no def-type.
    assert_eq!(
        errors("def build x(t) { t }\nfn f(build: Int): Int { build x() }"),
        format!("`build` is not a def-type at app.gcsl; {not_one}; {juxtaposed}")
    );
}

#[test]
fn no_source_can_write_a_namespace_s_key() {
    // A field of the key's form is no field at all, so it cannot reach
    // a namespace's definitions.
    let files = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }\n\"<build#1.0>\" = 1"),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "a field cannot be named `\"<build#1.0>\"`: names of that form are def-types' \
                 namespaces at app.gcsl"
        ]
    );
    let files = [BUILD, ("app.gcsl", "x = {}.\"<build#1.0>\"")];
    assert_eq!(
        check_errors(&files),
        [
            "a field cannot be named `\"<build#1.0>\"`: names of that form are def-types' \
                 namespaces at app.gcsl"
        ]
    );
}

#[test]
fn dump_ast_shows_definitions_and_references() {
    let mut workspace = checked(&[
        BUILD,
        ("app.gcsl", "def build x(t) { t }\ny = build x(\"m\")"),
    ]);
    let index = workspace.file("app.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(dump.contains("(def-type-def build x @0..20\n"), "{dump}");
    assert!(dump.contains("(params t)\n"), "{dump}");
    assert!(
        dump.contains("(def-type-ref build x : fn(mode: String): String @25..32)"),
        "{dump}"
    );
}

/// Jack's `/home/jack/projects/peios/newpkgs` as it is written, but for
/// `&` written as a spread and the commas of the package's block as
/// newlines, as in [`JACKS_PACKAGE`], with no line invoking the package:
/// `ppk.commit` stubbed by a module of the test's own, which returns its
/// arguments, the content joined to its caller's directory as the real
/// one joins it.
const NEWPKGS: [(&str, &str); 3] = [
    JACKS_PACKAGE[0],
    (
        "_lib/ppk.gcsl",
        "import path\n\
             import fileroot\n\
             def commit(name: String, version: String, arch: String, content: String) -> String {\n\
             \x20   let content = path.join(fileroot.caller, content)\n\
             \x20   \"{name} {version} {arch} {content}\"\n\
             }",
    ),
    (
        "dev.peios.testpkg/test.gcsl",
        "def package dev.peios.testpkg() {\n\
             \x20   version = \"0.1.0\"\n\
             \x20   arch = \"x86_64\"\n\
             \x20   content = \"usr\"\n\
             }\n",
    ),
];

/// Checks the workspace of `files`, located at [`SCRATCH`], and invokes
/// the definition `path` of the def-type `def_type` in it, with the
/// arguments `args`, each a parameter's name and the text of its value,
/// read by [`Workspace::arguments`].
fn invoke_with(
    files: &[(&str, &str)],
    (def_type, path): (&str, &str),
    args: &[(&str, &str)],
) -> Result<Value, InvokeError> {
    let mut workspace = rooted(files).unwrap_or_else(|errors| panic!("{errors:?}"));
    let texts: Vec<(&str, Option<&str>)> = args
        .iter()
        .map(|&(name, text)| (name, Some(text)))
        .collect();
    let args = workspace.arguments(def_type, path, &texts)?;
    workspace
        .invoke(def_type, path, &args)
        .map(|outcome| outcome.value)
}

/// The argument named `name` of the string `text`, as
/// [`Workspace::invoke`] takes it.
fn string(name: &str, text: &str) -> (String, Given) {
    (name.to_owned(), Given::Str(text.to_owned()))
}

/// [`invoke_with`]'s value with no arguments shown, [`SCRATCH`] shown as
/// `<root>`.
fn invoked_text(files: &[(&str, &str)], def_type: &str, path: &str) -> String {
    invoked_text_with(files, (def_type, path), &[])
}

/// [`invoke_with`]'s value shown, [`SCRATCH`] shown as `<root>`.
fn invoked_text_with(
    files: &[(&str, &str)],
    (def_type, path): (&str, &str),
    args: &[(&str, &str)],
) -> String {
    match invoke_with(files, (def_type, path), args) {
        Ok(value) => value.to_string().replace(SCRATCH, "<root>"),
        Err(error) => panic!("{def_type} {path} {args:?}: {error:?}"),
    }
}

/// The request error of invoking `path` of `def_type` in the workspace
/// of `files`, with no arguments: its message and its notes, joined by
/// `; `.
fn request_error(files: &[(&str, &str)], def_type: &str, path: &str) -> String {
    request_error_with(files, (def_type, path), &[])
}

/// [`request_error`] with the arguments `args`, each a parameter's name
/// and the text of its value.
fn request_error_with(
    files: &[(&str, &str)],
    (def_type, path): (&str, &str),
    args: &[(&str, &str)],
) -> String {
    match invoke_with(files, (def_type, path), args) {
        Err(InvokeError::Request(error)) => {
            let mut parts = vec![error.message()];
            parts.extend(
                error
                    .notes()
                    .into_iter()
                    .map(|note| format!("note: {note}")),
            );
            assert_eq!(error.to_string(), error.message());
            parts.join("; ")
        }
        other => panic!("{def_type} {path}: {other:?}"),
    }
}

/// The diagnostics invoking `path` of `def_type` in the workspace of
/// `files` fails with, each shown as [`check_errors`] shows them.
fn invoke_errors(files: &[(&str, &str)], def_type: &str, path: &str) -> Vec<String> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], SCRATCH);
    let errors = match checked.unwrap().invoke(def_type, path, &[]) {
        Err(InvokeError::Diagnostics(errors)) => errors,
        other => panic!("{def_type} {path}: {other:?}"),
    };
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    all.extend(paths);
    shown_errors(&errors, &all)
}

/// `errors`, each shown with its labels and notes, joined by `; `, its
/// input and each label's by its path in `all`, by input number.
fn shown_errors(errors: &[Diagnostic], all: &[&str]) -> Vec<String> {
    let path = |input: Option<u32>| {
        input
            .and_then(|i| all.get(i as usize))
            .copied()
            .unwrap_or("?")
    };
    errors
        .iter()
        .map(|error| {
            let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
            if let Some(label) = &error.label {
                parts[0] = format!("{} ({label})", parts[0]);
            }
            for label in &error.labels {
                let at = path(label.input.or(error.input));
                parts.push(format!("{at}: {}", label.text));
            }
            parts.extend(error.notes.iter().map(|note| format!("note: {note}")));
            parts.join("; ")
        })
        .collect()
}

#[test]
fn jacks_package_is_invoked_by_name() {
    // The record the def-type returns; the stub given the package's
    // directory as the content's base, through `fileroot.helper()`.
    assert_eq!(
        invoked_text(&NEWPKGS, "package", "dev.peios.testpkg"),
        "{pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
             <root>/dev.peios.testpkg/usr\"}"
    );
    // Listed: the def-type, its file and the span of its name, and the
    // definition, its file and the span of its path.
    let workspace = rooted(&NEWPKGS).unwrap();
    let listed = workspace.def_types().unwrap();
    let root = workspace.file("workspace.gcsl").unwrap();
    let package = workspace.file("dev.peios.testpkg/test.gcsl").unwrap();
    let name = count(NEWPKGS[0].1.find("def package(").unwrap() + 4);
    assert_eq!(
        listed,
        [DefType {
            path: "package".to_owned(),
            file: root,
            span: Span::new(name, name + 7),
            parameters: Vec::new(),
            optional: Vec::new(),
            definitions: vec![Definition {
                path: "dev.peios.testpkg".to_owned(),
                dir: "dev.peios.testpkg".to_owned(),
                file: package,
                span: Span::new(12, 29),
            }],
        }]
    );
}

/// The paths of the def-types `workspace` lists, each with the paths
/// and files of its definitions: `package: a.one in a/one.gcsl`.
fn listed(workspace: &Workspace, files: &[(&str, &str)]) -> Vec<String> {
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    let standard = workspace.scan.standard as usize;
    let path = |file: u32| paths[file as usize - standard];
    workspace
        .def_types()
        .unwrap()
        .iter()
        .map(|def_type| {
            let definitions: Vec<String> = def_type
                .definitions
                .iter()
                .map(|definition| format!("{} in {}", definition.path, path(definition.file)))
                .collect();
            format!(
                "{} in {}: {}",
                def_type.path,
                path(def_type.file),
                definitions.join(", ")
            )
        })
        .collect()
}

#[test]
fn every_definition_is_listed_under_the_def_type_it_names() {
    // Definitions in two sibling directories, which the root sees
    // neither of; one through an alias of its def-type, listed by its
    // path as written; a def-type with none; two def-types of the root
    // with definitions; and a dotted def-type.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def build(name: String, f: fn(): String): String { \"built {f()}\" }\n\
                 #[def_type]\n\
                 def test(name: String, f: fn(): String): String { \"tested {f()}\" }",
        ),
        (
            "lib/types.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String) -> String { \"{name}: {f()}\" }\n\
                 #[def_type]\n\
                 def unused(name: String, f: fn(): Int): Int { f() }\n\
                 #[def_type]\n\
                 def kinds.tool(name: String, f: fn(): String): String { f() }",
        ),
        (
            "a/one.gcsl",
            "import * from lib/types\n\
                 def package a.one() { \"one\" }\n\
                 def build a.one() { \"a\" }\n\
                 def kinds.tool hammer() { \"hit\" }",
        ),
        (
            "b/two.gcsl",
            "import package as pkg from lib/types\n\
                 def pkg b.two() { \"two\" }\n\
                 def test b.two() { \"b\" }",
        ),
    ];
    let workspace = rooted(&files).unwrap();
    assert_eq!(
        listed(&workspace, &files),
        [
            "package in lib/types.gcsl: a.one in a/one.gcsl, b.two in b/two.gcsl",
            "unused in lib/types.gcsl: ",
            "kinds.tool in lib/types.gcsl: hammer in a/one.gcsl",
            "build in workspace.gcsl: a.one in a/one.gcsl",
            "test in workspace.gcsl: b.two in b/two.gcsl",
        ]
    );
    // The dotted def-type's name is its whole path.
    let tool = &workspace.def_types().unwrap()[2];
    let text = files[1].1;
    let at = count(text.find("kinds.tool").unwrap());
    assert_eq!(tool.span, Span::new(at, at + 10));
    // Each is invoked by its def-type's path and its own.
    assert_eq!(invoked_text(&files, "package", "a.one"), "a.one: one");
    assert_eq!(invoked_text(&files, "package", "b.two"), "b.two: two");
    assert_eq!(invoked_text(&files, "build", "a.one"), "built a");
    assert_eq!(invoked_text(&files, "test", "b.two"), "tested b");
    assert_eq!(invoked_text(&files, "kinds.tool", "hammer"), "hit");
}

/// The paths of the files of the workspace of `files`, by input number,
/// the standard files `standard` first.
fn all_paths<'a>(standard: &'a [SourceFile], files: &[(&'a str, &str)]) -> Vec<&'a str> {
    let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
    paths.sort_unstable();
    let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
    all.extend(paths);
    all
}

/// The errors [`Workspace::def_types`] fails with in the workspace of
/// `files`, each shown as [`shown_errors`] shows it.
fn listing_errors(files: &[(&str, &str)]) -> Vec<String> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], SCRATCH);
    let errors = checked.unwrap().def_types().unwrap_err();
    shown_errors(&errors, &all_paths(&standard, files))
}

/// A def-type of the root, for the tests of duplicates.
const PACKAGE: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\ndef package(name: String, f: fn(): String): String { \"{name} {f()}\" }",
);

/// The note on a duplicate definition.
const DEFINED: &str = "note: a definition invoked by name must be the only one of its path in \
                       its directory, the directories above it and those below it";

/// The note on a definition under another.
const UNDER: &str = "note: a definition invoked by name must not lie under another in its \
                     directory, the directories above it or those below it";

#[test]
fn one_path_in_directories_apart_is_named_with_its_directory() {
    // In two sibling directories, which see neither the other: no error.
    // Another definition, unique, is invoked by its path alone.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package dup() { \"a\" }"),
        ("b/y.gcsl", "def package dup() { \"b\" }"),
        (
            "c/z.gcsl",
            "def package other() { \"c\" }\nz = package other()",
        ),
    ];
    assert_eq!(invoked_text(&siblings, "package", "other"), "other c");
    // The shared path alone names neither; each is named with its
    // directory.
    assert_eq!(
        request_error(&siblings, "package", "dup"),
        "`package dup` is defined in 2 directories, so it is named with its directory; \
         note: name one of `a/dup`, `b/dup`"
    );
    assert_eq!(invoked_text(&siblings, "package", "a/dup"), "dup a");
    assert_eq!(invoked_text(&siblings, "package", "b/dup"), "dup b");
    // A directory without the definition names none, and the listing
    // names the shared one qualified.
    assert_eq!(
        request_error(&siblings, "package", "c/dup"),
        "`package` has no definition `c/dup`; note: its definitions are `a/dup`, `b/dup`, \
         `other`"
    );
    // The listing gives each its directory.
    let workspace = checked(&siblings);
    let listed = workspace.def_types().unwrap();
    let dirs: Vec<(&str, &str, String)> = listed[0]
        .definitions
        .iter()
        .map(|d| (d.path.as_str(), d.dir.as_str(), d.qualified()))
        .collect();
    assert_eq!(
        dirs,
        [
            ("dup", "a", "a/dup".to_owned()),
            ("dup", "b", "b/dup".to_owned()),
            ("other", "c", "c/other".to_owned()),
        ]
    );
    // One of the root is qualified as `/PATH`.
    let rooted = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 def package top() { \"root\" }",
        ),
        ("a/x.gcsl", "def package top2() { \"a\" }"),
    ];
    assert_eq!(invoked_text(&rooted, "package", "/top"), "top root");
    assert_eq!(invoked_text(&rooted, "package", "top"), "top root");
}

#[test]
fn one_path_in_directories_that_see_each_other_fails_every_invocation() {
    // A `#[shadowable]` one of the root, shadowed in a directory below:
    // both are found, the first in path order first.
    let shadowed = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package dup() { \"root\" }",
        ),
        (
            "sub/s.gcsl",
            "def package dup() { \"sub\" }\ny = package dup()",
        ),
    ];
    let twice = format!(
        "`package dup` is defined 2 times in directories that see each other at sub/s.gcsl; \
             workspace.gcsl: also defined here; {DEFINED}"
    );
    assert_eq!(invoke_errors(&shadowed, "package", "dup"), [twice.as_str()]);
    assert_eq!(listing_errors(&shadowed), [twice.as_str()]);
    // An ordinary run allows them, and runs.
    assert_eq!(rooted_text(&shadowed, "sub/s.gcsl"), "{y = \"dup sub\"}");
    // And one shadowed in its own file, which is in no tree.
    let own = [
        PACKAGE,
        (
            "a.gcsl",
            "#[shadowable] def package dup() { \"one\" }\n\
                 def package dup() { \"two\" }\n\
                 y = package dup()",
        ),
    ];
    assert_eq!(
        invoke_errors(&own, "package", "dup"),
        [format!(
            "`package dup` is defined 2 times in directories that see each other at a.gcsl; \
                 a.gcsl: also defined here; {DEFINED}"
        )]
    );
    assert_eq!(rooted_text(&own, "a.gcsl"), "{y = \"dup two\"}");
    // Two chains of one path: the root's with one below it, and a pair
    // apart from it in siblings, each chain one error.
    let chains = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[shadowable] def package dup() { \"root\" }",
        ),
        ("a/x.gcsl", "def package dup() { \"a\" }"),
        ("b/y.gcsl", "def package dup() { \"b\" }"),
    ];
    assert_eq!(
        listing_errors(&chains),
        [format!(
            "`package dup` is defined 3 times in directories that see each other at \
                 a/x.gcsl; b/y.gcsl: also defined here; workspace.gcsl: also defined here; \
                 {DEFINED}"
        )]
    );
}

#[test]
fn two_def_types_of_one_path_fail_every_invocation() {
    let declared = "note: a def-type invoked by name must be the only one of its name \
                        in the workspace";
    let def_type = "#[def_type]\n\
                        def package(name: String, f: fn(): String): String { f() }";
    // In two sibling directories.
    let siblings = [
        ("workspace.gcsl", ""),
        ("a/x.gcsl", def_type),
        ("b/y.gcsl", def_type),
        (
            "c/z.gcsl",
            "#[def_type]\ndef other(name: String, f: fn(): Int): Int { f() }",
        ),
        ("c/w.gcsl", "def other one() { 1 }\nw = other one()"),
    ];
    let twice = format!(
        "def-type `package` is declared 2 times in the workspace at a/x.gcsl; \
             b/y.gcsl: also declared here; {declared}"
    );
    assert_eq!(invoke_errors(&siblings, "other", "one"), [twice.as_str()]);
    assert_eq!(listing_errors(&siblings), [twice.as_str()]);
    assert_eq!(rooted_text(&siblings, "c/w.gcsl"), "{w = 1}");
    // A `#[shadowable]` one and the one that shadows it.
    let shadowed = [(
        "workspace.gcsl",
        "#[shadowable]\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { \"first\" }\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { f() }\n\
             def package x() { \"x\" }\n\
             y = package x()",
    )];
    assert_eq!(
        invoke_errors(&shadowed, "package", "x"),
        [format!(
            "def-type `package` is declared 2 times in the workspace at workspace.gcsl; \
                 workspace.gcsl: also declared here; {declared}"
        )]
    );
    assert_eq!(
        rooted_text(&shadowed, "workspace.gcsl"),
        "{package = fn(name: String | Future<String>, f: fn(): String): String; y = \"x\"}"
    );
}

#[test]
fn a_definition_under_another_fails_every_invocation() {
    // In two sibling directories, which see neither the other: no error.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package app() { \"a\" }"),
        (
            "b/y.gcsl",
            "def package app.core() { \"b\" }\ny = package app.core()",
        ),
        ("c/z.gcsl", "def package apple() { \"c\" }"),
    ];
    assert!(checked(&siblings).def_types().is_ok());
    assert_eq!(invoked_text(&siblings, "package", "app.core"), "app.core b");
    // Under one in a directory above it, which it shadows: invoking
    // another definition, `apple`, which lies under nothing, fails, and so
    // does the listing.
    let below = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package app() { \"root\" }",
        ),
        (
            "b/y.gcsl",
            "def package app.core() { \"b\" }\ny = package app.core()",
        ),
        ("c/z.gcsl", "def package apple() { \"c\" }"),
    ];
    let under = format!(
        "`package app.core` lies under another `package` definition at b/y.gcsl; \
             workspace.gcsl: `app` is defined here; {UNDER}"
    );
    assert_eq!(invoke_errors(&below, "package", "apple"), [under.as_str()]);
    assert_eq!(listing_errors(&below), [under.as_str()]);
    // An ordinary run allows them, and runs.
    assert_eq!(rooted_text(&below, "b/y.gcsl"), "{y = \"app.core b\"}");
    // In one directory, through `#[shadowable]`.
    let shadowed = [
        PACKAGE,
        (
            "a.gcsl",
            "#[shadowable] def package app.core() { \"core\" }\n\
                 def package app() { \"app\" }\n\
                 y = package app()",
        ),
    ];
    assert_eq!(
        listing_errors(&shadowed),
        [format!(
            "`package app.core` lies under another `package` definition at a.gcsl; \
                 a.gcsl: `app` is defined here; {UNDER}"
        )]
    );
    assert_eq!(rooted_text(&shadowed, "a.gcsl"), "{y = \"app app\"}");
    // Three levels down one chain of directories: `a.b` lies under one,
    // `a.b.c` under two, labelled in path then source order of their files.
    let levels = [
        PACKAGE,
        ("x/y/one.gcsl", "def package a.b.c() { \"c\" }"),
        ("x/two.gcsl", "#[shadowable] def package a.b() { \"b\" }"),
        ("three.gcsl", "#[shadowable] def package a() { \"a\" }"),
    ];
    assert_eq!(
        listing_errors(&levels),
        [
            format!(
                "`package a.b` lies under another `package` definition at x/two.gcsl; \
                     three.gcsl: `a` is defined here; {UNDER}"
            ),
            format!(
                "`package a.b.c` lies under another `package` definition at x/y/one.gcsl; \
                     three.gcsl: `a` is defined here; x/two.gcsl: `a.b` is defined here; \
                     {UNDER}"
            ),
        ]
    );
    // Definitions of other def-types do not overlap: `build a.b.c` lies
    // under no `package` definition.
    let other = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def build(name: String, f: fn(): String): String { f() }",
        ),
        ("x/y/one.gcsl", "def build a.b.c() { \"c\" }"),
        levels[2],
        levels[3],
    ];
    assert_eq!(
        listing_errors(&other),
        [format!(
            "`package a.b` lies under another `package` definition at x/two.gcsl; \
                 three.gcsl: `a` is defined here; {UNDER}"
        )]
    );
}

#[test]
fn a_definition_both_duplicated_and_under_another_has_both_errors() {
    // `app` twice in one directory chain, and `app.core` twice under it in
    // directories below them: each `app.core` lies under both `app`s.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[shadowable] def package app() { \"root\" }",
        ),
        ("a/x.gcsl", "#[shadowable] def package app() { \"a\" }"),
        ("a/b/y.gcsl", "def package app.core() { \"b\" }"),
        ("a/c/z.gcsl", "def package app.core() { \"c\" }"),
    ];
    let under = |at: &str| {
        format!(
            "`package app.core` lies under another `package` definition at {at}; \
                 a/x.gcsl: `app` is defined here; workspace.gcsl: `app` is defined here; \
                 {UNDER}"
        )
    };
    // In path order of their files: `a/b/y.gcsl` first.
    let expected = [
        under("a/b/y.gcsl"),
        under("a/c/z.gcsl"),
        format!(
            "`package app` is defined 2 times in directories that see each other at \
                 a/x.gcsl; workspace.gcsl: also defined here; {DEFINED}"
        ),
    ];
    assert_eq!(listing_errors(&files), expected);
    assert_eq!(invoke_errors(&files, "package", "app"), expected);
}

#[test]
fn what_cannot_be_invoked_is_a_request_error() {
    let files = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }"),
        (
            "lib/defs.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def empty(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def deploy(name: String, f: fn(): String, target: String, mode: String): String { f() }\n\
                 def package dev.peios.b() { \"b\" }\n\
                 def package dev.peios.a() { \"a\" }\n\
                 def deploy site() { \"s\" }",
        ),
    ];
    assert_eq!(
        request_error(&files, "pkg", "x"),
        "no def-type `pkg`; note: the def-types are `build`, `deploy`, `empty`, `package`"
    );
    assert_eq!(
        request_error(&[("workspace.gcsl", "x = 1")], "pkg", "x"),
        "no def-type `pkg`; note: this workspace declares no def-types"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios.nope"),
        "`package` has no definition `dev.peios.nope`; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    assert_eq!(
        request_error(&files, "empty", "x"),
        "`empty` has no definition `x`; note: it has no definitions"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios"),
        "`dev.peios` is not a `package` definition; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    assert_eq!(
        request_error(&files, "package", "dev"),
        "`dev` is not a `package` definition; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    // A prefix of a name is no path above it.
    assert_eq!(
        request_error(&files, "package", "dev.pei"),
        "`package` has no definition `dev.pei`; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
    );
    assert_eq!(
        request_error(&files, "build", "x"),
        "`build` needs `mode`, which was not given"
    );
    assert_eq!(
        request_error(&files, "deploy", "site"),
        "`deploy` needs `target` and `mode`, which were not given"
    );
    // Before parameters, the path.
    assert_eq!(
        request_error(&files, "build", "y"),
        "`build` has no definition `y`; note: its only definition is `x`"
    );
    // A path that is a definition's, and above another's, in a sibling
    // directory, which sees nothing of it: each is invoked.
    let under = "`package app.core` lies under another `package` definition";
    let note = UNDER;
    let files = [
        PACKAGE,
        ("a/x.gcsl", "def package app() { \"a\" }"),
        ("b/y.gcsl", "def package app.core() { \"b\" }"),
    ];
    assert_eq!(invoked_text(&files, "package", "app"), "app a");
    assert_eq!(invoked_text(&files, "package", "app.core"), "app.core b");
    // A `#[shadowable]` definition that one of another path, above it,
    // shadows in its own directory, which is in no tree: found, and the
    // one below.
    let files = [
        PACKAGE,
        (
            "a.gcsl",
            "#[shadowable] def package app.core() { \"core\" }\n\
                 def package app() { \"app\" }",
        ),
    ];
    let shadowed = format!("{under} at a.gcsl; a.gcsl: `app` is defined here; {note}");
    assert_eq!(
        invoke_errors(&files, "package", "app.core"),
        [shadowed.as_str()]
    );
    assert_eq!(invoke_errors(&files, "package", "app"), [shadowed.as_str()]);
}

#[test]
fn a_request_error_names_the_only_one_there_is() {
    let files = [
        PACKAGE,
        ("lib/defs.gcsl", "def package dev.peios.a() { \"a\" }"),
    ];
    assert_eq!(
        request_error(&files, "pkg", "x"),
        "no def-type `pkg`; note: the only def-type is `package`"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios.nope"),
        "`package` has no definition `dev.peios.nope`; \
             note: its only definition is `dev.peios.a`"
    );
    assert_eq!(
        request_error(&files, "package", "dev.peios"),
        "`dev.peios` is not a `package` definition; \
             note: its only definition is `dev.peios.a`"
    );
    // Two or more are listed, as `what_cannot_be_invoked_is_a_request_error`
    // shows with more.
    let files = [
        BUILD,
        ("lib/defs.gcsl", PACKAGE.1),
        (
            "lib/more.gcsl",
            "def package a() { \"a\" }\ndef package b() { \"b\" }",
        ),
    ];
    assert_eq!(
        request_error(&files, "pkg", "x"),
        "no def-type `pkg`; note: the def-types are `build`, `package`"
    );
    assert_eq!(
        request_error(&files, "package", "c"),
        "`package` has no definition `c`; note: its definitions are `a`, `b`"
    );
}

/// A def-type of one invocation parameter, [`BUILD`]'s `build`, one of
/// two, `deploy`, and one of none, `package`, with a definition of
/// each.
const ARGUMENTS: [(&str, &str); 3] = [
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
const OPTIONAL: [(&str, &str); 2] = [
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
const TYPED: [(&str, &str); 2] = [
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

/// The field of record `value` whose name is `name`.
fn field_named(value: &Value, name: &str) -> Value {
    let Value::Record(record) = value else {
        panic!("{value} is no record");
    };
    let at = record.layout.texts.iter().position(|text| &**text == name);
    record.values[at.unwrap()].clone()
}

/// [`Workspace::definition_of`] as a pair of the def-type's path and the
/// definition's.
fn named_by(workspace: &Workspace, value: &Value) -> Option<(String, String)> {
    workspace
        .definition_of(value)
        .map(|found| (found.def_type, found.path))
}

/// A `build` def-type whose meta names a `source`, and a `package` one
/// whose record holds a `build`, as pekit's are: in `lib.types`, which
/// `app` imports `build` from by another name.
const BUILDS: [(&str, &str); 3] = [
    ("workspace.gcsl", "x = 1"),
    (
        "lib/types.gcsl",
        "#[def_type]\n\
         def source(name: String, f: fn(): String): String { f() }\n\
         #[def_type]\n\
         def build(name: String, f: fn(out: String) ->> String META: {source: fn(): String}, \
         out: String) ->> String { f(out) }\n\
         #[def_type]\n\
         def package(name: String, f: fn(): {build: fn(out: String) ->> String}): \
         {build: fn(out: String) ->> String} { f() }",
    ),
    (
        "app/a.gcsl",
        "import source, package from lib/types\n\
         import build as make from lib/types\n\
         def source up() { \"https://x\" }\n\
         defmeta make b { source = source up }\n\
         def make b(out) { out }\n\
         def package dev.x() { build = make b }\n\
         n = 1\n\
         plain = fn(out: String): String { out }\n\
         calls = fn(out: String) -> String { make b(out) }\n\
         builtin = len",
    ),
];

#[test]
fn a_reference_names_its_definition() {
    let mut workspace = rooted(&BUILDS).unwrap();
    // The reference an invocation returns names its def-type as declared,
    // not by the alias it is written with.
    let package = workspace.invoke("package", "dev.x", &[]).unwrap().value;
    let build = field_named(&package, "build");
    let named = Some(("build".to_owned(), "b".to_owned()));
    assert_eq!(named_by(&workspace, &build), named);
    // So does one read from a meta, which names the definition to read.
    let source = workspace.meta_field("build", "b", "source").unwrap();
    let source = source.unwrap().value;
    let named = Some(("source".to_owned(), "up".to_owned()));
    assert_eq!(named_by(&workspace, &source), named);
    // What it names invokes as the reference does.
    let invoked = workspace.invoke("source", "up", &[]).unwrap().value;
    assert_eq!(invoked.to_string(), "https://x");
    // It knows its directory, and so the path qualified by it, which names
    // this very definition whatever others share its path.
    let referenced = workspace.definition_of(&source).unwrap();
    assert_eq!(referenced.dir, "app");
    assert_eq!(referenced.qualified(), "app/up");
    let invoked = workspace.invoke("source", &referenced.qualified(), &[]);
    assert_eq!(invoked.unwrap().value.to_string(), "https://x");
    let source = workspace.meta_field("build", "app/b", "source").unwrap();
    assert_eq!(named_by(&workspace, &source.unwrap().value), named);
    // Nothing else names a definition: no other value, no closure that
    // calls one, no built-in function.
    let index = workspace.file("app/a.gcsl").unwrap();
    let file = workspace.run(index).unwrap().value;
    for name in ["n", "plain", "calls", "builtin"] {
        let value = field_named(&file, name);
        assert_eq!(named_by(&workspace, &value), None, "{name}");
    }
}

#[test]
fn a_closure_that_captures_a_definition_names_none() {
    // The def-type's closure captures `n` and then the definition, as a
    // reference's captures the def-type and then the definition, but it is
    // no reference.
    let files = [(
        "workspace.gcsl",
        "#[def_type]\n\
         def wrap(name: String, f: fn(): Int, n: Int): fn(): Int { fn(): Int { n + f() } }\n\
         def wrap w() { 1 }",
    )];
    let mut workspace = rooted(&files).unwrap();
    let n = ("n".to_owned(), Given::Int(1));
    let wrapped = workspace.invoke("wrap", "w", &[n]).unwrap().value;
    let Value::Fn(closure) = &wrapped else {
        panic!("{wrapped} is no closure");
    };
    assert!(matches!(closure.captures.get(1), Some(Value::Fn(_))));
    assert_eq!(named_by(&workspace, &wrapped), None);
}

#[test]
fn a_reference_made_in_a_child_names_an_ancestor_s_definition() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
             def tool(name: String, f: fn(): Int): Int { f() }\n\
             def tool t() { 1 }",
        ),
        ("app/deep/a.gcsl", "r = tool t"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let index = workspace.file("app/deep/a.gcsl").unwrap();
    let file = workspace.run(index).unwrap().value;
    let named = Some(("tool".to_owned(), "t".to_owned()));
    assert_eq!(named_by(&workspace, &field_named(&file, "r")), named);
}

const TAGGED: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\n\
         def tagged(name: String, f: fn(): String META: {tag: String, n: Int}): String {\n\
         \x20   \"{name}: {plain(f)} {tag(f)} {f.n}\"\n\
         }\n\
         fn tag(g: fn(): String META: {tag: String}): String { g.tag }\n\
         fn plain(g: fn(): String): String { g() }",
);

#[test]
fn a_meta_is_a_leaf_of_its_directory_s_metas() {
    // The meta's leaf is its path as one name, under the meta key of its
    // def-type and directory; the definition and the meta are top-level
    // functions, and the definition's item mentions the meta's leaf and
    // function, by its read of its meta.
    let files = [
        TAGGED,
        (
            "app/a.gcsl",
            "def tagged dev.x() { \"x\" }\ndefmeta tagged dev.x { tag = \"t\"; n = 1 }",
        ),
    ];
    let scan = ws(&files).unwrap();
    let app = scan.files.iter().find(|f| f.path == "app/a.gcsl").unwrap();
    let tree = &app.mentions.tree;
    let meta_root = &tree.nodes[tree.roots[1] as usize];
    let key = scan.names.get(tree.nodes[tree.roots[0] as usize].name);
    assert_eq!(
        Some(scan.names.get(meta_root.name)),
        names::meta_key(key, app.dir).as_deref()
    );
    let leaf = &tree.nodes[meta_root.children[0] as usize];
    assert_eq!(scan.names.get(leaf.name), "dev.x");
    assert!(leaf.children.is_empty());
    assert_eq!(app.mentions.functions.len(), 2);
    assert_eq!(app.mentions.items[0], [1]);
    assert!(app.mentions.fields[0].contains(&meta_root.children[0]));
    assert!(app.unpaired.is_empty());
}

#[test]
fn a_meta_is_read_as_fields_of_its_definition() {
    // The meta above or below its definition, in its file or in another
    // of its directory, a block or an expression; read in pure functions,
    // one whose meta's type is less than the definition's and one that
    // forgets the meta.
    let files = [
        TAGGED,
        (
            "app/a.gcsl",
            "defmeta tagged x { tag = \"t-x\"; n = 1 }\n\
                 def tagged x() { \"x\" }\n\
                 def tagged y() { \"y\" }\n\
                 defmeta tagged y = { tag = \"t-y\"; n = 2 }",
        ),
        (
            "app/b.gcsl",
            "defmeta tagged dev.z { tag = \"t-z\"; n = 3 }",
        ),
        (
            "app/c.gcsl",
            "def tagged dev.z() { \"z\" }\n\
                 r = [tagged x(), tagged y(), tagged dev.z()]",
        ),
    ];
    assert_eq!(
        value(&files, "app/c.gcsl"),
        "{r = [\"x: x t-x 1\", \"y: y t-y 2\", \"dev.z: z t-z 3\"]}"
    );
    // A def-type with no meta works as it always did.
    let files = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }\nr = build x(\"m\")"),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = \"x: m\"}");
}

#[test]
fn each_marker_reads_its_meta_as_its_effect() {
    let path = std::env::temp_dir().join(format!("gcsl-slice-38-{}", std::process::id()));
    let path = path.to_str().unwrap();
    let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
    let _ = std::fs::remove_file(path);
    // `META->` runs its body at each read, `META->>` once for the run.
    let impure = "#[def_type]\n\
             def twice(name: String, f: fn(): Int META-> {n: Int}) -> List<Int> { [f.n, f.n] }";
    let app = format!(
        "defmeta twice x {{ n = exec([\"sh\", \"-c\", \"echo x >> {path}\"]).status }}\n\
             def twice x() {{ 1 }}\n\
             r = twice x()"
    );
    let files = [("workspace.gcsl", impure), ("app.gcsl", app.as_str())];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]}");
    assert_eq!(count(), 2);
    let memo = impure.replace("META->", "META->>");
    let files = [
        ("workspace.gcsl", memo.as_str()),
        ("app.gcsl", app.as_str()),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]}");
    assert_eq!(count(), 3);
    std::fs::remove_file(path).unwrap();
    // A `META: Future<...>` meta is read purely, giving a future, which
    // `defmeta ... = lazy` makes: its field is awaited, which only
    // impure code may.
    let future = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def later(name: String, f: fn(): Int META: Future<{v: Int}>) -> Int {\n\
                 \x20   f.v + f()\n\
                 }",
        ),
        (
            "app.gcsl",
            "defmeta later x = lazy { v = 41 }\ndef later x() { 1 }\nr = later x()",
        ),
    ];
    assert_eq!(value(&future, "app.gcsl"), "{r = 42}");
    let pure = future[0].1.replace(") -> Int", "): Int");
    let files = [("workspace.gcsl", pure.as_str()), future[1]];
    assert_eq!(
        check_errors(&files),
        [
            "cannot use a value that may be a future in a pure function at workspace.gcsl \
                 (this is Future<{v: Int}>); workspace.gcsl: this function is pure, declared \
                 with `:`; note: using the value here would await it, which may run effects; a \
                 pure function may hold and pass on a future but not await it. Declare the \
                 function with `->` to make it impure"
        ]
    );
    // Under `META->`, a field that may be a future is awaited where it is
    // used.
    let lazy = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def sum(name: String, f: fn(): Int META-> {ns: List<Int>}) -> Int {\n\
                 \x20   let mut total = f()\n\
                 \x20   for n in f.ns { total := total + n }\n\
                 \x20   total\n\
                 }",
        ),
        (
            "app.gcsl",
            "defmeta sum x { ns = lazy [1, 2, lazy 3] }\ndef sum x() { 4 }\nr = sum x()",
        ),
    ];
    assert_eq!(value(&lazy, "app.gcsl"), "{r = 10}");
}

#[test]
fn a_meta_s_caller_is_its_reader_s() {
    // The meta's `fileroot.this` is its own file's directory, and its
    // caller the directory of the code that reads it: the def-type's, or
    // through its `fileroot.helper()`, the def-type's own caller's.
    let files = [
        (
            "workspace.gcsl",
            "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn(): String META-> {this: String, caller: String}) \
                 -> String {\n\
                 \x20   \"{f.this} {f.caller}\"\n\
                 }\n\
                 #[def_type]\n\
                 def helped(name: String, f: fn(): String META-> {this: String, caller: String}) \
                 -> String {\n\
                 \x20   fileroot.helper()\n\
                 \x20   \"{f.this} {f.caller}\"\n\
                 }",
        ),
        (
            "a/a.gcsl",
            "import fileroot\n\
                 defmeta where x { this = fileroot.this; caller = fileroot.caller }\n\
                 def where x() { \"\" }\n\
                 defmeta helped x { this = fileroot.this; caller = fileroot.caller }\n\
                 def helped x() { \"\" }",
        ),
        ("a/sub/c.gcsl", "r = [where x(), helped x()]"),
    ];
    assert_eq!(
        rooted_text(&files, "a/sub/c.gcsl"),
        "{r = [\"<root>/a <root>\", \"<root>/a <root>/a/sub\"]}"
    );
}

#[test]
fn a_meta_pairs_with_its_definition_in_its_directory() {
    // A `#[shadowable]` definition shadowed in a subdirectory, which
    // brings its own meta: each view has its own pair.
    let files = [
        (
            "workspace.gcsl",
            format!(
                "{}\n#[shadowable]\ndef tagged s() {{ \"top\" }}\n\
                     defmeta tagged s {{ tag = \"top-meta\"; n = 0 }}",
                TAGGED.1
            ),
        ),
        ("root.gcsl", "r = tagged s()".to_owned()),
        (
            "sub/a.gcsl",
            "def tagged s() { \"sub\" }\n\
                 defmeta tagged s { tag = \"sub-meta\"; n = 1 }\n\
                 below = tagged s()"
                .to_owned(),
        ),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    assert_eq!(value(&files, "root.gcsl"), "{r = \"s: top top-meta 0\"}");
    assert_eq!(
        value(&files, "sub/a.gcsl"),
        "{below = \"s: sub sub-meta 1\"}"
    );
    // A chain of `#[shadowable]` definitions in one directory has one
    // meta.
    let files = [
        TAGGED,
        ("app/a.gcsl", "#[shadowable]\ndef tagged c() { \"first\" }"),
        (
            "app/b.gcsl",
            "def tagged c() { \"second\" }\n\
                 defmeta tagged c { tag = \"one\"; n = 1 }\n\
                 r = tagged c()",
        ),
    ];
    assert_eq!(value(&files, "app/b.gcsl"), "{r = \"c: second one 1\"}");
    // A meta in a subdirectory of its definition is no meta of it: the
    // definition has none in its own directory either.
    let files = [
        TAGGED,
        ("app/a.gcsl", "def tagged x() { \"x\" }"),
        ("app/sub/b.gcsl", "defmeta tagged x { tag = \"t\"; n = 1 }"),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "`tagged x` has no `defmeta` in its directory at app/a.gcsl; note: `tagged` \
                 declares a meta, `META: {tag: String, n: Int}`, for each of its definitions",
            "`defmeta tagged x` has no definition `x` in its directory at app/sub/b.gcsl"
        ]
    );
    let files = [TAGGED, ("app/a.gcsl", "def tagged x() { \"x\" }")];
    assert_eq!(
        check_errors(&files),
        [
            "`tagged x` has no `defmeta` in its directory at app/a.gcsl; note: `tagged` \
                 declares a meta, `META: {tag: String, n: Int}`, for each of its definitions"
        ]
    );
}

#[test]
fn a_meta_is_invoked_with_its_definition() {
    // The meta in another file of the definition's directory: invoking
    // the definition by name runs that file too.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def tag(name: String, f: fn(): String META->> {t: String}) -> String {\n\
                 \x20   \"{f()} {f.t}\"\n\
                 }",
        ),
        ("pkg/a.gcsl", "defmeta tag p { t = \"meta\" }"),
        ("pkg/p.gcsl", "def tag p() { \"p\" }"),
    ];
    assert_eq!(invoked_text(&files, "tag", "p"), "p meta");
}

#[test]
fn meta_errors() {
    // Each error with its labels and notes, joined by `; `, the errors
    // by newlines.
    let errors = |text: &str| {
        let files = [TAGGED, ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    assert_eq!(
        errors("x: fn(): Int META: Int = 1"),
        "a meta must be a record type: its fields are read from the function at app.gcsl"
    );
    assert_eq!(
        errors("x: fn(): Int META-> Future<List<Int>> = 1"),
        "a meta must be a record type: its fields are read from the function at app.gcsl"
    );
    assert_eq!(
        errors("fn f(g: fn(): Int META: {a: Int, b: Int}): Int { g.c }"),
        "the meta of `g` has no field `c` at app.gcsl; note: the fields are `a` and `b`"
    );
    assert_eq!(
        errors("fn f(g: fn(): Int META-> {a: Int}): Int {\nlet held = g.a\n1\n}"),
        "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: reading the \
             meta of `g` is impure (`META->`); app.gcsl: this function is pure, declared with \
             `:`; note: a pure function, declared with `:`, may read only pure metas, `META:`; \
             declare it with `->` to make it impure"
    );
    // In a memoised function, as calling an impure function is.
    let memo = [
        TAGGED,
        (
            "app.gcsl",
            "fn f(g: fn(): Int META-> {a: Int}) ->> Int { g.a }\n\
                 fn h(g: fn(): Int META->> {a: Int}) -> Int { g.a }",
        ),
    ];
    assert_eq!(value(&memo, "app.gcsl"), "{}");
    // A function with no meta has no fields, as before.
    assert_eq!(
        errors("fn f(g: fn(): Int): Int { g.a }"),
        "cannot access field `a` of `fn(): Int` at app.gcsl; app.gcsl: this is fn(): Int"
    );
    // A function with no meta, or with a meta of a greater marker, does
    // not fit a type with one.
    assert_eq!(
        errors("fn f(g: fn(): Int META: {a: Int}): Int { g.a }\nx = f(fn(): Int { 1 })"),
        "mismatched types at app.gcsl (expected fn(): Int META: {a: Int}, found fn(): Int); \
             app.gcsl: the function is fn(g: fn(): Int META: {a: Int}): Int"
    );
    assert_eq!(
        errors(
            "fn f(g: fn(): Int META: {a: Int}): Int { g.a }\n\
                 fn h(g: fn(): Int META-> {a: Int}): Int { f(g) }"
        ),
        "mismatched types at app.gcsl (expected fn(): Int META: {a: Int}, found fn(): Int \
             META-> {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>); app.gcsl: the \
             function is fn(g: fn(): Int META: {a: Int}): Int"
    );
    // The pairing.
    assert_eq!(
        errors(
            "def tagged x() { \"x\" }\n\
                 defmeta tagged x { tag = \"a\"; n = 1 }\n\
                 defmeta tagged x { tag = \"b\"; n = 2 }\n\
                 defmeta tagged y { tag = \"c\"; n = 3 }"
        ),
        "`defmeta tagged x` is declared twice in this directory at app.gcsl; app.gcsl: \
             first declared here\n\
             `defmeta tagged y` has no definition `y` in its directory at app.gcsl"
    );
    assert_eq!(
        errors(
            "#[def_type]\ndef plain(name: String, f: fn(): Int): Int { f() }\n\
                 def plain p() { 1 }\n\
                 defmeta plain p { tag = \"x\" }"
        ),
        "`plain` declares no meta for its definitions at app.gcsl; note: a def-type \
             declares one by its definition parameter's type, as in `func: fn(): PackageDef \
             META: PackageMeta`"
    );
    assert_eq!(
        errors("defmeta nope x { a = 1 }"),
        "undefined variable `nope` at app.gcsl"
    );
    // The meta's body is checked against the meta's type, with its marker.
    assert_eq!(
        errors("def tagged x() { \"x\" }\ndefmeta tagged x { tag = 1; n = 1 }"),
        "mismatched types at app.gcsl (expected String, found Int); app.gcsl: the return \
             type is declared here"
    );
    assert_eq!(
        errors(
            "def tagged x() { \"x\" }\n\
                 defmeta tagged x { tag = exec([\"true\"]).stdout; n = 1 }"
        ),
        "cannot call an impure function from a pure function at app.gcsl; app.gcsl: this \
             is fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}; \
             app.gcsl: this function is pure, declared with `:`; note: a pure function, \
             declared with `:`, may call only pure functions; declare it with `->` to make it \
             impure"
    );
    // A definition needs its meta: one that reads a variable bound later
    // is shown as written in the chain.
    let found = errors(
        "r = tagged x()\nlet k = 1\ndefmeta tagged x { tag = \"t\"; n = k }\n\
             def tagged x() { \"x\" }",
    );
    assert!(
        found.contains("uses `defmeta tagged x`, and `defmeta tagged x` reads `k`"),
        "{found}"
    );
}

/// A def-type of the root of a workspace, for the tests of reading a
/// meta through a reference: `info`, whose definitions give a number and
/// carry a pure meta of a record, a list and a string.
const INFO: (&str, &str) = (
    "workspace.gcsl",
    "#[def_type]\n\
         def info(name: String, f: fn(): Int META: {about: {owner: String}, ns: List<Int>, \
         peios: String}): Int {\n\
         \x20   f() + len(f.ns)\n\
         }",
);

#[test]
fn a_reference_reads_its_definition_s_meta() {
    // In parentheses and by a path past the definition, deeper reads, a
    // reference stored and passed as a function with less of the meta,
    // in a pure function: the meta is pure.
    let def_types = format!("{}\n{}", INFO.1, TAGGED.1);
    let files = [
        ("workspace.gcsl", def_types.as_str()),
        (
            "app/a.gcsl",
            "defmeta info dev.x { about = { owner = \"jack\" }; ns = [4, 5]; peios = \"p\" }\n\
                 def info dev.x() { 1 }\n\
                 defmeta tagged dev.x { tag = \"t\"; n = 7 }\n\
                 def tagged dev.x() { \"x\" }\n\
                 fn owner(): String { info dev.x.about.owner }\n\
                 fn tag(g: fn(): String META: {tag: String}): String { g.tag }\n\
                 r = [\n\
                 \x20   info dev.x.peios,\n\
                 \x20   (info dev.x).peios,\n\
                 \x20   owner(),\n\
                 \x20   \"{info dev.x.ns[1]}\",\n\
                 \x20   \"{(info dev.x).ns[0]}\",\n\
                 ]\n\
                 let stored = tagged dev.x\n\
                 s = [stored.tag, tag(stored), tag(tagged dev.x), \"{stored.n}\"]\n\
                 called = info dev.x()",
        ),
    ];
    assert_eq!(
        value(&files, "app/a.gcsl"),
        "{r = [\"p\", \"p\", \"jack\", \"5\", \"4\"]; s = [\"t\", \"t\", \"t\", \"7\"]; \
             called = 3}"
    );
    // Under `META->`, each field past the definition that may be a future
    // is awaited where the next reads it, a lazy one included.
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def t(name: String, f: fn(): Int META-> {about: {owner: String}}) -> Int { f() }",
        ),
        (
            "app.gcsl",
            "defmeta t x { about = { owner = \"o\" } }\n\
                 def t x() { 1 }\n\
                 defmeta t y { about = lazy { owner = lazy \"lazy\" } }\n\
                 def t y() { 2 }\n\
                 r = \"{t x.about.owner} {t y.about.owner} {(t y).about.owner}\"",
        ),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = \"o lazy lazy\"}");
}

#[test]
fn a_reference_means_its_longest_definition() {
    // `info dev` is a definition, and so is `tagged dev.peios.x`: each
    // path means the definition of its own def-type, `info
    // dev.peios` the field `peios` of `info dev`'s meta.
    let def_types = format!("{}\n{}", INFO.1, TAGGED.1);
    let files = [
        ("workspace.gcsl", def_types.as_str()),
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"meta of dev\" }\n\
                 def info dev() { 1 }\n\
                 defmeta tagged dev.peios.x { tag = \"t\"; n = 1 }\n\
                 def tagged dev.peios.x() { \"x\" }\n\
                 r = [info dev.peios, tagged dev.peios.x.tag]",
        ),
    ];
    assert_eq!(
        value(&files, "app/a.gcsl"),
        "{r = [\"meta of dev\", \"t\"]}"
    );
    // A name past the definition that is no field of its meta: the
    // meta's error, noted with how the path was read.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = info dev.peios.x",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        ["cannot access field `x` of String at app/a.gcsl; app/a.gcsl: this is String"]
    );
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = info dev.pieos.x",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "the meta of `info dev` has no field `pieos` at app/a.gcsl; note: the fields \
                 are `about`, `ns` and `peios`; note: `info dev.pieos` is read as the field \
                 `pieos` of the meta of `info dev`, its longest prefix that is a definition"
        ]
    );
    // In parentheses the reference is all there is: no second note.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev).pieos",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "the meta of `info dev` has no field `pieos` at app/a.gcsl; note: the fields \
                 are `about`, `ns` and `peios`"
        ]
    );
    // A path no prefix of which is a definition: as before.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev.x { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev.x() { 1 }\n\
                 r = info dev.y.peios\n\
                 s = info dev",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "`info` has no definition `dev.y.peios` at app/a.gcsl; note: the only \
                 definition of `info` here is `dev.x`",
            "`dev` is not a `info` definition at app/a.gcsl"
        ]
    );
    // The result of a call has the fields of the result, not of the meta.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev()).peios",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        ["cannot access field `peios` of Int at app/a.gcsl; app/a.gcsl: this is Int"]
    );
}

#[test]
fn a_def_type_without_a_meta_reads_no_path_past_a_definition() {
    // `build x.y` names the definition `x.y`, as it always did.
    let files = [BUILD, ("app.gcsl", "def build x(t) { t }\nr = build x.y")];
    assert_eq!(
        check_errors(&files),
        [
            "`build` has no definition `x.y` at app.gcsl; note: the only definition of \
                 `build` here is `x`"
        ]
    );
    // Nor does its reference have a meta, nor any field.
    let files = [BUILD, ("app.gcsl", "def build x(t) { t }\nr = (build x).y")];
    assert_eq!(
        check_errors(&files),
        [
            "cannot access field `y` of `fn(mode: String): String` at app.gcsl; app.gcsl: \
                 this is fn(mode: String): String"
        ]
    );
}

#[test]
fn a_meta_read_through_a_reference_needs_the_meta() {
    // The meta in one file, the definition in another, the read in a
    // directory below: each runs before what reads it.
    let files = [
        INFO,
        (
            "pkg/m.gcsl",
            "defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = \"m\" }",
        ),
        ("pkg/d.gcsl", "def info x() { 1 }"),
        ("pkg/sub/app.gcsl", "r = info x.peios"),
    ];
    assert_eq!(value(&files, "pkg/sub/app.gcsl"), "{r = \"m\"}");
    // Read in the definition's file, above its meta, which reads a field
    // of the file.
    let files = [
        INFO,
        (
            "pkg/d.gcsl",
            "r = info x.peios\n\
                 def info x() { 1 }\n\
                 defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = owner }\n\
                 owner = \"later\"",
        ),
    ];
    assert_eq!(
        value(&files, "pkg/d.gcsl"),
        "{r = \"later\"; owner = \"later\"}"
    );
    // A meta that reads a variable bound below the read cannot be read
    // there: the chain names the definition and its meta.
    let files = [
        INFO,
        (
            "app.gcsl",
            "r = info x.peios\n\
                 let k = \"k\"\n\
                 def info x() { 1 }\n\
                 defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = k }",
        ),
    ];
    let found = check_errors(&files).join("\n");
    assert!(
        found.starts_with("`info x` cannot be used here yet at app.gcsl")
            && found.contains("`info x` uses `defmeta info x`, and `defmeta info x` reads `k`"),
        "{found}"
    );
}

#[test]
fn a_meta_read_through_a_reference_is_the_definition_s_meta() {
    let path = std::env::temp_dir().join(format!("gcsl-slice-39-{}", std::process::id()));
    let path = path.to_str().unwrap();
    let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
    let _ = std::fs::remove_file(path);
    // `META->>`: one run for the def-type's two reads and the
    // references' three, in parentheses, past the definition and
    // stored; at the top level of a file, which is impure.
    let memo = "#[def_type]\n\
             def twice(name: String, f: fn(): Int META->> {n: Int}) -> List<Int> { [f.n, f.n] }";
    let app = format!(
        "defmeta twice x {{ n = exec([\"sh\", \"-c\", \"echo x >> {path}\"]).status }}\n\
             def twice x() {{ 1 }}\n\
             r = twice x()\n\
             let stored = twice x\n\
             s = [(twice x).n, twice x.n, stored.n]"
    );
    let files = [("workspace.gcsl", memo), ("app.gcsl", app.as_str())];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]; s = [0, 0, 0]}");
    assert_eq!(count(), 1);
    // `META->`: one run for each read.
    let impure = memo.replace("META->>", "META->");
    let files = [
        ("workspace.gcsl", impure.as_str()),
        ("app.gcsl", app.as_str()),
    ];
    assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]; s = [0, 0, 0]}");
    assert_eq!(count(), 6);
    std::fs::remove_file(path).unwrap();
    // In a pure function, the read is the error, however the reference
    // is written; in an impure one, it is allowed.
    let files = [
        ("workspace.gcsl", impure.as_str()),
        (
            "app.gcsl",
            "defmeta twice x { n = 1 }\n\
                 def twice x() { 1 }\n\
                 fn a(): Int {\nlet held = twice x.n\n1\n}\n\
                 fn b(): Int {\nlet held = (twice x).n\n1\n}\n\
                 fn c() -> Int { twice x.n }",
        ),
    ];
    assert_eq!(
        check_errors(&files),
        [
            "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: \
                 reading the meta of `twice x` is impure (`META->`); app.gcsl: this function is \
                 pure, declared with `:`; note: a pure function, declared with `:`, may read \
                 only pure metas, `META:`; declare it with `->` to make it impure",
            "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: \
                 reading the meta of `twice x` is impure (`META->`); app.gcsl: this function is \
                 pure, declared with `:`; note: a pure function, declared with `:`, may read \
                 only pure metas, `META:`; declare it with `->` to make it impure"
        ]
    );
}

#[test]
fn dumps_show_a_reference_past_its_definition() {
    let mut workspace = checked(&[
        INFO,
        (
            "app.gcsl",
            "defmeta info x { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info x() { 1 }\n\
                 r = info x.about.owner",
        ),
    ]);
    let index = workspace.file("app.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    let meta = "META: {about: {owner: String}, ns: List<Int>, peios: String}";
    assert!(
        dump.contains(&format!(
            "(field r @84..106\n    (. owner : String @88..106\n      (. about : {{owner: \
                 String}} @88..100\n        (def-type-ref info x : fn(): Int {meta} @88..94))))"
        )),
        "{dump}"
    );
    // The syntax tree alone has the reference as it is written.
    let source = SourceFile::new("app.gcsl", "r = info x.about.owner").unwrap();
    let (tokens, names, strings) = crate::lexer::tokenize(&source).unwrap();
    let ast = crate::parser::parse(&tokens, names, strings, &source).unwrap();
    assert!(
        ast.pretty()
            .to_string()
            .contains("(def-type-ref info x.about.owner @4..22)"),
        "{}",
        ast.pretty()
    );
    // The reads of the meta read it through the definition's cell.
    let ir = workspace.dump_ir(index).unwrap();
    assert!(
        ir.contains("(field owner @93..106\n      (field about @93..100\n        (call @93..100\n"),
        "{ir}"
    );
}

/// A workspace whose definitions have `+` in their paths' names: the
/// names of real packages, `a+b` and `c++11`, defined with their metas in
/// two files of one directory, `pkgs`, and referred to from a third file
/// of it and from a directory below it, called and read past.
const PLUS: [(&str, &str); 5] = [
    (
        "workspace.gcsl",
        "#[def_type]\n\
             def package(name: String, f: fn(v: String): String META: {versions: List<String>}, \
             version: String) -> String { \"{name} {f(version)}\" }",
    ),
    (
        "pkgs/gnu.gcsl",
        "defmeta package org.gnu.gmp-c++ { versions = [\"6.3.0\"] }\n\
             def package org.gnu.gmp-c++(v) { \"gmp {v}\" }\n\
             defmeta package org.gnu.gcc-c++ { versions = [\"14\"] }\n\
             def package org.gnu.gcc-c++(v) { \"g++ {v}\" }\n\
             defmeta package org.gnu.gcc-libstdc++ { versions = [\"14.1\"] }\n\
             def package org.gnu.gcc-libstdc++(v) { \"libstdc++ {v}\" }\n\
             defmeta package org.gnu.gcc-libstdc++-devel { versions = [\"14.2\"] }\n\
             def package org.gnu.gcc-libstdc++-devel(v) { \"devel {v}\" }",
    ),
    (
        "pkgs/more.gcsl",
        "defmeta package dev.peios.build-essentials-c++ { versions = [\"1\"] }\n\
             def package dev.peios.build-essentials-c++(v) { \"essentials {v}\" }\n\
             defmeta package org.gnu.a+b { versions = [] }\n\
             def package org.gnu.a+b(v) { \"a+b {v}\" }\n\
             defmeta package c++11 { versions = [\"11\"] }\n\
             def package c++11(v) { \"c++11 {v}\" }",
    ),
    (
        "pkgs/use.gcsl",
        "x = package org.gnu.gmp-c++(\"6.3.0\")\n\
             y = package org.gnu.gmp-c++.versions\n\
             gxx = package org.gnu.gcc-c++.versions[0]\n\
             lib = package org.gnu.gcc-libstdc++(\"1\")\n\
             devel = (package org.gnu.gcc-libstdc++-devel).versions\n\
             essentials = package dev.peios.build-essentials-c++(\"2\")\n\
             ab = package org.gnu.a+b(\"3\")\n\
             c11 = package c++11.versions\n\
             z = 1 + 2\n\
             let a = 4\n\
             let b = 5\n\
             c = a+b",
    ),
    (
        "pkgs/sub/below.gcsl",
        "let f = package org.gnu.gcc-libstdc++-devel\n\
             below = f(\"4\")\n\
             none = package org.gnu.a+b.versions",
    ),
];

#[test]
fn definitions_with_plus_in_their_paths_are_ordinary_definitions() {
    // Referred to from another file of their directory and from one
    // below it: called, with their paths as written, and their metas
    // read; `a+b` anywhere else is the addition it always was.
    assert_eq!(
        rooted_text(&PLUS, "pkgs/use.gcsl"),
        "{x = \"org.gnu.gmp-c++ gmp 6.3.0\"; y = [\"6.3.0\"]; gxx = \"14\"; \
             lib = \"org.gnu.gcc-libstdc++ libstdc++ 1\"; devel = [\"14.2\"]; \
             essentials = \"dev.peios.build-essentials-c++ essentials 2\"; \
             ab = \"org.gnu.a+b a+b 3\"; c11 = [\"11\"]; z = 3; c = 9}"
    );
    assert_eq!(
        rooted_text(&PLUS, "pkgs/sub/below.gcsl"),
        "{below = \"org.gnu.gcc-libstdc++-devel devel 4\"; none = []}"
    );
    // Listed, invoked and their metas read by name, by their paths as
    // written.
    let mut workspace = rooted(&PLUS).unwrap();
    assert_eq!(
        listed(&workspace, &PLUS),
        [
            "package in workspace.gcsl: org.gnu.gmp-c++ in pkgs/gnu.gcsl, \
                 org.gnu.gcc-c++ in pkgs/gnu.gcsl, org.gnu.gcc-libstdc++ in pkgs/gnu.gcsl, \
                 org.gnu.gcc-libstdc++-devel in pkgs/gnu.gcsl, \
                 dev.peios.build-essentials-c++ in pkgs/more.gcsl, \
                 org.gnu.a+b in pkgs/more.gcsl, c++11 in pkgs/more.gcsl"
        ]
    );
    for (path, value) in [
        ("org.gnu.gmp-c++", "gmp"),
        ("org.gnu.gcc-c++", "g++"),
        ("org.gnu.gcc-libstdc++", "libstdc++"),
        ("org.gnu.gcc-libstdc++-devel", "devel"),
        ("dev.peios.build-essentials-c++", "essentials"),
        ("org.gnu.a+b", "a+b"),
        ("c++11", "c++11"),
    ] {
        let outcome = workspace.invoke("package", path, &[string("version", "9")]);
        assert_eq!(
            outcome.unwrap().value.to_string(),
            format!("{path} {value} 9")
        );
    }
    let versions = workspace.meta_field("package", "org.gnu.gmp-c++", "versions");
    assert_eq!(versions.unwrap().unwrap().value.to_string(), "[\"6.3.0\"]");
    // A path that names none is the error it is, its definitions listed
    // as written.
    assert_eq!(
        request_error_with(&PLUS, ("package", "org.gnu.gmp-c"), &[("version", "1")]),
        "`package` has no definition `org.gnu.gmp-c`; note: its definitions are \
             `c++11`, `dev.peios.build-essentials-c++`, `org.gnu.a+b`, `org.gnu.gcc-c++`, \
             `org.gnu.gcc-libstdc++`, `org.gnu.gcc-libstdc++-devel`, `org.gnu.gmp-c++`"
    );
    // The dumps show them as written, of files not yet run.
    let mut workspace = rooted(&PLUS).unwrap();
    let index = workspace.file("pkgs/sub/below.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(
        dump.contains("(def-type-ref package org.gnu.gcc-libstdc++-devel : fn(version:"),
        "{dump}"
    );
    assert!(
        dump.contains("(def-type-ref package org.gnu.a+b : fn(version:"),
        "{dump}"
    );
    let index = workspace.file("pkgs/gnu.gcsl").unwrap();
    let ir = workspace.dump_ir(index).unwrap();
    assert!(ir.contains("fn #1 package org.gnu.gmp-c++ meta #"), "{ir}");
}

#[test]
fn definitions_with_plus_in_their_paths_conflict_as_any_do() {
    // Two of one path in one directory: the language's error, the path
    // shown as written.
    let one = [
        PACKAGE,
        (
            "a.gcsl",
            "def package x.a+b() { \"1\" }\ndef package x.a+b() { \"2\" }",
        ),
    ];
    assert_eq!(
        check_errors(&one),
        ["field `package x.a+b` is defined twice at a.gcsl; a.gcsl: first defined here"]
    );
    // In two sibling directories, which see neither the other: named with
    // its directory when invoked by name.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package x.a+b() { \"a\" }"),
        ("b/y.gcsl", "def package x.a+b() { \"b\" }"),
    ];
    assert_eq!(
        request_error(&siblings, "package", "x.a+b"),
        "`package x.a+b` is defined in 2 directories, so it is named with its directory; \
             note: name one of `a/x.a+b`, `b/x.a+b`"
    );
    assert_eq!(invoked_text(&siblings, "package", "b/x.a+b"), "x.a+b b");
    // One lying under another in a directory above, which it shadows.
    let under = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package x.a+b() { \"a\" }",
        ),
        ("b/y.gcsl", "def package x.a+b.c() { \"b\" }"),
    ];
    assert_eq!(
        listing_errors(&under),
        [format!(
            "`package x.a+b.c` lies under another `package` definition at b/y.gcsl; \
                 workspace.gcsl: `x.a+b` is defined here; {UNDER}"
        )]
    );
    // `a+b` is a name of its own, under no `a`.
    let apart = [
        PACKAGE,
        ("a/x.gcsl", "def package x.a() { \"a\" }"),
        ("b/y.gcsl", "def package x.a+b() { \"b\" }"),
    ];
    assert_eq!(invoked_text(&apart, "package", "x.a"), "x.a a");
    assert_eq!(invoked_text(&apart, "package", "x.a+b"), "x.a+b b");
    // A reference followed by `+` and a space is added to, as it was.
    let added = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef package(name: String, func: fn(): Int) -> Int { func() }",
        ),
        ("a.gcsl", "def package x.a() { 1 }\nr = package x.a + 1"),
    ];
    assert_eq!(
        check_errors(&added),
        ["cannot add `fn() -> Int` and Int at a.gcsl; a.gcsl: fn() -> Int; a.gcsl: Int"]
    );
}

/// A workspace whose definitions have names starting with a digit in
/// their paths: the names of real toolchain lanes, a float's split at its
/// `.`, an integer's glued to a name, written with `_` and with an
/// exponent, defined with their metas in two files of one directory,
/// `pkgs`, and referred to from a third file of it and from a directory
/// below it, called and read past.
const DIGITS: [(&str, &str); 5] = [
    (
        "workspace.gcsl",
        "#[def_type]\n\
             def package(name: String, f: fn(v: String): String META: {versions: List<String>}, \
             version: String) -> String { \"{name} {f(version)}\" }",
    ),
    (
        "pkgs/lanes.gcsl",
        "defmeta package org.golang.go-stage0-1.24 { versions = [\"1.24.0\"] }\n\
             def package org.golang.go-stage0-1.24(v) { \"go {v}\" }\n\
             defmeta package org.rust-lang.rust-1.83 { versions = [\"1.83.0\"] }\n\
             def package org.rust-lang.rust-1.83(v) { \"rust {v}\" }",
    ),
    (
        "pkgs/more.gcsl",
        "defmeta package x.1.83.0 { versions = [\"0\"] }\n\
             def package x.1.83.0(v) { \"x {v}\" }\n\
             defmeta package x.24rc1 { versions = [] }\n\
             def package x.24rc1(v) { \"rc {v}\" }\n\
             defmeta package x.1.83rc1 { versions = [\"rc1\"] }\n\
             def package x.1.83rc1(v) { \"rc1 {v}\" }\n\
             defmeta package x.1_0 { versions = [\"10\"] }\n\
             def package x.1_0(v) { \"ten {v}\" }\n\
             defmeta package x.1e5 { versions = [\"1e5\"] }\n\
             def package x.1e5(v) { \"exp {v}\" }",
    ),
    (
        "pkgs/use.gcsl",
        "a = package org.golang.go-stage0-1.24(\"1.24.0\")\n\
             b = package org.rust-lang.rust-1.83.versions\n\
             c = package x.1.83.0(\"9\")\n\
             d = package x.24rc1.versions\n\
             e = package x.1.83rc1(\"1\")\n\
             f = package x.1_0.versions[0]\n\
             g = package x.1e5(\"2\")",
    ),
    (
        "pkgs/sub/below.gcsl",
        "let r = package org.rust-lang.rust-1.83\n\
             below = r(\"1.83.0\")\n\
             none = package x.24rc1.versions",
    ),
];

#[test]
fn definitions_with_digits_in_their_paths_are_ordinary_definitions() {
    // Referred to from another file of their directory and from one
    // below it: called, their def-type given their paths as written,
    // and their metas read.
    assert_eq!(
        rooted_text(&DIGITS, "pkgs/use.gcsl"),
        "{a = \"org.golang.go-stage0-1.24 go 1.24.0\"; b = [\"1.83.0\"]; \
             c = \"x.1.83.0 x 9\"; d = []; e = \"x.1.83rc1 rc1 1\"; f = \"10\"; \
             g = \"x.1e5 exp 2\"}"
    );
    assert_eq!(
        rooted_text(&DIGITS, "pkgs/sub/below.gcsl"),
        "{below = \"org.rust-lang.rust-1.83 rust 1.83.0\"; none = []}"
    );
    // Listed, invoked and their metas read by name, by their paths as
    // written.
    let mut workspace = rooted(&DIGITS).unwrap();
    assert_eq!(
        listed(&workspace, &DIGITS),
        [
            "package in workspace.gcsl: org.golang.go-stage0-1.24 in pkgs/lanes.gcsl, \
                 org.rust-lang.rust-1.83 in pkgs/lanes.gcsl, x.1.83.0 in pkgs/more.gcsl, \
                 x.24rc1 in pkgs/more.gcsl, x.1.83rc1 in pkgs/more.gcsl, \
                 x.1_0 in pkgs/more.gcsl, x.1e5 in pkgs/more.gcsl"
        ]
    );
    for (path, value) in [
        ("org.golang.go-stage0-1.24", "go"),
        ("org.rust-lang.rust-1.83", "rust"),
        ("x.1.83.0", "x"),
        ("x.24rc1", "rc"),
        ("x.1.83rc1", "rc1"),
        ("x.1_0", "ten"),
        ("x.1e5", "exp"),
    ] {
        let outcome = workspace.invoke("package", path, &[string("version", "9")]);
        assert_eq!(
            outcome.unwrap().value.to_string(),
            format!("{path} {value} 9")
        );
    }
    let versions = workspace.meta_field("package", "org.golang.go-stage0-1.24", "versions");
    assert_eq!(versions.unwrap().unwrap().value.to_string(), "[\"1.24.0\"]");
    let versions = workspace.meta_field("package", "org.rust-lang.rust-1.83", "versions");
    assert_eq!(versions.unwrap().unwrap().value.to_string(), "[\"1.83.0\"]");
    // A path that names none is the error it is, its definitions listed
    // as written; a number's value is not its name.
    assert_eq!(
        request_error_with(
            &DIGITS,
            ("package", "org.golang.go-stage0-1.2"),
            &[("version", "1")]
        ),
        "`package` has no definition `org.golang.go-stage0-1.2`; note: its definitions are \
             `org.golang.go-stage0-1.24`, `org.rust-lang.rust-1.83`, `x.1.83.0`, `x.1.83rc1`, \
             `x.1_0`, `x.1e5`, `x.24rc1`"
    );
    assert_eq!(
        request_error_with(&DIGITS, ("package", "x.10"), &[("version", "1")]),
        "`package` has no definition `x.10`; note: its definitions are \
             `org.golang.go-stage0-1.24`, `org.rust-lang.rust-1.83`, `x.1.83.0`, `x.1.83rc1`, \
             `x.1_0`, `x.1e5`, `x.24rc1`"
    );
    // The dumps show them as written, of files not yet run.
    let mut workspace = rooted(&DIGITS).unwrap();
    let index = workspace.file("pkgs/sub/below.gcsl").unwrap();
    let dump = workspace.dump_types(index).unwrap();
    assert!(
        dump.contains("(def-type-ref package org.rust-lang.rust-1.83 : fn(version:"),
        "{dump}"
    );
    assert!(
        dump.contains("(def-type-ref package x.24rc1 : fn(version:"),
        "{dump}"
    );
    let index = workspace.file("pkgs/lanes.gcsl").unwrap();
    let ir = workspace.dump_ir(index).unwrap();
    assert!(
        ir.contains("fn #1 package org.golang.go-stage0-1.24 meta #"),
        "{ir}"
    );
}

#[test]
fn definitions_with_digits_in_their_paths_conflict_as_any_do() {
    // In one directory, a definition under another, and two of one path:
    // the language's errors, the paths shown as written.
    let under = [
        PACKAGE,
        (
            "a.gcsl",
            "def package x.1() { \"1\" }\ndef package x.1.83() { \"2\" }",
        ),
    ];
    assert_eq!(
        check_errors(&under),
        [
            "cannot define `package x.1.83`: `package x.1` is `fn(): String`, not a record \
                 at a.gcsl; a.gcsl: `package x.1` defined here"
        ]
    );
    let twice = [
        PACKAGE,
        (
            "a.gcsl",
            "def package x.1.83() { \"1\" }\ndef package x.1.83() { \"2\" }",
        ),
    ];
    assert_eq!(
        check_errors(&twice),
        ["field `package x.1.83` is defined twice at a.gcsl; a.gcsl: first defined here"]
    );
    // In two sibling directories, which see neither the other: named with
    // its directory when invoked by name.
    let siblings = [
        PACKAGE,
        ("a/x.gcsl", "def package x.1.83() { \"a\" }"),
        ("b/y.gcsl", "def package x.1.83() { \"b\" }"),
    ];
    assert_eq!(
        request_error(&siblings, "package", "x.1.83"),
        "`package x.1.83` is defined in 2 directories, so it is named with its directory; \
             note: name one of `a/x.1.83`, `b/x.1.83`"
    );
    assert_eq!(invoked_text(&siblings, "package", "b/x.1.83"), "x.1.83 b");
    // One lying under another in a directory above, which it shadows.
    let under = [
        (
            "workspace.gcsl",
            "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package x.1() { \"a\" }",
        ),
        ("b/y.gcsl", "def package x.1.83() { \"b\" }"),
    ];
    assert_eq!(
        listing_errors(&under),
        [format!(
            "`package x.1.83` lies under another `package` definition at b/y.gcsl; \
                 workspace.gcsl: `x.1` is defined here; {UNDER}"
        )]
    );
    // `10` is a name of its own, under no `1`.
    let apart = [
        PACKAGE,
        ("a/x.gcsl", "def package x.1() { \"a\" }"),
        ("b/y.gcsl", "def package x.10() { \"b\" }"),
    ];
    assert_eq!(invoked_text(&apart, "package", "x.1"), "x.1 a");
    assert_eq!(invoked_text(&apart, "package", "x.10"), "x.10 b");
}

/// A scratch directory of the test `name`'s own, empty, and the path of
/// the log file in it, which the [`environment`] workspaces append to.
fn log_in(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("gcsl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log").display().to_string();
    (dir, log)
}

/// What the log `log` holds, empty if it is missing.
fn logged(log: &str) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

/// A workspace with a root `pekit.gcsl` whose memoised `environment`
/// logs `boot` to `log` and returns a handle, and whose `teardown` logs
/// `down` and the handle's `pid`; a def-type `run`, whose definition
/// `uses` calls `environment` through an import of `pekit`, and `plain`
/// does not.
fn environment(log: &str) -> [(&'static str, String); 3] {
    [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }".to_owned(),
        ),
        (
            "pekit.gcsl",
            format!(
                "def environment() ->> {{pid: Int, port: Int}} {{\n\
                     \x20   let out = sh(\"echo boot >> {log}\")\n\
                     \x20   {{ pid = 7; port = if out == \"\" {{ 22 }} else {{ 0 }} }}\n\
                     }}\n\
                     def teardown(env: {{pid: Int, port: Int}}) -> String {{\n\
                     \x20   sh(\"echo down {{env.pid}} >> {log}\")\n\
                     }}\n\
                     x = 1\n\
                     fn private() ->> Int {{ 1 }}"
            ),
        ),
        (
            "pkg/p.gcsl",
            "import pekit\n\
                 def run uses() { \"uses {pekit.environment().pid}\" }\n\
                 def run plain() { \"plain\" }"
                .to_owned(),
        ),
    ]
}

#[test]
fn a_top_level_function_is_found_by_name() {
    let (dir, log) = log_in("top-level");
    let files = environment(&log);
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    assert_eq!(
        workspace.top_level(pekit, "environment"),
        Some(TopLevel {
            ty: "fn() ->> {pid: Int, port: Int}".to_owned(),
            function: Some((ast::Effect::Memo, 0)),
        })
    );
    assert_eq!(
        workspace.top_level(pekit, "teardown"),
        Some(TopLevel {
            ty: "fn(env: {pid: Int, port: Int}) -> String".to_owned(),
            function: Some((ast::Effect::Impure, 1)),
        })
    );
    // A field that is no function; a private function, a name that is
    // none and a file that is none, nothing.
    assert_eq!(
        workspace.top_level(pekit, "x"),
        Some(TopLevel {
            ty: "Int".to_owned(),
            function: None,
        })
    );
    assert_eq!(workspace.top_level(pekit, "private"), None);
    assert_eq!(workspace.top_level(pekit, "nope"), None);
    assert_eq!(workspace.top_level(u32::MAX, "environment"), None);
    // An annotated field as its annotation is written, a function
    // field's as a diagnostic shows its type.
    let annotated = rooted(&[
        ("workspace.gcsl", "x = 1"),
        (
            "pekit.gcsl",
            "type Vm = {pid: Int}\n\
                 environment: fn() ->> Vm = fn() ->> Vm { { pid = 1 } }\n\
                 teardown = fn(env: Vm) -> {} { {} }",
        ),
    ])
    .unwrap();
    let file = annotated.file("pekit.gcsl").unwrap();
    assert_eq!(
        annotated.top_level(file, "environment"),
        Some(TopLevel {
            ty: "fn() ->> Vm".to_owned(),
            function: Some((ast::Effect::Memo, 0)),
        })
    );
    assert_eq!(
        annotated.top_level(file, "teardown"),
        Some(TopLevel {
            ty: "fn(env: Vm) -> {}".to_owned(),
            function: Some((ast::Effect::Impure, 1)),
        })
    );
    // Nothing has run.
    assert!(!workspace.memoised(pekit, "environment"));
    assert_eq!(
        workspace.check_composed(pekit, &["teardown", "environment"]),
        Ok(())
    );
    assert_eq!(logged(&log), "");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn memoised_says_whether_a_memoised_call_completed() {
    let (dir, log) = log_in("memoised");
    let files = environment(&log);
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    assert!(!workspace.memoised(pekit, "environment"));
    // `pekit.gcsl` runs, as `p.gcsl` needs it, but nothing calls
    // `environment`.
    let plain = workspace.invoke("run", "plain", &[]).unwrap();
    assert_eq!(plain.value.to_string(), "plain");
    assert!(!workspace.memoised(pekit, "environment"));
    // Called: it has returned, once, across invocations of the one
    // workspace.
    for _ in 0..2 {
        let uses = workspace.invoke("run", "uses", &[]).unwrap();
        assert_eq!(uses.value.to_string(), "uses 7");
        assert!(workspace.memoised(pekit, "environment"));
    }
    assert_eq!(logged(&log), "boot\n");
    // Not a memoised function, or no function, or none: never.
    for name in ["teardown", "x", "private", "nope"] {
        assert!(!workspace.memoised(pekit, name), "{name}");
    }
    // Another workspace has tables of its own.
    let fresh = rooted(&files).unwrap();
    assert!(!fresh.memoised(pekit, "environment"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_memoised_call_that_failed_or_read_another_caller_is_not_found() {
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }",
        ),
        (
            "pekit.gcsl",
            "import fileroot\n\
                 def environment() ->> String { panic(\"no vm\") }\n\
                 def called() ->> String { fileroot.caller }",
        ),
        (
            "pkg/p.gcsl",
            "import pekit\n\
                 def run boots() { pekit.environment() }\n\
                 def run reads() { pekit.called() }",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    assert!(workspace.invoke("run", "boots", &[]).is_err());
    assert!(!workspace.memoised(pekit, "environment"));
    // Its value depends on its caller, `pkg`, which a call written at
    // the root is not: such a call would run it again.
    let read = workspace.invoke("run", "reads", &[]).unwrap();
    assert_eq!(read.value.to_string(), format!("{SCRATCH}/pkg"));
    assert!(!workspace.memoised(pekit, "called"));
    // Called from the root, it is found.
    let mut rooted_call = rooted(&files).unwrap();
    let root = rooted_call.call_composed(pekit, &["called"]).unwrap();
    assert_eq!(root.value.to_string(), SCRATCH);
    assert!(rooted_call.memoised(pekit, "called"));
}

#[test]
fn memoised_calls_of_no_arguments_from_several_directories_are_told_apart() {
    // A memoised function of no parameters reading its caller, called
    // from two directories, and again from the first: one entry for
    // each, both with an empty key, each completing, and each found.
    let (dir, log) = log_in("empty-keys");
    let lib = format!(
        "import fileroot\nimport path\n\
             def called() ->> String {{\n\
             \x20   let out = sh(\"echo ran {{fileroot.caller}} >> {log}\")\n\
             \x20   \"{{fileroot.caller}}{{out}}\"\n\
             }}\n\
             def cycle() ->> String {{\n\
             \x20   let caller = fileroot.caller\n\
             \x20   let out = sh(\"echo cycle {{caller}} >> {log}\")\n\
             \x20   fileroot.helper()\n\
             \x20   if caller == path.join(fileroot.workspace, \"b\") {{ cycle() }} else {{ \"{{caller}}{{out}}\" }}\n\
             }}"
    );
    let files = [
        (
            "workspace.gcsl",
            "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }",
        ),
        ("lib.gcsl", lib.as_str()),
        (
            "a/a.gcsl",
            "import lib\ndef run a() { lib.called() }\ndef run ca() { lib.cycle() }",
        ),
        (
            "b/b.gcsl",
            "import lib\ndef run b() { lib.called() }\ndef run cb() { lib.cycle() }",
        ),
        ("top.gcsl", "import lib\ndef run top() { lib.called() }"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let lib = workspace.file("lib.gcsl").unwrap();
    let invoked = |workspace: &mut Workspace, name: &str| {
        let outcome = workspace.invoke("run", name, &[]).unwrap();
        outcome.value.to_string().replace(SCRATCH, "<root>")
    };
    for _ in 0..2 {
        assert_eq!(invoked(&mut workspace, "a"), "<root>/a");
        assert_eq!(invoked(&mut workspace, "b"), "<root>/b");
    }
    assert_eq!(
        logged(&log).replace(SCRATCH, "<root>"),
        "ran <root>/a\nran <root>/b\n"
    );
    // Neither entry is a call from the root's.
    assert!(!workspace.memoised(lib, "called"));
    assert_eq!(invoked(&mut workspace, "top"), "<root>");
    assert!(workspace.memoised(lib, "called"));
    for name in ["a", "b", "top", "a"] {
        invoked(&mut workspace, name);
    }
    assert_eq!(
        logged(&log).replace(SCRATCH, "<root>"),
        "ran <root>/a\nran <root>/b\nran <root>\n"
    );
    // A call of no arguments depending on its own result is still one,
    // beside an entry of the same key that completed, which it leaves.
    assert_eq!(invoked(&mut workspace, "ca"), "<root>/a");
    let Err(InvokeError::Diagnostics(errors)) = workspace.invoke("run", "cb", &[]) else {
        panic!("expected `cb` to fail");
    };
    assert!(
        errors
            .iter()
            .any(|error| error.message == "this call depends on its own result"),
        "{errors:?}"
    );
    assert_eq!(invoked(&mut workspace, "ca"), "<root>/a");
    assert_eq!(
        logged(&log).replace(SCRATCH, "<root>"),
        "ran <root>/a\nran <root>/b\nran <root>\ncycle <root>/a\ncycle <root>/b\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_composition_of_top_level_functions_is_called_by_name() {
    let (dir, log) = log_in("composed");
    let files = environment(&log);
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let mut workspace = rooted(&files).unwrap();
    let pekit = workspace.file("pekit.gcsl").unwrap();
    let uses = workspace.invoke("run", "uses", &[]).unwrap();
    assert_eq!(uses.value.to_string(), "uses 7");
    // The inner call takes what the first returned: no second boot.
    let down = workspace
        .call_composed(pekit, &["teardown", "environment"])
        .unwrap();
    assert_eq!(down.value.to_string(), "");
    assert_eq!(logged(&log), "boot\ndown 7\n");
    // Called alone, run again, as any call.
    workspace.call_composed(pekit, &["environment"]).unwrap();
    assert_eq!(logged(&log), "boot\ndown 7\n");
    // A composition of none is `{}`, and runs nothing.
    let none = workspace.call_composed(pekit, &[]).unwrap();
    assert_eq!(none.value, Value::Empty);
    // In a workspace where nothing has run, the call runs the file
    // first, and boots.
    let mut fresh = rooted(&files).unwrap();
    fresh
        .call_composed(pekit, &["teardown", "environment"])
        .unwrap();
    assert_eq!(logged(&log), "boot\ndown 7\nboot\ndown 7\n");
    // A name that is no top-level function of the file, before anything
    // runs.
    for name in ["nope", "x", "private"] {
        assert_eq!(
            fresh.call_composed(pekit, &["teardown", name]),
            Err(InvokeError::Request(RequestError::NoFunction {
                file: "pekit.gcsl".to_owned(),
                name: name.to_owned(),
            })),
            "{name}"
        );
    }
    assert_eq!(
        RequestError::NoFunction {
            file: "pekit.gcsl".to_owned(),
            name: "x".to_owned(),
        }
        .message(),
        "`pekit.gcsl` has no top-level function `x`"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_composition_is_checked_as_a_call_is() {
    // Each `teardown` with the `environment` it takes, or does not.
    let check = |environment: &str, teardown: &str| {
        let pekit = format!("type Vm = {{pid: Int}}\n{environment}\n{teardown}");
        let files = [("workspace.gcsl", "x = 1"), ("pekit.gcsl", pekit.as_str())];
        let mut workspace = rooted(&files).unwrap();
        let file = workspace.file("pekit.gcsl").unwrap();
        workspace
            .check_composed(file, &["teardown", "environment"])
            .map_err(|error| match error {
                InvokeError::Diagnostics(errors) => {
                    errors.into_iter().map(|e| e.message).collect::<Vec<_>>()
                }
                InvokeError::Request(error) => vec![error.message()],
            })
    };
    let memoised = "def environment() ->> Vm { { pid = 1 } }";
    for teardown in [
        "def teardown(env: Vm) -> {} { {} }",
        "def teardown(env: Vm) ->> {} { {} }",
        "def teardown(env: {pid: Int}) -> Int { env.pid }",
        "def teardown(env: Vm | String) -> {} { {} }",
    ] {
        assert_eq!(check(memoised, teardown), Ok(()), "{teardown}");
    }
    for teardown in [
        "def teardown(env: String) -> {} { {} }",
        "def teardown(env: {pid: Int, port: Int}) -> {} { {} }",
    ] {
        assert_eq!(
            check(memoised, teardown),
            Err(vec!["mismatched types".to_owned()]),
            "{teardown}"
        );
    }
    // An impure `environment`'s value too.
    assert_eq!(
        check(
            "def environment() -> Vm { { pid = 1 } }",
            "def teardown(env: Vm) ->> {} { {} }"
        ),
        Ok(())
    );
    // A `teardown` of no parameters takes nothing.
    let none = check(memoised, "def teardown() -> {} { {} }").unwrap_err();
    assert_eq!(none.len(), 1, "{none:?}");
}

// ----- Syntax errors -----

/// Every diagnostic of checking the workspace of `files`, as its
/// message, the path of its file and the source it is at.
fn every_error(files: &[(&str, &str)]) -> Vec<(String, String, String)> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (standard, checked) = check_with_standard(&sources, &[], "/");
    let mut shown = standard;
    let mut sorted = sources;
    sorted.sort_by(|a, b| a.name().cmp(b.name()));
    shown.extend(sorted);
    checked
        .unwrap_err()
        .into_iter()
        .map(|error| {
            let source = error.input.and_then(|input| shown.get(input as usize));
            (
                error.message,
                source.map_or_else(String::new, |s| s.name().to_owned()),
                source.map_or_else(String::new, |s| s.slice(error.span).to_owned()),
            )
        })
        .collect()
}

/// `triples` as [`every_error`] gives them.
fn expected_errors(triples: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    triples
        .iter()
        .map(|&(message, path, at)| (message.to_owned(), path.to_owned(), at.to_owned()))
        .collect()
}

#[test]
fn a_syntax_error_hides_no_other_files_errors() {
    // `a` has a syntax error, and a type error where it parses; `b` an
    // unrelated type error; `c` reads `q`, which `a` declares where it
    // could not be parsed, and `d` reads `x`, whose value could not be.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "q . = 1\nx = (1 2)\ny: Int = \"s\""),
        ("b.gcsl", "z: Int = true"),
        ("c.gcsl", "w = q + 1"),
        ("d.gcsl", "v = x + 1\nu: Int = \"t\""),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("expected a field name after `.`", "a.gcsl", "="),
            ("expected `)`, found `2`", "a.gcsl", "2"),
            ("mismatched types", "a.gcsl", "\"s\""),
            ("mismatched types", "b.gcsl", "true"),
        ])
    );
}

#[test]
fn a_half_typed_item_leaves_the_next_line_to_be_checked() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "u.gcsl",
            "let a = 1 +\nlet b: Int = \"x\"\nlet c = f(1,, 2)\nlet e = 3\nx = \ny: Int = true",
        ),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("expected expression, found newline", "u.gcsl", "\n"),
            ("mismatched types", "u.gcsl", "\"x\""),
            ("expected expression, found `,`", "u.gcsl", ","),
            ("expected expression, found newline", "u.gcsl", "\n"),
            ("mismatched types", "u.gcsl", "true"),
        ])
    );
}

#[test]
fn a_name_that_names_nothing_is_an_error_where_no_broken_file_is_seen() {
    // `sub2/d` sees the root's files, not `sub1/a`.
    let files = [
        ("workspace.gcsl", ""),
        ("sub1/a.gcsl", "q . = 1"),
        ("sub2/d.gcsl", "w = nope + 1\nt: Nope = 1"),
        ("sub3/e.gcsl", "import sub1/a\nw = a.q + nope"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("expected a field name after `.`", "sub1/a.gcsl", "="),
            ("undefined variable `nope`", "sub2/d.gcsl", "nope"),
            ("unknown type `Nope`", "sub2/d.gcsl", "Nope"),
        ])
    );
}

#[test]
fn a_failed_check_leaves_the_recovered_trees_and_the_types_found() {
    let sources: Vec<SourceFile> = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = (1 2)\ny = 3"),
        ("b.gcsl", "z = 4"),
    ]
    .iter()
    .map(|&(path, text)| SourceFile::new(path, text).unwrap())
    .collect();
    let (standard, checked) = check_partial(&sources, &[], "/");
    let Err(Failed {
        diagnostics,
        partial: Some(partial),
    }) = checked
    else {
        panic!("{checked:?}");
    };
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let (a, b) = (standard.len(), standard.len() + 1);
    assert_eq!(partial.scan.files[a].path, "a.gcsl");
    assert_eq!(partial.scan.files[a].ast.syntax(), Syntax::Recovered);
    assert_eq!(partial.scan.files[b].ast.syntax(), Syntax::Clean);
    // `y` is in `a`'s tree; `a` failed, `b` was checked.
    assert_eq!(
        partial.scan.files[a]
            .ast
            .items(partial.scan.files[a].ast.root())
            .len(),
        2
    );
    assert!(partial.checked[a].is_none());
    assert!(partial.checked[b].is_some());
}

#[test]
fn an_unbalanced_file_is_not_checked() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x: Int = \"s\"\n}\ny = 1"),
        ("b.gcsl", "z: Int = true"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("unexpected `}`", "a.gcsl", "}"),
            ("mismatched types", "b.gcsl", "true"),
        ])
    );
}

#[test]
fn a_file_with_a_syntax_error_has_no_warnings() {
    // A list of an `Int` and a `String` is warned about, being of a
    // union; not in a file with a syntax error, whose warnings may be of
    // source that could not be parsed.
    let clean = [("workspace.gcsl", ""), ("a.gcsl", "x = 1\ny = [1, \"a\"]")];
    let sources: Vec<SourceFile> = clean
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    assert_eq!(check(&sources).unwrap().warnings().len(), 1);
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = 1 1\ny = [1, \"a\"]"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[("expected end of item, found `1`", "a.gcsl", "1")])
    );
}

#[test]
fn scan_errors_beside_a_syntax_error_are_reported_only_where_no_broken_file_is_seen() {
    // Two files of `sub2` define `k`; `sub1/a` has a syntax error and
    // two definitions of `j`, which may be the recovery's. The scan
    // recovers from the conflict, so `sub1/a` is checked as its parser
    // recovered it, and its check reports its own `j` twice.
    let files = [
        ("workspace.gcsl", ""),
        ("sub1/a.gcsl", "j = 1\nj = 2\nx = 1 1"),
        ("sub2/b.gcsl", "k = 1"),
        ("sub2/c.gcsl", "k = 2"),
    ];
    assert_eq!(
        every_error(&files),
        expected_errors(&[
            ("field `j` is defined twice", "sub1/a.gcsl", "j"),
            ("expected end of item, found `1`", "sub1/a.gcsl", "1"),
            ("field `k` is defined in two files", "sub2/c.gcsl", "k"),
        ])
    );
}

// ----- Scan errors -----

/// A file with a syntax error, in a directory of its own that nothing
/// imports, which each case of the scan's errors is checked with too.
const BROKEN: (&str, &str) = ("zz/broken.gcsl", "b = (1 2)");

/// Asserts that checking the workspace of `files` reports `expected`,
/// as [`expected_errors`] takes them, and with [`BROKEN`] added, those
/// and its syntax error: a scan error hides no other file's errors.
fn scan_case(files: &[(&str, &str)], expected: &[(&str, &str, &str)]) {
    assert_eq!(every_error(files), expected_errors(expected), "{files:?}");
    let mut broken = files.to_vec();
    broken.push(BROKEN);
    let mut wanted = expected.to_vec();
    wanted.push(("expected `)`, found `2`", "zz/broken.gcsl", "2"));
    assert_eq!(every_error(&broken), expected_errors(&wanted), "{broken:?}");
}

#[test]
fn a_field_defined_in_two_files_hides_no_other_files_errors() {
    // clackatoo's minimal case: `r`'s error is reported beside the
    // conflict, and `p` is checked.
    let files = [
        ("workspace.gcsl", "# Root."),
        ("p/p.gcsl", "h = 1\nk = h"),
        ("q/a.gcsl", "x = 1"),
        ("q/b.gcsl", "x = 2"),
        ("r/r.gcsl", "y: Int = \"s\""),
    ];
    scan_case(
        &files,
        &[
            ("field `x` is defined in two files", "q/b.gcsl", "x"),
            ("mismatched types", "r/r.gcsl", "\"s\""),
        ],
    );
    // The scan alone reports the conflict, and recovers from it.
    let errors = ws(&files).unwrap_err();
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(messages, ["field `x` is defined in two files"]);
    // The file asked for does not run.
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    assert!(check(&sources).is_err());
}

#[test]
fn a_conflicting_field_is_dropped_and_read_as_erroneous_everywhere() {
    // `q/b`'s `x` and `d` are dropped, `x` still checked as written;
    // every read of the conflicted `x` and `d`, in its directory, in a
    // directory below, or through an import, is erroneous, silently.
    let files = [
        ("workspace.gcsl", ""),
        ("q/a.gcsl", "x = 1\nd.e = 1"),
        ("q/b.gcsl", "x: Int = \"s\"\nd = 2\nown: Int = true"),
        ("q/c.gcsl", "u: String = x\nv: Bool = d"),
        ("q/sub/s.gcsl", "t: Bool = x + 1"),
        (
            "s/s.gcsl",
            "import x, d from q\nn: String = x\nm: Bool = d\nl: List<Int> = x\nz: Int = true",
        ),
    ];
    scan_case(
        &files,
        &[
            ("field `x` is defined in two files", "q/b.gcsl", "x"),
            ("mismatched types", "q/b.gcsl", "\"s\""),
            ("field `d` is already defined", "q/b.gcsl", "d"),
            ("mismatched types", "q/b.gcsl", "true"),
            ("mismatched types", "s/s.gcsl", "true"),
        ],
    );
}

#[test]
fn a_call_of_a_conflicting_def_types_nothing_of_its_arguments() {
    // The `def` would have typed `[]`: an erroneous callee leaves it
    // untyped, which is no error of its own.
    let files = [
        ("workspace.gcsl", ""),
        ("q/a.gcsl", "def f(xs: List<Int>): Int { 1 }"),
        ("q/b.gcsl", "def f(xs: List<Int>): Int { 2 }"),
        (
            "s/s.gcsl",
            "import f from q\nr = f([])\nt: List<Int> = f(1)",
        ),
        ("q/c.gcsl", "u = f([], [])"),
    ];
    scan_case(
        &files,
        &[("field `f` is defined in two files", "q/b.gcsl", "f")],
    );
}

#[test]
fn a_three_way_conflict_keeps_the_first() {
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "x = 1"),
        ("b.gcsl", "x = \"b\""),
        ("c.gcsl", "x = true\ny: Int = x"),
    ];
    scan_case(
        &files,
        &[
            ("field `x` is defined in two files", "b.gcsl", "x"),
            ("field `x` is defined in two files", "c.gcsl", "x"),
        ],
    );
}

#[test]
fn an_import_of_no_module_binds_erroneous_names() {
    // What the imports would bind reads as `Error`; everything else of
    // the file is checked. A `*` of nothing leaves the names it would
    // bind unknown: a name that names nothing is no error there.
    let files = [
        ("workspace.gcsl", ""),
        (
            "m/m.gcsl",
            "import nope/thing\nimport a, b as c from nada\nv = thing.x + 1\n\
                 w: Bool = c + a\nt: thing.T = 1\nz = zork\ny: Int = \"s\"",
        ),
        (
            "m/n.gcsl",
            "import * from gone\nk = whatever + 1\nj: Int = \"s\"",
        ),
        ("o/o.gcsl", "import f from lib\nr = f + 1"),
        ("lib/l.gcsl", "g = 1"),
    ];
    scan_case(
        &files,
        &[
            ("no module `nope/thing`", "m/m.gcsl", "nope/thing"),
            ("no module `nada`", "m/m.gcsl", "nada"),
            ("undefined variable `zork`", "m/m.gcsl", "zork"),
            ("mismatched types", "m/m.gcsl", "\"s\""),
            ("no module `gone`", "m/n.gcsl", "gone"),
            ("mismatched types", "m/n.gcsl", "\"s\""),
            ("`lib` has no `f`", "o/o.gcsl", "f"),
        ],
    );
}

#[test]
fn files_that_need_each_other_are_not_checked_and_the_rest_is() {
    // `d` needs a file of the cycle, and is not checked either.
    let files = [
        ("workspace.gcsl", ""),
        ("cy/a.gcsl", "x = y + 1\nbad: Int = \"a\""),
        ("cy/b.gcsl", "y = x + 1"),
        ("d/d.gcsl", "import x from cy\nr = x\nq: Int = \"bad\""),
        ("e/e.gcsl", "z: Int = \"s\""),
    ];
    scan_case(
        &files,
        &[
            (
                "`cy/a.gcsl` and `cy/b.gcsl` need each other",
                "cy/a.gcsl",
                "y",
            ),
            ("mismatched types", "e/e.gcsl", "\"s\""),
        ],
    );
}

#[test]
fn a_type_imported_as_a_built_in_types_name_binds_nothing() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/t.gcsl", "type Foo = {a: Int}"),
        ("m.gcsl", "import Foo as Int from lib/t\nk: Int = 1"),
        ("o.gcsl", "z: Int = true"),
    ];
    scan_case(
        &files,
        &[
            ("`Int` is a built-in type", "m.gcsl", "Int"),
            ("mismatched types", "o.gcsl", "true"),
        ],
    );
}

#[test]
fn a_type_declared_in_two_files_is_erroneous_where_used() {
    let files = [
        ("workspace.gcsl", ""),
        ("t/a.gcsl", "type Foo = {a: Int}"),
        ("t/b.gcsl", "type Foo = String\nv: Foo = 1"),
        ("t/c.gcsl", "w: Foo = true"),
        ("o.gcsl", "z: Int = true"),
    ];
    scan_case(
        &files,
        &[
            ("mismatched types", "o.gcsl", "true"),
            ("type `Foo` is declared in two files", "t/b.gcsl", "Foo"),
        ],
    );
}

#[test]
fn a_conflict_of_a_view_is_erroneous_in_the_files_that_see_it() {
    // `a/b`'s `k` meets the `k` of the directory above, reported at
    // the directory above's: each file of `a/b` reads `k` as `Error`,
    // and `a`'s files read their own.
    let files = [
        ("workspace.gcsl", ""),
        ("a/x.gcsl", "k = 1\nm: String = k"),
        ("a/b/y.gcsl", "k = \"s\"\nu: Int = k"),
        ("a/b/z.gcsl", "w: Bool = k.f"),
    ];
    scan_case(
        &files,
        &[
            ("field `k` is defined in two files", "a/x.gcsl", "k"),
            ("mismatched types", "a/x.gcsl", "k"),
        ],
    );
}

#[test]
fn a_meta_declared_twice_is_dropped() {
    let files = [
        TAGGED,
        (
            "app/a.gcsl",
            "def tagged x() { \"x\" }\ndefmeta tagged x { tag = \"t\"; n = 1 }",
        ),
        ("app/b.gcsl", "defmeta tagged x { tag = \"u\"; n = 2 }"),
        ("o.gcsl", "z: Int = true"),
    ];
    let errors: Vec<String> = every_error(&files)
        .into_iter()
        .map(|(message, path, _)| format!("{message} at {path}"))
        .collect();
    assert_eq!(
        errors,
        [
            "`defmeta tagged x` is declared twice in this directory at app/b.gcsl",
            "mismatched types at o.gcsl",
        ]
    );
}

/// Each diagnostic of checking the workspace of `files`, as `MESSAGE at
/// FILE`, with its suggestion as `WRITTEN -> REPLACEMENT`, the text it
/// replaces read from its file at its span, if it has one.
fn suggestions(files: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
    let mut sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    sources.sort_by(|a, b| a.name().cmp(b.name()));
    let (standard, checked) = check_with_standard(&sources, &[], "/");
    let Err(errors) = checked else {
        panic!("{files:?}: checked without an error");
    };
    let mut all = standard;
    all.extend(sources);
    errors
        .iter()
        .map(|error| {
            let file = error.input.and_then(|input| all.get(input as usize));
            let name = file.map_or("?", SourceFile::name);
            let shown = error.suggestion.as_ref().map(|found| {
                let written = file.map_or("?", |file| file.slice(found.span));
                format!("{written} -> {}", found.replacement)
            });
            (format!("{} at {name}", error.message), shown)
        })
        .collect()
}

/// The suggestion of the one diagnostic of checking `files`, as
/// [`suggestions`] shows it, its message starting with `message`.
fn suggestion_of(files: &[(&str, &str)], message: &str) -> Option<String> {
    let found = suggestions(files);
    assert_eq!(found.len(), 1, "{found:?}");
    let (shown, suggestion) = found.into_iter().next().unwrap();
    assert!(shown.starts_with(message), "{shown}");
    suggestion
}

#[test]
fn an_import_of_no_module_suggests_one_at_its_level() {
    let lib = (
        "lib/util.gcsl",
        "version = 1\ntype Package = {name: String}",
    );
    let with = |text| [("workspace.gcsl", ""), lib, ("app.gcsl", text)];
    assert_eq!(
        suggestion_of(
            &with("import lib/utl\nx = 1"),
            "no module `lib/utl` at app.gcsl"
        ),
        Some("utl -> util".to_owned())
    );
    // A segment before the last: a directory.
    assert_eq!(
        suggestion_of(&with("import lbi/util\nx = 1"), "no module `lbi/util`"),
        Some("lbi -> lib".to_owned())
    );
    // One segment: a standard module too.
    assert_eq!(
        suggestion_of(&with("import pth\nx = 1"), "no module `pth`"),
        Some("pth -> path".to_owned())
    );
    // A member: a root or a type of the module.
    assert_eq!(
        suggestion_of(
            &with("import verison from lib/util\nx = 1"),
            "`lib.util` has no"
        ),
        Some("verison -> version".to_owned())
    );
    assert_eq!(
        suggestion_of(
            &with("import Pakage from lib/util\nx = 1"),
            "`lib.util` has no"
        ),
        Some("Pakage -> Package".to_owned())
    );
    // Too far: none.
    assert_eq!(
        suggestion_of(&with("import nothing\nx = 1"), "no module `nothing`"),
        None
    );
}

#[test]
fn a_name_of_the_view_is_suggested() {
    // A root of a directory above, a node's child, a type of the
    // directory.
    let files = [
        ("workspace.gcsl", "top = 1\ndev.peios.count = 2"),
        ("sub/a.gcsl", "x = tpo + 1"),
    ];
    assert_eq!(
        suggestion_of(&files, "undefined variable `tpo` at sub/a.gcsl"),
        Some("tpo -> top".to_owned())
    );
    let files = [
        ("workspace.gcsl", "top = 1\ndev.peios.count = 2"),
        ("sub/a.gcsl", "x = dev.peios.cuont + 1"),
    ];
    assert_eq!(
        suggestion_of(&files, "no field `cuont` in"),
        Some("cuont -> count".to_owned())
    );
    let files = [
        ("workspace.gcsl", ""),
        ("lib/types.gcsl", "type Package = {name: String}"),
        ("lib/a.gcsl", "p: Pakage = {name = \"n\"}"),
    ];
    assert_eq!(
        suggestion_of(&files, "unknown type `Pakage`"),
        Some("Pakage -> Package".to_owned())
    );
    // A meta's field.
    let files = [
        INFO,
        (
            "app/a.gcsl",
            "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev).pieos",
        ),
    ];
    assert_eq!(
        suggestion_of(&files, "the meta of `info dev` has no field `pieos`"),
        Some("pieos -> peios".to_owned())
    );
}

#[test]
fn a_failed_workspace_file_keeps_its_runtime_diagnostic() {
    let sources = [SourceFile::new("main.gcsl", "1 // 0").unwrap()];
    let mut workspace = check(&sources).unwrap();
    let main = workspace.file("main.gcsl").unwrap();
    let first = workspace.run(main).unwrap_err();
    assert_eq!(first[0].message, "division by zero");
    assert_eq!(first[0].input, Some(main));
    assert_eq!(workspace.run(main).unwrap_err(), first);
    assert_eq!(workspace.run(main).unwrap_err(), first);
}

#[test]
fn failed_files_do_not_repeat_effects_or_run_dependents() {
    let dir = fs_scratch("failed-workspace-file");
    let marker = format!("{dir}/attempts");
    let dependent_marker = format!("{dir}/dependent");
    let failed = format!(
        "import fs\n\
         let recorded = fs.write({marker:?}, if fs.exists({marker:?}) {{\n\
             fs.read({marker:?}) + \"x\"\n\
         }} else {{ \"x\" }})\n\
         answer = 1 // 0"
    );
    let dependent = format!(
        "import fs\n\
         let recorded = fs.write({dependent_marker:?}, \"ran\")\n\
         = answer"
    );
    let sources = [
        SourceFile::new("broken.gcsl", failed).unwrap(),
        SourceFile::new("dependent.gcsl", dependent).unwrap(),
        SourceFile::new("unrelated.gcsl", "= 42").unwrap(),
    ];
    let mut workspace = check(&sources).unwrap();
    let broken = workspace.file("broken.gcsl").unwrap();
    let dependent = workspace.file("dependent.gcsl").unwrap();
    let unrelated = workspace.file("unrelated.gcsl").unwrap();
    let first = workspace.run(broken).unwrap_err();
    assert_eq!(first[0].message, "division by zero");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    assert_eq!(workspace.run(broken).unwrap_err(), first);
    assert_eq!(workspace.run(dependent).unwrap_err(), first);
    assert_eq!(workspace.run(unrelated).unwrap().value, Value::Int(42));
    assert_eq!(workspace.run(dependent).unwrap_err(), first);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    assert!(!std::path::Path::new(&dependent_marker).exists());
    drop(workspace);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The attributes of the closure that field `name` of `value`, a record,
/// holds, each as its name and its arguments shown, `on = true`.
fn attributes_of(value: &Value, name: &str) -> Vec<String> {
    let Value::Record(record) = value else {
        panic!("{value} is no record");
    };
    let at = record
        .layout
        .texts
        .iter()
        .position(|text| &**text == name)
        .unwrap();
    let Value::Fn(closure) = &record.values[at] else {
        panic!("{name} is no closure");
    };
    closure
        .attributes()
        .iter()
        .map(|attribute| {
            let args: Vec<String> = attribute
                .args
                .iter()
                .map(|(field, constant)| format!("{field} = {constant:?}"))
                .collect();
            let id = match &attribute.id {
                crate::ast::AttributeId::Declared { module, name } => format!("{module}.{name}"),
                crate::ast::AttributeId::Builtin(name) => (*name).to_owned(),
            };
            format!("{id}({})", args.join(", "))
        })
        .collect()
}

#[test]
fn a_function_s_attributes_reach_its_closures_with_their_declarations() {
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] type hermetic = {on: Bool = true}\n\
             #[hermetic] #[retry(2)]\n\
             def build() -> Int { 1 }\n\
             def plain(): Int { 2 }",
        ),
        (
            "lib/attrs.gcsl",
            "#[attribute] type scratch = {keep: Size = 1GiB}",
        ),
        (
            "app/main.gcsl",
            "import scratch from lib/attrs\n#[scratch(512MiB)] def go(): Int { 1 }",
        ),
    ];
    let root = rooted_value(&files, "workspace.gcsl");
    assert_eq!(
        attributes_of(&root, "build"),
        [
            "workspace.hermetic(on = Bool(true))",
            "retry(times = Int(2))"
        ]
    );
    assert!(attributes_of(&root, "plain").is_empty());
    // By the module that declares it, wherever it is used.
    assert_eq!(
        attributes_of(&rooted_value(&files, "app/main.gcsl"), "go"),
        ["lib/attrs.scratch(keep = Quantity(Size, 536870912))"]
    );
    // One not imported is unknown there.
    let files = [
        files[0],
        files[1],
        ("app/main.gcsl", "#[scratch] def go(): Int { 1 }"),
    ];
    assert_eq!(rooted_messages(&files), ["unknown attribute `scratch`"]);
}

#[test]
fn a_def_type_s_attributes_are_defaults_for_its_definitions() {
    let files = [
        (
            "workspace.gcsl",
            "#[attribute] type hermetic = {on: Bool = true}\n\
             #[def_type] #[hermetic]\n\
             def test(name: String, f: fn(): Int): fn(): Int { f }",
        ),
        (
            "tests/t.gcsl",
            "def test a() { 1 }\n\
             #[hermetic(false)] def test b() { 2 }\n\
             #[retry(3)] def test c() { 3 }\n\
             ra = (test a)()\nrb = (test b)()\nrc = (test c)()",
        ),
    ];
    let value = rooted_value(&files, "tests/t.gcsl");
    assert_eq!(
        attributes_of(&value, "ra"),
        ["workspace.hermetic(on = Bool(true))"]
    );
    // Replaced whole by its own.
    assert_eq!(
        attributes_of(&value, "rb"),
        ["workspace.hermetic(on = Bool(false))"]
    );
    // Its own others after the def-type's.
    assert_eq!(
        attributes_of(&value, "rc"),
        [
            "workspace.hermetic(on = Bool(true))",
            "retry(times = Int(3))"
        ]
    );
}

#[test]
fn a_generic_def_is_called_from_another_file_and_its_parameters_shadow_types() {
    let files = [
        ("workspace.gcsl", "type T = Int"),
        ("lib/box.gcsl", "def wrap<T>(x: T): {v: T} { v = x }"),
        (
            "app/main.gcsl",
            "import wrap from lib/box\nr = wrap(5).v + 1\ns = wrap(\"a\").v",
        ),
    ];
    // `T` in `wrap` is its parameter, not the root's `type T = Int`.
    assert_eq!(rooted_text(&files, "app/main.gcsl"), "{r = 6; s = \"a\"}");
}

#[test]
fn a_generic_type_is_its_definition_with_its_arguments_substituted() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib/types.gcsl",
            "type Pair<A, B> = {first: A, second: B}\ntype Named<T> = Pair<String, T>\n\
             type Id<T> = T\ntype F<T> = fn(x: T): T",
        ),
        (
            "app/main.gcsl",
            "import lib/types as t\nimport Named, Id, F from lib/types\n\
             p: t.Pair<Int, Bool> = { first = 1; second = true }\n\
             n: Named<List<Int>> = { first = \"a\"; second = [2] }\n\
             i: Id<Int> = 3\n\
             fn twice(x: Int): Int { x * 2 }\nf: F<Int> = twice\n\
             r = p.first + n.second[0] + i + f(1)",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{p = {first = 1; second = true}; n = {first = \"a\"; second = [2]}; i = 3; \
         f = fn(x: Int): Int; r = 8}"
    );
}

#[test]
fn a_generic_type_takes_as_many_arguments_as_it_has_parameters() {
    let lib = (
        "lib/types.gcsl",
        "type Pair<A, B> = {first: A, second: B}\ntype Box = {v: Int}",
    );
    for (main, message) in [
        (
            "import Pair from lib/types\nx: Pair = { first = 1 }",
            "type `Pair` needs its type arguments",
        ),
        (
            "import Pair from lib/types\nx: Pair<Int> = { first = 1 }",
            "type `Pair` takes 2 type arguments, given 1",
        ),
        (
            "import lib/types as t\nx: t.Pair<Int, Int, Int> = { first = 1 }",
            "type `t.Pair` takes 2 type arguments, given 3",
        ),
        (
            "import lib/types as t\nx: t.Pair = { first = 1 }",
            "type `t.Pair` needs its type arguments",
        ),
        (
            "import Box from lib/types\nx: Box<Int> = { v = 1 }",
            "type `Box` takes no type argument",
        ),
        // A type refers to itself through its own instance.
        ("type L<T> = {next: L<T>}", "type `L` refers to itself"),
        (
            "#[attribute]\ntype Tag<T> = {v: T}",
            "attribute `Tag` takes no type parameters",
        ),
    ] {
        let files = [("workspace.gcsl", ""), lib, ("app/main.gcsl", main)];
        assert_eq!(rooted_messages(&files), [message], "{main}");
    }
}

#[test]
fn a_generic_type_s_parameters_shadow_declared_types_and_instances_are_fresh() {
    let files = [
        ("workspace.gcsl", "type T = Int"),
        (
            "app/main.gcsl",
            "type Box<T> = {v: T}\n\
             a: Box<String> = { v = \"s\" }\n\
             b: Box<Box<Bool>> = { v = { v = true } }\n\
             fn get<U>(box: Box<U>): U { box.v }\n\
             r = get(a)\ns = get(b).v",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = {v = \"s\"}; b = {v = {v = true}}; r = \"s\"; s = true}"
    );
}

#[test]
fn a_def_method_is_visible_through_every_import_and_the_directory_chain() {
    let lib = (
        "lib/ints.gcsl",
        "def (x: Int) twice(): Int { x * 2 }\ndef (x: Int) add(y: Int = 10): Int { x + y }\n\
         fn (x: Int) secret(): Int { 0 }\nk = 1",
    );
    let files = [
        ("workspace.gcsl", ""),
        lib,
        (
            "app/helpers.gcsl",
            "def (s: String) shout(): String { \"{s}!\" }",
        ),
        (
            "app/whole.gcsl",
            "import lib/ints\nwhole = 3.twice() + 1.add()",
        ),
        (
            "app/member.gcsl",
            "import k from lib/ints\nmember = 4.twice() + k",
        ),
        (
            "app/star.gcsl",
            "import * from lib/ints\nstar = 5.add(y = 1)",
        ),
        ("app/sub/deep.gcsl", "deep = \"hi\".shout()"),
    ];
    for (path, value) in [
        ("app/whole.gcsl", "{whole = 17}"),
        ("app/member.gcsl", "{member = 9}"),
        ("app/star.gcsl", "{star = 6}"),
        ("app/sub/deep.gcsl", "{deep = \"hi!\"}"),
    ] {
        assert_eq!(rooted_text(&files, path), value, "{path}");
    }
    // A `fn` method is its file's alone.
    let files = [
        ("workspace.gcsl", ""),
        lib,
        ("app/main.gcsl", "import lib/ints\nbad = 3.secret()"),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["cannot access field `secret` of Int"]
    );
}

#[test]
fn methods_of_other_files_are_chosen_and_conflict_as_a_file_s_own() {
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib/any.gcsl",
            "def (xs: List<T>) size<T>(): String { \"any\" }",
        ),
        (
            "lib/ints.gcsl",
            "def (xs: List<Int>) size(): String { \"ints\" }",
        ),
        (
            "app/main.gcsl",
            "import lib/any\nimport lib/ints\na = [1].size()\nb = [true].size()",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"ints\"; b = \"any\"}"
    );
    let files = [
        ("workspace.gcsl", ""),
        ("lib/one.gcsl", "def (x: Int) m(): Int { 1 }"),
        ("lib/two.gcsl", "def (y: Int) m(): Int { 2 }"),
        ("app/main.gcsl", "import lib/one\nimport lib/two\nr = 3.m()"),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["method `m` is declared twice for type Int"]
    );
}

#[test]
fn a_call_of_a_field_needs_the_files_of_the_methods_of_its_name() {
    // Conservatively: the call is of a record's field, but a method of its
    // name is visible, so the file needs the method's, which needs it back.
    let files = [
        ("workspace.gcsl", ""),
        (
            "lib.gcsl",
            "import app/main\ndef (x: Int) get(): Int { x }\nseen = main.v",
        ),
        (
            "app/main.gcsl",
            "v = 1\nrec = { get = fn(): Int { 2 } }\nw = rec.get()",
        ),
    ];
    let messages = rooted_messages(&files);
    assert_eq!(messages, ["`app/main.gcsl` and `lib.gcsl` need each other"]);
}

#[test]
fn variants_are_visible_where_def_methods_are_and_conflict_at_a_use() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/tool.gcsl", "enum Tool extensible<{*: String}>"),
        (
            "lib/c.gcsl",
            "import Tool from lib/tool\nenum Tool::C<{cc: String}>",
        ),
        (
            "app/main.gcsl",
            "import Tool from lib/tool\nimport lib/c\nt = Tool::C({ cc = \"gcc\" })\nok = Tool::C.matches(t)",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{t = Tool::C({cc = \"gcc\"}); ok = true}"
    );
    // Without the import of the variant's module, it is not seen.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/tool.gcsl", "enum Tool extensible"),
        ("lib/c.gcsl", "import Tool from lib/tool\nenum Tool::C"),
        (
            "app/main.gcsl",
            "import Tool from lib/tool\nt = Tool::C({})",
        ),
    ];
    assert_eq!(rooted_messages(&files), ["no variant `C` of `Tool`"]);
    // Two files' variants of one name are one name too many.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/tool.gcsl", "enum Tool extensible"),
        ("lib/a.gcsl", "import Tool from lib/tool\nenum Tool::C"),
        ("lib/b.gcsl", "import Tool from lib/tool\nenum Tool::C"),
        (
            "app/main.gcsl",
            "import Tool from lib/tool\nimport lib/a\nimport lib/b\nt = Tool::C({})",
        ),
    ];
    assert_eq!(
        rooted_messages(&files),
        ["variant `Tool::C` is declared in two files"]
    );
}

#[test]
fn values_of_enums_are_made_compared_tested_and_unwrapped() {
    let text = "enum Tool extensible<{*: String}>\n\
                enum Tool::C<{cc: String}>\n\
                enum Tool::Rust<{rustc: String}>\n\
                fn get<T>(kind: Variant<Tool, T>, tools: List<Tool>, fallback: T): T {\n\
                \x20   for t in tools { if kind.matches(t) { return kind.payload(t) } }\n\
                \x20   fallback\n\
                }\n\
                let tools = [Tool::Rust({ rustc = \"rustc\" }), Tool::C({ cc = \"gcc\" })]\n\
                a = get(Tool::C, tools, { cc = \"none\" }).cc\n\
                b = get(Tool::Rust, tools, { rustc = \"none\" }).rustc\n\
                c = Tool::C({ cc = \"x\" }) == Tool::C({ cc = \"x\" })\n\
                d = Tool::C({ cc = \"x\" }) == Tool::Rust({ rustc = \"x\" })\n\
                e = Tool::C == Tool::C\n\
                f = Tool::C\n\
                g = tools[1]\n\
                let v: Tool | Int = 3\n\
                h = v is Tool\n\
                type P = {arch: String, os: String}\n\
                let p: Partial<P> = { arch = \"x86_64\" }\n\
                i = p.matches({ arch = \"x86_64\"; os = \"linux\" })\n\
                j = p.matches({ arch = \"arm\"; os = \"linux\" })";
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"gcc\"; b = \"rustc\"; c = true; d = false; e = true; f = Tool::C; \
         g = Tool::C({cc = \"gcc\"}); h = false; i = true; j = false}"
    );
    let files = [
        ("workspace.gcsl", ""),
        (
            "app/main.gcsl",
            "enum T extensible\nenum T::A<Int>\nenum T::B<Int>\nr = T::A.payload(T::B(1))",
        ),
    ];
    let mut workspace = rooted(&files).unwrap();
    let main = workspace.file("app/main.gcsl").unwrap();
    let error = workspace.run(main).unwrap_err().remove(0);
    assert_eq!(error.message, "the value is `T::B`, not `T::A`");
}

/// An enum of toolchains and a generic def-type whose definitions are made
/// with its variants, as a workspace's root declares them.
const TOOLCHAIN: (&str, &str) = (
    "workspace.gcsl",
    "enum Toolchain extensible<{*: String}>\n\
     enum Toolchain::C<{cc: String}>\n\
     enum Toolchain::Rust<{rustc: String}>\n\
     type Def<T> = {arch: String, provides: T}\n\
     type Registered = {arch: String, provides: Toolchain}\n\
     #[def_type]\n\
     def Toolchain<T>(name: String, kind: Variant<Toolchain, T>, f: fn(): Def<T>): Registered {\n\
     \x20   let d = f()\n\
     \x20   arch = d.arch\n\
     \x20   provides = kind(d.provides)\n\
     }",
);

#[test]
fn a_definition_made_with_a_variant_binds_its_def_type_s_type_parameters() {
    let files = [
        TOOLCHAIN,
        (
            "tools.gcsl",
            "def Toolchain::C gcc() {\n    arch = \"x86_64\"\n    provides = { cc = \"gcc\" }\n}\n\
             def Toolchain::Rust gcc() {\n    arch = \"aarch64\"\n    provides = { rustc = \"rustc\" }\n}",
        ),
        (
            "app/main.gcsl",
            "import Toolchain from workspace\nimport workspace as w\n\
             let c = Toolchain::C gcc()\nlet rust = w.Toolchain::Rust gcc()\n\
             cc = Toolchain::C.payload(c.provides).cc\nrustc = Toolchain::Rust.payload(rust.provides).rustc\n\
             arches = [c.arch, rust.arch]",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{cc = \"gcc\"; rustc = \"rustc\"; arches = [\"x86_64\", \"aarch64\"]}"
    );
    // Not listed to be invoked by name: its definitions are reached by
    // references alone.
    let workspace = rooted(&files).unwrap();
    assert!(workspace.def_types().unwrap().is_empty());
}

#[test]
fn variant_def_type_errors() {
    let errors = |text: &str| {
        let files = [TOOLCHAIN, ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    for (text, message) in [
        (
            "def Toolchain::C x() {\n    arch = \"a\"\n    provides = { cc = 1 }\n}",
            "mismatched types",
        ),
        (
            "def Toolchain x() { 1 }",
            "def-type `Toolchain` needs a variant: `def Toolchain::V name()`",
        ),
        (
            "def Toolchain::Nope x() { 1 }",
            "no variant `Nope` of `Toolchain`",
        ),
        (
            "r = Toolchain::C nope()",
            "`Toolchain::C` has no definition `nope`",
        ),
        (
            "#[def_type]\ndef pk(name: String, f: fn(): Int): Int { f() }\ndef pk::C x() { 1 }",
            "`pk` takes no variant",
        ),
        (
            "#[def_type]\ndef g<T>(name: String, f: fn(): T): Int { 1 }",
            "a generic def-type takes its definitions' variant after the name, as in `def \
             Toolchain<T>(name: String, kind: Variant<Toolchain, T>, func: fn(): T)`",
        ),
        (
            "#[def_type]\ndef g<T, U>(name: String, kind: Variant<Toolchain, T>, f: fn(): U): Int { 1 }",
            "type parameter `U` of a def-type must appear in its variant parameter's type",
        ),
        (
            "#[def_type]\ndef s(name: String, kind: Variant<Toolchain, {cc: String}>, f: fn(): Int): \
             Int { f() }\ndef s::Rust x() { 1 }",
            "the variant of a `s` definition is a `Variant<Toolchain, {cc: String}>`, not \
             `Variant<Toolchain, {rustc: String}>`",
        ),
    ] {
        let found = errors(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
}

/// Contexts of platforms, as a library declares them, and a function that
/// reads them.
const PLATFORMS: (&str, &str) = (
    "lib/plat.gcsl",
    "type Platform = {arch: String, os: String}\n\
     context arch: String = \"x86_64\"\n\
     context host_platform: Platform = { arch = ^arch; os = \"linux\" }\n\
     context platform: Platform = ^host_platform\n\
     context required: Int\n\
     def describe(): String { \"{^platform.arch}-{^platform.os}\" }\n\
     def tool(^arch): String { \"cc-{arch}\" }\n\
     def needs(): Int { ^required }",
);

#[test]
fn contexts_are_read_bound_and_defaulted_where_they_are_read() {
    let files = [
        ("workspace.gcsl", ""),
        PLATFORMS,
        (
            "app/main.gcsl",
            "import lib/plat\n\
             a = plat.describe()\n\
             b = plat.describe() with { host_platform = { arch = \"aarch64\"; os = \"linux\" } }\n\
             c = plat.describe() with { platform.arch = \"riscv64\"; platform.os = \"peios\" }\n\
             d = with { arch = \"arm\" } { plat.describe() }\n\
             e = plat.tool()\n\
             f = plat.tool() with { arch = \"mips\" }\n\
             g = plat.tool(arch = \"explicit\")\n\
             h = plat.needs() with { required = 3 }\n\
             i = ^platform.os",
        ),
    ];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = \"x86_64-linux\"; b = \"aarch64-linux\"; c = \"riscv64-peios\"; d = \"arm-linux\"; \
         e = \"cc-x86_64\"; f = \"cc-mips\"; g = \"cc-explicit\"; h = 3; i = \"linux\"}"
    );
    // A read of one with no value is a runtime error.
    let files = [
        ("workspace.gcsl", ""),
        PLATFORMS,
        ("app/main.gcsl", "import lib/plat\nr = plat.needs()"),
    ];
    let mut workspace = rooted(&files).unwrap();
    let main = workspace.file("app/main.gcsl").unwrap();
    let error = workspace.run(main).unwrap_err().remove(0);
    assert_eq!(error.message, "context `required` has no value here");
}

#[test]
fn a_future_keeps_its_context_and_memoised_calls_key_on_what_they_read() {
    let text = "context x: Int = 1\n\
                fn m(n: Int) ->> Int { n + ^x }\n\
                fn inner() ->> Int { ^x }\n\
                fn outer() ->> Int { inner() * 100 }\n\
                fn outer2() ->> Int { inner() * 1000 }\n\
                fn later() ->> Future<Int> { lazy { ^x } }\n\
                fn fixed() ->> Int { with { x = 7 } { inner() } }\n\
                fn force(v: Int): Int { v }\n\
                a = m(1)\n\
                b = m(1) with { x = 10 }\n\
                c = m(1)\n\
                d = inner() with { x = 5 }\n\
                e = outer()\n\
                f = outer() with { x = 5 }\n\
                g = outer2() with { x = 5 }\n\
                h = outer2()\n\
                i = force(later())\n\
                j = force(later()) with { x = 42 }\n\
                k = fixed()\n\
                l = fixed() with { x = 9 }\n\
                let made = lazy { ^x } with { x = 99 }\n\
                n = force(made)";
    let files = [("workspace.gcsl", ""), ("app/main.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app/main.gcsl"),
        "{a = 2; b = 11; c = 2; d = 5; e = 100; f = 500; g = 5000; h = 1000; i = 1; j = 42; \
         k = 7; l = 7; n = 99}"
    );
}

#[test]
fn context_errors() {
    let errors = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    for (text, message) in [
        ("r = ^nope", "no context `nope` is declared"),
        (
            "context arch: String = \"x\"\nr = ^arhc",
            "no context `arhc` is declared",
        ),
        ("context x: Int\nr = ^x", "context `x` has no value here"),
        (
            "context x: Int = 1\ncontext x: Int = 2",
            "context `x` is declared twice",
        ),
        ("context x: Int = \"s\"", "mismatched types"),
        (
            "context x: Int = 1\nr = 1 with { x = \"s\" }",
            "mismatched types",
        ),
        (
            "context p: {a: Int} = {a = 1}\nr = 1 with { p.b = 2 }",
            "context `p` has no field `b`",
        ),
        (
            "context x: Int = 1\nr = 1 with { x = 2; x = 3 }",
            "context `x` is bound twice in this `with`",
        ),
        (
            "context x: Int = 1\nr = 1 with { y = 2 }",
            "no context `y` is declared",
        ),
        (
            "fn imp() -> Int { 1 }\ncontext x: Int = imp()",
            "the default of a context must be pure",
        ),
        ("context x: Nope = 1", "unknown type `Nope`"),
        ("fn f(^nope): Int { 1 }", "no context `nope` is declared"),
    ] {
        let found = errors(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
    // A context is declared once in a workspace.
    let files = [
        ("workspace.gcsl", ""),
        ("a.gcsl", "context x: Int = 1"),
        ("b.gcsl", "context x: Int = 2"),
    ];
    assert!(
        check_errors(&files)
            .join("\n")
            .starts_with("context `x` is declared twice")
    );
}

#[test]
fn a_context_s_default_is_scheduled_with_what_it_reads() {
    let files = [
        ("workspace.gcsl", ""),
        ("app.gcsl", "r = ^a\ncontext a: Int = b + 1\nb = 3\ns = ^a"),
    ];
    assert_eq!(rooted_text(&files, "app.gcsl"), "{r = 4; b = 3; s = 4}");
}

/// A def-type whose definitions read contexts, and a file of a platform's
/// record, to give an invocation its context with.
const HOSTED: [(&str, &str); 4] = [
    (
        "workspace.gcsl",
        "type Platform = {arch: String, os: String}\n\
         context host_platform: Platform = { arch = \"x86_64\"; os = \"linux\" }\n\
         context label: String = \"default\"\n\
         #[def_type]\n\
         def build(name: String, f: fn() -> String) -> String { f() }\n\
         def tagged(n: Int) ->> String { \"{n}:{^label}\" }\n\
         top = ^label",
    ),
    (
        "app.gcsl",
        "def build x() { \"{^host_platform.arch}-{tagged(1)}-{top}\" }",
    ),
    ("plat.gcsl", "arch = \"aarch64\"\nos = \"linux\""),
    ("other.gcsl", "n = 1"),
];

#[test]
fn an_invocation_runs_in_the_context_its_host_sets() {
    let mut workspace = rooted(&HOSTED).unwrap();
    let plat = workspace.file("plat.gcsl").unwrap();
    let platform = workspace.run(plat).unwrap().value;
    let set = |label: &str| {
        vec![
            ("host_platform".to_owned(), platform.clone()),
            ("label".to_owned(), Value::Str(Rc::from(label))),
        ]
    };
    let invoked = |workspace: &mut Workspace, contexts: &[(String, Value)]| {
        let outcome = workspace.invoke_with(("build", "x"), &[], contexts);
        outcome.unwrap().value.to_string()
    };
    // What a file computes reads the defaults, whatever the invocation.
    assert_eq!(
        invoked(&mut workspace, &set("first")),
        "aarch64-1:first-default"
    );
    // A memoised call is not found from another context.
    assert_eq!(
        invoked(&mut workspace, &set("second")),
        "aarch64-1:second-default"
    );
    assert_eq!(invoked(&mut workspace, &[]), "x86_64-1:default-default");
    let error = |contexts: Vec<(String, Value)>| {
        let mut workspace = rooted(&HOSTED).unwrap();
        match workspace.invoke_with(("build", "x"), &[], &contexts) {
            Err(InvokeError::Request(error)) => {
                let mut shown = vec![error.message()];
                shown.extend(error.notes());
                shown.join("; ")
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        error(vec![("lable".to_owned(), Value::Int(1))]),
        "no context `lable` is declared; the contexts are `host_platform`, `label`"
    );
    assert_eq!(
        error(vec![("label".to_owned(), Value::Int(1))]),
        "the value given for context `label` is not a `String`; a context is given data of \
         its type: no function or future"
    );
    let twice = vec![
        ("label".to_owned(), Value::Str(Rc::from("a"))),
        ("label".to_owned(), Value::Str(Rc::from("b"))),
    ];
    assert_eq!(error(twice), "context `label` is given more than once");
    // A record lacking a field of the type is not of it.
    let mut workspace = rooted(&HOSTED).unwrap();
    let other = workspace.file("other.gcsl").unwrap();
    let wrong = workspace.run(other).unwrap().value;
    assert_eq!(
        error(vec![("host_platform".to_owned(), wrong)]),
        "the value given for context `host_platform` is not a `{arch: String, os: String}`; a \
         context is given data of its type: no function or future"
    );
}

#[test]
fn a_recover_catches_what_retry_would_retry_and_an_assertion_shows_its_operands() {
    let text = "fn check(x: Int): Int {\n\
                \x20   assert x > 0, \"x must be positive\"\n\
                \x20   x * 2\n\
                }\n\
                a = recover { check(3) } else { 0 }\n\
                b = recover { check(-1) } else (e) { e.message }\n\
                c = recover { panic(\"boom\") } else (failure) { \"caught {failure.message}\" }\n\
                d = recover { [1, 2][5] } else { -1 }\n\
                f = recover { recover { panic(\"inner\") } else (e) { panic(\"re: {e.message}\") } } \
                else (e) { e.message }\n\
                let s = \"hello\"\n\
                g = recover { assert s == \"hullo\"; \"passed\" } else (e) { e.message }\n\
                fn leave(): Int {\n\
                \x20   let _ = for i in [1, 2, 3] {\n\
                \x20       recover { if i == 2 { return i * 10 }; {} } else { {} }\n\
                \x20   }\n\
                \x20   0\n\
                }\n\
                h = leave()";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = 6; b = \"assertion failed: x must be positive\"; c = \"caught boom\"; d = -1; \
         f = \"re: inner\"; g = \"assertion failed: `s == \\\"hullo\\\"`\"; h = 20}"
    );
    // A failed assertion labels each side of its comparison with its value.
    let files = [
        ("workspace.gcsl", ""),
        ("app.gcsl", "let s = \"hello\"\nassert s == \"hullo\""),
    ];
    let mut workspace = rooted(&files).unwrap();
    let app = workspace.file("app.gcsl").unwrap();
    let error = workspace.run(app).unwrap_err().remove(0);
    assert_eq!(error.message, "assertion failed: `s == \"hullo\"`");
    let labels: Vec<&str> = error
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .collect();
    assert_eq!(labels, ["this is `\"hello\"`", "this is `\"hullo\"`"]);
}

#[test]
fn a_recover_inside_a_timed_call_leaves_its_timeout_to_the_call() {
    let text = "#[timeout(200ms)]\n\
                fn slow() -> String {\n\
                \x20   recover { let r = exec([\"sleep\", \"3\"]); \"slept {r.status}\" } else (e) { \
                \"inner caught: {e.message}\" }\n\
                }\n\
                r = recover { slow() } else (e) { \"outer caught: {e.message}\" }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{r = \"outer caught: `slow` timed out after 200ms\"}"
    );
}

#[test]
fn assertion_and_recover_errors() {
    let errors = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        check_errors(&files).join("\n")
    };
    for (text, message) in [
        ("assert 1", "mismatched types"),
        ("assert true, 1", "mismatched types"),
        (
            "r = recover { 1 } else (e) { e.nope }",
            "no field `nope` in `{message: String}`",
        ),
        (
            "fn imp() -> Int { 1 }\nfn p(): Int { recover { imp() } else { 0 } }",
            "cannot call an impure function from a pure function",
        ),
    ] {
        let found = errors(text);
        assert!(found.starts_with(message), "{text}: {found}");
    }
}

#[test]
fn what_is_computed_on_a_secret_is_a_secret_and_unsafe_value_takes_its_value() {
    let text = "let s = secret(\"hunter2\", \"password\")\n\
                let n = secret(41, \"answer\")\n\
                a = s + \"!\"\n\
                b = (s + \"!\").unsafe_value()\n\
                c = (n + 1).unsafe_value()\n\
                d = \"--token={s}\".unsafe_value()\n\
                e = (s == \"hunter2\").unsafe_value()\n\
                f = (-n).unsafe_value()\n\
                let o = secret({k = s; n = 1}, \"login\")\n\
                g = o.k.unsafe_value()\n\
                h = (o == {k = \"hunter2\"; n = 1}).unsafe_value()\n\
                i = ({k = s} == {k = \"guess\"}).unsafe_value()\n\
                j = ([s, s] == [\"hunter2\", \"hunter2\"]).unsafe_value()\n\
                k = secret([1, 2], \"list\")[1].unsafe_value()\n\
                l = show({k = s})\n\
                fn same<T>(x: T): T { x }\n\
                m = same(s)\n\
                enum B extensible<{*: Secret<String>}>\n\
                enum B::X<{a: Secret<String>}>\n\
                p = (B::X({a = s}) == B::X({a = secret(\"hunter2\", \"other\")})).unsafe_value()";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        value(&files, "app.gcsl"),
        "{a = ***; b = \"hunter2!\"; c = 42; d = \"--token=hunter2\"; e = true; f = -41; \
         g = \"hunter2\"; h = true; i = false; j = true; k = 2; l = \"{{k = ***}}\"; m = ***; \
         p = true}"
    );
}

#[test]
fn a_secret_computed_on_secrets_is_identified_by_theirs() {
    let text = "let s = secret(\"pw\", \"password\")\n\
                let u = secret(\"jack\", \"user\")\n\
                a = s\n\
                b = u + \":\" + s\n\
                c = \"{u}:{s}\"\n\
                d = {k = s} == {k = \"pw\"}\n\
                e = secret({k = s}, \"login\").k\n\
                f = secret(s, \"again\")";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    let Value::Record(record) = rooted_value(&files, "app.gcsl") else {
        panic!("not a record");
    };
    let identities: Vec<(&str, &str)> = record
        .layout
        .texts
        .iter()
        .zip(&record.values)
        .map(|(name, value)| match value {
            Value::Secret(secret) => (name.as_ref(), secret.identity.as_ref()),
            value => panic!("{name} is {value}, not a secret"),
        })
        .collect();
    assert_eq!(
        identities,
        [
            ("a", "password"),
            ("b", "user + password"),
            ("c", "user + password"),
            ("d", "password"),
            ("e", "login + password"),
            ("f", "again"),
        ]
    );
}

#[test]
fn a_secret_decides_nothing_and_is_not_its_value() {
    let errors = |text: &str| {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        let error = rooted(&files)
            .err()
            .unwrap_or_else(|| panic!("{text}: no error"));
        (error[0].message.clone(), error[0].notes.clone())
    };
    let prefix = "let s = secret(\"pw\", \"id\")\n";
    for (text, message) in [
        ("r = if s == \"pw\" { 1 } else { 2 }", "mismatched types"),
        (
            "r = s == \"pw\" and true",
            "a secret cannot decide what runs",
        ),
        (
            "r = true or s == \"pw\"",
            "a secret cannot decide what runs",
        ),
        (
            "let v: Secret<String> | Int = s\nr = v is Int",
            "a secret cannot decide what runs",
        ),
        (
            "let v: Secret<String> | Int = s\nr = v as Int",
            "a secret cannot decide what runs",
        ),
        (
            "r = for c in secret([1], \"l\") { c }",
            "cannot iterate over a value",
        ),
        ("assert s == \"pw\"", "mismatched types"),
        (
            "r = [1, 2][secret(0, \"i\")]",
            "a list index must be an Int",
        ),
        ("r = trim(s)", "mismatched types"),
        ("r = s + 1", "cannot add `Secret<String>` and Int"),
        (
            "r = {...secret({k = 1}, \"r\")}",
            "cannot spread a value of type",
        ),
        (
            "let r: Secret<Future<Int>> = secret(1, \"f\")",
            "a secret's value is present",
        ),
    ] {
        let (found, notes) = errors(&format!("{prefix}{text}"));
        assert!(found.starts_with(message), "{text}: {found}");
        if found == "mismatched types" {
            assert!(
                notes.iter().any(|note| note.contains("`.unsafe_value()`")),
                "{text}: {notes:?}"
            );
        }
    }
}

#[test]
fn a_failure_on_a_secret_names_nothing_and_nothing_recovers_it() {
    for text in [
        "let n = secret(0, \"zero\")\nr = recover { 1 // n } else (e) { e.message }",
        "let l = secret([1, 2], \"l\")\nr = recover { l[5] } else { 0 }",
        "let n = secret(9223372036854775807, \"big\")\nr = recover { n + 1 } else { 0 }",
        "let n = secret(0, \"zero\")\nr = recover { (n + 1) // (n * 2) } else { 0 }",
    ] {
        let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
        let mut workspace = rooted(&files).unwrap();
        let app = workspace.file("app.gcsl").unwrap();
        let error = workspace.run(app).unwrap_err().remove(0);
        assert_eq!(error.message, "an operation on a secret failed", "{text}");
        assert!(error.labels.is_empty(), "{text}: {:?}", error.labels);
        assert!(error.notes.is_empty(), "{text}: {:?}", error.notes);
    }
}

#[test]
fn a_failure_of_an_operand_of_an_operation_on_a_secret_is_the_operand_s() {
    let text = "let s = secret(\"pw\", \"id\")\n\
                a = recover { s + panic(\"boom\") } else (e) { e.message }\n\
                b = recover { \"{s}{[\"x\"][3]}\" } else (e) { e.message }\n\
                let n = secret(0, \"zero\")\n\
                c = recover { (1 // n) + 1 } else { 0 }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    let mut workspace = rooted(&files).unwrap();
    let app = workspace.file("app.gcsl").unwrap();
    // The operand's own failures are recovered; the failure of an operand
    // that is itself an operation on a secret is that operation's, which
    // nothing recovers, wherever it reaches.
    let error = workspace.run(app).unwrap_err().remove(0);
    assert_eq!(error.message, "an operation on a secret failed");
    let files = [
        ("workspace.gcsl", ""),
        ("app.gcsl", &text[..text.find("let n").unwrap()]),
    ];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = \"boom\"; b = \"index out of range\"}"
    );
}

#[test]
fn a_timeout_inside_an_operation_on_a_secret_is_its_call_s() {
    let text = "let s = secret(\"pw\", \"id\")\n\
                #[timeout(200ms)]\n\
                fn slow() -> Secret<String> { s + exec([\"sleep\", \"3\"]).stdout }\n\
                r = recover { slow().unsafe_value() } else (e) { e.message }";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{r = \"`slow` timed out after 200ms\"}"
    );
}

#[test]
fn a_secret_is_one_with_another_of_its_identity_where_a_comparison_is_plain() {
    // Total equality compares fields the type does not name, and a cache
    // hit is seen in whether a call runs: neither tells of the value.
    let text = "let s = secret(\"pw\", \"id\")\n\
                let q: {x: Int} = {x = 1; y = s}\n\
                let wrong: {x: Int} = {x = 1; y = secret(\"guess\", \"id\")}\n\
                let other: {x: Int} = {x = 1; y = secret(\"pw\", \"other\")}\n\
                a = q === wrong\n\
                b = q === other\n\
                fn f(x: Secret<String>) ->> Secret<String> { x }\n\
                let first = f(s)\n\
                c = f(secret(\"guess\", \"id\")).unsafe_value()\n\
                d = (s == secret(\"guess\", \"id\")).unsafe_value()";
    let files = [("workspace.gcsl", ""), ("app.gcsl", text)];
    assert_eq!(
        rooted_text(&files, "app.gcsl"),
        "{a = true; b = false; c = \"pw\"; d = false}"
    );
}
