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
//!   the last, one left open while it is typed included;
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
//! before them is indexed. One whose braces do not match, as one is while
//! a block is typed, has nothing indexed: a query at a place of it, but a
//! rename, is answered from the workspace indexed again with the file's
//! braces balanced for the query, the `}`s it lacks put after the place's
//! line, or at the end of the file, and any `}` that closes nothing taken
//! out, the places found mapped back to the text as it is (see the
//! library's [Unbalanced files](gcsl::index#unbalanced-files)); a rename
//! there is refused, "the file has an unclosed `{`", as its check needs the
//! text as it is, and the diagnostics published are the syntax error
//! alone. The workspace is indexed again once for the queries at a place
//! of one version of the document, not for each. A query waits
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
//! is a stale index: a file whose braces do not match answers from the
//! text as it is, balanced for the query, not from the last index that
//! checked. A file that needs a file whose check failed,
//! or one of files that need each other, is indexed, but what it reads of
//! that file's own directory has no type (see [`gcsl::index`]). What the
//! standard library and the built-ins declare has no definition to go to:
//! their sources are compiled in, with no file on disk. A rename renames
//! no file or directory, so neither a name a directory or a file gives
//! (`sandbox` of `import _lib/sandbox`) nor `fileroot`'s can be renamed,
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
    CompletionTriggerKind, CancelParams, CodeActionKind, CodeActionOptions, CodeActionParams,
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

use gcsl_disk::WORKSPACE;
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

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            Self::Worker(error) => Some(error),
            Self::ExitWithoutShutdown | Self::Disconnected | Self::WorkerStopped => None,
        }
    }
}

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
        worker::Options {
            encoding,
            stat: !watch,
            insert_replace,
        },
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
                trigger_characters: Some(vec![".".to_owned(), "/".to_owned()]),
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
            // Only a character the server named as a trigger means anything
            // to the index: `/` opens a list only in an import's path.
            let trigger = params
                .context
                .filter(|context| context.trigger_kind == CompletionTriggerKind::TRIGGER_CHARACTER)
                .and_then(|context| context.trigger_character)
                .and_then(|text| text.chars().next());
            at(params.text_document_position, Asked::Completion(trigger))
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
mod tests;
