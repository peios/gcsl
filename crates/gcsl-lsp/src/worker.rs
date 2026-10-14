//! The worker: the one thread of a server that reads workspaces from disk,
//! checks them and publishes their diagnostics.
//!
//! The main thread hands it a [`Job`] at a time ([`spawn`]): a [`Check`],
//! the snapshot of the open documents of one workspace, or of one lone
//! file, as they are when it is handed over, or a [`Job::Clear`] of a
//! workspace or lone file there is no more to say of. It answers each
//! check with the check's [`Key`] once the diagnostics are published, and
//! the main thread hands it the next one only then, so that checks never
//! overlap and never queue up behind each other.
//!
//! It keeps each workspace's files as read from disk
//! ([`gcsl_disk::read_workspace`]), read once, and again only when a check
//! asks for it ([`Check::reread`]); a check's sources are those files with
//! the open documents' texts in place of theirs, and the open documents
//! that are none of them added in path order. The workspace is checked as
//! `gcsl run` and `pekit` check one,
//! [`gcsl::workspace::check_with_standard`] with its canonical root and the
//! directories left out as workspaces of their own; a lone file as `gcsl
//! run` checks one, [`gcsl::workspace::check_lone_with_standard`] with its
//! canonical directory. Nothing is run, and nothing is written to disk.
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
use gcsl::workspace;
use gcsl::{Diagnostic, Severity, SourceFile, Span};
use gcsl_disk::{ReadError, Sources};
use lsp_server::{Message, Notification};
use lsp_types::notification::{Notification as _, PublishDiagnostics, ShowMessage};
use lsp_types::{
    DiagnosticRelatedInformation, DiagnosticSeverity, Location, MessageType,
    PublishDiagnosticsParams, ShowMessageParams, Uri,
};

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

/// What the worker is asked to do.
#[derive(Debug)]
pub enum Job {
    /// Check, and publish what the check finds.
    Check(Check),
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
    let worker = Worker {
        client,
        encoding,
        checks,
        disk: HashMap::new(),
        published: HashMap::new(),
    };
    let handle = thread::Builder::new()
        .name("gcsl-lsp-worker".to_owned())
        .stack_size(gcsl::STACK_SIZE)
        .spawn(move || worker.run(&received, &done))?;
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
}

impl Worker {
    /// Does each job received, until there are none, or the main thread is
    /// gone.
    fn run(mut self, jobs: &Receiver<Job>, done: &Sender<Key>) {
        for job in jobs {
            match job {
                Job::Check(check) => {
                    let key = check.key.clone();
                    self.check(check);
                    if done.send(key).is_err() {
                        return;
                    }
                }
                Job::Clear(key) => self.clear(&key),
            }
        }
    }

    /// Makes `check` and publishes what it finds.
    fn check(&mut self, check: Check) {
        let report = match &check.key {
            Key::Root(root) => self.check_workspace(root, &check),
            Key::Lone(uri) => self.check_lone(uri, &check),
        };
        if let Some(report) = report {
            self.publish(check.key, report);
        }
    }

    /// Checks the workspace under `root`, its files read from disk if
    /// `check` asks for it or they never were, with the open documents in
    /// `check`. `None` if the check itself failed, which is logged.
    fn check_workspace(&mut self, root: &Path, check: &Check) -> Option<Report> {
        let fresh = check.reread || !self.disk.contains_key(root);
        if fresh {
            self.disk.insert(root.to_path_buf(), read(root));
        }
        let disk = match self.disk.get(root)? {
            Ok(disk) => disk,
            Err(error) => return Some(read_failure(root, error, check, fresh)),
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
        let found = self.checked(&root_name, || {
            workspace::check_with_standard(&files, &disk.nested, &root_name)
        })?;
        Some(self.report(&found, &files, None, (uri_at, version_at), check))
    }

    /// Checks the lone file of the open document whose URI's text is `uri`,
    /// as `gcsl run` checks a file outside any workspace. `None` if it is
    /// not open any more, or the check itself failed, which is logged.
    fn check_lone(&self, uri: &str, check: &Check) -> Option<Report> {
        let document = check.documents.iter().find(|doc| doc.uri.as_str() == uri)?;
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
                return Some(Report {
                    message: Some(format!("cannot load {}: {error}", uri.as_str())),
                    ..Report::default()
                });
            }
        };
        let dir = path.map_or_else(workspace::working_directory, file_dir);
        let found = self.checked(uri.as_str(), || {
            workspace::check_lone_with_standard(&source, &dir)
        })?;
        let files = [source];
        let uri_at = |_: usize| Some(uri.clone());
        let version_at = |_: usize| Some(document.version);
        // A diagnostic naming no input is the file's own, as `gcsl run`
        // renders it.
        let own = u32::try_from(found.0.len()).ok();
        Some(self.report(&found, &files, own, (uri_at, version_at), check))
    }

    /// Runs `check`, a check of what `name` names, counting it, and returns
    /// the standard files it loaded and what it found: the warnings if it
    /// succeeded, else the diagnostics. `None`, logged, if it panicked,
    /// which a bug in GCSL could make it do; the server goes on.
    fn checked(
        &self,
        name: &str,
        check: impl FnOnce() -> (
            Vec<SourceFile>,
            Result<workspace::Workspace, Vec<Diagnostic>>,
        ),
    ) -> Option<(Vec<SourceFile>, Vec<Diagnostic>)> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let checked = panic::catch_unwind(AssertUnwindSafe(|| {
            let (standard, checked) = check();
            let diagnostics = match checked {
                Ok(workspace) => workspace.warnings().to_vec(),
                Err(diagnostics) => diagnostics,
            };
            (standard, diagnostics)
        }));
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
        (standard, diagnostics): &(Vec<SourceFile>, Vec<Diagnostic>),
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
