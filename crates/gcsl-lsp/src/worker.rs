//! The worker: the one thread of a server that reads workspaces from disk,
//! checks and indexes them, publishes their diagnostics and answers hovers
//! and definitions.
//!
//! The main thread hands it a [`Job`] at a time ([`spawn`]): a [`Check`],
//! the snapshot of the open documents of one workspace, or of one lone
//! file, as they are when it is handed over; a [`Query`], a hover or a
//! definition asked at a place of an open document, with such a snapshot;
//! or a [`Job::Clear`] of a workspace or lone file there is no more to say
//! of. It answers each check with the check's [`Key`] once the diagnostics
//! are published, and each query with its key once its response is sent,
//! and the main thread hands it the next job only then, so that checks
//! never overlap and never queue up behind each other.
//!
//! It keeps each workspace's files as read from disk
//! ([`gcsl_disk::read_workspace`]), read once, and again only when a check
//! asks for it ([`Check::reread`]); a check's sources are those files with
//! the open documents' texts in place of theirs, and the open documents
//! that are none of them added in path order. The workspace is checked as
//! `gcsl run` and `pekit` check one, with the same diagnostics, but by
//! [`gcsl::workspace::index_with_standard`], which indexes what it finds,
//! with its canonical root and the directories left out as workspaces of
//! their own; a lone file as `gcsl run` checks one, by
//! [`gcsl::workspace::index_lone_with_standard`] with its canonical
//! directory. Nothing is run, and nothing is written to disk.
//!
//! It keeps the latest index of each workspace and lone file, with the
//! version of each open document it was made from, whether its check
//! succeeded or failed (see [`gcsl::index`]): a file with errors is
//! indexed for everything that resolved; one whose braces do not match, or
//! that needs a file whose check failed, is not checked, and has nothing
//! indexed, not even what the last good index had. A query is answered from
//! it only if every open document of what it is of has the version it was
//! made from, and the query's check does not read the files from disk
//! again: otherwise its positions could point at the wrong text, so the
//! query's check is made first, at once, its diagnostics published as any
//! check's are, and the query answered from the new index.
//!
//! Every file the check reports on, open or not, gets a
//! `textDocument/publishDiagnostics` with its diagnostics, by its URI (an
//! open document's as the client gave it), with an open document's version
//! as it was checked; every file that had some the last time and has none
//! now gets an empty list. A diagnostic of a standard file is logged to
//! stderr, as there is no file to attach it to.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender};
use gcsl::diagnostic::Renderer;
use gcsl::index::{Index, Indexed};
use gcsl::workspace;
use gcsl::{Diagnostic, Severity, SourceFile, Span};
use gcsl_disk::{ReadError, Sources};
use lsp_server::{Message, Notification, RequestId, Response};
use lsp_types::notification::{Notification as _, PublishDiagnostics, ShowMessage};
use lsp_types::{
    DiagnosticRelatedInformation, DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents,
    Location, MarkupContent, MarkupKind, MessageType, Position, PublishDiagnosticsParams,
    ShowMessageParams, Uri,
};
use serde_json::Value;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// `textDocument/hover`.
    Hover,
    /// `textDocument/definition`.
    Definition,
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
    /// Forget a workspace or lone file, giving every file it has published
    /// diagnostics for an empty list.
    Clear(Key),
}

/// Starts the worker, which publishes through `client` and answers each
/// check with its key on `done`, positions counted in `encoding`, and
/// counts each check it makes in `checks`. Returns where to send it jobs:
/// it stops once that is dropped, or `done` is.
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
    encoding: Encoding,
    checks: Arc<AtomicUsize>,
    done: Sender<Key>,
) -> io::Result<(Sender<Job>, JoinHandle<()>)> {
    let (jobs, received) = crossbeam_channel::unbounded();
    // Made on its thread: an index is not `Send`, and never leaves it.
    let run = move || {
        let worker = Worker {
            client,
            encoding,
            checks,
            disk: HashMap::new(),
            published: HashMap::new(),
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
}

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
    /// How many checks have been made.
    checks: Arc<AtomicUsize>,
    /// Each workspace's files as last read, by root, or why they could not
    /// be.
    disk: HashMap<PathBuf, Result<Disk, ReadError>>,
    /// The files each workspace or lone file last published diagnostics
    /// for, each URI by its text.
    published: HashMap<Key, HashMap<String, Uri>>,
    /// The latest index of each workspace and lone file.
    indexes: HashMap<Key, Kept>,
}

/// The latest index of a workspace or lone file, with what it was made of.
#[derive(Debug)]
struct Kept {
    /// The version of each open document it was made from, by the text of
    /// its URI.
    versions: HashMap<String, i32>,
    /// The index; `None` if the check's scan failed.
    index: Option<Index>,
    /// Each input of the check, by input number: the standard files', then
    /// the files checked.
    sources: Vec<SourceFile>,
    /// The URI of each input, by input number; `None` for a standard file,
    /// and a file whose path is not UTF-8.
    uris: Vec<Option<Uri>>,
}

impl Worker {
    /// Does each job received, until there are none, or the main thread is
    /// gone.
    fn run(mut self, jobs: &Receiver<Job>, done: &Sender<Key>) {
        for job in jobs {
            let key = match job {
                Job::Check(check) => {
                    let key = check.key.clone();
                    self.check(check);
                    key
                }
                Job::Query(query) => {
                    let key = query.check.key.clone();
                    self.query(query);
                    key
                }
                Job::Clear(key) => {
                    self.clear(&key);
                    continue;
                }
            };
            if done.send(key).is_err() {
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
        let fresh = !check.reread
            && self
                .indexes
                .get(&key)
                .is_some_and(|kept| kept.versions == versions(&check));
        if !fresh {
            self.check(check);
        }
        let answer = self
            .indexes
            .get_mut(&key)
            .and_then(|kept| kept.answer(asked, &uri, position, self.encoding))
            .unwrap_or(Value::Null);
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
        let kept = Kept::of(found, files, uris, check);
        (Some(report), Some(kept))
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
        let kept = Kept::of(found, files, vec![Some(uri.clone())], check);
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
            message: None,
        }
    }

    /// Publishes `report`, what the check of `key` found: each of its
    /// files' diagnostics, then an empty list for each file `key` published
    /// diagnostics for the last time and has none for now, and its message,
    /// if any.
    fn publish(&mut self, key: Key, report: Report) {
        let mut before = self.published.remove(&key).unwrap_or_default();
        let mut now = HashMap::new();
        for (uri, version, diagnostics) in report.files {
            before.remove(uri.as_str());
            now.insert(uri.as_str().to_owned(), uri.clone());
            self.send_diagnostics(uri, version, diagnostics);
        }
        for (text, uri) in before {
            let version = report.versions.get(&text).copied();
            self.send_diagnostics(uri, version, Vec::new());
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
            self.send_diagnostics(uri, None, Vec::new());
        }
    }

    /// Publishes `diagnostics` for the file named `uri`, of version
    /// `version` if it is open.
    fn send_diagnostics(
        &self,
        uri: Uri,
        version: Option<i32>,
        diagnostics: Vec<lsp_types::Diagnostic>,
    ) {
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        };
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
    /// files), whose URIs are `uris`, made of `check`'s documents.
    fn of(found: Indexed, files: Vec<SourceFile>, uris: Vec<Option<Uri>>, check: &Check) -> Self {
        let uris = found.standard.iter().map(|_| None).chain(uris).collect();
        let mut sources = found.standard;
        sources.extend(files);
        Self {
            versions: versions(check),
            index: found.index,
            sources,
            uris,
        }
    }

    /// The answer to `asked` at `position` of the document `uri`, counted
    /// in `encoding`: a hover, a location or locations, or `None` for
    /// nothing there (JSON's `null`).
    fn answer(
        &mut self,
        asked: Asked,
        uri: &Uri,
        position: Position,
        encoding: Encoding,
    ) -> Option<Value> {
        let input = self
            .uris
            .iter()
            .position(|at| at.as_ref().is_some_and(|at| at.as_str() == uri.as_str()))?;
        let file = u32::try_from(input).ok()?;
        let lines = LineIndex::new(self.sources.get(input)?.text());
        let offset = u32::try_from(lines.offset(position, encoding)).ok()?;
        let index = self.index.as_mut()?;
        match asked {
            Asked::Hover => {
                let hover = index.hover(file, offset)?;
                let hover = Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: hover.text,
                    }),
                    range: Some(lines.range(hover.span, encoding)),
                };
                serde_json::to_value(hover).ok()
            }
            Asked::Definition => {
                let mut indices: HashMap<u32, LineIndex<'_>> = HashMap::new();
                let mut locations = Vec::new();
                for found in index.definition(file, offset) {
                    let Some(Some(uri)) = self.uris.get(found.file as usize) else {
                        continue;
                    };
                    let Some(source) = self.sources.get(found.file as usize) else {
                        continue;
                    };
                    let lines = indices
                        .entry(found.file)
                        .or_insert_with(|| LineIndex::new(source.text()));
                    locations.push(Location::new(
                        uri.clone(),
                        lines.range(found.span, encoding),
                    ));
                }
                let response = match locations.len() {
                    0 => return None,
                    1 => GotoDefinitionResponse::Scalar(locations.pop()?),
                    _ => GotoDefinitionResponse::Array(locations),
                };
                serde_json::to_value(response).ok()
            }
        }
    }
}

/// Reads the files of the workspace under `root` from disk.
fn read(root: &Path) -> Result<Disk, ReadError> {
    let Sources {
        relative, nested, ..
    } = gcsl_disk::read_workspace(root, None)?;
    Ok(Disk {
        files: relative,
        nested,
    })
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

    /// `diagnostic`, of the file of index `file`, as LSP's: its message and
    /// a line `note: ...` for each note; its primary span's label, if it
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
        let mut related = Vec::new();
        if let Some(text) = diagnostic.label.as_ref().filter(|text| !text.is_empty()) {
            related.push(DiagnosticRelatedInformation {
                location: Location::new(uri, range),
                message: text.clone(),
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
