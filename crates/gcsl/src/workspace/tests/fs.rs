//! Tests of workspaces: `fs` and temporary directories.

use super::*;

/// A new empty directory for the `fs` test `name` to work in, in the
/// system's temporary directory, never the repository's: its absolute
/// path, canonical.
pub(super) fn fs_scratch(name: &str) -> String {
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
pub(super) const FS_SUMMARY: &str = "import fs\n\
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
pub(super) fn lone_in(dir: &str, text: &str) -> Result<String, Vec<Diagnostic>> {
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
pub(super) fn lone_error_in(dir: &str, text: &str) -> Diagnostic {
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
pub(super) fn assert_fresh_temp_dirs(value: &Value) -> Vec<String> {
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
    let direct = crate::eval::Direct { temp_dirs: None };
    let context = (
        Some(&workspace.exec_result),
        &workspace.scan.dir_texts[..],
        &direct as &dyn crate::eval::Effects,
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
pub(super) fn temp_dir_workspace() -> Workspace {
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
pub(super) fn temp_dirs_of(workspace: &mut Workspace, file: &str) -> Vec<std::path::PathBuf> {
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
