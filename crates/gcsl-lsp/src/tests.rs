//! In-memory LSP protocol and editor-behavior regression tests.

use std::collections::VecDeque;
use std::fs;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use lsp_types::notification::PublishDiagnostics;
use lsp_types::{DiagnosticSeverity, Position, PublishDiagnosticsParams, Range};
use serde_json::json;

use super::*;
use crate::place::uri_of;

/// How long a test waits for the server before failing.
const WAIT: Duration = Duration::from_secs(30);

/// A scratch directory of the test `name`'s own, empty, canonical,
/// under the system's temporary directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gcsl-lsp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(fs::canonicalize(dir).unwrap())
    }

    /// A workspace: the scratch directory with a `workspace.gcsl` of
    /// `text`.
    fn workspace(name: &str, text: &str) -> Self {
        let scratch = Self::new(name);
        scratch.write(WORKSPACE, text);
        scratch
    }

    /// Writes `text` to the file at `path` below the directory, and
    /// returns its URI.
    fn write(&self, path: &str, text: &str) -> Uri {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        uri_of(&path).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A client of a server serving on a thread of its own, through a
/// connection in memory.
struct Client {
    connection: Connection,
    server: JoinHandle<Result<(), Error>>,
    checks: Arc<AtomicUsize>,
    next: i32,
    /// Notifications received while waiting for something else.
    pending: VecDeque<Notification>,
}

impl Client {
    /// Starts a server and initialises it, offering the position
    /// encodings `encodings`; returns the client and the result of
    /// `initialize`.
    fn start(encodings: &[&str]) -> (Self, Value) {
        Self::start_with(&json!({ "general": { "positionEncodings": encodings } }))
    }

    /// Starts a server and initialises it, with the client's
    /// capabilities `capabilities`.
    fn start_with(capabilities: &Value) -> (Self, Value) {
        let (server_end, connection) = Connection::memory();
        let checks = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&checks);
        let server = thread::spawn(move || serve_counting(&server_end, &counted));
        let mut client = Self {
            connection,
            server,
            checks,
            next: 0,
            pending: VecDeque::new(),
        };
        let response = client.request("initialize", json!({ "capabilities": capabilities }));
        client.notify("initialized", json!({}));
        (client, response.response_result.unwrap())
    }

    /// Sends the notification `method` with `params`.
    fn notify(&self, method: &str, params: Value) {
        let notification = Notification::new(method.to_owned(), params);
        self.connection.sender.send(notification.into()).unwrap();
    }

    /// Sends the request `method` with `params` and returns the
    /// response, keeping the notifications that come first.
    fn request(&mut self, method: &str, params: Value) -> Response {
        self.next += 1;
        let id = RequestId::from(self.next);
        let request = Request::new(id.clone(), method.to_owned(), params);
        self.connection.sender.send(request.into()).unwrap();
        loop {
            match self.receive() {
                Message::Response(response) if response.id == id => return response,
                Message::Notification(notification) => self.pending.push_back(notification),
                message => panic!("unexpected {message:?}"),
            }
        }
    }

    /// The next message from the server.
    fn receive(&self) -> Message {
        self.connection
            .receiver
            .recv_timeout(WAIT)
            .expect("the server did not answer in time")
    }

    /// The next diagnostics published for `uri`, those for other files
    /// kept.
    fn published(&mut self, uri: &Uri) -> PublishDiagnosticsParams {
        let is_for = |notification: &Notification| {
            notification.method == PublishDiagnostics::METHOD
                && notification.params["uri"] == uri.as_str()
        };
        if let Some(at) = self.pending.iter().position(is_for) {
            let notification = self.pending.remove(at).unwrap();
            return serde_json::from_value(notification.params).unwrap();
        }
        loop {
            match self.receive() {
                Message::Notification(notification) if is_for(&notification) => {
                    return serde_json::from_value(notification.params).unwrap();
                }
                Message::Notification(notification) => self.pending.push_back(notification),
                message => panic!("unexpected {message:?}"),
            }
        }
    }

    /// The diagnostics published for `uri` for its version `version`,
    /// those for earlier versions skipped.
    fn published_at(&mut self, uri: &Uri, version: i32) -> PublishDiagnosticsParams {
        loop {
            let published = self.published(uri);
            if published.version == Some(version) {
                return published;
            }
        }
    }

    /// Opens the document `uri` with `text`, as version 1.
    fn open(&self, uri: &Uri, text: &str) {
        let item = json!({ "uri": uri, "languageId": "gcsl", "version": 1, "text": text });
        self.notify("textDocument/didOpen", json!({ "textDocument": item }));
    }

    /// Changes the document `uri` to its version `version`, replacing
    /// what is between `start` and `end` with `text`.
    fn change(&self, uri: &Uri, version: i32, (start, end): ((u32, u32), (u32, u32)), text: &str) {
        let range = Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1));
        let change = json!({ "range": range, "text": text });
        let document = json!({ "uri": uri, "version": version });
        let params = json!({ "textDocument": document, "contentChanges": [change] });
        self.notify("textDocument/didChange", params);
    }

    /// The result of the request `method` (a query) at `(line,
    /// character)` of the document `uri`.
    fn ask(&mut self, method: &str, uri: &Uri, (line, character): (u32, u32)) -> Value {
        let position = json!({ "line": line, "character": character });
        let params = json!({ "textDocument": { "uri": uri }, "position": position });
        self.request(method, params).response_result.unwrap()
    }

    /// The hover at `at` of the document `uri`.
    fn hover(&mut self, uri: &Uri, at: (u32, u32)) -> Value {
        self.ask("textDocument/hover", uri, at)
    }

    /// The definition at `at` of the document `uri`.
    fn definition(&mut self, uri: &Uri, at: (u32, u32)) -> Value {
        self.ask("textDocument/definition", uri, at)
    }

    /// Closes the document `uri`.
    fn close(&self, uri: &Uri) {
        let params = json!({ "textDocument": { "uri": uri } });
        self.notify("textDocument/didClose", params);
    }

    /// How many checks the server has made.
    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    /// Shuts the server down, and returns how it ended.
    fn shut_down(mut self) -> Result<(), Error> {
        let response = self.request("shutdown", Value::Null);
        assert_eq!(response.response_result.unwrap(), Value::Null);
        self.notify("exit", Value::Null);
        self.server.join().unwrap()
    }
}

/// The range from `(l1, c1)` to `(l2, c2)`.
fn range((l1, c1): (u32, u32), (l2, c2): (u32, u32)) -> Range {
    Range::new(Position::new(l1, c1), Position::new(l2, c2))
}

#[test]
fn initialize_negotiates_utf_8_when_offered() {
    let (client, result) = Client::start(&["utf-16", "utf-8"]);
    let capabilities = &result["capabilities"];
    assert_eq!(capabilities["positionEncoding"], "utf-8");
    assert_eq!(
        capabilities["textDocumentSync"],
        json!({ "openClose": true, "change": 2, "save": true })
    );
    assert_eq!(capabilities["hoverProvider"], true);
    assert_eq!(capabilities["definitionProvider"], true);
    assert_eq!(capabilities["referencesProvider"], true);
    assert_eq!(capabilities["documentHighlightProvider"], true);
    assert_eq!(
        capabilities["renameProvider"],
        json!({ "prepareProvider": true })
    );
    assert_eq!(capabilities["documentSymbolProvider"], true);
    assert_eq!(capabilities["workspaceSymbolProvider"], true);
    assert_eq!(capabilities["documentFormattingProvider"], true);
    assert_eq!(capabilities["documentRangeFormattingProvider"], true);
    assert_eq!(capabilities["inlayHintProvider"], true);
    assert_eq!(
        capabilities["codeActionProvider"],
        json!({ "codeActionKinds": ["quickfix", "source.fixAll"], "resolveProvider": false })
    );
    assert_eq!(result["serverInfo"]["name"], "gcsl");
    client.shut_down().unwrap();
}

/// The code actions the server answers for the range `range` of the
/// document `uri`, the client sending `diagnostics` and, if any, the
/// kinds `only`.
fn code_actions(
    client: &mut Client,
    uri: &Uri,
    range: Range,
    diagnostics: &Value,
    only: Option<&[&str]>,
) -> Vec<Value> {
    let mut context = json!({ "diagnostics": diagnostics });
    if let Some(only) = only {
        context["only"] = json!(only);
    }
    let params = json!({ "textDocument": { "uri": uri }, "range": range, "context": context });
    let response = client.request("textDocument/codeAction", params);
    let Value::Array(actions) = response.response_result.unwrap() else {
        panic!("not a list of actions");
    };
    actions
}

/// The edits of the code action `action` of the document `uri`, each as
/// its range and its new text.
fn action_edits(action: &Value, uri: &Uri) -> Vec<(Range, String)> {
    let edits = action["edit"]["changes"][uri.as_str()].as_array().unwrap();
    edits
        .iter()
        .map(|edit| {
            let text = edit["newText"].as_str().unwrap().to_owned();
            (as_range(&edit["range"]), text)
        })
        .collect()
}

#[test]
fn a_quick_fix_changes_a_misspelled_name_and_the_file_checks_clean() {
    let scratch = Scratch::workspace("quickfix", "");
    let text = "version = 1\nx = versoin + 1\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published_at(&uri, 1);
    assert_eq!(published.diagnostics.len(), 1);
    let diagnostic = &published.diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "undefined variable `versoin`\nhelp: did you mean `version`?"
    );
    let sent = json!([diagnostic]);
    let actions = code_actions(&mut client, &uri, diagnostic.range, &sent, None);
    assert_eq!(actions.len(), 1, "{actions:?}");
    let action = &actions[0];
    assert_eq!(action["title"], "Change to `version`");
    assert_eq!(action["kind"], "quickfix");
    assert_eq!(action["isPreferred"], true);
    assert_eq!(action["diagnostics"], sent);
    let edits = action_edits(action, &uri);
    assert_eq!(edits, [(range((1, 4), (1, 11)), "version".to_owned())]);
    // A cursor anywhere on the diagnostic has it too.
    let at = range((1, 6), (1, 6));
    assert_eq!(code_actions(&mut client, &uri, at, &sent, None).len(), 1);
    // Applied, the file checks clean.
    let (edit_range, new_text) = &edits[0];
    let (start, end) = (edit_range.start, edit_range.end);
    let span = ((start.line, start.character), (end.line, end.character));
    client.change(&uri, 2, span, new_text);
    assert!(client.published_at(&uri, 2).diagnostics.is_empty());
    let whole = range((0, 0), (2, 0));
    assert!(code_actions(&mut client, &uri, whole, &json!([]), None).is_empty());
    client.shut_down().unwrap();
}

#[test]
fn nothing_is_offered_for_a_diagnostic_without_a_suggestion() {
    let scratch = Scratch::workspace("nofix", "");
    let text = "version = 1\nx = 1 + \"a\"\ny = zzzzz\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published_at(&uri, 1);
    assert_eq!(published.diagnostics.len(), 2);
    assert!(
        published
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("help:"))
    );
    let sent = serde_json::to_value(&published.diagnostics).unwrap();
    let whole = range((0, 0), (3, 0));
    assert!(code_actions(&mut client, &uri, whole, &sent, None).is_empty());
    client.shut_down().unwrap();
}

#[test]
fn a_stale_code_action_request_is_answered_from_the_text_as_changed() {
    let scratch = Scratch::workspace("stalefix", "");
    let text = "version = 1\nx = versoin\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let old = client.published_at(&uri, 1).diagnostics;
    assert_eq!(old[0].range, range((1, 4), (1, 11)));
    // A line added above, and the request at once, with the diagnostic
    // as the client has it, of the old text: the index is checked again
    // first, and the edit is of the new text, with the diagnostic as it
    // is now.
    client.change(&uri, 2, ((0, 0), (0, 0)), "\n");
    let stale = json!(old);
    let actions = code_actions(&mut client, &uri, range((2, 4), (2, 11)), &stale, None);
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(
        action_edits(&actions[0], &uri),
        [(range((2, 4), (2, 11)), "version".to_owned())]
    );
    let attached = &actions[0]["diagnostics"][0];
    assert_eq!(as_range(&attached["range"]), range((2, 4), (2, 11)));
    // The old place has nothing now.
    let at = range((1, 4), (1, 4));
    assert!(code_actions(&mut client, &uri, at, &stale, None).is_empty());
    client.shut_down().unwrap();
}

#[test]
fn every_suggestion_of_a_file_is_applied_at_once_when_there_are_two() {
    let scratch = Scratch::workspace("fixall", "");
    let text = "version = 1\nx = versoin\ny = verison\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    assert_eq!(client.published_at(&uri, 1).diagnostics.len(), 2);
    let at = range((1, 5), (1, 5));
    let actions = code_actions(&mut client, &uri, at, &json!([]), None);
    let titles: Vec<&str> = actions
        .iter()
        .map(|action| action["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        ["Change to `version`", "Apply all suggestions in this file"]
    );
    let all = &actions[1];
    assert_eq!(all["kind"], "source.fixAll");
    assert_eq!(all["diagnostics"].as_array().unwrap().len(), 2);
    assert_eq!(
        action_edits(all, &uri),
        [
            (range((1, 4), (1, 11)), "version".to_owned()),
            (range((2, 4), (2, 11)), "version".to_owned())
        ]
    );
    // The kinds asked for, alone.
    let only = code_actions(&mut client, &uri, at, &json!([]), Some(&["quickfix"]));
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["kind"], "quickfix");
    let only = code_actions(&mut client, &uri, at, &json!([]), Some(&["source"]));
    assert_eq!(only.len(), 1);
    assert_eq!(only[0]["kind"], "source.fixAll");
    client.shut_down().unwrap();
}

#[test]
fn initialize_falls_back_to_utf_16() {
    let (client, result) = Client::start(&["utf-32", "utf-16"]);
    assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
    client.shut_down().unwrap();
    // Offering none at all.
    let (client, result) = Client::start(&[]);
    assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
    client.shut_down().unwrap();
}

#[test]
fn a_type_error_is_published_and_cleared_by_an_incremental_change() {
    let scratch = Scratch::workspace("type-error", "");
    let text = "x = 1\ny = x + \"a\"\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published(&uri);
    assert_eq!(published.version, Some(1));
    let [diagnostic] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(diagnostic.message, "cannot add Int and String");
    assert_eq!(diagnostic.range, range((1, 6), (1, 7)));
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diagnostic.source.as_deref(), Some("gcsl"));
    // The operands' types, as related information, in the file.
    let related = diagnostic.related_information.as_ref().unwrap();
    let shown: Vec<(&str, Range)> = related
        .iter()
        .map(|info| {
            assert_eq!(info.location.uri, uri);
            (info.message.as_str(), info.location.range)
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("Int", range((1, 4), (1, 5))),
            ("String", range((1, 8), (1, 11)))
        ]
    );
    // `"a"` becomes `1`: the error goes, and the list is emptied.
    client.change(&uri, 2, ((1, 8), (1, 11)), "1");
    let published = client.published(&uri);
    assert_eq!(published.version, Some(2));
    assert_eq!(published.diagnostics, []);
    client.shut_down().unwrap();
}

#[test]
fn a_burst_of_changes_is_checked_once() {
    let scratch = Scratch::workspace("burst", "");
    let text = "y = 1 + \"a\" # \n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    assert_eq!(client.published(&uri).diagnostics.len(), 1);
    let before = client.checks();
    // Twenty keystrokes into the comment, faster than the quiet period.
    for version in 2..=21 {
        let at = 12 + u32::try_from(version).unwrap();
        client.change(&uri, version, ((0, at), (0, at)), "x");
    }
    let published = client.published_at(&uri, 21);
    assert_eq!(published.diagnostics.len(), 1);
    thread::sleep(QUIET * 3);
    let checks = client.checks() - before;
    assert!((1..=2).contains(&checks), "{checks} checks");
    client.shut_down().unwrap();
}

#[test]
fn an_open_document_s_unsaved_text_is_checked_not_its_file_s() {
    let scratch = Scratch::workspace("unsaved", "");
    let a = scratch.write("a.gcsl", "z = y + 1\n");
    let b = scratch.write("b.gcsl", "y = 1\n");
    let (mut client, _) = Client::start(&["utf-16"]);
    // `b` as edited, not saved: `a`, not open, has the error.
    client.open(&b, "y = \"s\"\n");
    let published = client.published(&a);
    assert_eq!(published.version, None);
    let [diagnostic] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(diagnostic.message, "cannot add String and Int");
    assert_eq!(diagnostic.range, range((0, 6), (0, 7)));
    // Closed, `b` has its text on disk again, and `a` is clean.
    client.close(&b);
    assert_eq!(client.published(&a).diagnostics, []);
    client.shut_down().unwrap();
}

#[test]
fn related_information_points_into_other_files() {
    let scratch = Scratch::workspace("related", "");
    let a = scratch.write("a.gcsl", "x = 1\n");
    let b = scratch.write("b.gcsl", "x = 2\n");
    let (client, _) = Client::start(&["utf-8"]);
    client.open(&a, "x = 1\n");
    // Whichever file the error is in, a label is in the other.
    let message = client.receive();
    let Message::Notification(notification) = message else {
        panic!("{message:?}");
    };
    let published: PublishDiagnosticsParams = serde_json::from_value(notification.params).unwrap();
    let other = if published.uri == a { &b } else { &a };
    let [diagnostic] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    let related = diagnostic.related_information.as_ref().unwrap();
    assert!(
        related
            .iter()
            .any(|info| info.location.uri == *other
                && info.location.range == range((0, 0), (0, 1))),
        "{related:?}"
    );
    client.shut_down().unwrap();
}

#[test]
fn a_parse_error_is_published() {
    let scratch = Scratch::workspace("parse", "");
    let text = "x = 1\ny = (1 +\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published(&uri);
    let [diagnostic] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert!(
        diagnostic.message.starts_with("expected "),
        "{}",
        diagnostic.message
    );
    assert!(diagnostic.range.start.line >= 1, "{:?}", diagnostic.range);
    client.shut_down().unwrap();
}

#[test]
fn a_syntax_error_hides_no_other_files_type_error() {
    let scratch = Scratch::workspace("syntax-and-type", "");
    let other = scratch.write("other.gcsl", "z: Int = true\n");
    let text = "x = (1 2)\ny = 3\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published(&uri);
    let [syntax] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(syntax.message, "expected `)`, found `2`");
    let published = client.published(&other);
    let [mismatched] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(mismatched.message, "mismatched types");
    assert_eq!(mismatched.range, range((0, 9), (0, 13)));
    client.shut_down().unwrap();
}

#[test]
fn a_scan_error_hides_no_other_files_errors_or_answers() {
    // clackatoo's minimal case: a field of `q` defined in two files.
    let scratch = Scratch::workspace("scan-error", "# Root.\n");
    let text = "h = 1\nk = h\n";
    let p = scratch.write("p/p.gcsl", text);
    scratch.write("q/a.gcsl", "x = 1\n");
    let b = scratch.write("q/b.gcsl", "x = 2\n");
    let r = scratch.write("r/r.gcsl", "y: Int = \"s\"\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&p, text);
    let published = client.published(&b);
    let [conflict] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(conflict.message, "field `x` is defined in two files");
    let published = client.published(&r);
    let [mismatched] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(mismatched.message, "mismatched types");
    assert_eq!(mismatched.range, range((0, 9), (0, 12)));
    assert_eq!(client.published(&p).diagnostics, []);
    // `p` is indexed: its hover and definition answer.
    let hover = client.hover(&p, (1, 4));
    let shown = hover["contents"]["value"].as_str().unwrap();
    assert!(shown.contains("h: Int"), "{shown}");
    let definition = client.definition(&p, (1, 4));
    assert_eq!(definition["uri"], p.as_str());
    assert_eq!(as_range(&definition["range"]), range((0, 0), (0, 1)));
    client.shut_down().unwrap();
}

#[test]
fn a_non_ascii_line_is_counted_in_each_encoding() {
    // `é` is two bytes and one UTF-16 unit, `😀` four bytes and two.
    let text = "x = \"é😀\" + 1\n";
    for (encoding, plus) in [("utf-8", 13), ("utf-16", 10)] {
        let scratch = Scratch::workspace(&format!("non-ascii-{encoding}"), "");
        let uri = scratch.write("app.gcsl", text);
        let (mut client, result) = Client::start(&[encoding]);
        assert_eq!(result["capabilities"]["positionEncoding"], encoding);
        client.open(&uri, text);
        let published = client.published(&uri);
        let [diagnostic] = published.diagnostics.as_slice() else {
            panic!("{published:?}");
        };
        assert_eq!(
            diagnostic.range,
            range((0, plus), (0, plus + 1)),
            "{encoding}"
        );
        // An edit after the emoji, counted in the encoding: `+ 1` goes.
        let end = plus + 3;
        client.change(&uri, 2, ((0, plus - 1), (0, end)), "");
        assert_eq!(client.published(&uri).diagnostics, [], "{encoding}");
        client.shut_down().unwrap();
    }
}

#[test]
fn a_lone_file_is_checked_alone() {
    let scratch = Scratch::new("lone");
    let text = "x = 1 + \"a\"\n";
    let uri = scratch.write("lone.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published(&uri);
    assert_eq!(published.version, Some(1));
    let [diagnostic] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(diagnostic.range, range((0, 6), (0, 7)));
    // Closed, it is forgotten.
    client.close(&uri);
    assert_eq!(client.published(&uri).diagnostics, []);
    client.shut_down().unwrap();
}

#[test]
fn a_document_not_on_disk_yet_is_a_file_of_its_workspace() {
    let scratch = Scratch::workspace("unwritten", "");
    scratch.write("lib.gcsl", "base = 1\n");
    let path = scratch.0.join("sub/new.gcsl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let uri = uri_of(&path).unwrap();
    let (mut client, _) = Client::start(&["utf-8"]);
    // Alone, `base` would be undefined: in the workspace, it is an Int.
    client.open(&uri, "x = base + \"a\"\n");
    let published = client.published(&uri);
    let [diagnostic] = published.diagnostics.as_slice() else {
        panic!("{published:?}");
    };
    assert_eq!(diagnostic.message, "cannot add Int and String");
    assert!(!path.exists());
    client.shut_down().unwrap();
}

#[test]
fn watched_files_are_registered_for_and_read_again() {
    let scratch = Scratch::workspace("watched", "");
    let a = scratch.write("a.gcsl", "z = y + 1\n");
    let b = scratch.write("b.gcsl", "y = 1\n");
    let capabilities = json!({
        "general": { "positionEncodings": ["utf-8"] },
        "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } },
    });
    let (mut client, _) = Client::start_with(&capabilities);
    let message = client.receive();
    let Message::Request(request) = message else {
        panic!("{message:?}");
    };
    assert_eq!(request.method, "client/registerCapability");
    let registration = &request.params["registrations"][0];
    assert_eq!(registration["method"], "workspace/didChangeWatchedFiles");
    assert_eq!(
        registration["registerOptions"]["watchers"][0]["globPattern"],
        "**/*.gcsl"
    );
    let response = Response::new_ok(request.id, Value::Null);
    client.connection.sender.send(response.into()).unwrap();
    client.open(&a, "z = y + 1\n");
    // Checked, and clean: its first publish says so.
    assert_eq!(client.published(&a).diagnostics, []);
    // `b` changes on disk, which the client reports.
    scratch.write("b.gcsl", "y = \"s\"\n");
    let changes = json!({ "changes": [{ "uri": b, "type": 2 }] });
    client.notify("workspace/didChangeWatchedFiles", changes);
    let published = client.published(&a);
    assert_eq!(published.diagnostics.len(), 1);
    assert_eq!(published.version, Some(1));
    client.shut_down().unwrap();
}

#[test]
fn shutdown_then_exit_ends_cleanly() {
    let (mut client, _) = Client::start(&["utf-8"]);
    // Unknown requests are not found; `workspace/configuration` is
    // answered with nulls; unknown notifications are ignored.
    let response = client.request("gcsl/noSuchMethod", json!({}));
    let error = response.response_result.unwrap_err();
    assert_eq!(error.code, ErrorCode::MethodNotFound as i32);
    let items = json!({ "items": [{ "section": "gcsl" }, {}] });
    let response = client.request("workspace/configuration", items.clone());
    assert_eq!(response.response_result.unwrap(), json!([null, null]));
    client.notify("$/unknown", json!({}));
    // After `shutdown`, a request is an error.
    let response = client.request("shutdown", Value::Null);
    assert_eq!(response.response_result.unwrap(), Value::Null);
    let response = client.request("workspace/configuration", items);
    let error = response.response_result.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest as i32);
    client.notify("exit", Value::Null);
    client.server.join().unwrap().unwrap();
    // `exit` alone is not clean.
    let (client, _) = Client::start(&["utf-8"]);
    client.notify("exit", Value::Null);
    let ended = client.server.join().unwrap();
    assert!(
        matches!(ended, Err(Error::ExitWithoutShutdown)),
        "{ended:?}"
    );
}

#[test]
fn nothing_is_run() {
    let scratch = Scratch::workspace("nothing-runs", "");
    let marker = scratch.0.join("MARKER");
    let marker = marker.display();
    let text = format!("x = sh(\"touch {marker}\")\ny = exec([\"touch\", \"{marker}\"])\n");
    let uri = scratch.write("run.gcsl", &text);
    // A file with an error, so that the check's end is published.
    let bad = scratch.write("bad.gcsl", "z = 1 + \"a\"\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, &text);
    assert_eq!(client.published(&bad).diagnostics.len(), 1);
    assert!(client.checks() >= 1);
    assert!(!scratch.0.join("MARKER").exists());
    client.shut_down().unwrap();
}

/// `value`, a `Range` as JSON.
fn as_range(value: &Value) -> Range {
    serde_json::from_value(value.clone()).unwrap()
}

#[test]
fn hover_and_definition_count_positions_in_each_encoding() {
    // `é` is two bytes and one UTF-16 unit, `😀` four bytes and two:
    // the `s` after them is at character 15 in utf-8, 12 in utf-16.
    let text = "s = \"a\"\nt = \"é😀\" + s\n";
    for (encoding, at) in [("utf-8", 15), ("utf-16", 12)] {
        let scratch = Scratch::workspace(&format!("hover-{encoding}"), "");
        let uri = scratch.write("app.gcsl", text);
        let (mut client, _) = Client::start(&[encoding]);
        client.open(&uri, text);
        let hover = client.hover(&uri, (1, at));
        assert_eq!(hover["contents"]["kind"], "markdown", "{encoding}");
        let shown = hover["contents"]["value"].as_str().unwrap();
        assert!(shown.contains("s: String"), "{encoding}: {shown}");
        assert_eq!(
            as_range(&hover["range"]),
            range((1, at), (1, at + 1)),
            "{encoding}"
        );
        // A position inside the emoji's second half is its start, which
        // is in a string literal: nothing.
        assert_eq!(client.hover(&uri, (1, 7)), Value::Null, "{encoding}");
        let definition = client.definition(&uri, (1, at));
        assert_eq!(definition["uri"], uri.as_str(), "{encoding}");
        assert_eq!(
            as_range(&definition["range"]),
            range((0, 0), (0, 1)),
            "{encoding}"
        );
        client.shut_down().unwrap();
    }
}

#[test]
fn a_definition_in_another_file_is_located_there() {
    let scratch = Scratch::workspace("cross-file", "");
    let a = scratch.write("a.gcsl", "z = y + 1\n");
    let b = scratch.write("b.gcsl", "x = 0\ny = 1\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&a, "z = y + 1\n");
    let definition = client.definition(&a, (0, 4));
    assert_eq!(definition["uri"], b.as_str());
    assert_eq!(as_range(&definition["range"]), range((1, 0), (1, 1)));
    client.shut_down().unwrap();
}

#[test]
fn a_stale_query_is_answered_from_the_text_as_changed() {
    let scratch = Scratch::workspace("stale", "");
    let text = "x = 1\ny = x\nz = x + 1\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let shown = client.hover(&uri, (1, 4));
    assert!(
        shown["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("x: Int"),
        "{shown}"
    );
    // `1` becomes `"s"`, and the hover follows at once, well inside
    // the quiet period.
    let before = client.checks();
    client.change(&uri, 2, ((0, 4), (0, 5)), "\"s\"");
    let shown = client.hover(&uri, (1, 4));
    assert!(
        shown["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("x: String"),
        "{shown}"
    );
    // The check the query made is published as any is: `z` is now an
    // error. It was the check the change asked for, which is not made
    // again once its quiet period is over.
    assert_eq!(client.published_at(&uri, 2).diagnostics.len(), 1);
    thread::sleep(QUIET * 3);
    assert_eq!(client.checks() - before, 1);
    // A line moved: the definition is where the text now has it.
    client.change(&uri, 3, ((0, 0), (0, 0)), "\n\n");
    let definition = client.definition(&uri, (3, 4));
    assert_eq!(as_range(&definition["range"]), range((2, 0), (2, 1)));
    client.shut_down().unwrap();
}

#[test]
fn completion_and_signature_help_are_offered() {
    let (client, result) = Client::start(&["utf-8"]);
    let capabilities = &result["capabilities"];
    assert_eq!(
        capabilities["completionProvider"],
        json!({ "triggerCharacters": ["."], "resolveProvider": false })
    );
    assert_eq!(
        capabilities["signatureHelpProvider"],
        json!({ "triggerCharacters": ["(", ","], "retriggerCharacters": [","] })
    );
    client.shut_down().unwrap();
}

/// The labels of the completion list `list`.
fn completion_labels(list: &Value) -> Vec<String> {
    assert_eq!(list["isIncomplete"], false, "{list}");
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_completion_just_after_a_dot_is_typed_is_of_the_new_text_in_each_encoding() {
    // `é` is two bytes and one UTF-16 unit, `😀` four bytes and two: the
    // end of `c` is at character 24 in utf-8, 21 in utf-16.
    let text = "c = {port = 80; host = \"h\"}\nt = {s = \"é😀\"; v = c}\n";
    for (encoding, at) in [("utf-8", 24), ("utf-16", 21)] {
        let scratch = Scratch::workspace(&format!("complete-{encoding}"), "");
        let uri = scratch.write("app.gcsl", text);
        let (mut client, _) = Client::start(&[encoding]);
        client.open(&uri, text);
        client.published(&uri);
        // Typed: `c.`, and completion asked at once, after the `.`.
        client.change(&uri, 2, ((1, at), (1, at)), ".");
        let list = client.ask("textDocument/completion", &uri, (1, at + 1));
        assert_eq!(completion_labels(&list), ["host", "port"], "{encoding}");
        let port = &list["items"][1];
        assert_eq!(port["kind"], 5, "{encoding}: a field");
        assert_eq!(port["detail"], "Int", "{encoding}");
        // A name: the file's, the standard library's, keywords.
        let list = client.ask("textDocument/completion", &uri, (2, 0));
        let labels = completion_labels(&list);
        for label in ["c", "t", "len", "if"] {
            assert!(labels.iter().any(|l| l == label), "{encoding}: {labels:?}");
        }
        // In a string: an empty list.
        let list = client.ask("textDocument/completion", &uri, (1, 10));
        assert_eq!(completion_labels(&list), Vec::<String>::new(), "{encoding}");
        client.shut_down().unwrap();
    }
}

#[test]
fn signature_help_names_the_active_parameter() {
    let scratch = Scratch::workspace("signature", "");
    // Below the file's first line, whose comment would be its header.
    let text = "w = 0\n# Adds.\ndef f(a: Int, b: String): Int { a }\nr = f(1, \"s\")\n";
    let uri = scratch.write("app.gcsl", text);
    for encoding in ["utf-8", "utf-16"] {
        let (mut client, _) = Client::start(&[encoding]);
        client.open(&uri, text);
        let help = client.ask("textDocument/signatureHelp", &uri, (3, 9));
        let label = "def f(a: Int, b: String): Int";
        assert_eq!(help["signatures"][0]["label"], label, "{encoding}");
        assert_eq!(
            help["signatures"][0]["parameters"],
            json!([{ "label": [6, 12] }, { "label": [14, 23] }]),
            "{encoding}"
        );
        assert_eq!(help["signatures"][0]["documentation"]["value"], "Adds.");
        assert_eq!(help["activeSignature"], 0);
        assert_eq!(help["activeParameter"], 1, "{encoding}");
        // Typed: `f(` at the end of the file, unclosed.
        client.change(&uri, 2, ((4, 0), (4, 0)), "q = f(");
        let help = client.ask("textDocument/signatureHelp", &uri, (4, 6));
        assert_eq!(help["activeParameter"], 0, "{encoding}");
        // Outside any call: nothing.
        assert_eq!(
            client.ask("textDocument/signatureHelp", &uri, (3, 0)),
            Value::Null
        );
        client.shut_down().unwrap();
    }
}

#[test]
fn completions_edit_the_whole_name_and_offer_named_arguments() {
    let scratch = Scratch::workspace("completion-edits", "");
    let text = "odd.libcap-tools = 1\nodd.other = 2\nr = odd.libcap-t\n\
                s = {\"a b\" = 1}\nq = s.\ndef f(a: Int, b: Int = 2): Int { a }\n\
                n = f(1, )\ndef pair(a: String, body: {x: Int}): String { a }\n";
    let uri = scratch.write("app.gcsl", text);
    let item = |list: &Value, label: &str| {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == label)
            .unwrap_or_else(|| panic!("no {label} in {list}"))
            .clone()
    };
    let as_edit =
        |edit: &Value| -> lsp_types::TextEdit { serde_json::from_value(edit.clone()).unwrap() };
    // A client without `insertReplaceSupport`: a `TextEdit` over what is
    // typed of the name, `-` and all.
    let (mut client, _) = Client::start(&["utf-16"]);
    client.open(&uri, text);
    let list = client.ask("textDocument/completion", &uri, (2, 16));
    let edit = as_edit(&item(&list, "libcap-tools")["textEdit"]);
    assert_eq!(edit.range, range((2, 8), (2, 16)));
    assert_eq!(edit.new_text, "libcap-tools");
    // A quoted name is written quoted.
    let list = client.ask("textDocument/completion", &uri, (4, 6));
    let edit = as_edit(&item(&list, "a b")["textEdit"]);
    assert_eq!(edit.range, range((4, 6), (4, 6)));
    assert_eq!(edit.new_text, "\"a b\"");
    // A named argument, first, with its type.
    let list = client.ask("textDocument/completion", &uri, (6, 9));
    let named = &list["items"][0];
    assert_eq!(named["label"], "b =");
    assert_eq!(
        named["kind"],
        json!(lsp_types::CompletionItemKind::PROPERTY)
    );
    assert_eq!(named["detail"], "Int");
    assert_eq!(as_edit(&named["textEdit"]).new_text, "b = ");
    // Signature help in a trailing block left open at the end.
    client.change(&uri, 2, ((8, 0), (8, 0)), "p = pair(\"a\") {");
    let help = client.ask("textDocument/signatureHelp", &uri, (8, 15));
    assert_eq!(
        help["signatures"][0]["label"],
        "def pair(a: String, body: {x: Int}): String"
    );
    assert_eq!(help["activeParameter"], 1);
    client.shut_down().unwrap();
    // A client with it: an `InsertReplaceEdit`, from the middle of the
    // name.
    let capabilities = json!({
        "general": { "positionEncodings": ["utf-16"] },
        "textDocument": { "completion": { "completionItem": { "insertReplaceSupport": true } } },
    });
    let (mut client, _) = Client::start_with(&capabilities);
    client.open(&uri, text);
    let list = client.ask("textDocument/completion", &uri, (2, 11));
    let edit: lsp_types::InsertReplaceEdit =
        serde_json::from_value(item(&list, "libcap-tools")["textEdit"].clone()).unwrap();
    assert_eq!(edit.insert, range((2, 8), (2, 11)));
    assert_eq!(edit.replace, range((2, 8), (2, 16)));
    assert_eq!(edit.new_text, "libcap-tools");
    client.shut_down().unwrap();
}

#[test]
fn queries_are_answered_in_a_file_whose_braces_do_not_match() {
    let scratch = Scratch::workspace("unbalanced", "");
    let text = "base = 1\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    client.published(&uri);
    // Typed: `x = {` and a newline, and completion asked at once on the
    // next line.
    client.change(&uri, 2, ((1, 0), (1, 0)), "x = {\n");
    let before = client.checks();
    let list = client.ask("textDocument/completion", &uri, (2, 0));
    let labels = completion_labels(&list);
    assert!(labels.iter().any(|l| l == "base"), "{labels:?}");
    // The check of the change, and the workspace indexed again with the
    // block closed.
    assert_eq!(client.checks() - before, 2);
    // Typed in the block: a local, and queries of it.
    client.change(&uri, 3, ((2, 0), (2, 0)), "    k = base\n    j = k");
    let before = client.checks();
    let list = client.ask("textDocument/completion", &uri, (3, 9));
    assert!(completion_labels(&list).iter().any(|l| l == "k"));
    let shown = client.hover(&uri, (3, 8));
    let value = shown["contents"]["value"].as_str().unwrap();
    assert!(value.contains("k: Int"), "{shown}");
    assert_eq!(as_range(&shown["range"]), range((3, 8), (3, 9)));
    let definition = client.definition(&uri, (3, 8));
    assert_eq!(as_range(&definition["range"]), range((2, 4), (2, 5)));
    let help = client.ask("textDocument/signatureHelp", &uri, (3, 9));
    assert_eq!(help, Value::Null);
    let prepared = client.ask("textDocument/prepareRename", &uri, (3, 8));
    assert_eq!(prepared["placeholder"], "k");
    let highlights = client.ask("textDocument/documentHighlight", &uri, (3, 8));
    assert_eq!(highlights.as_array().map(Vec::len), Some(2), "{highlights}");
    // One check and one indexing again for all of them at one version.
    assert_eq!(client.checks() - before, 2);
    // A rename needs the text as it is: refused.
    let position = json!({ "line": 3, "character": 8 });
    let params = json!({ "textDocument": { "uri": uri }, "position": position, "newName": "z" });
    let error = client
        .request("textDocument/rename", params)
        .response_result
        .unwrap_err();
    assert_eq!(error.message, "the file has an unclosed `{`");
    // The diagnostics are the syntax error alone.
    let published = client.published_at(&uri, 3);
    assert_eq!(published.diagnostics.len(), 1, "{published:?}");
    assert!(published.diagnostics[0].message.starts_with("expected `}`"));
    client.shut_down().unwrap();
}

#[test]
fn inlay_hints_are_of_the_range_asked_for_and_of_the_text_as_it_is() {
    let scratch = Scratch::workspace("inlay", "");
    // `é` is two bytes and one UTF-16 unit.
    let text = "fn f(): Int { 1 }\nlet a = f()\nlet s = \"é\"; let b = f()\nx = f()\n";
    let uri = scratch.write("app.gcsl", text);
    let hints = |client: &mut Client, (start, end): ((u32, u32), (u32, u32))| {
        let params = json!({ "textDocument": { "uri": uri }, "range": range(start, end) });
        let found = client
            .request("textDocument/inlayHint", params)
            .response_result
            .unwrap();
        let found: Vec<lsp_types::InlayHint> = serde_json::from_value(found).unwrap();
        found
            .into_iter()
            .map(|hint| {
                assert_eq!(hint.kind, Some(lsp_types::InlayHintKind::TYPE));
                assert_eq!(hint.padding_left, Some(false));
                let lsp_types::InlayHintLabel::String(label) = hint.label else {
                    panic!("{:?}", hint.label)
                };
                (hint.position, label)
            })
            .collect::<Vec<_>>()
    };
    let (mut client, _) = Client::start(&["utf-16"]);
    client.open(&uri, text);
    let at = |line, character| Position::new(line, character);
    assert_eq!(
        hints(&mut client, ((0, 0), (4, 0))),
        [
            (at(1, 5), ": Int".to_owned()),
            (at(2, 18), ": Int".to_owned())
        ]
    );
    assert_eq!(
        hints(&mut client, ((2, 0), (3, 0))),
        [(at(2, 18), ": Int".to_owned())]
    );
    // Edited: answered from the text as it is now.
    client.change(&uri, 2, ((0, 8), (0, 11)), "String");
    client.change(&uri, 3, ((0, 17), (0, 18)), "\"s\"");
    assert_eq!(
        hints(&mut client, ((0, 0), (4, 0))),
        [
            (at(1, 5), ": String".to_owned()),
            (at(2, 18), ": String".to_owned())
        ]
    );
    client.shut_down().unwrap();
}

#[test]
fn whitespace_and_keywords_have_nothing() {
    let scratch = Scratch::workspace("nothing-there", "");
    let text = "fn f(a: Int): Int {  a }\nr = f(1)\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    // `fn`, the space after `{`, and `:`.
    for at in [(0, 0), (0, 20), (0, 12)] {
        assert_eq!(client.hover(&uri, at), Value::Null, "{at:?}");
        assert_eq!(client.definition(&uri, at), Value::Null, "{at:?}");
    }
    // A document that is not open has nothing either.
    let other = scratch.write("other.gcsl", "q = 1\n");
    assert_eq!(client.hover(&other, (0, 0)), Value::Null);
    client.shut_down().unwrap();
}

#[test]
fn a_lone_file_is_indexed() {
    let scratch = Scratch::new("lone-hover");
    let text = "fn twice(n: Int): Int { n * 2 }\nr = twice(1)\n";
    let uri = scratch.write("lone.gcsl", text);
    let (mut client, _) = Client::start(&["utf-16"]);
    client.open(&uri, text);
    let hover = client.hover(&uri, (1, 4));
    let shown = hover["contents"]["value"].as_str().unwrap();
    assert!(shown.contains("fn twice(n: Int): Int"), "{shown}");
    let definition = client.definition(&uri, (1, 4));
    assert_eq!(definition["uri"], uri.as_str());
    assert_eq!(as_range(&definition["range"]), range((0, 3), (0, 8)));
    // A definition of the standard library has no file: nothing.
    client.change(&uri, 2, ((1, 4), (1, 9)), "len([1]) + twice");
    assert_eq!(client.definition(&uri, (1, 4)), Value::Null);
    client.shut_down().unwrap();
}

/// The `(uri, range)` of each location of `value`, a `Location[]`.
fn locations(value: &Value) -> Vec<(String, Range)> {
    let locations: Vec<lsp_types::Location> = serde_json::from_value(value.clone()).unwrap();
    locations
        .into_iter()
        .map(|location| (location.uri.as_str().to_owned(), location.range))
        .collect()
}

#[test]
fn references_cover_files_not_open() {
    let scratch = Scratch::workspace("references", "");
    let text = "z = y + 1\nw = y\n";
    let a = scratch.write("a.gcsl", text);
    let b = scratch.write("b.gcsl", "y = 1\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&a, text);
    let params = |declarations: bool| {
        json!({
            "textDocument": { "uri": a },
            "position": { "line": 1, "character": 4 },
            "context": { "includeDeclaration": declarations },
        })
    };
    let response = client.request("textDocument/references", params(true));
    assert_eq!(
        locations(&response.response_result.unwrap()),
        [
            (a.as_str().to_owned(), range((0, 4), (0, 5))),
            (a.as_str().to_owned(), range((1, 4), (1, 5))),
            (b.as_str().to_owned(), range((0, 0), (0, 1))),
        ]
    );
    let response = client.request("textDocument/references", params(false));
    assert_eq!(locations(&response.response_result.unwrap()).len(), 2);
    client.shut_down().unwrap();
}

#[test]
fn highlights_tell_writes_from_reads() {
    let scratch = Scratch::workspace("highlights", "");
    let text = "fn g(): Int {\n    let mut n = 1\n    n := n + 1\n    n\n}\nr = g()\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-16"]);
    client.open(&uri, text);
    let found = client.ask("textDocument/documentHighlight", &uri, (3, 4));
    let highlights: Vec<lsp_types::DocumentHighlight> = serde_json::from_value(found).unwrap();
    let shown: Vec<(Range, Option<lsp_types::DocumentHighlightKind>)> =
        highlights.into_iter().map(|h| (h.range, h.kind)).collect();
    let (write, read) = (
        Some(lsp_types::DocumentHighlightKind::WRITE),
        Some(lsp_types::DocumentHighlightKind::READ),
    );
    assert_eq!(
        shown,
        [
            (range((1, 12), (1, 13)), write),
            (range((2, 4), (2, 5)), write),
            (range((2, 9), (2, 10)), read),
            (range((3, 4), (3, 5)), read),
        ]
    );
    client.shut_down().unwrap();
}

#[test]
fn a_rename_edits_files_not_open_and_is_refused_with_a_reason() {
    let scratch = Scratch::workspace("rename", "");
    let text = "z = y + len([1])\n";
    let a = scratch.write("a.gcsl", text);
    let b = scratch.write("b.gcsl", "y = 1\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&a, text);
    let prepared = client.ask("textDocument/prepareRename", &a, (0, 4));
    assert_eq!(as_range(&prepared["range"]), range((0, 4), (0, 5)));
    assert_eq!(prepared["placeholder"], "y");
    let position = json!({ "line": 0, "character": 4 });
    let params = json!({ "textDocument": { "uri": a }, "position": position, "newName": "base" });
    let edit = client
        .request("textDocument/rename", params)
        .response_result
        .unwrap();
    let edit_at = |uri: &Uri| {
        let edits: Vec<lsp_types::TextEdit> =
            serde_json::from_value(edit["changes"][uri.as_str()].clone()).unwrap();
        edits
            .into_iter()
            .map(|e| (e.range, e.new_text))
            .collect::<Vec<_>>()
    };
    assert_eq!(edit_at(&a), [(range((0, 4), (0, 5)), "base".to_owned())]);
    assert_eq!(edit_at(&b), [(range((0, 0), (0, 1)), "base".to_owned())]);
    // A standard function's name, and a new name that is none: refused,
    // with why.
    let position = json!({ "line": 0, "character": 9 });
    let params = json!({ "textDocument": { "uri": a }, "position": position });
    let error = client
        .request("textDocument/prepareRename", params)
        .response_result
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RequestFailed as i32);
    assert!(
        error.message.contains("standard library"),
        "{}",
        error.message
    );
    let position = json!({ "line": 0, "character": 4 });
    let params = json!({ "textDocument": { "uri": a }, "position": position, "newName": "1y" });
    let error = client
        .request("textDocument/rename", params)
        .response_result
        .unwrap_err();
    assert!(error.message.contains("not a name"), "{}", error.message);
    client.shut_down().unwrap();
}

#[test]
fn document_symbols_nest_and_workspace_symbols_span_workspaces() {
    let scratch = Scratch::workspace("symbols", "");
    let text = "cfg = {\n    a = 1\n}\nfn f(): Int { 1 }\n";
    let a = scratch.write("a.gcsl", text);
    let b = scratch.write("b.gcsl", "base_value = 1\n");
    let lone = Scratch::new("symbols-lone");
    let lonely = lone.write("lonely.gcsl", "lonely_value = 2\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&a, text);
    client.open(&lonely, "lonely_value = 2\n");
    let params = json!({ "textDocument": { "uri": a } });
    let symbols = client
        .request("textDocument/documentSymbol", params)
        .response_result
        .unwrap();
    assert_eq!(symbols[0]["name"], "cfg");
    assert_eq!(symbols[0]["kind"], json!(lsp_types::SymbolKind::FIELD));
    assert_eq!(symbols[0]["detail"], "{a: Int}");
    assert_eq!(
        as_range(&symbols[0]["selectionRange"]),
        range((0, 0), (0, 3))
    );
    assert_eq!(as_range(&symbols[0]["range"]), range((0, 0), (2, 1)));
    assert_eq!(symbols[0]["children"][0]["name"], "a");
    assert_eq!(symbols[1]["name"], "f");
    assert_eq!(symbols[1]["kind"], json!(lsp_types::SymbolKind::FUNCTION));
    let found = client
        .request("workspace/symbol", json!({ "query": "VALUE" }))
        .response_result
        .unwrap();
    let found: Vec<lsp_types::WorkspaceSymbol> = serde_json::from_value(found).unwrap();
    let shown: Vec<(String, String)> = found
        .into_iter()
        .map(|symbol| {
            let OneOf::Left(location) = symbol.location else {
                panic!()
            };
            (symbol.name, location.uri.as_str().to_owned())
        })
        .collect();
    assert!(
        shown.contains(&("base_value".to_owned(), b.as_str().to_owned())),
        "{shown:?}"
    );
    assert!(
        shown.contains(&("lonely_value".to_owned(), lonely.as_str().to_owned())),
        "{shown:?}"
    );
    client.shut_down().unwrap();
}

#[test]
fn a_clean_document_s_first_check_is_published_empty() {
    let scratch = Scratch::workspace("first-publish", "");
    let text = "x = 1\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    let published = client.published(&uri);
    assert_eq!(published.version, Some(1));
    assert_eq!(published.diagnostics, []);
    client.shut_down().unwrap();
}

#[test]
fn an_identical_publish_is_skipped_and_a_new_version_is_not() {
    let scratch = Scratch::workspace("dedup", "");
    let other = scratch.write("other.gcsl", "w = 1 + \"a\"\n");
    let text = "y = 1 + \"a\"\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    assert_eq!(client.published(&other).diagnostics.len(), 1);
    assert_eq!(client.published(&uri).diagnostics.len(), 1);
    // A comment added: the same error, at a new version of `app`. The
    // hover waits for the check its change asks for.
    client.change(&uri, 2, ((0, 11), (0, 11)), " # c");
    client.hover(&uri, (0, 0));
    let published = client.published(&uri);
    assert_eq!(published.version, Some(2));
    assert_eq!(published.diagnostics.len(), 1);
    // `other`'s, as they were and at no version, are not sent again.
    let again = client.pending.iter().any(|notification| {
        notification.method == PublishDiagnostics::METHOD
            && notification.params["uri"] == other.as_str()
    });
    assert!(!again, "{:?}", client.pending);
    client.shut_down().unwrap();
}

#[test]
fn without_watching_a_file_changed_on_disk_is_read_again() {
    let scratch = Scratch::workspace("mtime", "");
    let a = scratch.write("a.gcsl", "z = y + 1\n");
    scratch.write("b.gcsl", "y = 1\n");
    // No dynamic registration: the server stats the files itself.
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&a, "z = y + 1\n");
    assert_eq!(client.published(&a).diagnostics, []);
    scratch.write("b.gcsl", "y = \"s\"\n");
    // Nothing tells the server; its next check finds it.
    client.change(&a, 2, ((0, 9), (0, 9)), " # c");
    let published = client.published_at(&a, 2);
    assert_eq!(published.diagnostics.len(), 1);
    // A query answered from an index whose files changed on disk since
    // is answered after a check.
    scratch.write("b.gcsl", "y = 22\n");
    let hover = client.hover(&a, (0, 4));
    let shown = hover["contents"]["value"].as_str().unwrap();
    assert!(shown.contains("y: Int"), "{shown}");
    client.shut_down().unwrap();
}

#[test]
fn a_file_created_over_a_known_one_is_read_again() {
    let scratch = Scratch::workspace("created", "");
    let a = scratch.write("a.gcsl", "z = y + 1\n");
    let b = scratch.write("b.gcsl", "y = 1\n");
    let capabilities = json!({
        "general": { "positionEncodings": ["utf-8"] },
        "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } },
    });
    let (mut client, _) = Client::start_with(&capabilities);
    let Message::Request(request) = client.receive() else {
        panic!()
    };
    let response = Response::new_ok(request.id, Value::Null);
    client.connection.sender.send(response.into()).unwrap();
    client.open(&a, "z = y + 1\n");
    assert_eq!(client.published(&a).diagnostics, []);
    // A file renamed over `b`, which inotify reports as created.
    scratch.write("b.gcsl", "y = \"s\"\n");
    let changes = json!({ "changes": [{ "uri": b, "type": 1 }] });
    client.notify("workspace/didChangeWatchedFiles", changes);
    assert_eq!(client.published(&a).diagnostics.len(), 1);
    client.shut_down().unwrap();
}

/// The time of a stat pass of the workspace at `$GCSL_LSP_BENCH`: run
/// with `--ignored --nocapture` to see it.
#[test]
#[ignore = "a measurement, not a test"]
fn time_of_a_stat_pass() {
    let Some(root) = std::env::var_os("GCSL_LSP_BENCH") else {
        return;
    };
    let root = fs::canonicalize(root).unwrap();
    let sources = gcsl_disk::read_workspace(&root, None).unwrap();
    let files = sources.relative.len();
    let mut times = Vec::new();
    for _ in 0..50 {
        let start = Instant::now();
        let found = worker::stamps(&root, &sources.relative, None);
        times.push(start.elapsed());
        assert!(found.iter().all(Option::is_some));
    }
    // The walk that finds the files, which a stat pass does not make.
    let start = Instant::now();
    gcsl_disk::workspace_files(&root).unwrap();
    let walk = start.elapsed();
    times.sort();
    let _ = writeln!(
        io::stderr(),
        "stat pass of {files} files: median {:?}, min {:?}, max {:?}; a walk of the tree {walk:?}",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    );
}

#[test]
fn a_cancelled_request_does_no_harm() {
    let scratch = Scratch::workspace("cancel", "");
    let text = "x = 1\n";
    let uri = scratch.write("app.gcsl", text);
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(&uri, text);
    // A request already answered, and one that never was.
    let hover = client.hover(&uri, (0, 0));
    assert!(hover.is_object(), "{hover}");
    client.notify("$/cancelRequest", json!({ "id": 1 }));
    client.notify("$/cancelRequest", json!({ "id": "nothing" }));
    assert!(client.hover(&uri, (0, 0)).is_object());
    client.shut_down().unwrap();
}

/// The edits of a formatting request, each as its range and new text.
fn formatting_edits(response: &Response) -> Vec<(Range, String)> {
    let edits: Vec<lsp_types::TextEdit> =
        serde_json::from_value(response.response_result.clone().unwrap()).unwrap();
    edits
        .into_iter()
        .map(|edit| (edit.range, edit.new_text))
        .collect()
}

#[test]
fn a_messy_document_is_formatted_by_lines_and_a_broken_one_refused() {
    let scratch = Scratch::workspace("format", "");
    // On disk, the file is formatted: the document's own text is.
    let uri = scratch.write("app.gcsl", "x = 1\n");
    let text = "x = 1\nmeta = {\n  a=1;\n\n\n  b = [\n    1\n  ]\n}\ny = 2\n";
    let (mut client, _) = Client::start(&["utf-16"]);
    client.open(&uri, text);
    let options = json!({ "tabSize": 2, "insertSpaces": true });
    let whole = json!({ "textDocument": { "uri": uri }, "options": options });
    let response = client.request("textDocument/formatting", whole.clone());
    assert_eq!(
        formatting_edits(&response),
        [
            (range((2, 0), (3, 0)), "    a = 1\n".to_owned()),
            (
                range((4, 0), (8, 0)),
                "    b = [\n        1,\n    ]\n".to_owned()
            ),
        ]
    );
    // A range: the edits that touch its lines. A selection of whole
    // lines ends at the start of the line after them.
    let ranged = json!({
        "textDocument": { "uri": uri },
        "range": range((5, 0), (7, 0)),
        "options": options,
    });
    let response = client.request("textDocument/rangeFormatting", ranged);
    assert_eq!(formatting_edits(&response).len(), 1);
    let ranged = json!({
        "textDocument": { "uri": uri },
        "range": range((0, 0), (1, 3)),
        "options": options,
    });
    let response = client.request("textDocument/rangeFormatting", ranged.clone());
    assert_eq!(formatting_edits(&response), []);
    // With a syntax error: refused, saying where the first is, whole or
    // by a range.
    client.change(&uri, 2, ((9, 4), (9, 5)), "(");
    for (method, params) in [
        ("textDocument/formatting", whole),
        ("textDocument/rangeFormatting", ranged),
    ] {
        let error = client.request(method, params).response_result.unwrap_err();
        assert_eq!(error.code, ErrorCode::RequestFailed as i32, "{method}");
        assert_eq!(
            error.message, "not formatted: it has a syntax error (10:6)",
            "{method}"
        );
    }
    // A document not open: nothing to say.
    let other = scratch.write("other.gcsl", "y  =  2\n");
    let params = json!({ "textDocument": { "uri": other }, "options": options });
    let response = client.request("textDocument/formatting", params);
    assert_eq!(response.response_result.unwrap(), Value::Null);
    client.shut_down().unwrap();
}

/// The time of a hover and a definition, on a small workspace, and on
/// the one at `$GCSL_LSP_BENCH` if it is set, its `workspace.gcsl`
/// opened, in memory only: warm, from the index of the text as it is,
/// and with a change just before, which the query checks first. Run
/// with `--ignored --nocapture` to see it.
#[test]
#[ignore = "a measurement, not a test"]
fn time_of_a_hover_and_a_definition() {
    let scratch = Scratch::workspace("query-latency", "");
    scratch.write("lib.gcsl", "base = 1\n");
    let text = "x = base + 1\n";
    let uri = scratch.write("app.gcsl", text);
    measure_queries("a small workspace", &uri, text, (0, 4));
    if let Some(root) = std::env::var_os("GCSL_LSP_BENCH") {
        let path = fs::canonicalize(root).unwrap().join(WORKSPACE);
        let text = fs::read_to_string(&path).unwrap();
        let uri = uri_of(&path).unwrap();
        // The first name read in a type position, or the first line.
        let at = text
            .lines()
            .enumerate()
            .find_map(|(line, content)| {
                let column = content.find("Version")?;
                Some((u32::try_from(line).ok()?, u32::try_from(column).ok()?))
            })
            .unwrap_or((0, 0));
        measure_queries("$GCSL_LSP_BENCH", &uri, &text, at);
    }
}

/// Opens the document `uri` as `text` and asks for a hover and a
/// definition at `at` twenty times each, warm, then twenty times each
/// just after a change (a line added at the end and taken away again),
/// and reports the times.
fn measure_queries(name: &str, uri: &Uri, text: &str, at: (u32, u32)) {
    let mut text = text.to_owned();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let end = u32::try_from(text.matches('\n').count()).unwrap();
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(uri, &text);
    // The first query waits for the first check.
    client.ask("textDocument/hover", uri, at);
    let mut version = 1;
    for method in ["textDocument/hover", "textDocument/definition"] {
        let mut warm = Vec::new();
        let mut changed = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            client.ask(method, uri, at);
            warm.push(start.elapsed());
            version += 1;
            let edit = if version % 2 == 0 {
                ((end, 0), (end, 0))
            } else {
                ((end, 0), (end + 1, 0))
            };
            let added = if version % 2 == 0 { "probe = 1\n" } else { "" };
            let start = Instant::now();
            client.change(uri, version, edit, added);
            client.ask(method, uri, at);
            changed.push(start.elapsed());
        }
        warm.sort();
        changed.sort();
        let _ = writeln!(
            io::stderr(),
            "{method}, {name}: warm median {:?} (max {:?}), after a change median {:?} (max {:?})",
            warm[warm.len() / 2],
            warm[warm.len() - 1],
            changed[changed.len() / 2],
            changed[changed.len() - 1],
        );
    }
    client.shut_down().unwrap();
}

/// The time of a completion and of signature help, on a small workspace,
/// and on the one at `$GCSL_LSP_BENCH` if it is set, its
/// `workspace.gcsl` opened with two lines added, in memory only: warm,
/// and just after a keystroke, which the query checks first. Run with
/// `--ignored --nocapture` to see it.
#[test]
#[ignore = "a measurement, not a test"]
fn time_of_a_completion_and_signature_help() {
    let scratch = Scratch::workspace("typing-latency", "");
    let uri = scratch.write("app.gcsl", "x = 1\n");
    measure_typing("a small workspace", &uri, "x = 1\n");
    if let Some(root) = std::env::var_os("GCSL_LSP_BENCH") {
        let path = fs::canonicalize(root).unwrap().join(WORKSPACE);
        let text = fs::read_to_string(&path).unwrap();
        let uri = uri_of(&path).unwrap();
        measure_typing("$GCSL_LSP_BENCH", &uri, &text);
    }
}

/// The time of a completion in a block left open, its file's braces not
/// matching, on a small workspace, and on the one at `$GCSL_LSP_BENCH`
/// if it is set, its `workspace.gcsl` opened with the block at its end,
/// in memory only: just after a keystroke, which the query checks and
/// indexes again for first, and again at the same version. Run with
/// `--ignored --nocapture` to see it.
#[test]
#[ignore = "a measurement, not a test"]
fn time_of_a_completion_in_a_block_left_open() {
    let scratch = Scratch::workspace("unbalanced-latency", "");
    let uri = scratch.write("app.gcsl", "x = 1\n");
    measure_unbalanced("a small workspace", &uri, "x = 1\n");
    if let Some(root) = std::env::var_os("GCSL_LSP_BENCH") {
        let path = fs::canonicalize(root).unwrap().join(WORKSPACE);
        let text = fs::read_to_string(&path).unwrap();
        let uri = uri_of(&path).unwrap();
        measure_unbalanced("$GCSL_LSP_BENCH", &uri, &text);
    }
}

/// Opens the document `uri` as `text` with a block left open after it,
/// then twenty times types a key in the block and asks for a completion
/// at once, asks again at the same version, and takes the key away; and
/// reports the times.
fn measure_unbalanced(name: &str, uri: &Uri, text: &str) {
    let mut text = text.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let line = u32::try_from(text.matches('\n').count()).unwrap() + 2;
    text.push_str("lsp_open = {\n    lsp_k = 1\n    lsp_j = ");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(uri, &text);
    client.ask("textDocument/completion", uri, (line, 12));
    let (mut typed, mut again) = (Vec::new(), Vec::new());
    let mut version = 1;
    for _ in 0..20 {
        version += 1;
        let start = Instant::now();
        client.change(uri, version, ((line, 12), (line, 12)), "l");
        let answer = client.ask("textDocument/completion", uri, (line, 13));
        typed.push(start.elapsed());
        assert!(answer.to_string().contains("lsp_k"), "{answer}");
        let start = Instant::now();
        client.ask("textDocument/completion", uri, (line, 13));
        again.push(start.elapsed());
        version += 1;
        client.change(uri, version, ((line, 12), (line, 13)), "");
    }
    typed.sort();
    again.sort();
    let _ = writeln!(
        io::stderr(),
        "completion in a block left open, {name}: after a keystroke median {:?} (max {:?}), again at the version median {:?} (max {:?})",
        typed[typed.len() / 2],
        typed[typed.len() - 1],
        again[again.len() / 2],
        again[again.len() - 1],
    );
    client.shut_down().unwrap();
}

/// Opens the document `uri` as `text` with a record and a probe line
/// after it, then twenty times types a key and asks at once (a `.`
/// after the probe's name and a completion; a `(` after `len` and
/// signature help), asks again warm, and takes the key away; and
/// reports the times.
fn measure_typing(name: &str, uri: &Uri, text: &str) {
    let mut text = text.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let first = u32::try_from(text.matches('\n').count()).unwrap() + 1;
    text.push_str("lsp_base = {a = 1}\nlsp_probe = lsp_base\nlsp_call = len\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(uri, &text);
    client.ask("textDocument/completion", uri, (first, 0));
    let mut version = 1;
    for (method, (line, end), key, wanted) in [
        ("textDocument/completion", (first, 20), ".", "\"a\""),
        (
            "textDocument/signatureHelp",
            (first + 1, 14),
            "(",
            "list: List<T>",
        ),
    ] {
        let (mut warm, mut typed) = (Vec::new(), Vec::new());
        for _ in 0..20 {
            version += 1;
            let start = Instant::now();
            client.change(uri, version, ((line, end), (line, end)), key);
            let answer = client.ask(method, uri, (line, end + 1));
            typed.push(start.elapsed());
            assert!(answer.to_string().contains(wanted), "{answer}");
            let start = Instant::now();
            client.ask(method, uri, (line, end + 1));
            warm.push(start.elapsed());
            version += 1;
            client.change(uri, version, ((line, end), (line, end + 1)), "");
        }
        warm.sort();
        typed.sort();
        let _ = writeln!(
            io::stderr(),
            "{method}, {name}: warm median {:?} (max {:?}), after a keystroke median {:?} (max {:?})",
            warm[warm.len() / 2],
            warm[warm.len() - 1],
            typed[typed.len() / 2],
            typed[typed.len() - 1],
        );
    }
    client.shut_down().unwrap();
}

/// The time from a change to its diagnostics, on a small workspace, and
/// on the one at `$GCSL_LSP_BENCH` if it is set, its `workspace.gcsl`
/// opened with a line added, in memory only: run with `--ignored
/// --nocapture` to see it.
#[test]
#[ignore = "a measurement, not a test"]
fn time_from_a_change_to_its_diagnostics() {
    let scratch = Scratch::workspace("latency", "");
    scratch.write("lib.gcsl", "base = 1\n");
    let uri = scratch.write("app.gcsl", "x = base + 1\n");
    measure_latency("a small workspace", &uri, "");
    if let Some(root) = std::env::var_os("GCSL_LSP_BENCH") {
        let path = fs::canonicalize(root).unwrap().join(WORKSPACE);
        let text = fs::read_to_string(&path).unwrap();
        let uri = uri_of(&path).unwrap();
        measure_latency("$GCSL_LSP_BENCH", &uri, &text);
    }
}

/// Opens the document `uri` as `text` and a line of a probe after it,
/// alternately breaks and fixes the probe twenty times, and reports the
/// time from each change to its diagnostics.
fn measure_latency(name: &str, uri: &Uri, text: &str) {
    let mut text = text.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let line = u32::try_from(text.matches('\n').count()).unwrap();
    text.push_str("lsp_probe = 1 + \"a\"\n");
    let (mut client, _) = Client::start(&["utf-8"]);
    client.open(uri, &text);
    client.published(uri);
    let mut times = Vec::new();
    for version in 2..=21 {
        // Alternately fixing and breaking the probe, so that each
        // version publishes.
        let replacement = if version % 2 == 0 { "1  " } else { "\"a\"" };
        let start = Instant::now();
        client.change(uri, version, ((line, 16), (line, 19)), replacement);
        client.published_at(uri, version);
        times.push(start.elapsed());
    }
    times.sort();
    let median = times[times.len() / 2];
    let _ = writeln!(
        io::stderr(),
        "change to diagnostics, {name}: median {median:?}, min {:?}, max {:?} \
         (quiet period {QUIET:?})",
        times[0],
        times[times.len() - 1]
    );
    client.shut_down().unwrap();
}

/// The time of a full check of the workspace at `$GCSL_LSP_BENCH`, its
/// files read once from disk: run with `--ignored --nocapture` to see
/// it.
#[test]
#[ignore = "a measurement, not a test"]
fn time_of_a_full_check() {
    let Some(root) = std::env::var_os("GCSL_LSP_BENCH") else {
        return;
    };
    let root = fs::canonicalize(root).unwrap();
    let sources = gcsl_disk::read_workspace(&root, None).unwrap();
    let name = root.to_string_lossy().into_owned();
    // On a thread with the stack the library asks for, as the worker's.
    let measured = thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(move || {
            let mut times = Vec::new();
            let mut clean = true;
            for _ in 0..10 {
                let start = Instant::now();
                let (_, checked) =
                    gcsl::workspace::check_with_standard(&sources.relative, &sources.nested, &name);
                times.push(start.elapsed());
                clean &= checked.is_ok();
            }
            (sources.relative.len(), clean, times)
        })
        .unwrap()
        .join()
        .unwrap();
    let (files, clean, mut times) = measured;
    times.sort();
    let _ = writeln!(
        io::stderr(),
        "full check of {files} files (clean: {clean}): median {:?}, min {:?}, max {:?}",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    );
}
