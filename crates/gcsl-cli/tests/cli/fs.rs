//! Tests of the `gcsl` command: the file system, temporary directories and
//! `show`.

use super::*;

/// Asserts that `dir`, a temporary directory `fs.temp_dir` made in a run of
/// `gcsl` that has exited, was absolute, the first of its parent, in the
/// system's temporary directory, and is gone with its parent.
pub(super) fn assert_temp_dir_gone(dir: &str) {
    let path = std::path::Path::new(dir);
    assert!(path.is_absolute(), "{dir}");
    assert_eq!(path.file_name().unwrap(), "1", "{dir}");
    let parent = path.parent().unwrap();
    assert!(
        parent
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("gcsl-"),
        "{dir}"
    );
    assert_eq!(parent.parent().unwrap(), std::env::temp_dir());
    assert!(!path.exists(), "{dir}");
    assert!(!parent.exists(), "{dir}");
}

/// The summary of slice 42, run from other working directories than the
/// file's: its file is found from its own directory, and the temporary
/// directory it made, which it prints, is gone once `gcsl` has exited.
#[test]
fn fs_works_from_any_directory_and_its_temporary_directory_goes_with_gcsl() {
    let file = format!("{}/tests/fixtures/files.gcsl", crate_dir());
    for (cwd, file) in [
        (format!("{}/tests", crate_dir()), "fixtures/files.gcsl"),
        (
            format!("{}/tests/fixtures/ws", crate_dir()),
            "../files.gcsl",
        ),
        ("/".to_owned(), file.as_str()),
    ] {
        let output = Command::new(binary())
            .args(["run", file])
            .current_dir(&cwd)
            .output()
            .unwrap();
        assert_eq!(stderr(&output), "", "{cwd}");
        let shown = stdout(&output);
        let Some((_, rest)) = shown.split_once("temp = \"") else {
            panic!("{cwd}: {shown}");
        };
        let (temp, _) = rest.split_once('"').unwrap();
        assert_eq!(
            shown,
            format!("{{there = true; text = \"copied by fs\\n\"; temp = \"{temp}\"}}\n"),
            "{cwd}"
        );
        assert_temp_dir_gone(temp);
        assert_eq!(output.status.code(), Some(0));
    }
}

/// A run that fails after making a temporary directory, and putting a
/// directory in it, removes it as well.
#[test]
fn a_temporary_directory_goes_with_a_run_that_failed() {
    let output = gcsl(&["run", "tests/fixtures/files_panic.gcsl"]);
    assert_eq!(stdout(&output), "");
    let shown = stderr(&output);
    let Some((temp, _)) = shown
        .strip_prefix("error: ")
        .and_then(|rest| rest.split_once('\n'))
    else {
        panic!("{shown}");
    };
    assert_eq!(
        shown,
        format!(
            "error: {temp}\n\
             \x20--> tests/fixtures/files_panic.gcsl:4:1\n\
             \x20 |\n\
             4 | panic(dir)\n\
             \x20 | ^^^^^^^^^^\n"
        )
    );
    assert_temp_dir_gone(temp);
    assert_eq!(output.status.code(), Some(1));
}

/// A relative path given to `fs` is an error at the call, naming the member
/// as `fs.read` whatever it was imported as.
#[test]
fn a_relative_path_given_to_fs_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/files_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: `fs.read` needs an absolute path, found `fs_input.txt`\n\
         \x20--> tests/fixtures/files_error.gcsl:2:8\n\
         \x20 |\n\
         2 | text = slurp(\"fs_input.txt\")\n\
         \x20 |        ^^^^^^^^^^^^^^^^^^^^^\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The REPL: `import fs`, then a read on a later line; and a temporary
/// directory made on a line, gone once the REPL has ended.
#[test]
fn repl_imports_fs_and_reads_a_file() {
    let input = format!(
        "import fs\nfs.read(\"{}/tests/fixtures/fs_input.txt\")\nfs.temp_dir()\n",
        crate_dir()
    );
    let output = repl(&input);
    assert_eq!(stderr(&output), "");
    let shown = stdout(&output);
    let (read, temp) = shown.split_once('\n').unwrap();
    assert_eq!(read, "\"copied by fs\\n\" : String");
    let temp = temp
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix("\" : String\n"))
        .unwrap_or_else(|| panic!("{shown}"));
    assert_temp_dir_gone(temp);
    assert_eq!(output.status.code(), Some(0));
}

/// The summary of slice 43, its lock written into a temporary directory
/// and read back: `sort_versions`, `show` and `fs.write`.
#[test]
fn run_sorts_versions_shows_values_and_writes_files() {
    let output = gcsl(&["run", "tests/fixtures/locks.gcsl"]);
    assert_eq!(
        stdout(&output),
        "{a = [\"1.0~rc1\", \"1.0\", \"6.6\", \"6.6.1\", \"6.10\"]; \
         b = \"{{version = \\\"6.6\\\"; commit = \\\"3a8f\\\"}}\"; \
         c = \"\\\"say \\\\\\\"hi\\\\\\\" {{{{x}}}}\\\\n\\\"\"; d = \"[1, 2]\"; \
         written = \"locked = [{{version = \\\"6.6\\\"; commit = \\\"3a8f\\\"}}]\\n\"}\n"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// A function given to `show` is an error of the check, at the argument,
/// its type in a note.
#[test]
fn a_function_given_to_show_is_rendered() {
    let output = gcsl(&["run", "tests/fixtures/show_error.gcsl"]);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "error: cannot show a function: `show` renders data as source\n\
         \x20--> tests/fixtures/show_error.gcsl:2:13\n\
         \x20 |\n\
         2 | text = show(handlers)\n\
         \x20 |             ^^^^^^^^\n\
         \x20 |\n\
         \x20 = note: the argument is of type `{run: fn(): Int}`\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

/// The REPL: `show` on a line, through an alias a later line makes, and
/// `sort_versions`.
#[test]
fn repl_shows_values_and_sorts_versions() {
    let output = repl(
        "show({a = 1})\n\
         let s = show\n\
         s(\"x\")\n\
         sort_versions([\"10\", \"2\"])\n",
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(
        stdout(&output),
        "\"{{a = 1}}\" : String\n\
         \"\\\"x\\\"\" : String\n\
         [\"2\", \"10\"] : List<String>\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
