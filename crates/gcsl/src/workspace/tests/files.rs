//! Tests of workspaces: `File`, `file(path)` and its methods.

use super::*;
use crate::workspace::tests::shell::lone_error;
use crate::workspace::tests::standard::lone;

/// A file of `contents` in this process's temporary directory, named for
/// `test`, and its path.
fn temp_file(test: &str, contents: &[u8]) -> String {
    let path = std::env::temp_dir().join(format!("gcsl-14c-{test}-{}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_owned()
}

/// The text of `text` the diagnostic `error` is at.
fn at<'a>(text: &'a str, error: &Diagnostic) -> &'a str {
    &text[error.span.start as usize..error.span.end as usize]
}

#[test]
fn a_file_is_its_absolute_path_and_reads_its_contents() {
    let path = temp_file("read", b"hello\n");
    let text = format!("let f = file(\"{path}\")\np = f.path\nt = f.read()\ns = f");
    assert_eq!(
        lone(&text).unwrap(),
        format!("{{p = \"{path}\"; t = \"hello\\n\"; s = file(\"{path}\")}}")
    );
    std::fs::remove_file(&path).unwrap();
    // A relative path is the working directory's, normalised lexically.
    let cwd = std::env::current_dir().unwrap();
    let cwd = cwd.to_str().unwrap();
    assert_eq!(
        lone("= file(\"x/../y.txt\").path").unwrap(),
        format!("{cwd}/y.txt")
    );
    assert_eq!(lone("= file(\"/a//b/./c/../d\").path").unwrap(), "/a/b/d");
    assert_eq!(lone("= file(\"/../..\").path").unwrap(), "/");
    // Equal if their paths are, whatever was written; shown as the call
    // that makes one, which reads back as an equal file.
    assert_eq!(
        lone("= file(\"/a/b\") == file(\"/a/./b\")").unwrap(),
        "true"
    );
    assert_eq!(lone("= file(\"/a/b\") != file(\"/a/c\")").unwrap(), "true");
    assert_eq!(lone("= show(file(\"/a\"))").unwrap(), "file(\"/a\")");
    assert_eq!(lone("= [file(\"/a\")]").unwrap(), "[file(\"/a\")]");
    // Nothing is read until it is asked for.
    assert_eq!(
        lone("= file(\"/gcsl-no-such-file-14c\").path").unwrap(),
        "/gcsl-no-such-file-14c"
    );
}

#[test]
fn a_file_s_errors_are_reported_at_the_call() {
    let text = "x = file(\"\")";
    let error = lone_error(text);
    assert_eq!(error.message, "`file` needs a path");
    assert_eq!(at(text, &error), "file(\"\")");
    let text = "x = file(\"/gcsl-no-such-file-14c\").read()";
    let error = lone_error(text);
    assert_eq!(error.message, "cannot read `/gcsl-no-such-file-14c`");
    assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
    assert_eq!(at(text, &error), "file(\"/gcsl-no-such-file-14c\").read()");
    let path = temp_file("binary", &[0xff, 0xfe]);
    let error = lone_error(&format!("x = file(\"{path}\").read()"));
    assert_eq!(error.message, format!("`{path}` is not UTF-8 text"));
    std::fs::remove_file(&path).unwrap();
    // Checked: its one field, its own type, no order.
    let error = lone_error("x = file(\"/a\").name");
    assert_eq!(error.message, "no field `name` in File");
    assert_eq!(error.notes, ["a file's one field is `path`"]);
    assert_eq!(
        lone_error("let f: File = {path = \"/a\"}").message,
        "mismatched types"
    );
    assert_eq!(
        lone_error("let p: String = file(\"/a\")").message,
        "mismatched types"
    );
    assert_eq!(
        lone_error("x = file(\"/a\") < file(\"/b\")").message,
        "cannot order File"
    );
    // `file` reads the working directory: not from a pure function.
    assert_eq!(
        lone_error("fn f(): File { file(\"/a\") }\nx = f()").message,
        "cannot call an impure function from a pure function"
    );
}

#[test]
fn a_file_runs_as_a_program() {
    assert_eq!(
        lone("= file(\"/bin/echo\").exec(\"a\", \"b\").stdout").unwrap(),
        "a b\n"
    );
    assert_eq!(
        lone("let words = [\"x\", \"y\"]\n= file(\"/bin/echo\").exec(\"w\", ...words).stdout")
            .unwrap(),
        "w x y\n"
    );
    assert_eq!(
        lone("= file(\"/bin/pwd\").exec(cwd = \"/\").stdout").unwrap(),
        "/\n"
    );
    // A status other than 0 fails, at the call, unless `panic = false`.
    let text = "x = file(\"/bin/false\").exec()";
    let error = lone_error(text);
    assert_eq!(error.message, "`/bin/false` exited with status 1");
    assert_eq!(at(text, &error), "file(\"/bin/false\").exec()");
    assert_eq!(
        lone("= file(\"/bin/false\").exec(panic = false).status").unwrap(),
        "1"
    );
}

#[test]
fn a_file_set_finds_a_file_by_its_name() {
    let set = "let files: FileSet = [file(\"/usr/bin/cc\"), file(\"/opt/x/ld\")]\n";
    for (name, path) in [
        ("cc", "/usr/bin/cc"),
        ("bin/cc", "/usr/bin/cc"),
        ("/opt/x/ld", "/opt/x/ld"),
    ] {
        assert_eq!(
            lone(&format!("{set}= files.file(\"{name}\").path")).unwrap(),
            path,
            "{name}"
        );
    }
    // A name matches whole segments only.
    let text = format!("{set}x = files.file(\"c\")");
    let error = lone_error(&text);
    assert_eq!(error.message, "no file `c` among 2 files");
    assert_eq!(at(&text, &error), "files.file(\"c\")");
    assert_eq!(
        lone_error("let one: FileSet = [file(\"/a\")]\nx = one.file(\"b\")").message,
        "no file `b` among the one file"
    );
    assert_eq!(
        lone_error("x = [file(\"/a/cc\"), file(\"/b/cc\")].file(\"cc\")").message,
        "`cc` names 2 files: `/a/cc`, `/b/cc`"
    );
    // A path's alias is a `String`.
    assert_eq!(
        lone("fn twice(p: FilePath): String { p + p }\n= twice(\"a\")").unwrap(),
        "aa"
    );
}

#[test]
fn a_memoised_call_of_equal_files_runs_once() {
    let counter = std::env::temp_dir().join(format!("gcsl-14c-memo-{}", std::process::id()));
    let counter = counter.to_str().unwrap();
    let _ = std::fs::remove_file(counter);
    let text = format!(
        "fn mark(f: File) ->> Int {{ exec([\"sh\", \"-c\", \"echo x >> {counter}\"]).status }}\n\
         = [mark(file(\"/a/b\")), mark(file(\"/a/./b\")), mark(file(\"/a/c\"))]"
    );
    assert_eq!(lone(&text).unwrap(), "[0, 0, 0]");
    let runs = std::fs::read_to_string(counter).unwrap().lines().count();
    std::fs::remove_file(counter).unwrap();
    assert_eq!(runs, 2);
}

#[test]
fn a_value_s_files_are_found_at_any_depth_each_once() {
    let source = SourceFile::new(
        "lone.gcsl",
        "a = file(\"/a\")\nb = [{c = file(\"/c\")}, {c = file(\"/a\")}]\nn = \"/d\"",
    )
    .unwrap();
    let value = crate::run(&source).unwrap().value;
    let paths: Vec<String> = value
        .files()
        .iter()
        .map(|file| file.path.to_string())
        .collect();
    assert_eq!(paths, ["/a", "/c"]);
}
