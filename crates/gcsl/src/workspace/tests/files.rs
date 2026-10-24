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

/// A tree in this process's temporary directory, named for `test`: `a.txt`,
/// `sub/b.txt` and `sub/link`, a symbolic link to `b.txt`; and its path.
fn temp_tree(test: &str) -> String {
    let root = std::env::temp_dir().join(format!("gcsl-14d-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("sub/b.txt"), "b\n").unwrap();
    std::os::unix::fs::symlink("b.txt", root.join("sub/link")).unwrap();
    root.to_str().unwrap().to_owned()
}

#[test]
fn a_dir_is_a_tree_whose_files_and_dirs_are_inside_it() {
    let root = temp_tree("tree");
    let text = format!(
        "let d = dir(\"{root}/sub/..\")\n\
         p = d.path\n\
         names = for f in d.files() {{ f.path }}\n\
         b = d.file(\"sub/b.txt\").read()\n\
         same = d.dir(\"/sub\") == dir(\"{root}/sub\")\n\
         top = d.dir(\"\") == d\n\
         s = d"
    );
    assert_eq!(
        lone(&text).unwrap(),
        format!(
            "{{p = \"{root}\"; names = [\"{root}/a.txt\", \"{root}/sub/b.txt\", \"{root}/sub/link\"]; \
             b = \"b\\n\"; same = true; top = true; s = dir(\"{root}\")}}"
        )
    );
    // It runs a program in it.
    assert_eq!(
        lone(&format!("= dir(\"{root}\").exec(\"ls\", \"sub\").stdout")).unwrap(),
        "b.txt\nlink\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
    // Its files and dirs never leave it.
    let text = "x = dir(\"/a\").file(\"b/../../c\")";
    let error = lone_error(text);
    assert_eq!(error.message, "`b/../../c` is not inside `/a`");
    assert_eq!(at(text, &error), "dir(\"/a\").file(\"b/../../c\")");
    assert_eq!(
        lone_error("x = dir(\"/a\").file(\".\")").message,
        "`d.file` needs a path inside the directory"
    );
    assert_eq!(lone("= dir(\"/\").file(\"etc\").path").unwrap(), "/etc");
    // Typed as a file is: its one field, its own type, no order.
    let error = lone_error("x = dir(\"/a\").name");
    assert_eq!(error.message, "no field `name` in Dir");
    assert_eq!(error.notes, ["a directory's one field is `path`"]);
    assert_eq!(
        lone_error("let d: Dir = file(\"/a\")").message,
        "mismatched types"
    );
    assert_eq!(
        lone_error("x = dir(\"/a\") < dir(\"/b\")").message,
        "cannot order Dir"
    );
    assert_eq!(
        lone_error("x = dir(\"/gcsl-no-such-dir-14d\").files()").message,
        "cannot list `/gcsl-no-such-dir-14d`"
    );
    // A toolchain's layout holds both.
    assert_eq!(
        lone("let t: {*: File | Dir} = {root = dir(\"/a\"); cc = file(\"/a/cc\")}\n= t").unwrap(),
        "{root = dir(\"/a\"); cc = file(\"/a/cc\")}"
    );
}

#[test]
fn compose_dir_copies_each_entry_to_its_place() {
    let root = temp_tree("compose");
    let text = format!(
        "import fs\n\
         let src = dir(\"{root}\")\n\
         let made = fs.compose_dir({{\n\
             \"/\" = src\n\
             \"opt/one.txt\" = src.file(\"a.txt\")\n\
             \"flat\" = [src.file(\"a.txt\"), src.file(\"sub/b.txt\")]\n\
         }})\n\
         names = for f in made.files() {{ strip_prefix(f.path, made.path) }}\n\
         b = made.file(\"flat/b.txt\").read()\n\
         link = made.exec(\"readlink\", \"sub/link\").stdout"
    );
    assert_eq!(
        lone(&text).unwrap(),
        "{names = [\"/a.txt\", \"/flat/a.txt\", \"/flat/b.txt\", \"/opt/one.txt\", \"/sub/b.txt\", \
         \"/sub/link\"]; b = \"b\\n\"; link = \"b.txt\\n\"}"
    );
    // A program writing in it changes nothing it was made of.
    let text = format!(
        "import fs\n\
         let made = fs.compose_dir({{\"/\" = dir(\"{root}\")}})\n\
         x = made.exec(\"sh\", \"-c\", \"echo changed > a.txt\").status\n\
         a = dir(\"{root}\").file(\"a.txt\").read()"
    );
    assert_eq!(lone(&text).unwrap(), "{x = 0; a = \"a\\n\"}");
    // Two entries placing one file, a key leaving it, a file as it.
    for (entries, message) in [
        (
            format!("\"/\" = dir(\"{root}\"); \"a.txt\" = file(\"{root}/sub/b.txt\")"),
            "`fs.compose_dir`: two entries place `a.txt`",
        ),
        (
            format!("\"x\" = [file(\"{root}/a.txt\"), file(\"{root}/a.txt\")]"),
            "`fs.compose_dir`: two entries place `x/a.txt`",
        ),
        (
            format!("\"../up\" = file(\"{root}/a.txt\")"),
            "`fs.compose_dir`: `../up` is outside the directory",
        ),
    ] {
        let error = lone_error(&format!("import fs\nx = fs.compose_dir({{{entries}}})"));
        assert_eq!(error.message, message, "{entries}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_value_s_dirs_are_found_apart_from_its_files() {
    let source = SourceFile::new(
        "lone.gcsl",
        "a = dir(\"/a\")\nb = [{c = file(\"/c\")}, {d = dir(\"/a\")}, {e = dir(\"/e\")}]",
    )
    .unwrap();
    let value = crate::run(&source).unwrap().value;
    let dirs: Vec<String> = value
        .dirs()
        .iter()
        .map(|dir| dir.path.to_string())
        .collect();
    assert_eq!(dirs, ["/a", "/e"]);
    let files: Vec<String> = value
        .files()
        .iter()
        .map(|file| file.path.to_string())
        .collect();
    assert_eq!(files, ["/c"]);
}
