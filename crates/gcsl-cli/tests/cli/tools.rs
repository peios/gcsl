//! Tests of the `gcsl` command: the language server, `fmt` and `check`.

use super::*;

/// Runs `gcsl lsp` with `messages`, each framed as LSP frames a message,
/// on its stdin until it ends.
pub(super) fn lsp(messages: &[&str]) -> Output {
    let mut child = Command::new(binary())
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        write!(stdin, "Content-Length: {}\r\n\r\n{message}", message.len()).unwrap();
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

/// The messages of `output`, LSP frames on stdout, unframed: nothing else
/// may be there.
pub(super) fn frames(output: &Output) -> Vec<&str> {
    let mut rest = stdout(output);
    let mut messages = Vec::new();
    while !rest.is_empty() {
        let header = rest.strip_prefix("Content-Length: ").unwrap();
        let (length, body) = header.split_once("\r\n\r\n").unwrap();
        let (message, after) = body.split_at(length.parse().unwrap());
        messages.push(message);
        rest = after;
    }
    messages
}

/// LSP's `initialize`, from a client offering utf-8 and utf-16.
pub(super) const LSP_INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]}}}}"#;

/// LSP's `initialized`.
pub(super) const LSP_INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;

/// LSP's `shutdown`.
pub(super) const LSP_SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;

/// LSP's `exit`.
pub(super) const LSP_EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit"}"#;

/// `gcsl lsp` speaks LSP on stdin and stdout: it answers `initialize` with
/// its capabilities (diagnostics, hover, go to definition, references,
/// highlights, rename, symbols, formatting, completion, signature help and
/// inlay hints)
/// and `shutdown` with null, and exits with code 0 on
/// `exit` after `shutdown`, and 1 on `exit` alone.
#[test]
fn lsp_initializes_and_shuts_down() {
    let output = lsp(&[LSP_INITIALIZE, LSP_INITIALIZED, LSP_SHUTDOWN, LSP_EXIT]);
    let messages = frames(&output);
    let [initialized, shut_down] = messages.as_slice() else {
        panic!("{messages:?}");
    };
    assert_eq!(
        *initialized,
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"capabilities":{{"codeActionProvider":{{"codeActionKinds":["quickfix","source.fixAll"],"resolveProvider":false}},"completionProvider":{{"resolveProvider":false,"triggerCharacters":[".","/"]}},"definitionProvider":true,"documentFormattingProvider":true,"documentHighlightProvider":true,"documentRangeFormattingProvider":true,"documentSymbolProvider":true,"hoverProvider":true,"inlayHintProvider":true,"positionEncoding":"utf-8","referencesProvider":true,"renameProvider":{{"prepareProvider":true}},"signatureHelpProvider":{{"retriggerCharacters":[","],"triggerCharacters":["(",","]}},"textDocumentSync":{{"change":2,"openClose":true,"save":true}},"workspaceSymbolProvider":true}},"serverInfo":{{"name":"gcsl","version":"{}"}}}}}}"#,
            env!("CARGO_PKG_VERSION")
        )
    );
    assert_eq!(*shut_down, r#"{"jsonrpc":"2.0","id":2,"result":null}"#);
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // `exit` without `shutdown`.
    let output = lsp(&[LSP_INITIALIZE, LSP_INITIALIZED, LSP_EXIT]);
    assert_eq!(frames(&output).len(), 1);
    assert_eq!(output.status.code(), Some(1));
}

/// A scratch directory for a `gcsl fmt` test, `name`'s own, empty and
/// canonical, under the system's temporary directory, removed when dropped.
pub(super) struct FmtScratch(pub(super) std::path::PathBuf);

impl FmtScratch {
    pub(super) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gcsl-cli-fmt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(std::fs::canonicalize(dir).unwrap())
    }

    /// Writes `text` to `path` below the directory, making the directories
    /// on the way, its modification time set an hour back; returns its
    /// path.
    pub(super) fn write(&self, path: &str, text: &str) -> std::path::PathBuf {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        let back = std::time::SystemTime::now() - std::time::Duration::from_hours(1);
        file.set_modified(back).unwrap();
        path
    }

    /// The text of the file at `path` below the directory.
    pub(super) fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.0.join(path)).unwrap()
    }

    /// The modification time of the file at `path` below the directory.
    pub(super) fn modified(&self, path: &str) -> std::time::SystemTime {
        std::fs::metadata(self.0.join(path))
            .unwrap()
            .modified()
            .unwrap()
    }
}

impl Drop for FmtScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `gcsl` with `args` in `dir`, with `input` as its standard input.
pub(super) fn gcsl_in(dir: &std::path::Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(binary())
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// A file that `gcsl fmt` changes, and what it changes it to.
pub(super) const MESSY: &str = "x = {\n  a=1;\n\n\n  b = [\n    1\n  ]   # one\n}\n\n";

pub(super) const FORMATTED: &str = "x = {\n    a = 1\n\n    b = [\n        1,\n    ] # one\n}\n";

/// `gcsl fmt FILE` formats a file in place, and leaves a formatted one
/// alone: not written, its modification time kept.
#[test]
fn fmt_formats_a_file_in_place_and_writes_nothing_formatted() {
    let scratch = FmtScratch::new("in-place");
    scratch.write("messy.gcsl", MESSY);
    scratch.write("tidy.gcsl", FORMATTED);
    let before = scratch.modified("tidy.gcsl");
    let output = gcsl_in(&scratch.0, &["fmt", "messy.gcsl", "tidy.gcsl"], "");
    assert_eq!(stderr(&output), "");
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("messy.gcsl"), FORMATTED);
    assert_eq!(scratch.read("tidy.gcsl"), FORMATTED);
    assert_eq!(scratch.modified("tidy.gcsl"), before);
    // Formatting it again writes nothing either.
    let before = scratch.modified("messy.gcsl");
    let output = gcsl_in(&scratch.0, &["fmt", "messy.gcsl"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.modified("messy.gcsl"), before);
    // No temporary file is left beside them.
    let names: Vec<String> = std::fs::read_dir(&scratch.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

/// `gcsl fmt` of a symbolic link formats the file it leads to, and the link
/// stays a link.
#[test]
fn fmt_writes_through_a_symbolic_link() {
    let scratch = FmtScratch::new("link");
    scratch.write("target.gcsl", MESSY);
    std::os::unix::fs::symlink(scratch.0.join("target.gcsl"), scratch.0.join("link.gcsl")).unwrap();
    let output = gcsl_in(&scratch.0, &["fmt", "link.gcsl"], "");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("target.gcsl"), FORMATTED);
    let link = std::fs::symlink_metadata(scratch.0.join("link.gcsl")).unwrap();
    assert!(link.file_type().is_symlink());
}

/// `gcsl fmt --check` writes nothing, lists the files formatting would
/// change and exits with code 1 if there are any.
#[test]
fn fmt_check_lists_what_would_change_and_writes_nothing() {
    let scratch = FmtScratch::new("check");
    scratch.write("messy.gcsl", MESSY);
    scratch.write("tidy.gcsl", FORMATTED);
    let before = scratch.modified("messy.gcsl");
    let output = gcsl_in(
        &scratch.0,
        &["fmt", "--check", "tidy.gcsl", "messy.gcsl"],
        "",
    );
    assert_eq!(stdout(&output), "messy.gcsl\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(scratch.read("messy.gcsl"), MESSY);
    assert_eq!(scratch.modified("messy.gcsl"), before);
    let output = gcsl_in(&scratch.0, &["fmt", "--check", "tidy.gcsl"], "");
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// `gcsl fmt -` formats standard input to standard output; with `--check`
/// it writes `<stdin>` if that would change, and nothing else.
#[test]
fn fmt_formats_standard_input_to_standard_output() {
    let scratch = FmtScratch::new("stdin");
    let output = gcsl_in(&scratch.0, &["fmt", "-"], MESSY);
    assert_eq!(stdout(&output), FORMATTED);
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl_in(&scratch.0, &["fmt", "--check", "-"], MESSY);
    assert_eq!(stdout(&output), "<stdin>\n");
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl_in(&scratch.0, &["fmt", "--check", "-"], FORMATTED);
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0));
    // With a syntax error: nothing on standard output, the error shown.
    let output = gcsl_in(&scratch.0, &["fmt", "-"], "x = (1 2)\n");
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("error: expected `)`, found `2`"));
    assert!(
        stderr(&output).contains("<stdin>:1:8"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1));
    // `-` with other paths is a usage error.
    let output = gcsl_in(&scratch.0, &["fmt", "-", "x.gcsl"], "");
    assert_eq!(output.status.code(), Some(2));
}

/// A file with a syntax error is left alone, its error shown, and the
/// other files are formatted; the exit code is 1. A path that is not there
/// exits with code 2.
#[test]
fn fmt_leaves_a_file_with_an_error_alone() {
    let scratch = FmtScratch::new("error");
    let broken = "x  =  (1 2)\n";
    scratch.write("broken.gcsl", broken);
    scratch.write("messy.gcsl", MESSY);
    let before = scratch.modified("broken.gcsl");
    let output = gcsl_in(&scratch.0, &["fmt", "broken.gcsl", "messy.gcsl"], "");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("error: expected `)`, found `2`"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("broken.gcsl:1:10"),
        "{}",
        stderr(&output)
    );
    assert_eq!(scratch.read("broken.gcsl"), broken);
    assert_eq!(scratch.modified("broken.gcsl"), before);
    assert_eq!(scratch.read("messy.gcsl"), FORMATTED);
    let output = gcsl_in(&scratch.0, &["fmt", "missing.gcsl"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("error: cannot read missing.gcsl"));
}

/// With no paths, `gcsl fmt` formats every file of the workspace the
/// working directory is in, but those of a nested workspace and those under
/// a name starting with `.`; a directory given has every `.gcsl` file under
/// it formatted, those of the workspaces nested in it too. Outside any
/// workspace, no paths is an error.
#[test]
fn fmt_formats_a_workspace_or_a_directory() {
    let scratch = FmtScratch::new("workspace");
    for path in [
        "workspace.gcsl",
        "sub/a.gcsl",
        ".hidden/b.gcsl",
        "nested/workspace.gcsl",
        "nested/c.gcsl",
    ] {
        scratch.write(path, MESSY);
    }
    scratch.write("sub/notes.txt", MESSY);
    // Each named relative to the working directory when under it, in the
    // order of their paths in the workspace.
    let output = gcsl_in(&scratch.0.join("sub"), &["fmt", "--check"], "");
    let root = scratch.0.display();
    assert_eq!(stdout(&output), format!("a.gcsl\n{root}/workspace.gcsl\n"));
    assert_eq!(output.status.code(), Some(1));
    let output = gcsl_in(&scratch.0.join("sub"), &["fmt"], "");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("workspace.gcsl"), FORMATTED);
    assert_eq!(scratch.read("sub/a.gcsl"), FORMATTED);
    for path in [
        ".hidden/b.gcsl",
        "nested/workspace.gcsl",
        "nested/c.gcsl",
        "sub/notes.txt",
    ] {
        assert_eq!(scratch.read(path), MESSY, "{path}");
    }
    // A directory: every `.gcsl` file under it, nested workspaces' too.
    let output = gcsl_in(&scratch.0, &["fmt", "."], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(scratch.read("nested/workspace.gcsl"), FORMATTED);
    assert_eq!(scratch.read("nested/c.gcsl"), FORMATTED);
    assert_eq!(scratch.read(".hidden/b.gcsl"), MESSY);
    assert_eq!(scratch.read("sub/notes.txt"), MESSY);
    // No workspace above.
    let outside = FmtScratch::new("outside");
    let output = gcsl_in(&outside.0, &["fmt"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("no `workspace.gcsl`"),
        "{}",
        stderr(&output)
    );
}

/// A file with a warning and nothing wrong, whose top-level field runs `sh`
/// when the file is run.
pub(super) const CHECKED: &str = "z = sh(\"touch MARKER\")\np: {x: Int} = {x = 1; y = 2}\n";

/// `gcsl check FILE` checks the file's workspace and runs nothing: no `sh`
/// of a file that would run, so no marker is made. What it shows is what
/// `gcsl run` shows of the workspace's diagnostics, then a summary; the
/// exit code is 0 with warnings alone.
#[test]
fn check_runs_nothing_and_shows_what_run_shows() {
    let scratch = FmtScratch::new("check-runs-nothing");
    scratch.write("workspace.gcsl", "");
    scratch.write("app.gcsl", CHECKED);
    let output = gcsl_in(&scratch.0, &["check", "app.gcsl"], "");
    let made = scratch.0.join("MARKER").exists();
    assert!(!made, "`gcsl check` ran `sh`");
    assert_eq!(stdout(&output), "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    // `gcsl run` runs it, and shows the same warning.
    let run = gcsl_in(&scratch.0, &["run", "app.gcsl"], "");
    assert!(scratch.0.join("MARKER").exists());
    assert!(stderr(&run).starts_with("warning: field `y` is not in"));
    assert_eq!(
        stderr(&output),
        format!("{}0 errors, 1 warning in 2 files checked\n", stderr(&run))
    );
}

/// Each form of path `gcsl check` takes: none (the working directory's
/// workspace), a directory (every workspace at or under it, nested ones
/// included, and the one it is in), a file (its workspace, or itself
/// alone); each workspace once. An error exits with code 1; a path not
/// there, or no workspace for no paths, with code 2.
#[test]
fn check_takes_files_directories_and_nothing() {
    let scratch = FmtScratch::new("check-paths");
    scratch.write("workspace.gcsl", "");
    scratch.write("sub/a.gcsl", "a = 1 + \"s\"\n");
    scratch.write("nested/workspace.gcsl", "");
    scratch.write("nested/deeper/workspace.gcsl", "");
    scratch.write("nested/b.gcsl", CHECKED);
    scratch.write(".hidden/workspace.gcsl", "");
    scratch.write(".hidden/h.gcsl", "h = 1 + \"s\"\n");
    let check = |dir: &str, args: &[&str]| {
        let mut all = vec!["check"];
        all.extend_from_slice(args);
        let output = gcsl_in(&scratch.0.join(dir), &all, "");
        let summary = stderr(&output).lines().last().unwrap_or("").to_owned();
        (summary, output)
    };
    // No paths: the workspace of the working directory, alone.
    let (summary, output) = check("sub", &[]);
    assert_eq!(summary, "1 error, 0 warnings in 2 files checked");
    assert!(
        stderr(&output).contains("--> a.gcsl:1:7"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1));
    // A directory: the workspace there and those nested in it, but those
    // under a name starting with `.`.
    let (summary, output) = check("", &["."]);
    assert_eq!(summary, "1 error, 1 warning in 5 files checked");
    assert!(
        stderr(&output).contains("--> sub/a.gcsl:1:7"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("--> nested/b.gcsl:2:"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(1));
    // A nested workspace, alone: warnings only, code 0. Given twice, as a
    // directory and by a file of it, it is checked once.
    let (summary, output) = check("", &["nested", "nested/b.gcsl"]);
    assert_eq!(summary, "0 errors, 1 warning in 3 files checked");
    assert_eq!(output.status.code(), Some(0));
    // A directory inside a workspace: that workspace.
    let (summary, _) = check("", &["sub"]);
    assert_eq!(summary, "1 error, 0 warnings in 2 files checked");
    // A file of the hidden directory, which is a workspace of its own.
    let (summary, _) = check("", &[".hidden/h.gcsl"]);
    assert_eq!(summary, "1 error, 0 warnings in 2 files checked");
    // A file in no workspace, alone; a path that is not there.
    let outside = FmtScratch::new("check-outside");
    outside.write("lone.gcsl", "x = 1\n");
    let output = gcsl_in(&outside.0, &["check", "lone.gcsl"], "");
    assert_eq!(stderr(&output), "no problems in 1 file checked\n");
    assert_eq!(output.status.code(), Some(0));
    let output = gcsl_in(&outside.0, &["check", "lone.gcsl", "missing.gcsl"], "");
    assert_eq!(
        stderr(&output),
        "error: cannot read missing.gcsl: no such file or directory\n"
    );
    assert_eq!(output.status.code(), Some(2));
    // No paths, and no workspace above.
    let output = gcsl_in(&outside.0, &["check"], "");
    assert!(
        stderr(&output).contains("no `workspace.gcsl`"),
        "{}",
        stderr(&output)
    );
    assert_eq!(output.status.code(), Some(2));
}

/// `gcsl check --message-format json` prints each diagnostic as a JSON
/// object on a line of standard output, columns counted in characters, and
/// the summary to standard error, rendering nothing.
#[test]
fn check_prints_json_lines() {
    let scratch = FmtScratch::new("check-json");
    scratch.write("workspace.gcsl", "");
    scratch.write("app.gcsl", "y = \"é\" + 1\n");
    scratch.write("b.gcsl", CHECKED);
    let output = gcsl_in(&scratch.0, &["check", "--message-format", "json"], "");
    assert_eq!(stderr(&output), "1 error, 1 warning in 3 files checked\n");
    assert_eq!(output.status.code(), Some(1));
    let lines: Vec<serde_json::Value> = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let [error, warning] = lines.as_slice() else {
        panic!("{}", stdout(&output));
    };
    assert_eq!(
        *error,
        serde_json::json!({
            "path": "app.gcsl",
            "severity": "error",
            "message": "cannot add String and Int",
            "line": 1,
            "column": 9,
            "end_line": 1,
            "end_column": 10,
            "labels": [
                {"path": "app.gcsl", "line": 1, "column": 5, "end_line": 1, "end_column": 8, "text": "String"},
                {"path": "app.gcsl", "line": 1, "column": 11, "end_line": 1, "end_column": 12, "text": "Int"},
            ],
            "notes": [],
        })
    );
    assert_eq!(warning["path"], "b.gcsl");
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["line"], 2);
    assert!(
        !warning["notes"].as_array().unwrap().is_empty(),
        "{warning}"
    );
    // Nothing on standard output for a clean workspace.
    scratch.write("app.gcsl", "y = 1\n");
    let output = gcsl_in(
        &scratch.0,
        &["check", "--message-format", "json", "app.gcsl"],
        "",
    );
    assert_eq!(stdout(&output).lines().count(), 1);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn piped_repl_input_never_prints_terminal_prompts() {
    for term in ["dumb", "xterm"] {
        let mut child = Command::new(binary())
            .env("TERM", term)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"1 + 1\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(stdout(&output), "2 : Int\n", "TERM={term}");
        assert_eq!(stderr(&output), "", "TERM={term}");
        assert!(output.status.success(), "TERM={term}");
    }
}
