//! `gcsl lsp`: a language server for GCSL, which reports each file's errors
//! and warnings as it is edited. Diagnostics only, for now.
//!
//! It speaks LSP through `lsp-server`, synchronously, on stdin and stdout
//! ([`stdio`]), or on any [`Connection`] ([`serve`]); it logs to stderr
//! only, so that nothing but LSP's messages reaches stdout. All language
//! work is done by the `gcsl` library, and finding and reading a
//! workspace's files by the `gcsl-disk` crate, as for `gcsl run` and
//! `pekit`.
//!
//! INITIALISATION. The server counts positions in utf-8 if the client
//! offers it (`general.positionEncodings`), else in utf-16, LSP's default
//! (see [`position`]). It offers only `textDocumentSync`: open and close,
//! incremental changes, and saves. It answers `shutdown`, and any request
//! after it with an error; `exit` ends it, cleanly after a `shutdown`, else
//! not ([`Error::ExitWithoutShutdown`], exit code 1). A
//! `workspace/configuration` request is answered with a null for each
//! item; any other request with `MethodNotFound`; notifications it does not
//! know are ignored. If the client can register for watched files
//! dynamically, the server asks it to watch `**/*.gcsl`.
//!
//! SOURCES. It keeps the text of each open document, applying incremental
//! changes in the negotiated encoding. A `.gcsl` file is a file of the
//! workspace whose root [`gcsl_disk::find_workspace`] finds above it, a
//! file not on disk yet of the one its directory is in, and any other
//! document is checked alone, as `gcsl run FILE` checks a lone file (see
//! [`place`]). A server serves any number of workspaces, each by its root.
//! A workspace's files are read from disk once and kept, and read again
//! when the client reports watched files changed under its root, when one
//! of its documents is saved, and when one is closed (its file then has its
//! text on disk again); the open documents' texts stand in for their
//! files', and those not on disk yet are added in path order. Nothing is
//! ever written to disk.
//!
//! CHECKING. A workspace is checked as `gcsl run` and `pekit` check one
//! ([`gcsl::workspace::check_with_standard`], with its canonical root and
//! the directories left out as workspaces of their own), and a lone file as
//! `gcsl run` checks one; nothing is ever run, so opening a file never runs
//! `sh`, `exec` or any GCSL code. Every file the check reports on, open or
//! not, has its diagnostics published, with an open document's version as
//! checked, and a file that had some and has none now gets an empty list
//! (see the `worker` module).
//!
//! THREADS. Not to lag, the main thread only reads messages, keeps the
//! documents and schedules checks; it never checks. One worker thread (the
//! `worker` module) reads workspaces from disk, checks them and publishes,
//! one check at a time: the main thread hands it the next only once it has
//! answered the last, so checks never overlap and stale work never queues
//! up behind new edits, a check taking the documents as they are when it is
//! handed over.
//!
//! DEBOUNCING. A change to a workspace asks for a check of it [`QUIET`]
//! (150 ms) after the change, and a later change moves that on: a burst of
//! edits is one check. Opening a document of a workspace not checked yet
//! asks for one at once. When a check is done, its diagnostics are
//! published even if the workspace changed meanwhile, with each document's
//! version as it was checked, and the change's own check follows its quiet
//! period.
//!
//! KNOWN LIMIT. While the scan of a workspace fails, a file not parsing or
//! an import not resolved, its check reports only those errors, so the
//! other files' type errors disappear until it parses again: what the check
//! reports is published, and stale diagnostics are not kept.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro
)]

pub mod place;
pub mod position;
mod worker;

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use gcsl_disk::WORKSPACE;
use lsp_server::{
    Connection, ErrorCode, Message, Notification, ProtocolError, Request, RequestId, Response,
};
use lsp_types::notification::{
    DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument,
    DidSaveTextDocument, Notification as _,
};
use lsp_types::request::{RegisterCapability, Request as _};
use lsp_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, FileSystemWatcher, GlobPattern,
    InitializeResult, Registration, RegistrationParams, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Uri,
};
use serde_json::Value;

use crate::place::{Place, locate, path_of};
use crate::position::Encoding;
use crate::worker::{Check, Job, Key, Snapshot};

/// How long a workspace must go unchanged before it is checked.
pub const QUIET: Duration = Duration::from_millis(150);

/// The id of the server's request to watch `.gcsl` files.
const WATCH_ID: &str = "gcsl-watch-files";

/// Why the server stopped other than cleanly.
#[derive(Debug)]
pub enum Error {
    /// The client broke the protocol, before or during initialisation.
    Protocol(ProtocolError),
    /// The client sent `exit` without `shutdown` first.
    ExitWithoutShutdown,
    /// The client went away without `exit`.
    Disconnected,
    /// The worker thread could not be started.
    Worker(io::Error),
    /// The worker thread stopped, which it does only by panicking.
    WorkerStopped,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Protocol(error) => write!(f, "{error}"),
            Error::ExitWithoutShutdown => f.write_str("the client sent `exit` before `shutdown`"),
            Error::Disconnected => f.write_str("the client went away without `exit`"),
            Error::Worker(error) => write!(f, "cannot start the worker thread: {error}"),
            Error::WorkerStopped => f.write_str("internal error: the worker thread stopped"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ProtocolError> for Error {
    fn from(error: ProtocolError) -> Self {
        Error::Protocol(error)
    }
}

/// Writes `message` to stderr, as a line of the server's log.
///
/// A failure to write to stderr is ignored: there is nowhere left to report
/// it.
pub(crate) fn log(message: impl fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "gcsl lsp: {message}");
}

/// Serves LSP on stdin and stdout until the client says `exit`, and returns
/// the exit code: 0 after `shutdown`, else 1, with why logged to stderr.
#[must_use]
pub fn stdio() -> ExitCode {
    let (connection, io_threads) = Connection::stdio();
    let served = serve(&connection);
    // The writer thread stops once every sender of the connection is gone.
    drop(connection);
    match served {
        Ok(()) => match io_threads.join() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log(format_args!("error: {error}"));
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            log(format_args!("error: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// Serves LSP on `connection` until the client says `exit`: `Ok` if it
/// said `shutdown` first.
///
/// # Errors
///
/// The client broke the protocol, sent `exit` without `shutdown` or went
/// away, or the worker thread failed.
pub fn serve(connection: &Connection) -> Result<(), Error> {
    serve_counting(connection, &Arc::default())
}

/// [`serve`], counting each check made in `checks`.
fn serve_counting(connection: &Connection, checks: &Arc<AtomicUsize>) -> Result<(), Error> {
    let (id, params) = connection.initialize_start()?;
    let offered = params
        .pointer("/capabilities/general/positionEncodings")
        .and_then(Value::as_array)
        .map(|offered| offered.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let encoding = Encoding::negotiate(offered);
    let watch = params
        .pointer("/capabilities/workspace/didChangeWatchedFiles/dynamicRegistration")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    connection.initialize_finish(id, initialize_result(encoding))?;
    let (done, finished) = crossbeam_channel::unbounded();
    let (jobs, worker) = worker::spawn(
        connection.sender.clone(),
        encoding,
        Arc::clone(checks),
        done,
    )
    .map_err(Error::Worker)?;
    let mut state = Server {
        connection,
        encoding,
        jobs,
        documents: HashMap::new(),
        due: HashMap::new(),
        busy: None,
        shutdown: false,
    };
    let ended = if watch { state.watch() } else { Ok(()) }.and_then(|()| state.run(&finished));
    // The worker stops once its jobs' sender is gone.
    drop(state);
    if worker.join().is_err() {
        log("internal error: the worker thread panicked");
    }
    ended
}

/// What the server answers `initialize` with: its capabilities, positions
/// counted in `encoding`.
fn initialize_result(encoding: Encoding) -> Value {
    let sync = TextDocumentSyncOptions {
        open_close: Some(true),
        change: Some(TextDocumentSyncKind::INCREMENTAL),
        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
        ..TextDocumentSyncOptions::default()
    };
    let result = InitializeResult {
        capabilities: ServerCapabilities {
            position_encoding: Some(encoding.kind()),
            text_document_sync: Some(TextDocumentSyncCapability::Options(sync)),
            ..ServerCapabilities::default()
        },
        server_info: Some(ServerInfo {
            name: "gcsl".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    };
    serde_json::to_value(result).unwrap_or(Value::Null)
}

/// An open document.
#[derive(Debug)]
struct Document {
    /// Its URI, as the client gave it.
    uri: Uri,
    /// Its text, as the client has it.
    text: String,
    /// Its version, as the client numbers it.
    version: i32,
    /// Where it is.
    place: Place,
}

/// A check asked for.
#[derive(Debug, Default)]
struct Due {
    /// When to check, if a check is asked for.
    at: Option<Instant>,
    /// Whether the files are read from disk again first.
    reread: bool,
}

/// What the main loop does after a message.
#[derive(PartialEq, Eq)]
enum Flow {
    /// Goes on.
    Continue,
    /// Stops: the client said `exit`.
    Exit,
}

/// The main thread's state.
struct Server<'c> {
    /// The connection to the client.
    connection: &'c Connection,
    /// The encoding positions are counted in.
    encoding: Encoding,
    /// Where the worker's jobs go.
    jobs: Sender<Job>,
    /// The open documents, by the text of their URIs. (A [`Uri`] keeps a
    /// cache inside it, so maps are keyed by its text.)
    documents: HashMap<String, Document>,
    /// The checks asked for, of every workspace known and every lone file
    /// open.
    due: HashMap<Key, Due>,
    /// What the worker is checking, if anything.
    busy: Option<Key>,
    /// Whether the client has said `shutdown`.
    shutdown: bool,
}

impl Server<'_> {
    /// Reads messages and schedules checks until the client says `exit`,
    /// handing the worker the next due check whenever it is idle; the
    /// worker says on `finished` when it has done one.
    fn run(&mut self, finished: &Receiver<Key>) -> Result<(), Error> {
        loop {
            self.dispatch()?;
            let wake = match self.next_wake() {
                Some(at) => crossbeam_channel::at(at),
                None => crossbeam_channel::never(),
            };
            crossbeam_channel::select! {
                recv(self.connection.receiver) -> message => {
                    let message = message.map_err(|_| Error::Disconnected)?;
                    if self.handle(message)? == Flow::Exit {
                        return if self.shutdown {
                            Ok(())
                        } else {
                            Err(Error::ExitWithoutShutdown)
                        };
                    }
                }
                recv(finished) -> key => {
                    key.map_err(|_| Error::WorkerStopped)?;
                    self.busy = None;
                }
                recv(wake) -> _ => {}
            }
        }
    }

    /// When the next check is due, if the worker is idle to make it.
    fn next_wake(&self) -> Option<Instant> {
        if self.busy.is_some() {
            return None;
        }
        self.due.values().filter_map(|due| due.at).min()
    }

    /// Hands the worker the check due first, if one is due and it is idle:
    /// the documents of what it checks as they are now.
    fn dispatch(&mut self) -> Result<(), Error> {
        if self.busy.is_some() {
            return Ok(());
        }
        let now = Instant::now();
        let next = self
            .due
            .iter()
            .filter_map(|(key, due)| due.at.filter(|&at| at <= now).map(|at| (at, key)))
            .min_by_key(|&(at, _)| at)
            .map(|(_, key)| key.clone());
        let Some(key) = next else {
            return Ok(());
        };
        let reread = self.due.get_mut(&key).is_some_and(|due| {
            due.at = None;
            std::mem::take(&mut due.reread)
        });
        let documents = self
            .documents
            .values()
            .filter(|document| Key::of(&document.uri, &document.place) == key)
            .map(|document| Snapshot {
                uri: document.uri.clone(),
                version: document.version,
                text: document.text.clone(),
                place: document.place.clone(),
            })
            .collect();
        let check = Check {
            key: key.clone(),
            documents,
            reread,
        };
        self.jobs
            .send(Job::Check(check))
            .map_err(|_| Error::WorkerStopped)?;
        self.busy = Some(key);
        Ok(())
    }

    /// Asks for a check of `key` after the quiet period, reading its files
    /// from disk again first if `reread`.
    fn change(&mut self, key: Key, reread: bool) {
        let due = self.due.entry(key).or_default();
        due.at = Some(Instant::now() + QUIET);
        due.reread |= reread;
    }

    /// Forgets `key`, and has the worker clear what it published for it.
    fn forget(&mut self, key: Key) -> Result<(), Error> {
        self.due.remove(&key);
        self.jobs
            .send(Job::Clear(key))
            .map_err(|_| Error::WorkerStopped)
    }

    /// Asks the client to report changes to `.gcsl` files.
    fn watch(&self) -> Result<(), Error> {
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.gcsl".to_owned()),
                kind: None,
            }],
        };
        let params = RegistrationParams {
            registrations: vec![Registration {
                id: WATCH_ID.to_owned(),
                method: DidChangeWatchedFiles::METHOD.to_owned(),
                register_options: serde_json::to_value(options).ok(),
            }],
        };
        let request = Request::new(
            RequestId::from(WATCH_ID.to_owned()),
            RegisterCapability::METHOD.to_owned(),
            params,
        );
        self.send(Message::Request(request))
    }

    /// Sends `message` to the client.
    fn send(&self, message: Message) -> Result<(), Error> {
        self.connection
            .sender
            .send(message)
            .map_err(|_| Error::Disconnected)
    }

    /// Handles one message from the client.
    fn handle(&mut self, message: Message) -> Result<Flow, Error> {
        match message {
            Message::Request(request) => {
                let response = self.answer(request);
                self.send(Message::Response(response))?;
                Ok(Flow::Continue)
            }
            Message::Notification(notification) => self.notified(notification),
            // The answer to registering for watched files.
            Message::Response(_) => Ok(Flow::Continue),
        }
    }

    /// The response to `request`.
    fn answer(&mut self, request: Request) -> Response {
        if self.shutdown {
            return Response::new_err(
                request.id,
                ErrorCode::InvalidRequest as i32,
                "the server is shutting down".to_owned(),
            );
        }
        match request.method.as_str() {
            "shutdown" => {
                self.shutdown = true;
                Response::new_ok(request.id, Value::Null)
            }
            "workspace/configuration" => {
                let items = request
                    .params
                    .get("items")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Response::new_ok(request.id, vec![Value::Null; items])
            }
            method => Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("unknown method `{method}`"),
            ),
        }
    }

    /// Handles `notification`: `exit`, or a change to the documents or the
    /// files. Parameters that cannot be read are logged and ignored, as is
    /// a notification not known.
    fn notified(&mut self, notification: Notification) -> Result<Flow, Error> {
        let Notification { method, params } = notification;
        match method.as_str() {
            "exit" => return Ok(Flow::Exit),
            DidOpenTextDocument::METHOD => {
                let read = serde_json::from_value::<DidOpenTextDocumentParams>(params);
                if let Some(params) = parsed(&method, read) {
                    self.opened(params);
                }
            }
            DidChangeTextDocument::METHOD => {
                let read = serde_json::from_value::<DidChangeTextDocumentParams>(params);
                if let Some(params) = parsed(&method, read) {
                    self.changed(params);
                }
            }
            DidSaveTextDocument::METHOD => {
                let read = serde_json::from_value::<DidSaveTextDocumentParams>(params);
                if let Some(params) = parsed(&method, read) {
                    self.saved(&params.text_document.uri);
                }
            }
            DidCloseTextDocument::METHOD => {
                let read = serde_json::from_value::<DidCloseTextDocumentParams>(params);
                if let Some(params) = parsed(&method, read) {
                    self.closed(&params.text_document.uri)?;
                }
            }
            DidChangeWatchedFiles::METHOD => {
                let read = serde_json::from_value::<DidChangeWatchedFilesParams>(params);
                if let Some(params) = parsed(&method, read) {
                    self.files_changed(&params)?;
                }
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// A document was opened: it is kept, and its workspace checked, at
    /// once if it has not been yet, else after the quiet period.
    fn opened(&mut self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        let place = locate(&item.uri);
        let key = Key::of(&item.uri, &place);
        let document = Document {
            uri: item.uri,
            text: item.text,
            version: item.version,
            place,
        };
        self.documents
            .insert(document.uri.as_str().to_owned(), document);
        self.arrived(key);
    }

    /// A document of `key` was opened there, or moved there: `key` is
    /// checked at once if it has not been yet, else after the quiet period.
    fn arrived(&mut self, key: Key) {
        let quiet = self.due.contains_key(&key);
        let due = self.due.entry(key).or_default();
        due.at = Some(Instant::now() + if quiet { QUIET } else { Duration::ZERO });
    }

    /// A document changed: its changes are applied, and its workspace
    /// checked after the quiet period.
    fn changed(&mut self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let Some(document) = self.documents.get_mut(uri.as_str()) else {
            return;
        };
        document.version = params.text_document.version;
        for change in params.content_changes {
            position::apply(&mut document.text, change, self.encoding);
        }
        let key = Key::of(&uri, &document.place);
        self.change(key, false);
    }

    /// A document was saved: its workspace's files are read again, and it
    /// is checked after the quiet period.
    fn saved(&mut self, uri: &Uri) {
        let Some(document) = self.documents.get(uri.as_str()) else {
            return;
        };
        if let key @ Key::Root(_) = Key::of(uri, &document.place) {
            self.change(key, true);
        }
    }

    /// A document was closed: its file has its text on disk again, so its
    /// workspace's files are read again and it is checked after the quiet
    /// period; a lone file is forgotten, its diagnostics cleared.
    fn closed(&mut self, uri: &Uri) -> Result<(), Error> {
        let Some(document) = self.documents.remove(uri.as_str()) else {
            return Ok(());
        };
        match Key::of(uri, &document.place) {
            key @ Key::Root(_) => {
                self.change(key, true);
                Ok(())
            }
            key @ Key::Lone(_) => self.forget(key),
        }
    }

    /// Files changed on disk: each workspace a change is under has its
    /// files read again and is checked after the quiet period. When a
    /// `workspace.gcsl` is among them, a workspace may have begun or ended:
    /// one whose root has none any more is forgotten, and each open
    /// document is placed again, its old and new workspace checked.
    fn files_changed(&mut self, params: &DidChangeWatchedFilesParams) -> Result<(), Error> {
        let paths: Vec<PathBuf> = params
            .changes
            .iter()
            .filter_map(|change| path_of(&change.uri))
            .collect();
        let roots: Vec<Key> = self
            .due
            .keys()
            .filter(|key| matches!(key, Key::Root(root) if paths.iter().any(|path| path.starts_with(root))))
            .cloned()
            .collect();
        for key in roots {
            self.change(key, true);
        }
        let workspaces = paths
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == WORKSPACE));
        if !workspaces {
            return Ok(());
        }
        let gone: Vec<Key> = self
            .due
            .keys()
            .filter(|key| matches!(key, Key::Root(root) if !root.join(WORKSPACE).is_file()))
            .cloned()
            .collect();
        for key in gone {
            self.forget(key)?;
        }
        let moved: Vec<(String, Place)> = self
            .documents
            .iter()
            .filter_map(|(text, document)| {
                let place = locate(&document.uri);
                (place != document.place).then(|| (text.clone(), place))
            })
            .collect();
        for (text, place) in moved {
            let Some(document) = self.documents.get_mut(&text) else {
                continue;
            };
            let old = Key::of(&document.uri, &document.place);
            let new = Key::of(&document.uri, &place);
            document.place = place;
            match old {
                Key::Lone(_) => self.forget(old)?,
                Key::Root(_) => {
                    if self.due.contains_key(&old) {
                        self.change(old, true);
                    }
                }
            }
            self.arrived(new);
        }
        Ok(())
    }
}

/// The parameters of a notification `method`, as `read` read them; `None`,
/// logged, if they could not be.
fn parsed<P>(method: &str, read: serde_json::Result<P>) -> Option<P> {
    match read {
        Ok(params) => Some(params),
        Err(error) => {
            log(format_args!(
                "cannot read the parameters of `{method}`: {error}"
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
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
        fn change(
            &self,
            uri: &Uri,
            version: i32,
            (start, end): ((u32, u32), (u32, u32)),
            text: &str,
        ) {
            let range = Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1));
            let change = json!({ "range": range, "text": text });
            let document = json!({ "uri": uri, "version": version });
            let params = json!({ "textDocument": document, "contentChanges": [change] });
            self.notify("textDocument/didChange", params);
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
        assert_eq!(result["serverInfo"]["name"], "gcsl");
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
        let published: PublishDiagnosticsParams =
            serde_json::from_value(notification.params).unwrap();
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
        // `b` changes on disk, which the client reports.
        let checks = client.checks();
        let started = Instant::now();
        while client.checks() == checks {
            assert!(started.elapsed() < WAIT, "the server did not check in time");
            thread::sleep(Duration::from_millis(10));
        }
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
        let response = client.request("textDocument/hover", json!({}));
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
                    let (_, checked) = gcsl::workspace::check_with_standard(
                        &sources.relative,
                        &sources.nested,
                        &name,
                    );
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
}
