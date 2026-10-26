//! Tests of workspaces: the locks summary, `fs.write`, `sort_versions` and
//! `show`.

use super::*;

/// The summary of slice 43, its lock file written to `dir`.
pub(super) fn locks_summary(dir: &str) -> String {
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
pub(super) const LOCKS_SUMMARY_VALUE: &str = r#"{a = ["1.0~rc1", "1.0", "6.6", "6.6.1", "6.10"]; b = "{{version = \"6.6\"; commit = \"3a8f\"}}"; c = "\"say \\\"hi\\\" {{{{x}}}}\\n\""; d = "[1, 2]"}"#;

/// The names of the entries of `dir` and of every directory below it
/// that are temporary files of `fs.write`.
pub(super) fn temp_files_below(dir: &str) -> Vec<String> {
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
pub(super) fn mode_of(path: &str) -> u32 {
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
pub(super) struct Rng(pub(super) u64);

impl Rng {
    /// The next number.
    pub(super) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n`.
    pub(super) fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
    }

    /// One of `choices`.
    pub(super) fn pick<T: Copy>(&mut self, choices: &[T]) -> T {
        choices[self.below(choices.len())]
    }
}

/// The type of a value the round-trip test of `show` generates.
pub(super) enum Shape {
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
    pub(super) fn random(rng: &mut Rng, depth: u32) -> Shape {
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
    pub(super) fn text(&self) -> String {
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
    pub(super) fn literal(&self, rng: &mut Rng) -> (String, bool) {
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
pub(super) const TEXT_CHARS: &str = "aZ0 \"\\{}\n\t\r\0\u{1}\u{1b}\u{7f}\u{80}\u{85}\u{a0}\u{e9}\u{2028}\
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
pub(super) fn random_name(rng: &mut Rng) -> String {
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
pub(super) fn escaped(text: &str) -> String {
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
pub(super) fn lone_fields(text: &str) -> Vec<(String, Value)> {
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
pub(super) fn text_of(value: &Value) -> String {
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
