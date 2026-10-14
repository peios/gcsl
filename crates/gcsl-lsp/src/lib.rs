//! `gcsl lsp`: a language server for GCSL, which reports each file's errors
//! and warnings as it is edited, answers what a name is (hover), where it
//! is declared (go to definition) and written (references, highlights),
//! renames it, lists symbols, formats, completes what is being typed, and
//! shows the signature of the call being written.
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
//! (see [`position`]). It offers `textDocumentSync` (open and close,
//! incremental changes, and saves), `hoverProvider`, `definitionProvider`,
//! `referencesProvider`, `documentHighlightProvider`, `renameProvider`
//! (with `prepareProvider`), `documentSymbolProvider`,
//! `workspaceSymbolProvider`, `documentFormattingProvider`,
//! `documentRangeFormattingProvider`, `completionProvider` (triggered by
//! `.`, with no `completionItem/resolve`), `signatureHelpProvider`
//! (triggered by `(` and `,`, retriggered by `,`), `inlayHintProvider`
//! (with no `inlayHint/resolve`) and `codeActionProvider` (of the kinds
//! `quickfix` and `source.fixAll`, with no `codeAction/resolve`). It
//! answers `shutdown`, and any
//! request after it with an error; `exit` ends it, cleanly after a
//! `shutdown`, else not ([`Error::ExitWithoutShutdown`], exit code 1). A
//! `workspace/configuration` request is answered with a null for each
//! item; any other request it does not know with `MethodNotFound`;
//! notifications it does not know are ignored. If the client can register
//! for watched files dynamically, the server asks it to watch `**/*.gcsl`.
//!
//! QUERIES. `textDocument/hover`, `definition`, `references`,
//! `documentHighlight`, `prepareRename`, `rename`, `documentSymbol`,
//! `completion`, `signatureHelp`, `inlayHint` and `codeAction`, and
//! `workspace/symbol`, are answered
//! from the index the latest check of the document's workspace, or lone
//! file, made (see [`gcsl::index`]):
//!
//! - a hover as Markdown, with the range of the name or segment hovered:
//!   what it is, its doc comment (the `#` lines directly above its
//!   declaration), and where it comes from;
//! - a definition as a `Location`, or a `Location[]` when what is there is
//!   declared by several items;
//! - references as a `Location[]`, in every file of the workspace, open or
//!   not, the declarations among them if `context.includeDeclaration`;
//!   highlights the same in the document alone, each a write (a
//!   declaration, or a `:=`) or a read;
//! - a rename's preparation as the range of the name and its text, a
//!   rename as a `WorkspaceEdit` whose `changes` are each file's edits by
//!   its URI, open or not; either is refused, with an error saying why
//!   (`RequestFailed`), for what cannot be renamed (what the standard
//!   library, a built-in, a directory or a file declares: a rename renames
//!   no file or directory, though a definition's name often mirrors its
//!   directory's), for a new name of the wrong kind, and for a rename that
//!   would add an error or change what any name refers to, which is
//!   checked by indexing the workspace again with the edits made;
//! - a document's symbols as `DocumentSymbol`s, nested, and the
//!   workspace's, matched by a subsequence of the query ignoring case in
//!   every workspace and lone file the server has indexed, as
//!   `WorkspaceSymbol`s, 200 at most;
//! - a completion as a `CompletionList`, never incomplete, the client
//!   filtering by what is typed: after a `.`, the fields of what is before
//!   it (a record's, a node's children, a module's members); in a type, the
//!   types in scope; in an `import`'s path, the directories, files and
//!   standard modules by segment; after a def-type's name, the definitions
//!   under the path; elsewhere every name in scope, nearest first by
//!   `sortText` (locals, the file's, its directories', imported, the
//!   standard library's, then keywords); each with its kind, its type as
//!   `detail` and its doc comment, an `insertText` only for a name
//!   that must be quoted, and a `textEdit` over the name the place is in,
//!   by GCSL's rules for names (`-` is part of one, so `odd.libcap-t`
//!   completes to `odd.libcap-tools`): an `InsertReplaceEdit` if the client
//!   supports them (`completionItem.insertReplaceSupport`), inserting over
//!   what is typed of the name and replacing all of it, else a `TextEdit`
//!   over what is typed. Where an argument of a call starts, the named
//!   arguments not filled come first, `name = `, of kind `Property`.
//!   Nothing, an empty list, in a string or a comment or where a name is
//!   declared;
//! - signature help as one signature, the callee's, its parameters'
//!   defaults as written, each parameter as
//!   offsets in its label, counted in the negotiated encoding, with the
//!   active parameter: the one a named argument names, else the next one
//!   the arguments before the place have not filled; in a trailing block,
//!   the last, one left open while it is typed included (the workspace is
//!   indexed again for it with the block closed);
//! - inlay hints as `InlayHint[]`, those whose positions are in the range
//!   asked for, its ends included: the types the document does not write
//!   (a `let`'s, a `for` variable's, a definition's parameters' and
//!   result's; see the library's [Inlay hints](gcsl::index#inlay-hints)),
//!   each of kind `Type`, its label `: Int` (a result's as GCSL writes it
//!   after a `)`, ` -> Int` for an impure one), with no padding;
//! - code actions as `CodeAction[]`: for each diagnostic of the document
//!   with a suggestion (`= help: did you mean `version`?`, see the
//!   library's [`gcsl::suggest`]) whose range touches the range asked for,
//!   its ends included, a `quickfix`, "Change to `version`", preferred,
//!   whose `WorkspaceEdit` replaces the misspelled name, with the
//!   diagnostic: the one the client sent of the same range and message if
//!   it sent one, else the diagnostic as it is published; and, when the
//!   document has two suggestions or more, a `source.fixAll`, "Apply all
//!   suggestions in this file", which makes every one (one of two that
//!   overlap left out). A client's `context.only` keeps those of the kinds
//!   it names. The edits are made against the text the answer is made
//!   from, which a stale index is checked again for first, as for every
//!   query, so a fix never edits old text. Formatting is no code action:
//!   `textDocument/formatting` does it;
//!
//! and `null` for nothing there, for a hover of what has no type the check
//! found, for a definition of what the standard library or a built-in
//! declares, which has no file on disk, and for signature help outside a
//! call. A file with errors is indexed for everything that resolved, and so
//! is one that needs a file whose check failed, and the parser keeps what
//! an editor leaves half-typed, `a.` and an unclosed `f(`, so that what is
//! before them is indexed; one whose braces do not match has nothing
//! indexed. A query waits
//! for the worker as a check does, and is handed over before any check
//! due: if a document of the workspace has changed since its index was
//! made, the worker checks it at once, skipping the quiet period, publishes
//! the diagnostics, and answers from the new index (see the `worker`
//! module), so an answer never points at old text; a `workspace/symbol`
//! does so for every workspace and lone file. A `$/cancelRequest` of a
//! query not handed over yet answers it with `RequestCancelled`; one
//! handed over is answered anyway.
//!
//! FORMATTING. `textDocument/formatting` is answered with the edits that
//! lay the document's text out as `gcsl fmt` does ([`gcsl::fmt`]), each a
//! run of whole lines that change, as a line diff finds them, and
//! `textDocument/rangeFormatting` with those of them that touch the range's
//! lines (see [`formatting`]). The formatting options the client sends are
//! not read: GCSL has one style. A document with a lexical or syntax error
//! is answered with an error, `RequestFailed`, whose message says why and
//! where its first error is, `not formatted: it has a syntax error
//! (LINE:COL)`, the line and the column in characters counted from 1, so
//! that an editor can tell the user (why is logged too); one that is not
//! open with `null`.
//!
//! SOURCES. It keeps the text of each open document, applying incremental
//! changes in the negotiated encoding. A `.gcsl` file is a file of the
//! workspace whose root [`gcsl_disk::find_workspace`] finds above it, a
//! file not on disk yet of the one its directory is in, and any other
//! document is checked alone, as `gcsl run FILE` checks a lone file (see
//! [`place`]). A server serves any number of workspaces, each by its root.
//! A workspace's files are read from disk once and kept, and read again
//! when the client reports watched files changed under its root (created,
//! changed or deleted alike), when one of its documents is saved, and when
//! one is closed (its file then has its text on disk again); the open
//! documents' texts stand in for their files', and those not on disk yet
//! are added in path order. A client that cannot register for watched files
//! reports none, so then the server looks itself: before each check of a
//! workspace, and before a query is answered from its index, it stats its
//! files as last read, reads again those whose modification time or size
//! changed, and the whole workspace if one is gone (see the `worker`
//! module). A `.gcsl` file new on disk is found when the workspace is read
//! whole, on a save or a close: finding it takes a walk of the whole tree,
//! which is far slower than a stat of its files. Nothing is ever written to
//! disk.
//!
//! CHECKING. A workspace is checked as `gcsl run` and `pekit` check one,
//! with the same diagnostics, and indexed as it is
//! ([`gcsl::workspace::index_with_standard`], with its canonical root and
//! the directories left out as workspaces of their own), and a lone file as
//! `gcsl run` checks one; nothing is ever run, so opening a file never runs
//! `sh`, `exec` or any GCSL code. Every file the check reports on, open or
//! not, has its diagnostics published, with an open document's version as
//! checked; a file that had some and has none now gets an empty list; a
//! document's first check after it is opened always publishes for it, an
//! empty list included; and a publish whose version and list are both
//! those last published for its URI is skipped, as clients editing in
//! bursts would see it over and over (see the `worker` module).
//!
//! THREADS. Not to lag, the main thread only reads messages, keeps the
//! documents, schedules checks and queries, and formats; it never checks.
//! Formatting is work on one document's text alone, a lex and a parse or
//! two, so it is answered at once there, not queued behind a check. One worker
//! thread (the `worker` module) reads workspaces from disk, checks and
//! indexes them, publishes, and answers queries, one job at a time: the
//! main thread hands it the next only once it has answered the last, so
//! checks never overlap and stale work never queues up behind new edits, a
//! check or a query taking the documents as they are when it is handed
//! over. The indexes live on the worker, and never leave it.
//!
//! DEBOUNCING. A change to a workspace asks for a check of it [`QUIET`]
//! (150 ms) after the change, and a later change moves that on: a burst of
//! edits is one check. Opening a document of a workspace not checked yet
//! asks for one at once. When a check is done, its diagnostics are
//! published even if the workspace changed meanwhile, with each document's
//! version as it was checked, and the change's own check follows its quiet
//! period.
//!
//! KNOWN LIMITS. A file that does not parse no longer hides the rest: its
//! syntax errors are published with the type errors of every file, its own
//! where it parses among them (see the library's [Syntax
//! errors](gcsl::workspace#syntax-errors)). Nor does an error of the scan of
//! a workspace, an import not resolved, two files declaring one name, files
//! that need each other: it is published with every other file's errors and
//! warnings, and every file it does not concern is indexed and answers (see
//! the library's [Scan errors](gcsl::workspace#scan-errors)); a read of a
//! name two files declare has no hover, and goes to both declarations. What
//! the check reports is published, and stale diagnostics are not kept. Nor
//! is a stale index: a file whose braces do not match has nothing to answer
//! until it is checked again. A file that needs a file whose check failed,
//! or one of files that need each other, is indexed, but what it reads of
//! that file's own directory has no type (see [`gcsl::index`]). What the
//! standard library and the built-ins declare has no definition to go to:
//! their sources are compiled in, with no file on disk. A rename renames
//! no file or directory, so neither a name a directory or a file gives
//! (`sandbox` of `import _lib.sandbox`) nor `fileroot`'s can be renamed,
//! and a definition renamed no longer mirrors its directory's name.

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

pub mod formatting;
pub mod place;
pub mod position;
mod worker;

use std::collections::{HashMap, VecDeque};
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
    Cancel, DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument,
    DidOpenTextDocument, DidSaveTextDocument, Notification as _,
};
use lsp_types::request::{
    CodeActionRequest, Completion, DocumentHighlightRequest, DocumentSymbolRequest, Formatting,
    GotoDefinition, HoverRequest, InlayHintRequest, PrepareRenameRequest, RangeFormatting,
    References, RegisterCapability, Rename, Request as _, SignatureHelpRequest,
    WorkspaceSymbolRequest,
};
use lsp_types::{
    CancelParams, CodeActionKind, CodeActionOptions, CodeActionParams,
    CodeActionProviderCapability, CompletionOptions, CompletionParams, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentFormattingParams, DocumentHighlightParams, DocumentRangeFormattingParams,
    DocumentSymbolParams, FileSystemWatcher, GlobPattern, HoverProviderCapability,
    InitializeResult, InlayHintParams, NumberOrString, OneOf, Position, ReferenceParams,
    Registration, RegistrationParams, RenameOptions, RenameParams, ServerCapabilities, ServerInfo,
    SignatureHelpOptions, SignatureHelpParams, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Uri, WorkspaceSymbolParams,
};
use serde_json::Value;

use crate::place::{Place, locate, path_of};
use crate::position::Encoding;
use crate::worker::{Asked, Check, Job, Key, Query, Snapshot, Symbols};

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
    let insert_replace = params
        .pointer("/capabilities/textDocument/completion/completionItem/insertReplaceSupport")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    connection.initialize_finish(id, initialize_result(encoding))?;
    let (done, finished) = crossbeam_channel::unbounded();
    // Without the client watching files, the worker looks at them itself.
    let (jobs, worker) = worker::spawn(
        connection.sender.clone(),
        (encoding, !watch, insert_replace),
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
        queries: VecDeque::new(),
        busy: false,
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
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            document_highlight_provider: Some(OneOf::Left(true)),
            rename_provider: Some(OneOf::Right(RenameOptions {
                prepare_provider: Some(true),
                work_done_progress_options: lsp_types::WorkDoneProgressOptions::default(),
            })),
            document_symbol_provider: Some(OneOf::Left(true)),
            workspace_symbol_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            document_range_formatting_provider: Some(OneOf::Left(true)),
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(vec![".".to_owned()]),
                resolve_provider: Some(false),
                ..CompletionOptions::default()
            }),
            signature_help_provider: Some(SignatureHelpOptions {
                trigger_characters: Some(vec!["(".to_owned(), ",".to_owned()]),
                retrigger_characters: Some(vec![",".to_owned()]),
                work_done_progress_options: lsp_types::WorkDoneProgressOptions::default(),
            }),
            inlay_hint_provider: Some(OneOf::Left(true)),
            code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
                code_action_kinds: Some(vec![
                    CodeActionKind::QUICKFIX,
                    CodeActionKind::SOURCE_FIX_ALL,
                ]),
                resolve_provider: Some(false),
                ..CodeActionOptions::default()
            })),
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
    /// Whether a check has been handed over with it since it was opened.
    handed: bool,
}

/// A check asked for.
#[derive(Debug, Default)]
struct Due {
    /// When to check, if a check is asked for.
    at: Option<Instant>,
    /// Whether the files are read from disk again first.
    reread: bool,
}

/// A query asked and not handed to the worker yet.
#[derive(Debug)]
enum Pending {
    /// Of a place of a document.
    Query {
        /// The request's id.
        id: RequestId,
        /// What it asks.
        asked: Asked,
        /// The document's URI, as the client gave it.
        uri: Uri,
        /// Where in it.
        position: Position,
        /// What the document is checked as part of.
        key: Key,
    },
    /// A `workspace/symbol`, of every workspace and lone file.
    Symbols {
        /// The request's id.
        id: RequestId,
        /// What the names must have as a subsequence.
        query: String,
    },
}

impl Pending {
    /// The request's id.
    fn id(&self) -> &RequestId {
        match self {
            Pending::Query { id, .. } | Pending::Symbols { id, .. } => id,
        }
    }
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
    /// The queries asked and not handed to the worker yet, in order.
    queries: VecDeque<Pending>,
    /// Whether the worker is checking, or answering a query.
    busy: bool,
    /// Whether the client has said `shutdown`.
    shutdown: bool,
}

impl Server<'_> {
    /// Reads messages and schedules checks until the client says `exit`,
    /// handing the worker the next due check whenever it is idle; the
    /// worker says on `finished` when it has done one.
    fn run(&mut self, finished: &Receiver<()>) -> Result<(), Error> {
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
                recv(finished) -> done => {
                    done.map_err(|_| Error::WorkerStopped)?;
                    self.busy = false;
                }
                recv(wake) -> _ => {}
            }
        }
    }

    /// When the next check is due, if the worker is idle to make it.
    fn next_wake(&self) -> Option<Instant> {
        if self.busy {
            return None;
        }
        self.due.values().filter_map(|due| due.at).min()
    }

    /// Hands the worker, if it is idle, the first query asked, or else the
    /// check due first, if one is due: with the documents of what it is of
    /// as they are now.
    fn dispatch(&mut self) -> Result<(), Error> {
        if self.busy {
            return Ok(());
        }
        match self.queries.pop_front() {
            Some(Pending::Query {
                id,
                asked,
                uri,
                position,
                key,
            }) => {
                let query = Query {
                    id,
                    asked,
                    uri,
                    position,
                    check: self.handed(key),
                };
                return self.hand(Job::Query(query));
            }
            Some(Pending::Symbols { id, query }) => {
                let mut keys: Vec<Key> = self.due.keys().cloned().collect();
                keys.sort_by_key(|key| format!("{key:?}"));
                let checks = keys.into_iter().map(|key| self.handed(key)).collect();
                return self.hand(Job::Symbols(Symbols { id, query, checks }));
            }
            None => {}
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
        let check = self.handed(key);
        self.hand(Job::Check(check))
    }

    /// Hands `job` to the worker, which is busy until it says it is done.
    fn hand(&mut self, job: Job) -> Result<(), Error> {
        self.jobs.send(job).map_err(|_| Error::WorkerStopped)?;
        self.busy = true;
        Ok(())
    }

    /// The check of `key` with its open documents as they are now, to be
    /// handed over: it takes what is due of `key`, and each document's
    /// first check since it was opened is this one.
    fn handed(&mut self, key: Key) -> Check {
        let reread = self.due.get_mut(&key).is_some_and(|due| {
            due.at = None;
            std::mem::take(&mut due.reread)
        });
        let mut documents = Vec::new();
        for document in self.documents.values_mut() {
            if Key::of(&document.uri, &document.place) != key {
                continue;
            }
            documents.push(Snapshot {
                uri: document.uri.clone(),
                version: document.version,
                text: document.text.clone(),
                place: document.place.clone(),
                opened: !document.handed,
            });
            document.handed = true;
        }
        Check {
            key,
            documents,
            reread,
        }
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
                if let Some(response) = self.answer(request) {
                    self.send(Message::Response(response))?;
                }
                Ok(Flow::Continue)
            }
            Message::Notification(notification) => self.notified(notification),
            // The answer to registering for watched files.
            Message::Response(_) => Ok(Flow::Continue),
        }
    }

    /// The response to `request`; `None` for a query, which the worker
    /// answers.
    fn answer(&mut self, request: Request) -> Option<Response> {
        if self.shutdown {
            return Some(Response::new_err(
                request.id,
                ErrorCode::InvalidRequest as i32,
                "the server is shutting down".to_owned(),
            ));
        }
        let response = match request.method.as_str() {
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
            HoverRequest::METHOD
            | GotoDefinition::METHOD
            | References::METHOD
            | DocumentHighlightRequest::METHOD
            | PrepareRenameRequest::METHOD
            | Rename::METHOD
            | DocumentSymbolRequest::METHOD
            | Completion::METHOD
            | SignatureHelpRequest::METHOD
            | InlayHintRequest::METHOD
            | CodeActionRequest::METHOD
            | WorkspaceSymbolRequest::METHOD => return self.ask(request),
            Formatting::METHOD | RangeFormatting::METHOD => self.format(request),
            method => Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("unknown method `{method}`"),
            ),
        };
        Some(response)
    }

    /// Queues `request`, a query of a document, or `workspace/symbol`, for
    /// the worker to answer; `None` once it is. Parameters that cannot be
    /// read are `InvalidParams`, and a document that is not open has
    /// nothing to say: `null`.
    fn ask(&mut self, request: Request) -> Option<Response> {
        let Request { id, method, params } = request;
        let read = match method.as_str() {
            WorkspaceSymbolRequest::METHOD => {
                match serde_json::from_value::<WorkspaceSymbolParams>(params) {
                    Ok(params) => {
                        let query = params.query;
                        self.queries.push_back(Pending::Symbols { id, query });
                        return None;
                    }
                    Err(error) => Err(error),
                }
            }
            _ => asked(&method, params),
        };
        let (uri, position, asked) = match read {
            Ok(read) => read,
            Err(error) => {
                let message = format!("cannot read the parameters of `{method}`: {error}");
                return Some(Response::new_err(
                    id,
                    ErrorCode::InvalidParams as i32,
                    message,
                ));
            }
        };
        let Some(document) = self.documents.get(uri.as_str()) else {
            return Some(Response::new_ok(id, Value::Null));
        };
        let key = Key::of(&uri, &document.place);
        self.queries.push_back(Pending::Query {
            id,
            asked,
            uri,
            position,
            key,
        });
        None
    }

    /// The response to `request`, a `textDocument/formatting` or a
    /// `textDocument/rangeFormatting`, made here, on the main thread, from
    /// the document's text as the server has it (see [`formatting`]): the
    /// edits that format it, those that touch the range's lines for a
    /// range (its last line left out when the range ends at its start, as a
    /// selection of whole lines does). A document with a lexical or syntax
    /// error is refused with `RequestFailed`, `not formatted: it has a syntax
    /// error (LINE:COL)`, at its first error, why logged too; a document that
    /// is not open has nothing to say: `null`. Parameters that cannot be read
    /// are `InvalidParams`.
    fn format(&self, request: Request) -> Response {
        let Request { id, method, params } = request;
        let read = if method == RangeFormatting::METHOD {
            serde_json::from_value::<DocumentRangeFormattingParams>(params).map(|params| {
                let (start, end) = (params.range.start, params.range.end);
                let last = if end.character == 0 && end.line > start.line {
                    end.line - 1
                } else {
                    end.line
                };
                (
                    params.text_document.uri,
                    Some(start.line as usize..=last as usize),
                )
            })
        } else {
            serde_json::from_value::<DocumentFormattingParams>(params)
                .map(|params| (params.text_document.uri, None))
        };
        let (uri, lines) = match read {
            Ok(read) => read,
            Err(error) => {
                let message = format!("cannot read the parameters of `{method}`: {error}");
                return Response::new_err(id, ErrorCode::InvalidParams as i32, message);
            }
        };
        let Some(document) = self.documents.get(uri.as_str()) else {
            return Response::new_ok(id, Value::Null);
        };
        match formatting::edits(uri.as_str(), &document.text, self.encoding, lines.as_ref()) {
            Ok(edits) => Response::new_ok(id, edits),
            Err(why) => Response::new_err(id, ErrorCode::RequestFailed as i32, why),
        }
    }

    /// A `$/cancelRequest` of the request `id`: answered with
    /// `RequestCancelled` if it is a query not handed to the worker yet,
    /// else let be.
    fn cancel(&mut self, id: &NumberOrString) -> Result<(), Error> {
        let id = match id {
            NumberOrString::Number(number) => RequestId::from(*number),
            NumberOrString::String(text) => RequestId::from(text.clone()),
        };
        let Some(at) = self.queries.iter().position(|pending| *pending.id() == id) else {
            return Ok(());
        };
        self.queries.remove(at);
        let response = Response::new_err(
            id,
            ErrorCode::RequestCanceled as i32,
            "the request was cancelled".to_owned(),
        );
        self.send(Message::Response(response))
    }

    /// Handles `notification`: `exit`, or a change to the documents or the
    /// files. Parameters that cannot be read are logged and ignored, as is
    /// a notification not known.
    fn notified(&mut self, notification: Notification) -> Result<Flow, Error> {
        let Notification { method, params } = notification;
        match method.as_str() {
            "exit" => return Ok(Flow::Exit),
            Cancel::METHOD => {
                let read = serde_json::from_value::<CancelParams>(params);
                if let Some(params) = parsed(&method, read) {
                    self.cancel(&params.id)?;
                }
            }
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
            handed: false,
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

/// What the query `method` with `params` asks, of which document and where
/// in it (a document's symbols, of no place in it, at its start).
///
/// # Errors
///
/// The parameters cannot be read.
fn asked(method: &str, params: Value) -> serde_json::Result<(Uri, Position, Asked)> {
    let at = |params: TextDocumentPositionParams, asked| {
        (params.text_document.uri, params.position, asked)
    };
    Ok(match method {
        GotoDefinition::METHOD => at(serde_json::from_value(params)?, Asked::Definition),
        References::METHOD => {
            let params: ReferenceParams = serde_json::from_value(params)?;
            let declarations = params.context.include_declaration;
            at(
                params.text_document_position,
                Asked::References(declarations),
            )
        }
        DocumentHighlightRequest::METHOD => {
            let params: DocumentHighlightParams = serde_json::from_value(params)?;
            at(params.text_document_position_params, Asked::Highlight)
        }
        PrepareRenameRequest::METHOD => at(serde_json::from_value(params)?, Asked::PrepareRename),
        Rename::METHOD => {
            let params: RenameParams = serde_json::from_value(params)?;
            at(
                params.text_document_position,
                Asked::Rename(params.new_name),
            )
        }
        DocumentSymbolRequest::METHOD => {
            let params: DocumentSymbolParams = serde_json::from_value(params)?;
            let start = Position::default();
            (params.text_document.uri, start, Asked::DocumentSymbols)
        }
        Completion::METHOD => {
            let params: CompletionParams = serde_json::from_value(params)?;
            at(params.text_document_position, Asked::Completion)
        }
        SignatureHelpRequest::METHOD => {
            let params: SignatureHelpParams = serde_json::from_value(params)?;
            at(params.text_document_position_params, Asked::SignatureHelp)
        }
        InlayHintRequest::METHOD => {
            let params: InlayHintParams = serde_json::from_value(params)?;
            let (start, end) = (params.range.start, params.range.end);
            (params.text_document.uri, start, Asked::InlayHints(end))
        }
        CodeActionRequest::METHOD => {
            let params: CodeActionParams = serde_json::from_value(params)?;
            let (start, end) = (params.range.start, params.range.end);
            let asked = Asked::CodeActions(end, params.context);
            (params.text_document.uri, start, asked)
        }
        _ => at(serde_json::from_value(params)?, Asked::Hover),
    })
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
        let params =
            json!({ "textDocument": { "uri": a }, "position": position, "newName": "base" });
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
