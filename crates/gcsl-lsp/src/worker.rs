//! The worker: the one thread of a server that reads workspaces from disk,
//! checks and indexes them, publishes their diagnostics and answers the
//! queries the index answers.
//!
//! The main thread hands it a [`Job`] at a time ([`spawn`]): a [`Check`],
//! the snapshot of the open documents of one workspace, or of one lone
//! file, as they are when it is handed over; a [`Query`], a hover, a
//! definition, references, highlights, a rename or its preparation, a
//! document's symbols, a completion, signature help, inlay hints or code
//! actions asked of an open document, with such a snapshot; a
//! [`Job::Symbols`], `workspace/symbol`, with a snapshot of every
//! workspace and lone file known; or a [`Job::Clear`] of a workspace or
//! lone file there is no more to say of. It says on its `done` channel when
//! it has published a check's diagnostics, or sent a query's response, and
//! the main thread hands it the next job only then, so that checks never
//! overlap and never queue up behind each other.
//!
//! It keeps each workspace's files as read from disk
//! ([`gcsl_disk::read_workspace`]), read once, and again only when a check
//! asks for it ([`Check::reread`]); a check's sources are those files with
//! the open documents' texts in place of theirs, and the open documents
//! that are none of them added in path order. When the client does not
//! watch files for the server, before each check, and before a query is
//! answered from the index it has, the worker stats the workspace's files
//! as last read and reads again those whose modification time or size
//! changed, and the whole workspace if one is gone (a STAT PASS). A new
//! `.gcsl` file is found only when the workspace is read whole (a save, a
//! close): that takes a walk of the whole tree, which in a workspace of
//! build trees is far too slow to make before each check. The
//! workspace is checked as `gcsl run` and `pekit` check one, with the same
//! diagnostics, but by [`gcsl::workspace::index_with_standard`], which
//! indexes what it finds, with its canonical root and the directories left
//! out as workspaces of their own; a lone file as `gcsl run` checks one, by
//! [`gcsl::workspace::index_lone_with_standard`] with its canonical
//! directory. Nothing is run, and nothing is written to disk.
//!
//! It keeps the latest index of each workspace and lone file, with the
//! version of each open document it was made from and the diagnostics of
//! its check that carry a suggestion, which code actions make, whether its
//! check succeeded or failed (see [`gcsl::index`]): a file with errors is
//! indexed for everything that resolved, and so is one that needs a file
//! whose check failed; one whose braces do not match is not checked, and
//! has nothing indexed, not even what the last good index had: a query at
//! a place of it (but a rename, a document's symbols, code actions) is
//! answered from the workspace indexed again, as a rename's check indexes
//! it, with the file's braces balanced virtually (see the library's
//! "Unbalanced files"), counted as a check. That index is kept with the
//! index it was made from, for the queries after it at the same versions,
//! and made again only for a place it does not serve; it never changes
//! what is published. A rename in such a file is refused. A query is
//! answered from it only if every open document of what it is of has the
//! version it was made from, none was opened since, the query's check does
//! not read the files from disk again, and no stat pass found a file
//! changed: otherwise its positions could point at the wrong text, so the
//! query's check is made first, at once, its diagnostics published as any
//! check's are, and the query answered from the new index. A rename is
//! checked by indexing the workspace again with its edits made (see
//! [`gcsl::index`]), as the workspace was, with its directories left out
//! and its root, or as the lone file was, in its directory.
//!
//! Every file the check reports on, open or not, gets a
//! `textDocument/publishDiagnostics` with its diagnostics, by its URI (an
//! open document's as the client gave it), with an open document's version
//! as it was checked; every file that had some the last time and has none
//! now gets an empty list; and a document opened since the last check of
//! it gets one, an empty list included, so that a client can tell "clean"
//! from "not checked yet". A publish whose version and list are both those
//! of the last one for its URI is not sent again; one with a new version
//! is, for a client that keys diagnostics by version. A diagnostic of a
//! standard file is logged to stderr, as there is no file to attach it to.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};
use lsp_server::{ErrorCode, Message, Notification, RequestId, Response};
use lsp_types::notification::{Notification as _, PublishDiagnostics, ShowMessage};
use lsp_types::{
    CodeActionContext, CodeActionKind, CompletionItem, CompletionItemKind, CompletionList,
    CompletionTextEdit, DiagnosticRelatedInformation, DiagnosticSeverity, DocumentHighlight,
    DocumentHighlightKind, Documentation, GotoDefinitionResponse, Hover, HoverContents, InlayHint,
    InlayHintKind, InlayHintLabel, InsertReplaceEdit, Location, MarkupContent, MarkupKind,
    MessageType, OneOf, ParameterInformation, ParameterLabel, Position, PublishDiagnosticsParams,
    ShowMessageParams, SignatureHelp, SignatureInformation, TextEdit, Uri, WorkspaceSymbol,
};
use serde_json::{Map, Value, json};

use gcsl::diagnostic::Renderer;
use gcsl::index::{
    Closed, ClosedCache, Completion, CompletionKind, DocumentSymbol, Edit, Highlight, Index,
    Indexed, Prepared, Signature, SymbolKind,
};
use gcsl::workspace;
use gcsl::{Diagnostic, Severity, SourceFile, Span, Suggestion};
use gcsl_disk::{ReadError, Sources};
use crate::log;
use crate::place::{Place, uri_of};
use crate::position::{Encoding, LineIndex};

/// What a check is of: the workspace under a root, or a lone file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// The workspace whose canonical root this is.
    Root(PathBuf),
    /// The lone file of the open document of the URI of this text. (A
    /// [`Uri`] keeps a cache inside it, so maps are keyed by its text.)
    Lone(String),
}

impl Key {
    /// What the document named `uri`, at `place`, is checked as part of.
    #[must_use]
    pub fn of(uri: &Uri, place: &Place) -> Self {
        match place {
            Place::Workspace { root, .. } => Key::Root(root.clone()),
            Place::Lone { .. } => Key::Lone(uri.as_str().to_owned()),
        }
    }
}

/// An open document as it was when a check was handed over.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Its URI, as the client gave it.
    pub uri: Uri,
    /// Its version.
    pub version: i32,
    /// Its text.
    pub text: String,
    /// Where it is.
    pub place: Place,
    /// Whether it was opened since a check was last handed over with it:
    /// its first check publishes for it, an empty list included.
    pub opened: bool,
}

/// A check to make.
#[derive(Debug)]
pub struct Check {
    /// What to check.
    pub key: Key,
    /// The open documents of what is checked: of a lone file, its own.
    pub documents: Vec<Snapshot>,
    /// Whether the workspace's files are read from disk again first.
    pub reread: bool,
}

/// What a [`Query`] asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// `textDocument/hover`.
    Hover,
    /// `textDocument/definition`.
    Definition,
    /// `textDocument/references`, the declarations included if `true`.
    References(bool),
    /// `textDocument/documentHighlight`.
    Highlight,
    /// `textDocument/prepareRename`.
    PrepareRename,
    /// `textDocument/rename`, to this new name.
    Rename(String),
    /// `textDocument/documentSymbol`, which has no position.
    DocumentSymbols,
    /// `textDocument/completion`, asked for because this trigger character
    /// was typed, if one was.
    Completion(Option<char>),
    /// `textDocument/signatureHelp`.
    SignatureHelp,
    /// `textDocument/inlayHint`, of the range from the query's position to
    /// this one.
    InlayHints(Position),
    /// `textDocument/codeAction`, of the range from the query's position to
    /// this one, with what the client sent of its diagnostics there and
    /// the kinds it wants.
    CodeActions(Position, CodeActionContext),
}

/// The most symbols a `workspace/symbol` answers with.
pub const MAX_SYMBOLS: usize = 200;

/// A `workspace/symbol`: answered from the index of every workspace and
/// lone file, each made fresh first if it is stale.
#[derive(Debug)]
pub struct Symbols {
    /// The request's id.
    pub id: RequestId,
    /// What the names must have as a subsequence, ignoring case.
    pub query: String,
    /// The check of each workspace and lone file that makes its index
    /// fresh, if it is stale.
    pub checks: Vec<Check>,
}

/// A request about a place of an open document, answered from the index of
/// what it is checked as part of.
#[derive(Debug)]
pub struct Query {
    /// The request's id.
    pub id: RequestId,
    /// What it asks.
    pub asked: Asked,
    /// The document's URI, as the client gave it.
    pub uri: Uri,
    /// The place, counted in the negotiated encoding.
    pub position: Position,
    /// The check that makes a stale index fresh: of the open documents of
    /// what the document is checked as part of, as they are when the query
    /// is handed over.
    pub check: Check,
}

/// What the worker is asked to do.
#[derive(Debug)]
pub enum Job {
    /// Check, and publish what the check finds.
    Check(Check),
    /// Answer a query, checking first if the index is stale.
    Query(Query),
    /// Answer a `workspace/symbol`, checking first what is stale.
    Symbols(Symbols),
    /// Forget a workspace or lone file, giving every file it has published
    /// diagnostics for an empty list.
    Clear(Key),
}

/// Negotiated client capabilities and the disk-refresh policy for one worker.
#[derive(Clone, Copy)]
pub struct Options {
    /// Position encoding negotiated with the client.
    pub encoding: Encoding,
    /// Refresh known files on disk when the client cannot watch them.
    pub stat: bool,
    /// Whether completions may use insert/replace edits.
    pub insert_replace: bool,
}

/// Starts the worker, which publishes through `client` and says on `done`
/// when it has done a check or a query, positions counted in `encoding`,
/// and counts each check it makes in `checks`; it makes a stat pass before
/// each check of a workspace if `stat`, when the client does not watch
/// files for the server, and gives completions `InsertReplaceEdit`s if
/// `insert_replace`, when the client supports them. Returns where to send
/// it jobs: it stops once that is dropped, or `done` is.
///
/// Its thread has the stack the `gcsl` library asks for
/// ([`gcsl::STACK_SIZE`]), since a check recurses once per level of
/// nesting.
///
/// # Errors
///
/// The thread cannot be started.
pub fn spawn(
    client: Sender<Message>,
    options: Options,
    checks: Arc<AtomicUsize>,
    done: Sender<()>,
) -> io::Result<(Sender<Job>, JoinHandle<()>)> {
    let Options {
        encoding,
        stat,
        insert_replace,
    } = options;
    let (jobs, received) = crossbeam_channel::unbounded();
    // Made on its thread: an index is not `Send`, and never leaves it.
    let run = move || {
        let worker = Worker {
            client,
            encoding,
            stat,
            insert_replace,
            checks,
            disk: HashMap::new(),
            published: HashMap::new(),
            last: HashMap::new(),
            indexes: HashMap::new(),
        };
        worker.run(&received, &done);
    };
    let handle = thread::Builder::new()
        .name("gcsl-lsp-worker".to_owned())
        .stack_size(gcsl::STACK_SIZE)
        .spawn(run)?;
    Ok((jobs, handle))
}

/// A workspace's files as read from disk.
#[derive(Debug)]
struct Disk {
    /// Every file, in path order, named by its path relative to the root.
    files: Vec<SourceFile>,
    /// The directories left out as workspaces of their own.
    nested: Vec<String>,
    /// What a stat of each file found as it was read.
    stamps: Vec<Stamp>,
}

/// What a stat of a file finds ([`stamps`]): its modification time and
/// size, `None` for a file that is gone or cannot be stat'd; the time
/// `None` if it is not known, or is not before the file was read, so that
/// the next stat differs.
type Stamp = Option<(Option<SystemTime>, u64)>;

/// What a check found: the diagnostics of each file, and what to tell the
/// user apart.
#[derive(Debug, Default)]
struct Report {
    /// Each file with diagnostics, by URI, with its version if it is open,
    /// in the order the check numbers them.
    files: Vec<(Uri, Option<i32>, Vec<lsp_types::Diagnostic>)>,
    /// The version of each open document as it was checked, by the text of
    /// its URI.
    versions: HashMap<String, i32>,
    /// The open documents opened since a check was last handed over with
    /// them, with their versions: each gets a publish, if empty.
    opened: Vec<(Uri, i32)>,
    /// A message to show the user, for what cannot be put in a file.
    message: Option<String>,
}

/// The worker's state.
#[derive(Debug)]
struct Worker {
    /// Where messages to the client go.
    client: Sender<Message>,
    /// The encoding positions are counted in.
    encoding: Encoding,
    /// Whether a stat pass is made before each check of a workspace: the
    /// client does not watch files for the server.
    stat: bool,
    /// Whether completions have `InsertReplaceEdit`s, the client supporting
    /// them; else `TextEdit`s.
    insert_replace: bool,
    /// How many checks have been made.
    checks: Arc<AtomicUsize>,
    /// Each workspace's files as last read, by root, or why they could not
    /// be.
    disk: HashMap<PathBuf, Result<Disk, ReadError>>,
    /// The files each workspace or lone file last published diagnostics
    /// for, each URI by its text.
    published: HashMap<Key, HashMap<String, Uri>>,
    /// The last diagnostics published for each URI, by its text, with
    /// their version.
    last: HashMap<String, (Option<i32>, Vec<lsp_types::Diagnostic>)>,
    /// The latest index of each workspace and lone file.
    indexes: HashMap<Key, Kept>,
}

/// The latest index of a workspace or lone file, with what it was made of.
#[derive(Debug)]
struct Kept {
    /// The version of each open document it was made from, by the text of
    /// its URI.
    versions: HashMap<String, i32>,
    /// The index; `None` if the check made none, as for a lone file given
    /// no file: a scan's errors leave one (see the library's "Scan errors").
    index: Option<Index>,
    /// Each input of the check, by input number: the standard files', then
    /// the files checked.
    sources: Vec<SourceFile>,
    /// The URI of each input, by input number; `None` for a standard file,
    /// and a file whose path is not UTF-8.
    uris: Vec<Option<Uri>>,
    /// How many of the inputs are standard files, the first.
    standard: usize,
    /// The diagnostics of the check that carry a suggestion, which code
    /// actions make, as the check found them.
    suggested: Vec<Diagnostic>,
    /// The input a diagnostic naming none is of: a lone file's own.
    own: Option<u32>,
    /// How what it is of is indexed again, as a rename's check does.
    again: Again,
    /// The index made again for the queries of a file whose braces do not
    /// match, with them balanced, kept for the queries after the first:
    /// made again only for a place it does not serve.
    closed: ClosedCache,
}

/// How a workspace, or a lone file, is indexed again with its files
/// changed, as it was indexed.
#[derive(Clone, Debug)]
enum Again {
    /// A workspace: with the directories left out and its root's name.
    Workspace {
        /// The directories left out as workspaces of their own.
        nested: Vec<String>,
        /// The root, as the library is given it.
        root: String,
    },
    /// A lone file, in this directory.
    Lone(String),
}

impl Again {
    /// The index of `files`, the files after the standard ones.
    fn index(&self, files: &[SourceFile]) -> Indexed {
        match self {
            Again::Workspace { nested, root } => {
                workspace::index_with_standard(files, nested, root)
            }
            Again::Lone(dir) => match files.first() {
                Some(file) => workspace::index_lone_with_standard(file, dir),
                None => Indexed {
                    standard: Vec::new(),
                    diagnostics: Vec::new(),
                    succeeded: false,
                    index: None,
                },
            },
        }
    }
}

impl Worker {
    /// Does each job received, until there are none, or the main thread is
    /// gone.
    fn run(mut self, jobs: &Receiver<Job>, done: &Sender<()>) {
        for job in jobs {
            match job {
                Job::Check(check) => self.check(check),
                Job::Query(query) => self.query(query),
                Job::Symbols(symbols) => self.symbols(symbols),
                Job::Clear(key) => {
                    self.clear(&key);
                    continue;
                }
            }
            if done.send(()).is_err() {
                return;
            }
        }
    }

    /// Makes `check`, publishes what it finds, and keeps its index.
    fn check(&mut self, check: Check) {
        let (report, kept) = match &check.key {
            Key::Root(root) => self.check_workspace(root, &check),
            Key::Lone(uri) => self.check_lone(uri, &check),
        };
        match kept {
            Some(kept) => self.indexes.insert(check.key.clone(), kept),
            None => self.indexes.remove(&check.key),
        };
        if let Some(report) = report {
            self.publish(check.key, report);
        }
    }

    /// Answers `query`, from the index of what its document is checked as
    /// part of, made fresh first if it is stale (see the module
    /// documentation).
    fn query(&mut self, query: Query) {
        let Query {
            id,
            asked,
            uri,
            position,
            check,
        } = query;
        let key = check.key.clone();
        self.refresh(check);
        let answer = match self.indexes.get_mut(&key) {
            Some(kept) => {
                let how = (self.encoding, self.insert_replace);
                kept.answer(asked, &uri, position, (how, &self.checks))
            }
            None => Ok(Value::Null),
        };
        let response = match answer {
            Ok(answer) => Response::new_ok(id, answer),
            Err(why) => Response::new_err(id, ErrorCode::RequestFailed as i32, why),
        };
        // A disconnected client has no receiver; the server owns shutdown.
        let _ = self.client.send(Message::Response(response));
    }

    /// Makes `check` if the index of what it is of is stale: if a document
    /// of it has changed or was opened since the index was made, if it
    /// asks for the files to be read again, or if a stat pass finds a file
    /// changed on disk.
    fn refresh(&mut self, check: Check) {
        let changed = match &check.key {
            Key::Root(root) if self.stat && !check.reread => self.stat_pass(root),
            _ => false,
        };
        let fresh = !check.reread
            && !changed
            && !check.documents.iter().any(|document| document.opened)
            && self
                .indexes
                .get(&check.key)
                .is_some_and(|kept| kept.versions == versions(&check));
        if !fresh {
            self.check(check);
        }
    }

    /// Answers `symbols`, a `workspace/symbol`, from the index of every
    /// workspace and lone file, each made fresh first if it is stale: the
    /// symbols of each, in the order of their roots or URIs, [`MAX_SYMBOLS`]
    /// at most.
    fn symbols(&mut self, symbols: Symbols) {
        let Symbols { id, query, checks } = symbols;
        for check in checks {
            self.refresh(check);
        }
        let mut keys: Vec<&Key> = self.indexes.keys().collect();
        keys.sort_by_key(|key| match key {
            Key::Root(root) => root.to_string_lossy().into_owned(),
            Key::Lone(uri) => uri.clone(),
        });
        let mut found = Vec::new();
        for key in keys {
            let Some(kept) = self.indexes.get(key) else {
                continue;
            };
            let left = MAX_SYMBOLS.saturating_sub(found.len());
            if left == 0 {
                break;
            }
            found.extend(kept.workspace_symbols(&query, left, self.encoding));
        }
        let answer = serde_json::to_value(found).unwrap_or(Value::Null);
        // A disconnected client has no receiver; the server owns shutdown.
        let _ = self
            .client
            .send(Message::Response(Response::new_ok(id, answer)));
    }

    /// Checks the workspace under `root`, its files read from disk if
    /// `check` asks for it or they never were, with the open documents in
    /// `check`; with its index. No report if the check itself failed, which
    /// is logged, and no index if its files could not be read either.
    fn check_workspace(&mut self, root: &Path, check: &Check) -> (Option<Report>, Option<Kept>) {
        let fresh = check.reread || !self.disk.contains_key(root);
        if fresh {
            self.disk.insert(root.to_path_buf(), read(root));
        } else if self.stat {
            self.stat_pass(root);
        }
        let disk = match self.disk.get(root) {
            Some(Ok(disk)) => disk,
            Some(Err(error)) => return (Some(read_failure(root, error, check, fresh)), None),
            None => return (None, None),
        };
        let (files, open) = merged(disk, &check.documents);
        let uri_at = |index: usize| match open.get(index).copied().flatten() {
            Some(document) => check.documents.get(document).map(|doc| doc.uri.clone()),
            None => files
                .get(index)
                .and_then(|file| uri_of(&root.join(file.name()))),
        };
        let version_at = |index: usize| {
            let document = open.get(index).copied().flatten()?;
            check.documents.get(document).map(|doc| doc.version)
        };
        let root_name = root.to_string_lossy();
        let Some(found) = self.checked(&root_name, || {
            workspace::index_with_standard(&files, &disk.nested, &root_name)
        }) else {
            return (None, None);
        };
        let report = self.report(&found, &files, None, (&uri_at, version_at), check);
        let uris = (0..files.len()).map(uri_at).collect();
        let again = Again::Workspace {
            nested: disk.nested.clone(),
            root: root_name.into_owned(),
        };
        let kept = Kept::of(found, (files, uris), (again, None), check);
        (Some(report), Some(kept))
    }

    /// The stat pass of the workspace under `root`, whose files were read
    /// before (see the module documentation): reads again each file whose
    /// modification time or size changed since, or the whole workspace if
    /// one is gone. Whether anything changed. A file new since is not
    /// found: that takes a walk of the whole tree, as a read of the
    /// workspace makes, which is not cheap (see the crate's
    /// documentation).
    fn stat_pass(&mut self, root: &Path) -> bool {
        let Some(Ok(disk)) = self.disk.get_mut(root) else {
            return false;
        };
        let now = stamps(root, &disk.files, None);
        if now == disk.stamps {
            return false;
        }
        for (at, (stamp, before)) in now.iter().zip(&disk.stamps).enumerate() {
            if stamp == before {
                continue;
            }
            let source = disk.files.get(at).and_then(|file| {
                stamp.as_ref()?;
                let text = fs::read_to_string(root.join(file.name())).ok()?;
                SourceFile::new(file.name(), text).ok()
            });
            let (Some(source), Some(file)) = (source, disk.files.get_mut(at)) else {
                // Gone, or not readable: as a read of the workspace finds.
                self.disk.insert(root.to_path_buf(), read(root));
                return true;
            };
            *file = source;
        }
        disk.stamps = now;
        true
    }

    /// Checks the lone file of the open document whose URI's text is `uri`,
    /// as `gcsl run` checks a file outside any workspace; with its index.
    /// Neither if it is not open any more, or the check itself failed,
    /// which is logged.
    fn check_lone(&self, uri: &str, check: &Check) -> (Option<Report>, Option<Kept>) {
        let Some(document) = check.documents.iter().find(|doc| doc.uri.as_str() == uri) else {
            return (None, None);
        };
        let uri = &document.uri;
        let path = match &document.place {
            Place::Lone { path } => path.as_deref(),
            Place::Workspace { .. } => None,
        };
        let name = path.map_or_else(
            || uri.as_str().to_owned(),
            |path| path.display().to_string(),
        );
        let source = match SourceFile::new(name, document.text.clone()) {
            Ok(source) => source,
            Err(error) => {
                let report = Report {
                    message: Some(format!("cannot load {}: {error}", uri.as_str())),
                    ..Report::default()
                };
                return (Some(report), None);
            }
        };
        let dir = path.map_or_else(workspace::working_directory, file_dir);
        let Some(found) = self.checked(uri.as_str(), || {
            workspace::index_lone_with_standard(&source, &dir)
        }) else {
            return (None, None);
        };
        let files = vec![source];
        let uri_at = |_: usize| Some(uri.clone());
        let version_at = |_: usize| Some(document.version);
        // A diagnostic naming no input is the file's own, as `gcsl run`
        // renders it.
        let own = u32::try_from(found.standard.len()).ok();
        let report = self.report(&found, &files, own, (&uri_at, version_at), check);
        let uris = vec![Some(uri.clone())];
        let kept = Kept::of(found, (files, uris), (Again::Lone(dir), own), check);
        (Some(report), Some(kept))
    }

    /// Runs `check`, a check of what `name` names, counting it, and returns
    /// what it found: the standard files it loaded, the warnings if it
    /// succeeded, else the diagnostics, and its index. `None`, logged, if it
    /// panicked, which a bug in GCSL could make it do; the server goes on.
    fn checked(&self, name: &str, check: impl FnOnce() -> Indexed) -> Option<Indexed> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let checked = panic::catch_unwind(AssertUnwindSafe(check));
        if checked.is_err() {
            log(format_args!("internal error: the check of {name} panicked"));
        }
        checked.ok()
    }

    /// The report of what a check found, `found`, the standard files it
    /// loaded and its diagnostics, of `files`, its files after those, each
    /// with the URI `uri_at` and the version (if open) `version_at` give by
    /// its index; a diagnostic naming no input is of the input `own`, when
    /// there is one.
    fn report(
        &self,
        Indexed {
            standard,
            diagnostics,
            ..
        }: &Indexed,
        files: &[SourceFile],
        own: Option<u32>,
        (uri_at, version_at): (impl Fn(usize) -> Option<Uri>, impl Fn(usize) -> Option<i32>),
        check: &Check,
    ) -> Report {
        let mut mapper = Mapper {
            standard,
            files,
            encoding: self.encoding,
            lines: HashMap::new(),
            uris: HashMap::new(),
            uri_at,
        };
        let mut by_file: BTreeMap<usize, Vec<lsp_types::Diagnostic>> = BTreeMap::new();
        for diagnostic in diagnostics {
            let input = diagnostic.input.or(own);
            match input.and_then(|input| mapper.file(input)) {
                Some(file) => {
                    if let Some(converted) = mapper.convert(diagnostic, file) {
                        by_file.entry(file).or_default().push(converted);
                    }
                }
                None => mapper.log(diagnostic, input),
            }
        }
        let files = by_file
            .into_iter()
            .filter_map(|(file, diagnostics)| {
                let uri = mapper.uri(file)?;
                Some((uri, version_at(file), diagnostics))
            })
            .collect();
        Report {
            files,
            versions: versions(check),
            opened: opened(check),
            message: None,
        }
    }

    /// Publishes `report`, what the check of `key` found: each of its
    /// files' diagnostics, then an empty list for each file `key` published
    /// diagnostics for the last time and has none for now, and for each
    /// document opened since its last check that has none, and its message,
    /// if any. A publish like the last one for its URI is not sent, but for
    /// a document just opened.
    fn publish(&mut self, key: Key, report: Report) {
        let mut before = self.published.remove(&key).unwrap_or_default();
        let mut now = HashMap::new();
        let mut opened: HashMap<String, (Uri, i32)> = report
            .opened
            .into_iter()
            .map(|(uri, version)| (uri.as_str().to_owned(), (uri, version)))
            .collect();
        for (uri, version, diagnostics) in report.files {
            before.remove(uri.as_str());
            let force = opened.remove(uri.as_str()).is_some();
            now.insert(uri.as_str().to_owned(), uri.clone());
            self.send_diagnostics(uri, version, diagnostics, force);
        }
        for (text, uri) in before {
            let version = report.versions.get(&text).copied();
            let force = opened.remove(&text).is_some();
            self.send_diagnostics(uri, version, Vec::new(), force);
        }
        let mut opened: Vec<(Uri, i32)> = opened.into_values().collect();
        opened.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (uri, version) in opened {
            self.send_diagnostics(uri, Some(version), Vec::new(), true);
        }
        if let Some(message) = report.message {
            let params = ShowMessageParams {
                typ: MessageType::ERROR,
                message,
            };
            self.send(Notification::new(ShowMessage::METHOD.to_owned(), params));
        }
        self.published.insert(key, now);
    }

    /// Forgets `key`, giving every file it published diagnostics for an
    /// empty list, and, for a workspace, its files as read.
    fn clear(&mut self, key: &Key) {
        if let Key::Root(root) = key {
            self.disk.remove(root);
        }
        self.indexes.remove(key);
        for uri in self.published.remove(key).unwrap_or_default().into_values() {
            self.send_diagnostics(uri, None, Vec::new(), false);
        }
    }

    /// Publishes `diagnostics` for the file named `uri`, of version
    /// `version` if it is open: unless they and their version are those
    /// last published for it, and the publish is not `forced`.
    fn send_diagnostics(
        &mut self,
        uri: Uri,
        version: Option<i32>,
        diagnostics: Vec<lsp_types::Diagnostic>,
        forced: bool,
    ) {
        let published = (version, diagnostics);
        let text = uri.as_str().to_owned();
        if !forced && self.last.get(&text) == Some(&published) {
            return;
        }
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics: published.1.clone(),
            version,
        };
        self.last.insert(text, published);
        self.send(Notification::new(
            PublishDiagnostics::METHOD.to_owned(),
            params,
        ));
    }

    /// Sends `notification` to the client. A client that is gone is not
    /// told: the main thread finds it gone too, and stops.
    fn send(&self, notification: Notification) {
        let _ = self.client.send(Message::Notification(notification));
    }
}

impl Kept {
    /// What is kept of `found`, a check of `files` (after its standard
    /// files), whose URIs are `uris`, made of `check`'s documents, indexed
    /// again as `again` says; a diagnostic naming no input is of `own`.
    fn of(
        found: Indexed,
        (files, uris): (Vec<SourceFile>, Vec<Option<Uri>>),
        (again, own): (Again, Option<u32>),
        check: &Check,
    ) -> Self {
        let standard = found.standard.len();
        let uris = found.standard.iter().map(|_| None).chain(uris).collect();
        let mut sources = found.standard;
        sources.extend(files);
        let suggested = found
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.suggestion.is_some())
            .collect();
        Self {
            versions: versions(check),
            index: found.index,
            sources,
            uris,
            standard,
            suggested,
            own,
            again,
            closed: ClosedCache::default(),
        }
    }

    /// The code actions of the document `uri`, the input `input`, for the
    /// range of its bytes `asked` (its ends included), positions counted in
    /// `encoding`, as the client's `context` asks for them (see the crate's
    /// documentation, "Queries"): a quick fix for each diagnostic with a
    /// suggestion that touches the range, and, for two suggestions or more
    /// in the document, one that makes them all.
    fn code_actions(
        &self,
        (uri, input): (&Uri, usize),
        asked: Span,
        context: &CodeActionContext,
        encoding: Encoding,
    ) -> Vec<Value> {
        let Some(file) = input.checked_sub(self.standard) else {
            return Vec::new();
        };
        let mut mapper = Mapper {
            standard: self.sources.get(..self.standard).unwrap_or_default(),
            files: self.sources.get(self.standard..).unwrap_or_default(),
            encoding,
            lines: HashMap::new(),
            uris: HashMap::new(),
            uri_at: |at: usize| self.uris.get(self.standard + at).cloned().flatten(),
        };
        let own = u32::try_from(input).ok();
        // Each suggestion of the document, with its edit and its diagnostic
        // as it is published, in source order.
        let mut fixes: Vec<(&Diagnostic, &Suggestion, TextEdit, lsp_types::Diagnostic)> = self
            .suggested
            .iter()
            .filter(|diagnostic| diagnostic.input.or(self.own) == own)
            .filter_map(|diagnostic| {
                let found = diagnostic.suggestion.as_deref()?;
                let published = mapper.convert(diagnostic, file)?;
                let range = mapper.range(file, found.span);
                let edit = TextEdit::new(range, found.replacement.clone());
                Some((diagnostic, found, edit, published))
            })
            .collect();
        fixes.sort_by_key(|&(_, found, _, _)| (found.span.start, found.span.end));
        let mut actions = Vec::new();
        if wants(context, &CodeActionKind::QUICKFIX) {
            for (diagnostic, found, edit, published) in &fixes {
                let span = diagnostic.span;
                if span.start > asked.end || asked.start > span.end {
                    continue;
                }
                // The client's own diagnostic, if it sent this one.
                let sent = context.diagnostics.iter().find(|sent| {
                    sent.range == published.range && sent.message == published.message
                });
                let title = format!("Change to `{}`", found.replacement);
                let attached = [sent.unwrap_or(published)];
                let mut action = code_action(&title, &CodeActionKind::QUICKFIX, &attached);
                action["edit"] = workspace_change(uri, &[edit]);
                action["isPreferred"] = Value::Bool(true);
                actions.push(action);
            }
        }
        if wants(context, &CodeActionKind::SOURCE_FIX_ALL) {
            // Every suggestion, but one overlapping one taken before it.
            let mut edits: Vec<&TextEdit> = Vec::new();
            let mut resolved: Vec<&lsp_types::Diagnostic> = Vec::new();
            let mut end = None;
            for (_, found, edit, published) in &fixes {
                if end.is_some_and(|end| found.span.start < end) {
                    continue;
                }
                end = Some(found.span.end);
                edits.push(edit);
                resolved.push(published);
            }
            if edits.len() >= 2 {
                let title = "Apply all suggestions in this file";
                let kind = CodeActionKind::SOURCE_FIX_ALL;
                let mut action = code_action(title, &kind, &resolved);
                action["edit"] = workspace_change(uri, &edits);
                actions.push(action);
            }
        }
        actions
    }

    /// The answer to `asked` at `position` of the document `uri`, counted
    /// in `encoding`: a hover, a location or locations, highlights, a
    /// rename's range or edits, symbols, completions (each with an
    /// `InsertReplaceEdit` if `insert_replace`, the client supporting them,
    /// else a `TextEdit`), or JSON's `null` for nothing there. A query at a
    /// place of a file whose braces do not match is answered from the
    /// workspace indexed again with them balanced (see [`Answerer`]), each
    /// such indexing counted in `checks`.
    ///
    /// # Errors
    ///
    /// Why a rename, or its preparation, is refused.
    fn answer(
        &mut self,
        asked: Asked,
        uri: &Uri,
        position: Position,
        ((encoding, insert_replace), checks): ((Encoding, bool), &AtomicUsize),
    ) -> Result<Value, String> {
        let input = self
            .uris
            .iter()
            .position(|at| at.as_ref().is_some_and(|at| at.as_str() == uri.as_str()));
        let (Some(input), Some(index)) = (input, self.index.as_mut()) else {
            return Ok(Value::Null);
        };
        let Some(source) = self.sources.get(input) else {
            return Ok(Value::Null);
        };
        let lines = LineIndex::new(source.text());
        let (Ok(file), Ok(offset)) = (
            u32::try_from(input),
            u32::try_from(lines.offset(position, encoding)),
        ) else {
            return Ok(Value::Null);
        };
        let mut places = Places {
            uris: &self.uris,
            sources: &self.sources,
            encoding,
            lines: HashMap::new(),
        };
        let hinted = match &asked {
            Asked::InlayHints(end) => Some(range_to(&lines, (offset, *end), encoding)),
            _ => None,
        };
        let mut answerer = Answerer::of(
            (index, &mut self.closed),
            (&self.again, checks),
            &asked,
            (file, offset, hinted),
        );
        let answer = match asked {
            Asked::Hover => answerer.hover(offset).map(|hover| {
                let hover = Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: hover.text,
                    }),
                    range: Some(lines.range(hover.span, encoding)),
                };
                serde_json::to_value(hover)
            }),
            Asked::Definition => {
                let mut locations: Vec<Location> = answerer
                    .definition(offset)
                    .into_iter()
                    .filter_map(|found| places.location(found.file, found.span))
                    .collect();
                let response = match locations.len() {
                    0 => None,
                    1 => locations.pop().map(GotoDefinitionResponse::Scalar),
                    _ => Some(GotoDefinitionResponse::Array(locations)),
                };
                response.map(serde_json::to_value)
            }
            Asked::References(declarations) => {
                let locations: Vec<Location> = answerer
                    .references(offset, declarations)
                    .into_iter()
                    .filter_map(|found| places.location(found.file, found.span))
                    .collect();
                Some(serde_json::to_value(locations))
            }
            Asked::Highlight | Asked::DocumentSymbols | Asked::InlayHints(_) => {
                let shown = (&lines, encoding);
                Some(of_document(&mut answerer, &asked, offset, shown))
            }
            Asked::CodeActions(end, context) => {
                let asked = range_to(&lines, (offset, end), encoding);
                let actions = self.code_actions((uri, input), asked, &context, encoding);
                Some(Ok(Value::Array(actions)))
            }
            Asked::PrepareRename => {
                let prepared = answerer.prepare_rename(offset)?;
                let range = lines.range(prepared.span, encoding);
                Some(Ok(
                    json!({ "range": range, "placeholder": prepared.placeholder }),
                ))
            }
            Asked::Rename(new) => {
                let Answerer::Index(index, _) = answerer else {
                    return Ok(Value::Null);
                };
                let again = &self.again;
                let edits = index.rename((file, offset), &new, |renamed| again.index(renamed))?;
                Some(Ok(workspace_edit(edits, &mut places)?))
            }
            Asked::Completion(_) | Asked::SignatureHelp => {
                let typed = Typed {
                    lines: &lines,
                    text: source.text(),
                    encoding,
                    insert_replace,
                };
                Some(typing(&answerer, &asked, offset, &typed))
            }
        };
        Ok(answer.and_then(Result::ok).unwrap_or(Value::Null))
    }

    /// The symbols of the files of this index whose names have `query` as a
    /// subsequence, ignoring case, `limit` at most, positions counted in
    /// `encoding`.
    fn workspace_symbols(
        &self,
        query: &str,
        limit: usize,
        encoding: Encoding,
    ) -> Vec<WorkspaceSymbol> {
        let Some(index) = &self.index else {
            return Vec::new();
        };
        let mut places = Places {
            uris: &self.uris,
            sources: &self.sources,
            encoding,
            lines: HashMap::new(),
        };
        index
            .workspace_symbols(query, limit)
            .into_iter()
            .filter_map(|found| {
                let location = places.location(found.file, found.span)?;
                Some(WorkspaceSymbol {
                    name: found.name,
                    kind: symbol_kind(found.kind),
                    tags: None,
                    container_name: found.container,
                    location: OneOf::Left(location),
                    data: None,
                })
            })
            .collect()
    }
}

/// Turns places of an index's files into LSP's locations, each file's lines
/// indexed once, when first needed.
struct Places<'a> {
    /// The URI of each input, by input number.
    uris: &'a [Option<Uri>],
    /// The source of each input, by input number.
    sources: &'a [SourceFile],
    /// The encoding positions are counted in.
    encoding: Encoding,
    /// The lines of each file needed so far, by input number.
    lines: HashMap<u32, LineIndex<'a>>,
}

impl Places<'_> {
    /// The location of `span` of the file of input number `file`: `None`
    /// for a file with no URI, a standard file's.
    fn location(&mut self, file: u32, span: Span) -> Option<Location> {
        let uri = self.uris.get(file as usize)?.as_ref()?;
        let source = self.sources.get(file as usize)?;
        let lines = self
            .lines
            .entry(file)
            .or_insert_with(|| LineIndex::new(source.text()));
        Some(Location::new(uri.clone(), lines.range(span, self.encoding)))
    }
}

/// `edits`, a rename's, as LSP's `WorkspaceEdit`: its `changes`, each
/// file's edits by its URI, located by `places`.
///
/// # Errors
///
/// A file to edit has no URI.
fn workspace_edit(edits: Vec<Edit>, places: &mut Places<'_>) -> Result<Value, String> {
    let mut changes = Map::new();
    for edit in edits {
        let Some(location) = places.location(edit.file, edit.span) else {
            return Err("a file to rename in has no URI".to_owned());
        };
        let text_edit = serde_json::to_value(TextEdit::new(location.range, edit.text));
        let list = changes
            .entry(location.uri.as_str().to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let (Value::Array(list), Ok(text_edit)) = (list, text_edit) {
            list.push(text_edit);
        }
    }
    Ok(json!({ "changes": changes }))
}

/// Whether the client's `context` wants code actions of `kind`: it names no
/// kinds, or names `kind` or a kind it is under (`source` for
/// `source.fixAll`).
fn wants(context: &CodeActionContext, kind: &CodeActionKind) -> bool {
    let kind = kind.as_str();
    context.only.as_ref().is_none_or(|only| {
        only.iter().any(|asked| {
            let asked = asked.as_str();
            asked.is_empty()
                || kind == asked
                || kind
                    .strip_prefix(asked)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    })
}

/// A code action as LSP's `CodeAction`, its edit to be added: its title,
/// its kind and the diagnostics it resolves. (Made as JSON, as a rename's
/// edit is: a map keyed by [`Uri`]s would key by values with a cache
/// inside them.)
fn code_action(
    title: &str,
    kind: &CodeActionKind,
    diagnostics: &[&lsp_types::Diagnostic],
) -> Value {
    json!({ "title": title, "kind": kind.as_str(), "diagnostics": diagnostics })
}

/// A `WorkspaceEdit` making `edits` in the document `uri`, as its
/// `changes`.
fn workspace_change(uri: &Uri, edits: &[&TextEdit]) -> Value {
    let mut changes = Map::new();
    let edits = serde_json::to_value(edits).unwrap_or(Value::Null);
    changes.insert(uri.as_str().to_owned(), edits);
    json!({ "changes": changes })
}

/// `symbol`, of the file whose lines are `lines`, as LSP's
/// `DocumentSymbol`, its children too, positions counted in `encoding`.
/// (Made as JSON: `lsp_types` has a field LSP deprecated in it.)
fn symbol_json(symbol: &DocumentSymbol, lines: &LineIndex<'_>, encoding: Encoding) -> Value {
    let children: Vec<Value> = symbol
        .children
        .iter()
        .map(|child| symbol_json(child, lines, encoding))
        .collect();
    let mut shown = json!({
        "name": symbol.name,
        "kind": symbol_kind(symbol.kind),
        "range": lines.range(symbol.span, encoding),
        "selectionRange": lines.range(symbol.selection, encoding),
    });
    if let Some(detail) = &symbol.detail {
        shown["detail"] = Value::String(detail.clone());
    }
    if !children.is_empty() {
        shown["children"] = Value::Array(children);
    }
    shown
}

/// What a query of a document is answered from: the index, or, for a file
/// whose braces do not match, the workspace indexed again with them
/// balanced, what it finds mapped back to the text as it is (see the
/// library's "Unbalanced files").
enum Answerer<'a> {
    /// The index, and the file's input number.
    Index(&'a mut Index, u32),
    /// The workspace indexed again.
    Closed(&'a mut Closed),
}

impl<'a> Answerer<'a> {
    /// What `asked`, a query at byte `offset` of the file of input number
    /// `file` (of the inlay hints of `hinted`), is answered from: `index`,
    /// but for a query at a place of a file whose braces do not match, a
    /// rename, a document's symbols and code actions apart, the index
    /// `closed` keeps or makes (by indexing again as `again` says, counted
    /// in `checks`), if it balances the file.
    fn of(
        (index, closed): (&'a mut Index, &'a mut ClosedCache),
        (again, checks): (&Again, &AtomicUsize),
        asked: &Asked,
        (file, offset, hinted): (u32, u32, Option<Span>),
    ) -> Self {
        let balanced = !index.unbalanced(file)
            || matches!(
                asked,
                Asked::Rename(_) | Asked::DocumentSymbols | Asked::CodeActions(..)
            );
        if balanced {
            return Answerer::Index(index, file);
        }
        let reindex = |files: &[SourceFile]| reindexed(again, files, checks);
        let found = match hinted {
            Some(range) => closed.over(index, (file, range), reindex),
            None => closed.at(index, (file, offset), reindex),
        };
        match found {
            Some(closed) => Answerer::Closed(closed),
            None => Answerer::Index(index, file),
        }
    }

    /// The hover at byte `offset` of the file.
    fn hover(&mut self, offset: u32) -> Option<gcsl::index::Hover> {
        match self {
            Answerer::Index(index, file) => index.hover(*file, offset),
            Answerer::Closed(closed) => closed.hover(offset),
        }
    }

    /// Where what is at byte `offset` of the file is declared.
    fn definition(&self, offset: u32) -> Vec<gcsl::index::Location> {
        match self {
            Answerer::Index(index, file) => index.definition(*file, offset),
            Answerer::Closed(closed) => closed.definition(offset),
        }
    }

    /// The references to what is at byte `offset` of the file, its
    /// declarations among them if `declarations`.
    fn references(&self, offset: u32, declarations: bool) -> Vec<gcsl::index::Location> {
        match self {
            Answerer::Index(index, file) => index.references(*file, offset, declarations),
            Answerer::Closed(closed) => closed.references(offset, declarations),
        }
    }

    /// The highlights of what is at byte `offset` of the file.
    fn highlights(&self, offset: u32) -> Vec<Highlight> {
        match self {
            Answerer::Index(index, file) => index.highlights(*file, offset),
            Answerer::Closed(closed) => closed.highlights(offset),
        }
    }

    /// The name a rename at byte `offset` of the file renames, or why there
    /// is none.
    fn prepare_rename(&self, offset: u32) -> Result<Prepared, String> {
        match self {
            Answerer::Index(index, file) => index.prepare_rename(*file, offset),
            Answerer::Closed(closed) => closed.prepare_rename(offset),
        }
    }

    /// What may be written at byte `offset` of the file, asked for because
    /// `trigger` was typed, if it was.
    fn completion(&self, offset: u32, trigger: Option<char>) -> Vec<Completion> {
        match self {
            Answerer::Index(index, file) => index.completion_triggered(*file, offset, trigger),
            Answerer::Closed(closed) => closed.completion_triggered(offset, trigger),
        }
    }

    /// The signature of the call byte `offset` of the file is in.
    fn signature(&self, offset: u32) -> Option<Signature> {
        match self {
            Answerer::Index(index, file) => index.signature(*file, offset),
            Answerer::Closed(closed) => closed.signature(offset),
        }
    }

    /// The inlay hints of `range` of the file.
    fn inlay_hints(&mut self, range: Span) -> Vec<gcsl::index::InlayHint> {
        match self {
            Answerer::Index(index, file) => index.inlay_hints(*file, range),
            Answerer::Closed(closed) => closed.inlay_hints(range),
        }
    }

    /// The symbols of the file: none of one indexed again, as they are
    /// never asked of it.
    fn document_symbols(&self) -> Vec<DocumentSymbol> {
        match self {
            Answerer::Index(index, file) => index.document_symbols(*file),
            Answerer::Closed(_) => Vec::new(),
        }
    }
}

/// The span from byte `offset` of the file whose lines are `lines` to
/// `end`, counted in `encoding`: empty if `end` is before `offset`.
fn range_to(lines: &LineIndex<'_>, (offset, end): (u32, Position), encoding: Encoding) -> Span {
    let end = u32::try_from(lines.offset(end, encoding)).unwrap_or(u32::MAX);
    Span::new(offset, end.max(offset))
}

/// `files` indexed again as `again` says, with a file's braces balanced,
/// counted as a check in `checks`: an index of nothing, logged, if that
/// panicked, which a bug in GCSL could make it do.
fn reindexed(again: &Again, files: &[SourceFile], checks: &AtomicUsize) -> Indexed {
    checks.fetch_add(1, Ordering::SeqCst);
    panic::catch_unwind(AssertUnwindSafe(|| again.index(files))).unwrap_or_else(|_| {
        log(format_args!(
            "internal error: indexing again with a file's braces balanced panicked"
        ));
        Indexed {
            standard: Vec::new(),
            diagnostics: Vec::new(),
            succeeded: false,
            index: None,
        }
    })
}

/// The answer of `answerer` to `asked`, highlights, a document's symbols or
/// its inlay hints, of its file, at its byte `offset` (where the range of
/// the hints starts), whose lines are `lines`, positions counted in
/// `encoding`. An inlay hint is of kind `Type`, its label as
/// [`Index::inlay_hints`] makes it, with no padding.
fn of_document(
    answerer: &mut Answerer<'_>,
    asked: &Asked,
    offset: u32,
    (lines, encoding): (&LineIndex<'_>, Encoding),
) -> serde_json::Result<Value> {
    match asked {
        Asked::InlayHints(end) => {
            let end = u32::try_from(lines.offset(*end, encoding)).unwrap_or(u32::MAX);
            let hints: Vec<InlayHint> = answerer
                .inlay_hints(Span::new(offset, end))
                .into_iter()
                .map(|hint| InlayHint {
                    position: lines
                        .range(Span::new(hint.offset, hint.offset), encoding)
                        .start,
                    label: InlayHintLabel::String(hint.label),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(false),
                    padding_right: Some(false),
                    data: None,
                })
                .collect();
            serde_json::to_value(hints)
        }
        Asked::Highlight => {
            let highlights: Vec<DocumentHighlight> = answerer
                .highlights(offset)
                .into_iter()
                .map(|found| DocumentHighlight {
                    range: lines.range(found.span, encoding),
                    kind: Some(if found.write {
                        DocumentHighlightKind::WRITE
                    } else {
                        DocumentHighlightKind::READ
                    }),
                })
                .collect();
            serde_json::to_value(highlights)
        }
        _ => {
            let symbols: Vec<Value> = answerer
                .document_symbols()
                .iter()
                .map(|symbol| symbol_json(symbol, lines, encoding))
                .collect();
            Ok(Value::Array(symbols))
        }
    }
}

/// The document a completion or signature help is asked of, as an answer
/// needs it.
struct Typed<'a> {
    /// Its lines.
    lines: &'a LineIndex<'a>,
    /// Its text.
    text: &'a str,
    /// The encoding positions are counted in.
    encoding: Encoding,
    /// Whether the client takes an `InsertReplaceEdit` for a completion.
    insert_replace: bool,
}

/// The answer of `answerer` to `asked`, a completion or signature help, at
/// byte `offset` of its file, of the document `typed`: a `CompletionList`,
/// empty for nothing to offer, or a `SignatureHelp`, `null` for no call
/// there.
fn typing(
    answerer: &Answerer<'_>,
    asked: &Asked,
    offset: u32,
    typed: &Typed<'_>,
) -> serde_json::Result<Value> {
    if *asked == Asked::SignatureHelp {
        return match answerer.signature(offset) {
            Some(found) => serde_json::to_value(signature_help(&found, typed.encoding)),
            None => Ok(Value::Null),
        };
    }
    // What each item's edit writes over: the name typed before the place,
    // or all of the name the place is in.
    let name = gcsl::index::name_around(typed.text, offset);
    let trigger = match *asked {
        Asked::Completion(trigger) => trigger,
        _ => None,
    };
    let items = answerer
        .completion(offset, trigger)
        .into_iter()
        .map(|found| {
            // A completion that says what it replaces, a module path's
            // segment, replaces that; any other the name typed.
            let around = found.range.unwrap_or(name);
            let insert = typed.lines.range(
                Span::new(around.start, offset.max(around.start)),
                typed.encoding,
            );
            let replace = typed.lines.range(around, typed.encoding);
            let new_text = found.insert.clone().unwrap_or_else(|| found.label.clone());
            let edit = if typed.insert_replace {
                CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                    new_text,
                    insert,
                    replace,
                })
            } else {
                CompletionTextEdit::Edit(TextEdit::new(insert, new_text))
            };
            completion_item(found, edit)
        })
        .collect();
    serde_json::to_value(CompletionList {
        is_incomplete: false,
        items,
    })
}

/// `found`, a completion, as LSP's `CompletionItem`: its label, kind,
/// detail, documentation as Markdown, the text it is ordered by, what is
/// written, if that is not its label, and `edit`, which writes it.
fn completion_item(found: Completion, edit: CompletionTextEdit) -> CompletionItem {
    let kind = match found.kind {
        CompletionKind::Variable => CompletionItemKind::VARIABLE,
        CompletionKind::Field => CompletionItemKind::FIELD,
        CompletionKind::Function => CompletionItemKind::FUNCTION,
        CompletionKind::Module => CompletionItemKind::MODULE,
        CompletionKind::Type => CompletionItemKind::STRUCT,
        CompletionKind::Keyword => CompletionItemKind::KEYWORD,
        CompletionKind::Argument => CompletionItemKind::PROPERTY,
    };
    CompletionItem {
        label: found.label,
        kind: Some(kind),
        detail: found.detail,
        documentation: found.documentation.map(|value| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            })
        }),
        sort_text: Some(found.sort),
        insert_text: found.insert,
        text_edit: Some(edit),
        ..CompletionItem::default()
    }
}

/// `found`, a call's signature, as LSP's `SignatureHelp`, its one
/// signature's parameters as offsets in its label counted in `encoding`.
fn signature_help(found: &Signature, encoding: Encoding) -> SignatureHelp {
    let count = |at: u32| {
        let before = found.label.get(..at as usize).unwrap_or_default();
        let units = match encoding {
            Encoding::Utf8 => before.len(),
            Encoding::Utf16 => before.encode_utf16().count(),
        };
        u32::try_from(units).unwrap_or(u32::MAX)
    };
    let parameters = found
        .params
        .iter()
        .map(|&(start, end)| ParameterInformation {
            label: ParameterLabel::LabelOffsets([count(start), count(end)]),
            documentation: None,
        })
        .collect();
    let documentation = found.documentation.clone().map(|value| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        })
    });
    SignatureHelp {
        signatures: vec![SignatureInformation {
            label: found.label.clone(),
            documentation,
            parameters: Some(parameters),
            active_parameter: None,
        }],
        active_signature: Some(0),
        active_parameter: found.active,
    }
}

/// LSP's kind of a symbol of `kind`.
fn symbol_kind(kind: SymbolKind) -> lsp_types::SymbolKind {
    match kind {
        SymbolKind::Field => lsp_types::SymbolKind::FIELD,
        SymbolKind::Variable => lsp_types::SymbolKind::VARIABLE,
        SymbolKind::Function => lsp_types::SymbolKind::FUNCTION,
        SymbolKind::Type => lsp_types::SymbolKind::STRUCT,
        SymbolKind::Module => lsp_types::SymbolKind::MODULE,
        SymbolKind::Definition => lsp_types::SymbolKind::OBJECT,
    }
}

/// Reads the files of the workspace under `root` from disk.
fn read(root: &Path) -> Result<Disk, ReadError> {
    let started = SystemTime::now();
    let Sources {
        relative, nested, ..
    } = gcsl_disk::read_workspace(root, None)?;
    // A file changed since the read started may have been read before or
    // after: its stamp says so, and the next stat pass reads it again.
    let stamps = stamps(root, &relative, Some(started));
    Ok(Disk {
        files: relative,
        nested,
        stamps,
    })
}

/// What a stat of each of `files`, the files of the workspace under `root`
/// by their paths relative to it, finds: of a file modified at or after
/// `read`, when it was read, the time is not known.
pub(crate) fn stamps(root: &Path, files: &[SourceFile], read: Option<SystemTime>) -> Vec<Stamp> {
    files
        .iter()
        .map(|file| {
            let metadata = fs::metadata(root.join(file.name())).ok()?;
            let modified = metadata
                .modified()
                .ok()
                .filter(|&modified| read.is_none_or(|read| modified < read));
            Some((modified, metadata.len()))
        })
        .collect()
}

/// The sources a workspace is checked from: `disk`'s files, each with the
/// text of the open document of its path in `documents` in place of its
/// own, and each open document that is none of them added in path order;
/// with each source, the index in `documents` of its open document, if any.
fn merged(disk: &Disk, documents: &[Snapshot]) -> (Vec<SourceFile>, Vec<Option<usize>>) {
    let mut files = disk.files.clone();
    let mut open = vec![None; files.len()];
    for (index, document) in documents.iter().enumerate() {
        let Place::Workspace { relative, .. } = &document.place else {
            continue;
        };
        let source = match SourceFile::new(relative.clone(), document.text.clone()) {
            Ok(source) => source,
            Err(error) => {
                log(format_args!(
                    "cannot load {}: {error}",
                    document.uri.as_str()
                ));
                continue;
            }
        };
        match files.binary_search_by(|file| file.name().cmp(relative)) {
            Ok(at) => {
                if let (Some(file), Some(slot)) = (files.get_mut(at), open.get_mut(at)) {
                    *file = source;
                    *slot = Some(index);
                }
            }
            Err(at) => {
                files.insert(at, source);
                open.insert(at, Some(index));
            }
        }
    }
    (files, open)
}

/// The version of each open document of `check`, by the text of its URI.
fn versions(check: &Check) -> HashMap<String, i32> {
    check
        .documents
        .iter()
        .map(|document| (document.uri.as_str().to_owned(), document.version))
        .collect()
}

/// The open documents of `check` opened since a check was last handed
/// over with them, with their versions.
fn opened(check: &Check) -> Vec<(Uri, i32)> {
    check
        .documents
        .iter()
        .filter(|document| document.opened)
        .map(|document| (document.uri.clone(), document.version))
        .collect()
}

/// The report of a workspace under `root` whose files could not be read,
/// for `error`: one error at the start of the file it names, if that is a
/// file of the workspace, else a message to show the user, only when the
/// files were read for this check, `fresh`, so that it is shown once.
fn read_failure(root: &Path, error: &ReadError, check: &Check, fresh: bool) -> Report {
    let message = error.to_string();
    // What the error names, as it is shown: relative to the working
    // directory when under it.
    let shown = match error {
        ReadError::Unreadable { path, .. } => path.clone(),
        ReadError::Unloadable { name, .. } => PathBuf::from(name),
    };
    let path = if shown.is_relative() {
        std::env::current_dir()
            .and_then(fs::canonicalize)
            .map_or(shown.clone(), |cwd| cwd.join(&shown))
    } else {
        shown
    };
    let file = gcsl_disk::find_workspace(&path).filter(|(found, _)| found == root);
    let Some((_, relative)) = file else {
        return Report {
            versions: versions(check),
            opened: opened(check),
            message: fresh.then_some(message),
            ..Report::default()
        };
    };
    let open = check.documents.iter().find(|document| {
        matches!(&document.place, Place::Workspace { relative: at, .. } if *at == relative)
    });
    let uri = open
        .map(|document| document.uri.clone())
        .or_else(|| uri_of(&path));
    let diagnostic = lsp_types::Diagnostic {
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("gcsl".to_owned()),
        message,
        ..lsp_types::Diagnostic::default()
    };
    Report {
        files: uri
            .map(|uri| (uri, open.map(|document| document.version), vec![diagnostic]))
            .into_iter()
            .collect(),
        versions: versions(check),
        opened: opened(check),
        message: None,
    }
}

/// The directory of the lone file at `path`, as `gcsl run` gives the
/// library a lone file's: its canonical path's parent, lossily if it is not
/// UTF-8, or the working directory when that cannot be found, as for a file
/// not on disk yet.
fn file_dir(path: &Path) -> String {
    fs::canonicalize(path)
        .ok()
        .and_then(|file| file.parent().map(|dir| dir.to_string_lossy().into_owned()))
        .unwrap_or_else(workspace::working_directory)
}

/// Turns a check's diagnostics into LSP's, each file's lines indexed and
/// its URI found once, when first needed.
struct Mapper<'a, U> {
    /// The standard files the check loaded, its first inputs.
    standard: &'a [SourceFile],
    /// The files checked, its inputs after them.
    files: &'a [SourceFile],
    /// The encoding positions are counted in.
    encoding: Encoding,
    /// The lines of each file needed so far, by index.
    lines: HashMap<usize, LineIndex<'a>>,
    /// The URI of each file needed so far, by index.
    uris: HashMap<usize, Option<Uri>>,
    /// The URI of a file, by its index.
    uri_at: U,
}

impl<U: Fn(usize) -> Option<Uri>> Mapper<'_, U> {
    /// The index among the files of the input `input`; `None` for a
    /// standard file, or a number that is none.
    fn file(&self, input: u32) -> Option<usize> {
        let file = (input as usize).checked_sub(self.standard.len())?;
        (file < self.files.len()).then_some(file)
    }

    /// The URI of the file of index `file`.
    fn uri(&mut self, file: usize) -> Option<Uri> {
        let uri_at = &self.uri_at;
        self.uris
            .entry(file)
            .or_insert_with(|| uri_at(file))
            .clone()
    }

    /// The range of `span` in the file of index `file`.
    fn range(&mut self, file: usize, span: Span) -> lsp_types::Range {
        let files = self.files;
        let lines = self
            .lines
            .entry(file)
            .or_insert_with(|| LineIndex::new(files.get(file).map_or("", SourceFile::text)));
        lines.range(span, self.encoding)
    }

    /// `diagnostic`, of the file of index `file`, as LSP's: its message, a
    /// line `note: ...` for each note and a line `help: did you mean `x`?`
    /// for its suggestion, if it has one; its primary span's label, if it
    /// has text, and each of its labels in a file, as related information.
    /// `None`, logged, if the file has no URI (its path is not UTF-8).
    fn convert(&mut self, diagnostic: &Diagnostic, file: usize) -> Option<lsp_types::Diagnostic> {
        let Some(uri) = self.uri(file) else {
            let name = self.files.get(file).map_or("?", SourceFile::name);
            log(format_args!(
                "{name}: {}: {} (the file has no URI)",
                diagnostic.severity, diagnostic.message
            ));
            return None;
        };
        let range = self.range(file, diagnostic.span);
        let mut message = diagnostic.message.clone();
        for note in &diagnostic.notes {
            message.push_str("\nnote: ");
            message.push_str(note);
        }
        if let Some(found) = &diagnostic.suggestion {
            message.push_str("\nhelp: ");
            message.push_str(&found.help());
        }
        let mut related = Vec::new();
        if let Some(text) = diagnostic.label.as_ref().filter(|text| !text.is_empty()) {
            related.push(DiagnosticRelatedInformation {
                location: Location::new(uri, range),
                message: text.to_string(),
            });
        }
        let own = u32::try_from(self.standard.len() + file).unwrap_or(u32::MAX);
        for label in &diagnostic.labels {
            let input = label.input.unwrap_or(own);
            // A label in a standard file has no file to point at.
            let Some(at) = self.file(input).filter(|_| !label.text.is_empty()) else {
                continue;
            };
            let Some(uri) = self.uri(at) else {
                continue;
            };
            related.push(DiagnosticRelatedInformation {
                location: Location::new(uri, self.range(at, label.span)),
                message: label.text.clone(),
            });
        }
        Some(lsp_types::Diagnostic {
            range,
            severity: Some(match diagnostic.severity {
                Severity::Error => DiagnosticSeverity::ERROR,
                Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            source: Some("gcsl".to_owned()),
            message,
            related_information: (!related.is_empty()).then_some(related),
            ..lsp_types::Diagnostic::default()
        })
    }

    /// Logs `diagnostic`, of the input `input`, which is no file's: a
    /// standard file's, rendered as `gcsl run` renders it, or one naming no
    /// input, which no diagnostic of a workspace should.
    fn log(&self, diagnostic: &Diagnostic, input: Option<u32>) {
        let source = |input: u32| {
            let input = input as usize;
            self.standard
                .get(input)
                .or_else(|| self.files.get(input.checked_sub(self.standard.len())?))
        };
        match input.and_then(source) {
            Some(fallback) => {
                let source_of = |input: Option<u32>| input.and_then(source).unwrap_or(fallback);
                log(Renderer::plain()
                    .render_in(diagnostic, source_of)
                    .trim_end());
            }
            None => log(format_args!(
                "{}: {} (in no file)",
                diagnostic.severity, diagnostic.message
            )),
        }
    }
}
