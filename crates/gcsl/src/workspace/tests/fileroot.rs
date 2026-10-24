//! Tests of workspaces: a workspace at a root on disk, `this`, `caller` and
//! `fileroot`.

use super::*;

/// The root the `fileroot` tests' workspaces are located at: a path in
/// the scratch directory, which nothing reads, since the library reads
/// no file.
pub(super) const SCRATCH: &str = "/scratch/fileroot";

/// The library of the `fileroot` summary, `_lib/ppk.gcsl`, with what the
/// other tests call besides: `commit`, `wrapped` (a helper), `twice`
/// (a helper of a helper), `here` (`this` in a `def`), `caller` (the
/// caller of a `def` of the library), `mid` (`helper()` between two
/// calls), closures made in the library, and a step made in it.
pub(super) const PPK: &str = "import path\nimport fileroot\n\
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
pub(super) fn rooted(files: &[(&str, &str)]) -> Result<Workspace, Vec<Diagnostic>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    check_with_standard(&sources, &[], SCRATCH).1
}

/// The value of the file at `path` of the workspace of `files` located
/// at [`SCRATCH`].
pub(super) fn rooted_value(files: &[(&str, &str)], path: &str) -> Value {
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
pub(super) fn rooted_text(files: &[(&str, &str)], path: &str) -> String {
    rooted_value(files, path)
        .to_string()
        .replace(SCRATCH, "<root>")
}

/// The messages of the diagnostics of checking the workspace of `files`
/// located at [`SCRATCH`].
pub(super) fn rooted_messages(files: &[(&str, &str)]) -> Vec<String> {
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
