//! Tests of workspaces: `File`, `file(path)` and its methods.

use super::*;
use crate::workspace::tests::fs::fs_scratch;
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
fn compose_places_each_entry_at_a_computed_path() {
    let root = temp_tree("compose-list");
    let text = format!(
        "import fs\n\
         let src = dir(\"{root}\")\n\
         let name = \"one\"\n\
         let made = fs.compose([\n\
             {{at = \"opt/{{name}}.txt\"; place = src.file(\"a.txt\")}},\n\
             {{at = \"tree\"; place = src.dir(\"sub\")}},\n\
             {{at = \"flat\"; place = [src.file(\"sub/b.txt\")]}}\n\
         ])\n\
         = for f in made.files() {{ strip_prefix(f.path, made.path) }}"
    );
    assert_eq!(
        lone(&text).unwrap(),
        "[\"/flat/b.txt\", \"/opt/one.txt\", \"/tree/b.txt\", \"/tree/link\"]"
    );
    // None at all is an empty directory; its errors are `compose_dir`'s,
    // naming it.
    assert_eq!(
        lone("import fs\n= fs.compose([]).files().len()").unwrap(),
        "0"
    );
    let error = lone_error(&format!(
        "import fs\nx = fs.compose([{{at = \"../up\"; place = file(\"{root}/a.txt\")}}])"
    ));
    assert_eq!(
        error.message,
        "`fs.compose`: `../up` is outside the directory"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn set_executable_changes_a_file_s_mode_but_never_a_shared_one() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_tree("executable");
    let mode = |path: &str| {
        std::fs::metadata(format!("{root}/{path}"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    std::fs::set_permissions(
        format!("{root}/a.txt"),
        std::fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    // Executable by whoever can read it, and then by no one.
    let text = format!("import fs\n= fs.set_executable(\"{root}/a.txt\", true)");
    lone(&text).unwrap();
    assert_eq!(mode("a.txt"), 0o750);
    let text = format!("import fs\n= fs.set_executable(\"{root}/a.txt\", false)");
    lone(&text).unwrap();
    assert_eq!(mode("a.txt"), 0o640);
    // A file of two links is refused, and changes neither.
    std::fs::hard_link(format!("{root}/a.txt"), format!("{root}/twin.txt")).unwrap();
    let error = lone_error(&format!(
        "import fs\nx = fs.set_executable(\"{root}/twin.txt\", true)"
    ));
    assert_eq!(
        error.message,
        format!(
            "`fs.set_executable` could not change `{root}/twin.txt`: it has 2 links, which \
             share its mode; copy it first"
        )
    );
    assert_eq!(mode("a.txt"), 0o640);
    // A directory is no file; a relative path is refused before anything.
    let error = lone_error(&format!(
        "import fs\nx = fs.set_executable(\"{root}/sub\", true)"
    ));
    assert!(
        error.message.ends_with("it is not a file"),
        "{}",
        error.message
    );
    let error = lone_error("import fs\nx = fs.set_executable(\"a.txt\", true)");
    assert_eq!(
        error.message,
        "`fs.set_executable` needs an absolute path, found `a.txt`"
    );
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

/// A directory of `test`'s in this process's temporary directory that a
/// sandbox can run a shell in: the host's `/bin/sh`, and the libraries it
/// loads, at their own paths in it, as `ldd` names them; and its path.
fn shell_root(test: &str) -> String {
    let root = std::env::temp_dir().join(format!("gcsl-14e-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::copy(
        std::fs::canonicalize("/bin/sh").unwrap(),
        root.join("bin/sh"),
    )
    .unwrap();
    let ldd = std::process::Command::new("ldd")
        .arg("/bin/sh")
        .output()
        .unwrap();
    for library in String::from_utf8(ldd.stdout)
        .unwrap()
        .split_whitespace()
        .filter(|word| word.starts_with('/'))
    {
        let to = root.join(library.trim_start_matches('/'));
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(library, &to).unwrap();
    }
    root.to_str().unwrap().to_owned()
}

#[test]
fn a_sandbox_runs_its_commands_with_its_copy_as_their_root() {
    let root = shell_root("root");
    let text = format!(
        "let s = dir(\"{root}\").sandbox(cwd = \"/work\", env = {{GREETING = \"hi\"}})\n\
         # Its own `/`, its working directory, its environment alone.\n\
         a = s.exec(\"sh\", \"-c\", \"cd /; echo *; pwd; echo $GREETING x$HOME\", cwd = \"/\").stdout\n\
         b = s.exec(\"sh\", \"-c\", \"pwd\").stdout\n\
         c = s.exec(\"sh\", \"-c\", \"echo $GREETING\", env = {{GREETING = \"there\"}}).stdout\n\
         # What a command writes is there for the next and to read out.\n\
         d = s.exec(\"sh\", \"-c\", \"echo made > out.txt\").status\n\
         e = s.exec(\"sh\", \"-c\", \"read line < /work/out.txt; echo $line\").stdout\n\
         f = s.file(\"out.txt\").read()\n\
         g = s.file(\"/work/out.txt\") == s.dir(\"/work\").file(\"out.txt\")"
    );
    let shown = lone(&text).unwrap();
    assert!(
        shown.starts_with("{a = \"bin dev lib64 proc tmp usr work\\n/\\nhi x\\n\"; "),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "b = \"/work\\n\"; c = \"there\\n\"; d = 0; e = \"made\\n\"; f = \"made\\n\"; g = true}"
        ),
        "{shown}"
    );
    // Nothing of it reaches the directory it was made of.
    assert!(!std::path::Path::new(&root).join("work").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn every_sandbox_of_one_directory_is_its_own() {
    let root = shell_root("own");
    let text = format!(
        "let d = dir(\"{root}\")\n\
         let a = d.sandbox()\n\
         let b = d.sandbox()\n\
         w = a.exec(\"sh\", \"-c\", \"echo a > /mark\").status\n\
         x = b.exec(\"sh\", \"-c\", \"test -e /mark\", panic = false).status\n\
         same = a == b\n\
         itself = a == a"
    );
    assert_eq!(
        lone(&text).unwrap(),
        "{w = 0; x = 1; same = false; itself = true}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_sandbox_s_commands_are_killed_whole_and_its_paths_stay_inside() {
    let root = shell_root("kill");
    // `until` and a timeout end what runs in it, at once.
    let started = std::time::Instant::now();
    let text = format!(
        "let s = dir(\"{root}\").sandbox()\n\
         a = s.exec(\"sh\", \"-c\", \"echo ready; while :; do :; done\", until = \"ready\").stdout\n\
         #[timeout(200ms)]\n\
         fn slow() -> Int {{ s.exec(\"sh\", \"-c\", \"while :; do :; done\").status }}\n\
         b = recover {{ slow() }} else (e) {{ 0 - 1 }}"
    );
    assert_eq!(lone(&text).unwrap(), "{a = \"ready\\n\"; b = -1}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    // Its failures are its programs', at the call.
    let text = format!("x = dir(\"{root}\").sandbox().exec(\"sh\", \"-c\", \"exit 3\")");
    let error = lone_error(&text);
    assert_eq!(error.message, "`sh` exited with status 3");
    assert_eq!(at(&text, &error), &text[4..]);
    let text = format!("x = dir(\"{root}\").sandbox().file(\"../../etc/passwd\")");
    assert_eq!(
        lone_error(&text).message,
        "`../../etc/passwd` is not inside the sandbox"
    );
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(
        lone_error("x = dir(\"/gcsl-no-such-dir-14e\").sandbox()").message,
        "a sandbox could not copy `/gcsl-no-such-dir-14e`: could not list \
         `/gcsl-no-such-dir-14e`: No such file or directory (os error 2)"
    );
    let empty = std::env::temp_dir().join(format!("gcsl-14e-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let text = format!(
        "let s = dir(\"{}\").sandbox()\nx = show(s)",
        empty.display()
    );
    let error = lone_error(&text);
    std::fs::remove_dir_all(&empty).unwrap();
    assert_eq!(
        error.message,
        "cannot show a sandbox: it is a live directory, not data"
    );
}

/// The SHA-256 digest of `hello\n`.
const HELLO_SHA256: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

/// A `file://` URL of a new file holding `hello\n`, or `None` where
/// `curl` cannot run, which `fetch` needs.
fn hello_url(name: &str) -> Option<String> {
    let has_curl = std::process::Command::new("curl")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !has_curl {
        return None;
    }
    let dir = fs_scratch(name);
    std::fs::create_dir_all(&dir).unwrap();
    let path = format!("{dir}/hello.txt");
    std::fs::write(&path, "hello\n").unwrap();
    Some(format!("file://{path}"))
}

#[test]
fn a_pinned_fetch_gives_the_file_it_names() {
    let Some(url) = hello_url("fetch-pinned") else {
        return;
    };
    let text = format!(
        "let f = fetch(\"{url}\", sha256 = \"{}\")\n= [f.read(), f.path.ends_with(\"/hello.txt\")]",
        HELLO_SHA256.to_ascii_uppercase()
    );
    assert_eq!(lone(&text).unwrap(), "[\"hello\\n\", true]");
}

#[test]
fn a_fetch_that_is_not_what_was_pinned_fails_with_what_it_is() {
    let Some(url) = hello_url("fetch-unpinned") else {
        return;
    };
    let unpinned = lone_error(&format!("x = fetch(\"{url}\")"));
    assert_eq!(
        unpinned.message,
        format!("`fetch` of `{url}` is not pinned: its sha256 is `{HELLO_SHA256}`")
    );
    assert_eq!(
        unpinned.notes,
        [format!(
            "pin it: `fetch(\"{url}\", sha256 = \"{HELLO_SHA256}\")`"
        )]
    );
    let other = "0".repeat(64);
    let changed = lone_error(&format!("x = fetch(\"{url}\", sha256 = \"{other}\")"));
    assert_eq!(
        changed.message,
        format!("`fetch` of `{url}` gave a file whose sha256 is `{HELLO_SHA256}`, not `{other}`")
    );
    let bad = lone_error(&format!("x = fetch(\"{url}\", sha256 = \"abc\")"));
    assert!(
        bad.message
            .ends_with("which is no SHA-256 digest: 64 hexadecimal digits")
    );
    let missing = lone_error("x = fetch(\"file:///nonexistent/pekit-nothing\", sha256 = \"\")");
    assert!(
        missing
            .message
            .starts_with("`fetch` could not download `file:///nonexistent/pekit-nothing`"),
        "{}",
        missing.message
    );
}

#[test]
fn a_fetched_file_is_named_by_its_url_s_last_segment() {
    use crate::eval::fetched_name;
    assert_eq!(
        fetched_name("https://zlib.net/fossils/zlib-1.3.1.tar.gz"),
        "zlib-1.3.1.tar.gz"
    );
    assert_eq!(
        fetched_name("https://x.org/a/b.tar.xz?mirror=1#top"),
        "b.tar.xz"
    );
    assert_eq!(fetched_name("https://x.org/"), "download");
    assert_eq!(fetched_name("https://x.org"), "download");
    assert_eq!(fetched_name("https://x.org/a/.."), "download");
}

/// A gzipped tar archive of `top/a.txt` and `top/sub/b.txt`, made with
/// `tar`, or `None` where it cannot run, which `archive.unpack` needs: its
/// path.
fn archive_of_top(name: &str, compress: &str) -> Option<String> {
    let dir = fs_scratch(name);
    std::fs::create_dir_all(format!("{dir}/src/top/sub")).unwrap();
    std::fs::write(format!("{dir}/src/top/a.txt"), "a\n").unwrap();
    std::fs::write(format!("{dir}/src/top/sub/b.txt"), "b\n").unwrap();
    let archive = format!("{dir}/top.tar{compress}");
    let made = std::process::Command::new("tar")
        .arg("--create")
        .arg("--auto-compress")
        .arg("--file")
        .arg(&archive)
        .arg("--directory")
        .arg(format!("{dir}/src"))
        .arg("top")
        .status()
        .is_ok_and(|status| status.success());
    made.then_some(archive)
}

#[test]
fn an_archive_unpacks_into_a_directory_of_its_own() {
    for compress in ["", ".gz", ".xz"] {
        let Some(archive) = archive_of_top(&format!("unpack{compress}"), compress) else {
            return;
        };
        let text = format!(
            "import archive\n\
             let whole = archive.unpack(file(\"{archive}\"))\n\
             let stripped = archive.unpack(file(\"{archive}\"), strip = 1)\n\
             = [whole.file(\"top/a.txt\").read(), stripped.file(\"sub/b.txt\").read(), \
             \"{{len(stripped.files())}}\"]"
        );
        assert_eq!(
            lone(&text).unwrap(),
            "[\"a\\n\", \"b\\n\", \"2\"]",
            "{compress}"
        );
    }
    let Some(archive) = archive_of_top("unpack-errors", "") else {
        return;
    };
    let negative = lone_error(&format!(
        "import archive\nx = archive.unpack(file(\"{archive}\"), strip = -1)"
    ));
    assert!(
        negative.message.ends_with("`strip` is 0 or more"),
        "{}",
        negative.message
    );
    let missing = lone_error("import archive\nx = archive.unpack(file(\"/nonexistent/x.tar\"))");
    assert!(
        missing
            .message
            .starts_with("`archive.unpack` could not unpack `/nonexistent/x.tar`"),
        "{}",
        missing.message
    );
}
