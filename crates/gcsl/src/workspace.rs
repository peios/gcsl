//! A workspace: the files of a directory tree, each directory's files
//! sharing one scope.
//!
//! A *workspace* is the directory tree under a directory that holds a file
//! named `workspace.gcsl`; finding it and reading its files is the CLI's,
//! and this module performs no I/O: it is given the files as
//! [`SourceFile`]s named by their paths relative to the root, `/`
//! separated, and answers with diagnostics or values.
//!
//! # Directory scope
//!
//! The *declarations* of a file are its top-level fields and `def`s, the
//! items slices 23 and 24 made order-free ([`crate::mentions`]). Each
//! directory has a tree of them, the union of its files' field trees
//! ([`Node`]): a leaf is owned by the file that declares it, and an
//! interior node belongs to every file with a leaf under it (its
//! *contributors*), so dotted paths merge across files. A file sees its own
//! private names (`let`, `let mut`, `fn`), the tree of its own directory,
//! and the trees of the directories above it, up to the root; never a
//! sibling's or a subdirectory's. A reference means its longest declared
//! prefix in the merged tree of its directory chain, never in the file's
//! own tree alone: [`crate::mentions::Mentions::resolve`] is given a
//! [`FileScope`], whose numbering keeps the file's own nodes and functions
//! as [`crate::mentions::find`] numbers them and puts every other file's
//! after them.
//!
//! One leaf path declared in two files, a leaf under a leaf or over an
//! interior node of another file, and a root name declared both in a
//! directory and in a directory above are errors, found while the trees
//! are merged ([`scan`]).
//!
//! # Order
//!
//! File `A` *needs* file `B` when an item of `A` (an ordered item, a
//! leaf's initialiser, a function's body) mentions a declaration `B` owns:
//! a leaf or a `def` of `B`, or an interior node one of whose leaves is
//! `B`'s. Files are checked and run in an order where each comes after the
//! files it needs, path order breaking ties ([`Scan::order`]); files that
//! need each other are an error, one per cycle. A file runs as a whole
//! before a file that needs it.
//!
//! # Cells and slots
//!
//! Every node of every directory tree is one cell of the workspace's one
//! frame, and its slot is its index in [`Scan::nodes`]: slots `0..` the
//! number of nodes are the workspace's cells, and every other slot (a
//! file's private names, its temporaries) is allocated from there up by
//! the checker, as it allocates from the first free slot of a session's
//! scope. A file's own field node shares the slot of the merged node of
//! its path ([`File::global`]). An interior node with leaves in several
//! files is computed by the last of them in the order, as a node of that
//! file's schedule ([`File::schedule`]); every other contributor treats it
//! as computed elsewhere. Its point is the earlier of "after its last child
//! in that file" and the first mention of it there, so a whole read of it
//! never finds its slot unwritten. A file's own value, when it has no
//! result, is the record of its own cells only, with a nested record for
//! each prefix it shares with other files, never the shared node: so
//! `app.gcsl`'s value has `com.x.app` and not `com.x.lib`.
//!
//! # A workspace as a session
//!
//! [`check`] scans the workspace and then checks and lowers every file in
//! the order, as the inputs of a [`crate::Session`]: one table of names, of
//! types and of slots, each file's diagnostics named by its input number,
//! its index in path order ([`Diagnostic::input`]). After a file is
//! checked, its declarations are committed to its directory's scope for
//! the files after it; its private names (`let`, `let mut`, `fn`) are not,
//! though their slots stay taken. The scope is committed after the check,
//! not after a run, since every file is checked before any runs: any error
//! anywhere stops everything. A file that needs a file whose check failed
//! is not checked, since its errors would follow from the first.
//!
//! [`Workspace::run`] then runs the file asked for, after the files it
//! needs, transitively ([`Scan::needed`]), each as a whole and in the
//! order, on one frame and with one memo table, as session inputs share
//! theirs: a closure or a future of one file used in another reads its
//! file's cells, and a future awaited from several files runs once. Files
//! nothing needs are checked but never run. The effects of a file all
//! happen before those of a file that needs it. A runtime error names the
//! file whose code failed.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::mem;

use crate::Outcome;
use crate::ast::{Ast, ItemKind, PathSegment};
use crate::check::{Checker, Scope, Slot, Types};
use crate::diagnostic::Diagnostic;
use crate::eval::MemoTables;
use crate::ir::Ir;
use crate::lexer::{self, TokenKind};
use crate::mentions::{self, Mentions, Need, Resolve, Schedule};
use crate::names::{Name, Names};
use crate::parser;
use crate::session;
use crate::source::{SourceFile, Span};
use crate::types::{TyId, TypeTable};
use crate::value::Value;

/// The key of a root in [`Scan`]'s index of nodes by parent and name.
const ROOT: u32 = u32::MAX;

/// One file of a workspace, scanned.
#[derive(Debug)]
pub struct File {
    /// Its path relative to the root.
    pub path: String,
    /// Its directory, an index in [`Scan::dirs`].
    pub dir: u32,
    /// Its tree. Its names table is lent out ([`Ast::replace_names`]); the
    /// workspace's is [`Scan::names`].
    pub ast: Ast,
    /// Every name it spells, in [`Name`] order: its identifiers, and each
    /// plain string literal whose text is a name (a quoted field name).
    /// Its checker looks up no other name, so the scope it is checked from
    /// holds only the committed declarations of these (see [`check`]).
    pub spelled: Vec<Name>,
    /// Its declarations and what each of its items mentions, resolved
    /// against its directory chain ([`FileScope`]).
    pub mentions: Mentions,
    /// For each node of its own field tree, the merged node of its path.
    pub global: Vec<u32>,
    /// For each of its own top-level functions, its index in
    /// [`Scan::functions`].
    pub functions: Vec<u32>,
    /// When its field nodes are computed: its own nodes that another file
    /// computes (shared interior nodes it is not the last contributor of)
    /// are not in it.
    pub schedule: Schedule,
}

/// A directory of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    /// Its path relative to the root, empty for the root.
    pub path: String,
    /// The directory above it; `None` for the root.
    pub parent: Option<u32>,
}

/// A node of a directory's merged tree: a cell of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Its last segment.
    pub name: Name,
    /// The node it is a child of; `None` for a root.
    pub parent: Option<u32>,
    /// Its children, in order of their first file, then of their first
    /// item in it.
    pub children: Vec<u32>,
    /// The directory whose tree it is in.
    pub dir: u32,
    /// The file it is a leaf of; `None` for an interior node.
    pub leaf: Option<u32>,
    /// The files with a leaf at or under it, in path order.
    pub contributors: Vec<u32>,
}

/// A top-level function of a file of the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function {
    /// Its file.
    pub file: u32,
    /// Its index among the file's own functions.
    pub index: u32,
}

/// The scanned workspace: what [`scan`] finds before anything is checked.
#[derive(Debug)]
pub struct Scan {
    /// The files, in path order; a file's index is its input number.
    pub files: Vec<File>,
    /// The directories, each after the directory above it.
    pub dirs: Vec<Dir>,
    /// The nodes of every directory's merged tree; a node's index is its
    /// slot.
    pub nodes: Vec<Node>,
    /// The top-level functions of every file.
    pub functions: Vec<Function>,
    /// For each file, the files it needs, each once, in index order.
    pub needs: Vec<Vec<u32>>,
    /// The files in the order they are checked and run.
    pub order: Vec<u32>,
    /// The names of every file.
    pub names: Names,
    /// The nodes by directory, parent ([`ROOT`] for a root) and name.
    by_name: HashMap<(u32, u32, Name), u32>,
    /// The `def`s by directory and name, as indices in `functions`.
    defs: HashMap<(u32, Name), u32>,
    /// The position of each file in `order`.
    rank: Vec<usize>,
}

impl Scan {
    /// The directories from `dir` up to the root, `dir` first.
    pub fn chain(&self, dir: u32) -> impl Iterator<Item = u32> + '_ {
        std::iter::successors(Some(dir), |&at| {
            self.dirs.get(at as usize).and_then(|d| d.parent)
        })
    }

    /// The files `file` needs, transitively, with `file`, in the order:
    /// the files to run to run `file`.
    #[must_use]
    pub fn needed(&self, file: u32) -> Vec<u32> {
        let mut wanted = vec![false; self.files.len()];
        let mut stack = vec![file];
        while let Some(at) = stack.pop() {
            if let Some(seen) = wanted.get_mut(at as usize)
                && !*seen
            {
                *seen = true;
                stack.extend(self.needs.get(at as usize).into_iter().flatten());
            }
        }
        self.order
            .iter()
            .copied()
            .filter(|&f| wanted.get(f as usize) == Some(&true))
            .collect()
    }

    /// The position of each file in [`Scan::order`].
    fn ranks(&self) -> Vec<usize> {
        let mut rank = vec![0; self.files.len()];
        for (at, &file) in self.order.iter().enumerate() {
            if let Some(entry) = rank.get_mut(file as usize) {
                *entry = at;
            }
        }
        rank
    }

    /// The file that computes the merged node `node`: its leaf's file, or
    /// the last of its contributors in the order.
    #[must_use]
    pub fn computed_by(&self, node: u32) -> Option<u32> {
        let entry = self.nodes.get(node as usize)?;
        if entry.leaf.is_some() {
            return entry.leaf;
        }
        entry
            .contributors
            .iter()
            .copied()
            .max_by_key(|&f| self.rank.get(f as usize).copied())
    }

    /// The dotted path of the merged node `node`.
    fn show(&self, node: u32) -> String {
        let mut names = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.nodes.get(index as usize)
        {
            names.push(self.names.get(entry.name));
            at = entry.parent;
        }
        names.reverse();
        names.join(".")
    }
}

/// The scope a file's references are resolved in: the merged trees of its
/// directory chain, and the `def`s there, numbered as
/// [`crate::mentions::Resolve`] asks: the file's own nodes and functions
/// first, by their own numbers, and every other one at the count of the
/// file's own plus its index in [`Scan::nodes`] or [`Scan::functions`].
#[derive(Debug)]
pub struct FileScope<'a> {
    /// The workspace.
    scan: &'a Scan,
    /// The file's directory.
    dir: u32,
    /// The file's own functions.
    own: &'a [mentions::TopLevel],
    /// For each of the file's own nodes, the merged node of its path.
    global: &'a [u32],
    /// The file's own node of each merged node it has one of.
    local: HashMap<u32, u32>,
}

impl<'a> FileScope<'a> {
    /// The scope of `file` of `scan`.
    #[must_use]
    pub fn new(scan: &'a Scan, file: &'a File) -> Self {
        let local = (0u32..)
            .zip(&file.global)
            .map(|(own, &global)| (global, own))
            .collect();
        Self {
            scan,
            dir: file.dir,
            own: &file.mentions.functions,
            global: &file.global,
            local,
        }
    }

    /// The scope's number of the merged node `node`.
    fn number(&self, node: u32) -> u32 {
        self.local
            .get(&node)
            .copied()
            .unwrap_or_else(|| count(self.global.len()).saturating_add(node))
    }

    /// The merged node of the scope's number `node`.
    #[must_use]
    pub fn merged(&self, node: u32) -> Option<u32> {
        match node.checked_sub(count(self.global.len())) {
            Some(other) => Some(other),
            None => self.global.get(node as usize).copied(),
        }
    }

    /// The function of [`Scan::functions`] of the scope's number
    /// `function`, if it is another file's.
    #[must_use]
    pub fn foreign_function(&self, function: u32) -> Option<u32> {
        function.checked_sub(count(self.own.len()))
    }
}

impl Resolve for FileScope<'_> {
    fn root(&self, name: Name) -> Option<u32> {
        self.scan
            .chain(self.dir)
            .find_map(|dir| self.scan.by_name.get(&(dir, ROOT, name)))
            .map(|&node| self.number(node))
    }

    fn child(&self, node: u32, name: Name) -> Option<u32> {
        let merged = self.merged(node)?;
        let dir = self.scan.nodes.get(merged as usize)?.dir;
        let child = *self.scan.by_name.get(&(dir, merged, name))?;
        Some(self.number(child))
    }

    fn function(&self, name: Name) -> Option<u32> {
        if let Some(own) = self.own.iter().position(|f| f.name == name) {
            return Some(count(own));
        }
        let def = self
            .scan
            .chain(self.dir)
            .find_map(|dir| self.scan.defs.get(&(dir, name)))?;
        Some(count(self.own.len()).saturating_add(*def))
    }

    fn own(&self) -> (u32, u32) {
        (count(self.global.len()), count(self.own.len()))
    }
}

/// A count as a `u32`, saturating; a workspace's counts are bounded by its
/// sources' lengths.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The directory part of `path`, empty for a file of the root.
fn parent_path(path: &str) -> &str {
    path.rfind('/')
        .map_or("", |at| path.get(..at).unwrap_or(""))
}

/// Scans the workspace whose files are `sources`, named by their paths
/// relative to the root: parses every file, merges each directory's trees,
/// resolves every file's references against its directory chain, and
/// finds the order and each file's schedule (see the module
/// documentation).
///
/// # Errors
///
/// Every syntax error of every file; else every conflict between files'
/// declarations; else every cycle of files that need each other. Each
/// names its file in [`Diagnostic::input`], its index in path order.
pub fn scan(sources: &[SourceFile]) -> Result<Scan, Vec<Diagnostic>> {
    let mut sorted: Vec<&SourceFile> = sources.iter().collect();
    sorted.sort_by(|a, b| a.name().as_bytes().cmp(b.name().as_bytes()));
    let mut scan = Scan {
        files: Vec::with_capacity(sorted.len()),
        dirs: Vec::new(),
        nodes: Vec::new(),
        functions: Vec::new(),
        needs: Vec::new(),
        order: Vec::new(),
        names: Names::new(),
        by_name: HashMap::new(),
        defs: HashMap::new(),
        rank: Vec::new(),
    };
    let mut dirs: HashMap<String, u32> = HashMap::new();
    let mut errors = Vec::new();
    for (index, source) in (0u32..).zip(sorted) {
        let (ast, spelled) = match parse(source, &mut scan.names) {
            Ok(parsed) => parsed,
            Err(error) => {
                errors.push(error.in_input(index));
                continue;
            }
        };
        let dir = intern_dir(&mut scan.dirs, &mut dirs, parent_path(source.name()));
        let mentions = mentions::scan(&ast);
        scan.files.push(File {
            path: source.name().to_owned(),
            dir,
            ast,
            spelled,
            mentions,
            global: Vec::new(),
            functions: Vec::new(),
            schedule: Schedule::default(),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for file in 0..count(scan.files.len()) {
        merge(&mut scan, file, &mut errors);
    }
    roots_above(&scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    for index in 0..scan.files.len() {
        let mut mentions = scan
            .files
            .get_mut(index)
            .map(|file| mem::take(&mut file.mentions))
            .unwrap_or_default();
        if let Some(file) = scan.files.get(index) {
            let (functions, tree) = (
                mem::take(&mut mentions.functions),
                mem::take(&mut mentions.tree),
            );
            let mut scope = FileScope::new(&scan, file);
            scope.own = &functions;
            mentions.resolve(&file.ast, &scope);
            mentions.functions = functions;
            mentions.tree = tree;
        }
        if let Some(file) = scan.files.get_mut(index) {
            file.mentions = mentions;
        }
    }
    let mentioned = find_needs(&mut scan);
    report_cycles(&scan, &mentioned, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    scan.order = order(&scan.needs);
    scan.rank = scan.ranks();
    schedule(&mut scan);
    Ok(scan)
}

/// Lexes and parses `source`, interning into `names`; the tree's own table
/// is left empty. With the tree, the names its checker can look up (see
/// [`File::spelled`]).
fn parse(source: &SourceFile, names: &mut Names) -> Result<(Ast, Vec<Name>), Diagnostic> {
    let (tokens, mut strings) = lexer::tokenize_with(source, names)?;
    let (builder, root) = parser::parse_parts(&tokens, source, names, &mut strings)?;
    let mut spelled: Vec<Name> = tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Ident(name) => Some(name),
            // A quoted field name is a plain string literal's text.
            TokenKind::Str(text) => names.lookup(strings.get(text)),
            _ => None,
        })
        .collect();
    spelled.sort_unstable();
    spelled.dedup();
    let mut ast = builder.finish(root, mem::take(names), strings);
    *names = ast.replace_names(Names::new());
    Ok((ast, spelled))
}

/// The directory `path`, added with the directories above it if new.
fn intern_dir(dirs: &mut Vec<Dir>, index: &mut HashMap<String, u32>, path: &str) -> u32 {
    if let Some(&dir) = index.get(path) {
        return dir;
    }
    let parent = (!path.is_empty()).then(|| intern_dir(dirs, index, parent_path(path)));
    let dir = count(dirs.len());
    dirs.push(Dir {
        path: path.to_owned(),
        parent,
    });
    index.insert(path.to_owned(), dir);
    dir
}

/// The span of the path of the field item of `file`'s leaf `own`, and of
/// the whole item.
fn leaf_spans(file: &File, own: u32) -> Option<(Span, Span)> {
    let node = file.mentions.tree.nodes.get(own as usize)?;
    let leaf = node.leaf?;
    let item = file.ast.item(leaf.item);
    let path = match item.kind {
        ItemKind::Field { path, .. } => {
            let segments = file.ast.path(path);
            let first = segments.first().map(|s: &PathSegment| s.span)?;
            let last = segments.last().map_or(first, |s| s.span);
            Span::new(first.start, last.end)
        }
        _ => item.span,
    };
    Some((path, item.span))
}

/// The span of the first leaf of `file` at or under its own node `own`.
fn first_leaf(file: &File, own: u32) -> Option<(Span, Span)> {
    let mut at = own;
    loop {
        let node = file.mentions.tree.nodes.get(at as usize)?;
        if node.leaf.is_some() {
            return leaf_spans(file, at);
        }
        at = *node.children.first()?;
    }
}

/// Adds `file`'s own tree to its directory's merged tree, reporting the
/// conflicts with the files before it, and numbers its functions.
fn merge(scan: &mut Scan, file: u32, errors: &mut Vec<Diagnostic>) {
    let Some(entry) = scan.files.get(file as usize) else {
        return;
    };
    let dir = entry.dir;
    let mut global = Vec::with_capacity(entry.mentions.tree.nodes.len());
    for (own, node) in (0u32..).zip(&entry.mentions.tree.nodes) {
        let parent = match node.parent {
            Some(up) => global.get(up as usize).copied().unwrap_or(ROOT),
            None => ROOT,
        };
        let key = (dir, parent, node.name);
        let merged = if let Some(&existing) = scan.by_name.get(&key) {
            if let Some(clash) = conflict(scan, file, own, existing) {
                errors.push(clash);
            }
            existing
        } else {
            let merged = count(scan.nodes.len());
            scan.nodes.push(Node {
                name: node.name,
                parent: (parent != ROOT).then_some(parent),
                children: Vec::new(),
                dir,
                leaf: None,
                contributors: Vec::new(),
            });
            scan.by_name.insert(key, merged);
            if let Some(up) = scan.nodes.get_mut(parent as usize) {
                up.children.push(merged);
            }
            merged
        };
        if let Some(target) = scan.nodes.get_mut(merged as usize) {
            if node.leaf.is_some() && target.leaf.is_none() && target.children.is_empty() {
                target.leaf = Some(file);
            }
            if target.contributors.last() != Some(&file) {
                target.contributors.push(file);
            }
        }
        global.push(merged);
    }
    let mut functions = Vec::with_capacity(entry.mentions.functions.len());
    for (index, function) in (0u32..).zip(&entry.mentions.functions) {
        let id = count(scan.functions.len());
        scan.functions.push(Function { file, index });
        let is_def = entry
            .mentions
            .tree
            .item_leaf
            .get(function.position)
            .is_some_and(Option::is_some);
        if is_def {
            scan.defs.entry((dir, function.name)).or_insert(id);
        }
        functions.push(id);
    }
    if let Some(entry) = scan.files.get_mut(file as usize) {
        entry.global = global;
        entry.functions = functions;
    }
}

/// The conflict, if any, of `file`'s own node `own` with the merged node
/// `existing` that earlier files declared.
fn conflict(scan: &Scan, input: u32, own: u32, existing: u32) -> Option<Diagnostic> {
    let file = scan.files.get(input as usize)?;
    let node = file.mentions.tree.nodes.get(own as usize)?;
    let merged = scan.nodes.get(existing as usize)?;
    let shown = scan.show(existing);
    if let Some(other) = merged.leaf {
        let other_file = scan.files.get(other as usize)?;
        let other_own = other_file
            .global
            .iter()
            .position(|&g| g == existing)
            .map(count)?;
        let (other_path, other_item) = leaf_spans(other_file, other_own)?;
        let (path, _) = first_leaf(file, own)?;
        return Some(if node.leaf.is_some() {
            Diagnostic::error(format!("field `{shown}` is defined in two files"), path)
                .with_label("defined here")
                .with_context_in(other, other_path, "defined here")
                .in_input(input)
        } else {
            Diagnostic::error(format!("field `{shown}` is already defined"), path)
                .with_context_in(other, other_item, "defined here")
                .in_input(input)
        });
    }
    if node.leaf.is_some() {
        let first = *merged.contributors.first()?;
        let other_file = scan.files.get(first as usize)?;
        let other_own = other_file
            .global
            .iter()
            .position(|&g| g == existing)
            .map(count)?;
        let (_, other_item) = first_leaf(other_file, other_own)?;
        let (path, _) = leaf_spans(file, own)?;
        return Some(
            Diagnostic::error(format!("field `{shown}` is already defined"), path)
                .with_context_in(first, other_item, "defined here")
                .in_input(input),
        );
    }
    None
}

/// Reports each root of a directory's tree that is also a root of a
/// directory above it.
fn roots_above(scan: &Scan, errors: &mut Vec<Diagnostic>) {
    for (index, node) in (0u32..).zip(&scan.nodes) {
        if node.parent.is_some() {
            continue;
        }
        let above = scan
            .chain(node.dir)
            .skip(1)
            .find_map(|dir| scan.by_name.get(&(dir, ROOT, node.name)));
        let Some(&above) = above else { continue };
        let spans = |merged: u32| {
            let first = *scan.nodes.get(merged as usize)?.contributors.first()?;
            let file = scan.files.get(first as usize)?;
            let own = file.global.iter().position(|&g| g == merged).map(count)?;
            Some((first, first_leaf(file, own)?.0))
        };
        let (Some((file, here)), Some((other, there))) = (spans(index), spans(above)) else {
            continue;
        };
        let name = scan.names.get(node.name);
        errors.push(
            Diagnostic::error(format!("`{name}` is declared in the directory above"), here)
                .with_label("declared here")
                .with_context_in(other, there, "declared in the directory above")
                .in_input(file),
        );
    }
}

/// Fills [`Scan::needs`], and returns for each file the first mention, in
/// source order, of a declaration of each file it needs, with what it
/// shows.
fn find_needs(scan: &mut Scan) -> Vec<HashMap<u32, (Span, String)>> {
    let mut all = Vec::with_capacity(scan.files.len());
    for (index, file) in (0u32..).zip(&scan.files) {
        let mut first: HashMap<u32, (Span, String)> = HashMap::new();
        let mut record = |owner: u32, span: Span, shown: &dyn Fn() -> String| {
            if owner == index {
                return;
            }
            let entry = first.entry(owner).or_insert_with(|| (span, shown()));
            if span.start < entry.0.start {
                *entry = (span, shown());
            }
        };
        let own_nodes = count(file.global.len());
        let own_functions = count(file.functions.len());
        for (position, fields) in file.mentions.fields.iter().enumerate() {
            let spans = file.mentions.field_spans.get(position);
            let foreign = file.mentions.foreign.get(position).into_iter().flatten();
            let own = fields
                .iter()
                .zip(spans.into_iter().flatten())
                .filter_map(|(&n, &span)| Some((*file.global.get(n as usize)?, span)));
            let mut nodes: Vec<(u32, Span)> = own.collect();
            for &(need, span) in foreign {
                match need {
                    Need::Field(n) => nodes.push((n.saturating_sub(own_nodes), span)),
                    Need::Function(f) => {
                        let id = f.saturating_sub(own_functions);
                        if let Some(function) = scan.functions.get(id as usize) {
                            let owner = function.file;
                            let name = scan
                                .files
                                .get(owner as usize)
                                .and_then(|o| o.mentions.functions.get(function.index as usize))
                                .map(|t| t.name);
                            record(owner, span, &|| {
                                name.map_or_else(String::new, |n| scan.names.get(n).to_owned())
                            });
                        }
                    }
                }
            }
            for (merged, span) in nodes {
                let Some(node) = scan.nodes.get(merged as usize) else {
                    continue;
                };
                let owners = node
                    .leaf
                    .as_ref()
                    .map_or(node.contributors.as_slice(), std::slice::from_ref);
                for &owner in owners {
                    record(owner, span, &|| scan.show(merged));
                }
            }
        }
        all.push(first);
    }
    scan.needs = all
        .iter()
        .map(|first| {
            let mut needs: Vec<u32> = first.keys().copied().collect();
            needs.sort_unstable();
            needs
        })
        .collect();
    all
}

/// Reports each cycle of files that need each other, once.
fn report_cycles(
    scan: &Scan,
    mentioned: &[HashMap<u32, (Span, String)>],
    errors: &mut Vec<Diagnostic>,
) {
    let edges: Vec<&[u32]> = scan.needs.iter().map(Vec::as_slice).collect();
    let (mut groups, _) = mentions::components(&edges);
    for group in &mut groups {
        group.sort_unstable();
    }
    groups.sort();
    for group in groups.iter().filter(|group| group.len() > 1) {
        let path = |file: u32| {
            scan.files
                .get(file as usize)
                .map_or_else(String::new, |f| format!("`{}`", f.path))
        };
        let shown: Vec<String> = group.iter().map(|&f| path(f)).collect();
        let listed = match shown.split_last() {
            Some((last, rest)) if rest.len() > 1 => format!("{} and {last}", rest.join(", ")),
            Some((last, rest)) => format!("{} and {last}", rest.join("")),
            None => String::new(),
        };
        // Each file's first mention of another file of the cycle.
        let uses = group.iter().filter_map(|&file| {
            let first = mentioned.get(file as usize)?;
            let (at, what) = group
                .iter()
                .filter_map(|other| first.get(other))
                .min_by_key(|(at, _)| at.start)?;
            Some((file, *at, format!("{} uses `{what}` here", path(file))))
        });
        let mut diagnostic: Option<Diagnostic> = None;
        for (file, span, text) in uses {
            diagnostic = Some(match diagnostic {
                None => Diagnostic::error(format!("{listed} need each other"), span)
                    .with_label(text)
                    .in_input(file),
                Some(diagnostic) => diagnostic.with_context_in(file, span, text),
            });
        }
        if let Some(diagnostic) = diagnostic {
            errors.push(diagnostic.with_note(
                "a file runs as a whole before a file that needs it, so two files cannot need \
                 each other",
            ));
        }
    }
}

/// The files in the order they run: repeatedly the path-smallest file all
/// of whose needs are placed. `needs` has no cycles.
fn order(needs: &[Vec<u32>]) -> Vec<u32> {
    let mut waiting: Vec<usize> = needs.iter().map(Vec::len).collect();
    let mut users: Vec<Vec<u32>> = vec![Vec::new(); needs.len()];
    for (file, list) in (0u32..).zip(needs) {
        for &need in list {
            if let Some(entry) = users.get_mut(need as usize) {
                entry.push(file);
            }
        }
    }
    let mut ready: BinaryHeap<Reverse<u32>> = (0u32..)
        .zip(&waiting)
        .filter(|&(_, &n)| n == 0)
        .map(|(file, _)| Reverse(file))
        .collect();
    let mut order = Vec::with_capacity(needs.len());
    while let Some(Reverse(file)) = ready.pop() {
        order.push(file);
        for &user in users.get(file as usize).into_iter().flatten() {
            if let Some(entry) = waiting.get_mut(user as usize) {
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    ready.push(Reverse(user));
                }
            }
        }
    }
    order
}

/// Finds each file's schedule, its shared interior nodes that a later
/// file in the order computes left out.
fn schedule(scan: &mut Scan) {
    let rank = &scan.rank;
    let mut schedules = Vec::with_capacity(scan.files.len());
    for (index, file) in (0u32..).zip(&scan.files) {
        let done: Vec<bool> = file
            .global
            .iter()
            .map(|&merged| {
                scan.nodes.get(merged as usize).is_some_and(|node| {
                    node.leaf.is_none()
                        && node
                            .contributors
                            .iter()
                            .max_by_key(|&&f| rank.get(f as usize).copied())
                            != Some(&index)
                })
            })
            .collect();
        schedules.push(file.mentions.schedule_except(&done));
    }
    for (file, schedule) in scan.files.iter_mut().zip(schedules) {
        file.schedule = schedule;
    }
}

/// What the checker knows of the workspace while it checks one of its
/// files (see [Directory scope](crate::check#directory-scope)).
#[derive(Debug)]
pub(crate) struct Context<'a> {
    /// The workspace.
    scan: &'a Scan,
    /// The file being checked.
    file: &'a File,
    /// The scope the file's references are resolved in.
    pub(crate) scope: FileScope<'a>,
    /// The type of each merged node's cell, by its slot: `Error` until the
    /// file that computes it is checked.
    cell_types: &'a [TyId],
}

impl<'a> Context<'a> {
    /// The context of file `index` of `scan`.
    fn new(scan: &'a Scan, index: u32, cell_types: &'a [TyId]) -> Option<Self> {
        let file = scan.files.get(index as usize)?;
        Some(Self {
            scan,
            file,
            scope: FileScope::new(scan, file),
            cell_types,
        })
    }

    /// The file being checked.
    pub(crate) fn file(&self) -> &'a File {
        self.file
    }

    /// Whether the merged node `node` has leaves in more than one file.
    pub(crate) fn shared(&self, node: u32) -> bool {
        self.scan
            .nodes
            .get(node as usize)
            .is_some_and(|entry| entry.contributors.len() > 1)
    }

    /// The type of the cell of the merged node `node`, as its file's check
    /// found it.
    pub(crate) fn cell_type(&self, node: u32) -> TyId {
        self.cell_types
            .get(node as usize)
            .copied()
            .unwrap_or(TyId::ERROR)
    }

    /// The children of the file's own node `node` that other files
    /// declare: each one's name, cell and type.
    pub(crate) fn foreign_children(&self, node: u32) -> Vec<(Name, Slot, TyId)> {
        self.merged_children(node)
            .into_iter()
            .filter(|&(_, own, _, _)| own.is_none())
            .map(|(name, _, slot, ty)| (name, slot, ty))
            .collect()
    }

    /// Every child of the file's own node `node` if it is shared with
    /// other files, in the merged node's order: each one's name, the
    /// file's own node of it if it has one, its cell and, for another
    /// file's, its type. Empty for a node of the file alone.
    pub(crate) fn merged_children(&self, node: u32) -> Vec<(Name, Option<u32>, Slot, TyId)> {
        let Some(&global) = self.file().global.get(node as usize) else {
            return Vec::new();
        };
        if !self.shared(global) {
            return Vec::new();
        }
        let children = self
            .scan
            .nodes
            .get(global as usize)
            .map_or(&[][..], |n| &n.children);
        children
            .iter()
            .filter_map(|&child| {
                let entry = self.scan.nodes.get(child as usize)?;
                let own = self.scope.local.get(&child).copied();
                Some((entry.name, own, Slot::at(child), self.cell_type(child)))
            })
            .collect()
    }
}

/// A workspace checked and lowered, ready to run any of its files: driven
/// like a [`crate::session::Session`], one frame and one memo table for
/// every file, but with each file's declarations committed once it is
/// checked, and only the files a run needs evaluated. See the module
/// documentation.
#[derive(Debug)]
pub struct Workspace {
    /// The scanned workspace.
    scan: Scan,
    /// Each file's IR, by input number, until it runs.
    irs: Vec<Option<Ir>>,
    /// The type of every expression of each file, by input number, for
    /// [`Workspace::dump_types`].
    checked: Vec<Option<Types>>,
    /// The value of each file that has run, `{}` for the others.
    values: Vec<Value>,
    /// The warnings of every file, in path then source order.
    warnings: Vec<Diagnostic>,
    /// The top level's frame of every file.
    frame: Vec<Value>,
    /// The memo tables of every memoised closure.
    memo: MemoTables,
    /// The table the files' types are interned into.
    types: TypeTable,
    /// The name of every slot of the frame, the last variable's in it, for
    /// [`Workspace::dump_ir`]: one table for every file, each file's IR
    /// keeping none (a copy of the whole frame's in each would make the
    /// workspace's memory quadratic in its files).
    slot_names: Vec<Name>,
}

/// Scans, checks and lowers the workspace of `sources`, each with its
/// path relative to the root, in the order of [`Scan::order`].
///
/// # Errors
///
/// The diagnostics of the scan, or those of every file whose check or
/// lowering fails, with the warnings found, in path then source order;
/// each names its file in [`Diagnostic::input`]. A file that needs a file
/// that failed is not checked: its errors would follow from the first.
pub fn check(sources: &[SourceFile]) -> Result<Workspace, Vec<Diagnostic>> {
    let mut scan = scan(sources)?;
    let mut names = mem::take(&mut scan.names);
    let mut types = TypeTable::new();
    let mut cell_types = vec![TyId::ERROR; scan.nodes.len()];
    let mut slot_names: Vec<Name> = scan.nodes.iter().map(|node| node.name).collect();
    let mut scopes = vec![Committed::default(); scan.dirs.len()];
    let mut irs: Vec<Option<Ir>> = (0..scan.files.len()).map(|_| None).collect();
    let mut checked: Vec<Option<Types>> = (0..scan.files.len()).map(|_| None).collect();
    let mut warnings = vec![Vec::new(); scan.files.len()];
    let mut errors = Vec::new();
    // The files whose check failed, or that need one that did: those are
    // not checked, since their errors would follow from the first.
    let mut failed = vec![false; scan.files.len()];
    for index in scan.order.clone() {
        if blocked(&scan, &mut failed, index) {
            continue;
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            file.ast.replace_names(mem::take(&mut names));
        }
        // Every file of the order is a file of the workspace.
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        // The slots' names are moved, not copied: the check gives them back
        // with the file's own slots added.
        let scope = Committed::scope(&scan, &scopes, file, mem::take(&mut slot_names));
        let start = scope.len();
        let compiled = Context::new(&scan, index, &cell_types).map(|context| {
            let output = Checker::in_workspace(&file.ast, scope, &mut types, &context).check();
            output.and_then(|output| session::lower_checked(&file.ast, output, &types, index))
        });
        // What the file declares, committed for the files after it.
        let mut committed = Vec::new();
        if let Some(Ok((_, output))) = &compiled {
            for (own, (&global, &ty)) in file.global.iter().zip(&output.cell_types).enumerate() {
                if scan.computed_by(global) == Some(index) {
                    committed.push((own, global, ty));
                }
            }
        }
        let keep = |name: Name| {
            file.mentions.tree.root(name).is_some_and(|root| {
                let global = file.global.get(root as usize).copied();
                global.and_then(|global| scan.computed_by(global)) == Some(index)
            }) || file.mentions.functions.iter().any(|top| {
                top.name == name && matches!(file.ast.item(top.item).kind, ItemKind::Field { .. })
            })
        };
        let result = match compiled {
            Some(Ok((ir, output))) => {
                if let Some(dir) = scopes.get_mut(file.dir as usize) {
                    dir.commit(&output.scope, start, keep);
                }
                slot_names = output.scope.into_slot_names();
                Ok((ir, output.checked.types, output.warnings))
            }
            failed_check => {
                // The slots' names went to the failed check. Nothing runs
                // once a file has failed, so the files checked after it
                // need only the merged nodes' slots kept apart from theirs.
                slot_names = scan.nodes.iter().map(|node| node.name).collect();
                match failed_check {
                    Some(Err(errors)) => Err(errors),
                    // Every file of the order is a file of the workspace.
                    _ => Err(Vec::new()),
                }
            }
        };
        if let Some(file) = scan.files.get_mut(index as usize) {
            names = file.ast.replace_names(Names::new());
        }
        for (_, global, ty) in committed {
            set(&mut cell_types, global, ty);
        }
        match result {
            Ok((mut ir, file_types, found)) => {
                // Its slots' names are the workspace's (see `slot_names`).
                ir.replace_slot_names(Vec::new());
                set(&mut irs, index, Some(ir));
                set(&mut checked, index, Some(file_types));
                set(&mut warnings, index, in_file(found, index));
            }
            Err(found) => {
                errors.extend(in_file(found, index));
                set(&mut failed, index, true);
            }
        }
    }
    if !errors.is_empty() {
        return Err(merge_files(
            errors,
            warnings.into_iter().flatten().collect(),
        ));
    }
    scan.names = names;
    Ok(Workspace {
        values: vec![Value::Empty; scan.files.len()],
        scan,
        irs,
        checked,
        warnings: warnings.into_iter().flatten().collect(),
        frame: Vec::new(),
        slot_names,
        memo: MemoTables::new(),
        types,
    })
}

/// The declarations the files of one directory have committed, for the
/// files checked after them: a scope, with its variables indexed by name,
/// so that a file is checked from the declarations of the names it spells
/// alone, never a copy of every declaration of its directory chain (which
/// would make checking a workspace quadratic in its files).
#[derive(Clone, Debug, Default)]
struct Committed {
    /// The variables, in the order they were committed.
    scope: Scope,
    /// The indices in `scope` of the variables of each name, in order.
    by_name: HashMap<Name, Vec<usize>>,
}

impl Committed {
    /// The scope `file` is checked from, given the committed declarations
    /// of each directory of `scan` and the names of the frame's slots so
    /// far: those of its directory chain, from the root down, of the names
    /// it spells.
    fn scope(scan: &Scan, committed: &[Self], file: &File, slot_names: Vec<Name>) -> Scope {
        let chain: Vec<u32> = scan.chain(file.dir).collect();
        Scope::joined(
            chain.iter().rev().filter_map(|&dir| {
                let committed = committed.get(dir as usize)?;
                Some((&committed.scope, committed.picked(&file.spelled)))
            }),
            slot_names,
        )
    }

    /// The indices in `scope` of the variables named in `spelled`, in
    /// commit order, so that of two variables of one name the later is
    /// still the later.
    fn picked(&self, spelled: &[Name]) -> Vec<usize> {
        let mut picked: Vec<usize> = spelled
            .iter()
            .filter_map(|name| self.by_name.get(name))
            .flatten()
            .copied()
            .collect();
        picked.sort_unstable();
        picked
    }

    /// Commits the variables of `from` after its first `start` whose names
    /// `keep` accepts, as [`Scope::commit`] does, and indexes them.
    fn commit(&mut self, from: &Scope, start: usize, keep: impl Fn(Name) -> bool) {
        let before = self.scope.len();
        self.scope.commit(from, start, keep);
        for (at, name) in self.scope.names().enumerate().skip(before) {
            self.by_name.entry(name).or_default().push(at);
        }
    }
}

/// Sets the entry `index` of `entries` to `value`, if it has one.
fn set<T>(entries: &mut [T], index: u32, value: T) {
    if let Some(entry) = entries.get_mut(index as usize) {
        *entry = value;
    }
}

/// Whether file `index` needs a file in `failed` (whose check failed, or
/// that was not checked for this reason), marking it failed if it does.
fn blocked(scan: &Scan, failed: &mut [bool], index: u32) -> bool {
    let blocked = scan.needs.get(index as usize).is_some_and(|needs| {
        needs
            .iter()
            .any(|&need| failed.get(need as usize).copied().unwrap_or(false))
    });
    if blocked && let Some(entry) = failed.get_mut(index as usize) {
        *entry = true;
    }
    blocked
}

/// Merges `errors` and `warnings` of several files into one vector, as
/// [`crate::diagnostic::merge`] does for one: in path order of their files (their
/// [`Diagnostic::input`]), then in source order, errors first among those
/// that start at one place.
fn merge_files(errors: Vec<Diagnostic>, warnings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut all = errors;
    all.extend(warnings);
    all.sort_by_key(|diagnostic| (diagnostic.input, diagnostic.span.start));
    all
}

/// `diagnostics`, each named as of file `index` unless it names one.
fn in_file(mut diagnostics: Vec<Diagnostic>, index: u32) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        diagnostic.input.get_or_insert(index);
    }
    diagnostics
}

impl Workspace {
    /// The scanned workspace: its files, their order and needs.
    #[must_use]
    pub fn scan(&self) -> &Scan {
        &self.scan
    }

    /// The input number of the file at `path`, relative to the root.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<u32> {
        self.scan
            .files
            .iter()
            .position(|file| file.path == path)
            .and_then(|at| u32::try_from(at).ok())
    }

    /// The table the files' types are interned into.
    #[must_use]
    pub fn types(&self) -> &TypeTable {
        &self.types
    }

    /// The warnings of every file, in path then source order.
    #[must_use]
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// The syntax tree of file `index` annotated with the types it was
    /// checked with in the workspace, as `--dump-types` prints it.
    pub fn dump_types(&mut self, index: u32) -> Option<String> {
        let types = self.checked.get(index as usize)?.as_ref()?;
        let file = self.scan.files.get_mut(index as usize)?;
        file.ast.replace_names(mem::take(&mut self.scan.names));
        let annotation = types.annotation(&self.types, file.ast.names());
        let text = file.ast.pretty_with(&annotation).to_string();
        self.scan.names = file.ast.replace_names(Names::new());
        Some(text)
    }

    /// The IR file `index` was lowered to in the workspace, as `--dump-ir`
    /// prints it, until the file runs.
    #[must_use]
    pub fn dump_ir(&mut self, index: u32) -> Option<String> {
        let ir = self.irs.get_mut(index as usize)?.as_mut()?;
        // Lent the names of every slot, of which its frame is a prefix.
        ir.replace_slot_names(mem::take(&mut self.slot_names));
        let dump = ir.pretty(&self.scan.names).to_string();
        self.slot_names = ir.replace_slot_names(Vec::new());
        Some(dump)
    }

    /// Runs file `index` and, before it, every file it needs that has not
    /// run, in the order, and returns its own value: its result, or the
    /// record of its own fields, with the warnings of the whole workspace.
    ///
    /// # Errors
    ///
    /// The runtime error of the first file that fails, naming its file,
    /// with the workspace's warnings.
    pub fn run(&mut self, index: u32) -> Result<Outcome, Vec<Diagnostic>> {
        for at in self.scan.needed(index) {
            let Some(ir) = self.irs.get_mut(at as usize).and_then(Option::take) else {
                continue;
            };
            match session::run_input(ir, &mut self.frame, &mut self.memo) {
                Ok(value) => {
                    if let Some(entry) = self.values.get_mut(at as usize) {
                        *entry = value;
                    }
                }
                Err(mut error) => {
                    error.input.get_or_insert(at);
                    return Err(merge_files(vec![error], self.warnings.clone()));
                }
            }
        }
        Ok(Outcome {
            value: self
                .values
                .get(index as usize)
                .cloned()
                .unwrap_or(Value::Empty),
            warnings: self.warnings.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scans the workspace of `files`, each a path and a text.
    fn ws(files: &[(&str, &str)]) -> Result<Scan, Vec<Diagnostic>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        scan(&sources)
    }

    /// The single error of scanning `files`: its message, input, and the
    /// inputs of its labels.
    fn error(files: &[(&str, &str)]) -> (String, Option<u32>, Vec<Option<u32>>) {
        let errors = ws(files).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let error = &errors[0];
        let labels = error.labels.iter().map(|l| l.input).collect();
        (error.message.clone(), error.input, labels)
    }

    /// The files of the summary, in path order: app, common, lib,
    /// workspace.
    const SUMMARY: [(&str, &str); 4] = [
        ("workspace.gcsl", ""),
        (
            "common.gcsl",
            "version = \"1.2\"\ndef tagged(n: String): String { \"{n}-{version}\" }",
        ),
        (
            "app.gcsl",
            "com.x.app.name = tagged(\"app\")\ncom.x.app.deps = [com.x.lib]",
        ),
        (
            "lib.gcsl",
            "com.x.lib.name = tagged(\"lib\")\ncom.x.lib.src = \"lib-{version}.tar\"",
        ),
    ];

    #[test]
    fn the_summary_runs_in_need_order() {
        let scan = ws(&SUMMARY).unwrap();
        let paths: Vec<&str> = scan.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["app.gcsl", "common.gcsl", "lib.gcsl", "workspace.gcsl"]
        );
        assert_eq!(scan.needs, [vec![1, 2], vec![], vec![1], vec![]]);
        assert_eq!(scan.order, [1, 2, 0, 3]);
        assert_eq!(scan.needed(0), [1, 2, 0]);
        assert_eq!(scan.needed(2), [1, 2]);
        assert_eq!(scan.needed(3), [3]);
    }

    #[test]
    fn a_reference_means_its_prefix_in_the_merged_tree() {
        let scan = ws(&SUMMARY).unwrap();
        let app = &scan.files[0];
        // `com.x.lib` in app's second item is lib's interior node, not
        // app's own `com.x`.
        let foreign = &app.mentions.foreign[1];
        assert_eq!(foreign.len(), 1);
        let Need::Field(node) = foreign[0].0 else {
            panic!("{foreign:?}")
        };
        let merged = FileScope::new(&scan, app).merged(node).unwrap();
        assert_eq!(scan.show(merged), "com.x.lib");
        assert_eq!(scan.nodes[merged as usize].contributors, [2]);
    }

    #[test]
    fn dotted_paths_merge_and_the_last_contributor_assembles() {
        let scan = ws(&[("a.gcsl", "com.a = 1"), ("b.gcsl", "all = com\ncom.b = 2")]).unwrap();
        assert_eq!(scan.order, [0, 1]);
        let com = scan.by_name[&(0, ROOT, scan.files[0].mentions.tree.nodes[0].name)];
        assert_eq!(scan.nodes[com as usize].contributors, [0, 1]);
        assert_eq!(scan.computed_by(com), Some(1));
        // a leaves `com` out; b computes it before its first mention.
        assert!(!scan.files[0].schedule.order.contains(&0));
        let b = &scan.files[1];
        let local = b.global.iter().position(|&g| g == com).unwrap();
        assert!(b.schedule.order.contains(&count(local)));
        assert_eq!(b.schedule.point[local], 0);
    }

    #[test]
    fn a_shared_node_read_whole_from_a_third_file() {
        let scan = ws(&[
            ("a.gcsl", "com.a = 1"),
            ("b.gcsl", "com.b = 2\nz = 3"),
            ("c.gcsl", "all = com"),
        ])
        .unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![0, 1]]);
        // b, the last contributor, assembles `com` after its last child.
        let b = &scan.files[1];
        assert_eq!(b.schedule.point[0], 1);
        assert!(!scan.files[0].schedule.order.contains(&0));
    }

    #[test]
    fn a_def_of_one_file_called_from_another_is_a_need() {
        let scan = ws(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "x = f()")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![0]]);
    }

    #[test]
    fn siblings_and_subdirectories_are_hidden() {
        let scan = ws(&[("x/a.gcsl", "v = 1"), ("y/b.gcsl", "w = v")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![]]);
        let scan = ws(&[("a.gcsl", "w = v"), ("sub/b.gcsl", "v = 1")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![]]);
        let scan = ws(&[("a.gcsl", "v = 1"), ("sub/b.gcsl", "w = v")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![0]]);
    }

    #[test]
    fn conflicts_between_files() {
        assert_eq!(
            error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x.y = 2")]),
            (
                "field `x.y` is defined in two files".to_owned(),
                Some(1),
                vec![Some(0)]
            )
        );
        assert_eq!(
            error(&[("a.gcsl", "x = 1"), ("b.gcsl", "x.y = 2")]).0,
            "field `x` is already defined"
        );
        assert_eq!(
            error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x = 2")]).0,
            "field `x` is already defined"
        );
        assert_eq!(
            error(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "f = 2")]).0,
            "field `f` is defined in two files"
        );
        assert_eq!(
            error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
            (
                "`version` is declared in the directory above".to_owned(),
                Some(1),
                vec![Some(0)]
            )
        );
    }

    #[test]
    fn files_that_need_each_other() {
        let (message, input, labels) = error(&[("a.gcsl", "p = q"), ("b.gcsl", "q = p")]);
        assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
        assert_eq!((input, labels), (Some(0), vec![Some(1)]));
        let (message, _, labels) = error(&[
            ("a.gcsl", "p = q"),
            ("b.gcsl", "q = r"),
            ("c.gcsl", "r = p"),
        ]);
        assert_eq!(message, "`a.gcsl`, `b.gcsl` and `c.gcsl` need each other");
        assert_eq!(labels, [Some(1), Some(2)]);
    }

    #[test]
    fn path_order_breaks_ties_and_errors_name_their_file() {
        let scan = ws(&[("b.gcsl", "x = 1"), ("a.gcsl", "y = 2")]).unwrap();
        assert_eq!(scan.order, [0, 1]);
        assert_eq!(scan.files[0].path, "a.gcsl");
        let errors = ws(&[("a.gcsl", "x = 1"), ("b.gcsl", "y = (")]).unwrap_err();
        assert_eq!(errors[0].input, Some(1));
    }

    /// Checks the workspace of `files` and runs the file at `path`,
    /// returning its value shown, or the messages and inputs of its
    /// diagnostics.
    fn run_file(files: &[(&str, &str)], path: &str) -> Result<String, Vec<(String, Option<u32>)>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let shown = |errors: Vec<Diagnostic>| {
            errors
                .into_iter()
                .map(|error| (error.message, error.input))
                .collect::<Vec<_>>()
        };
        let mut workspace = check(&sources).map_err(shown)?;
        let index = workspace.file(path).unwrap();
        let outcome = workspace.run(index).map_err(shown)?;
        Ok(outcome.value.to_string())
    }

    /// The value of the file at `path` of the workspace of `files`.
    fn value(files: &[(&str, &str)], path: &str) -> String {
        run_file(files, path).unwrap_or_else(|errors| panic!("{path}: {errors:?}"))
    }

    /// The messages of the diagnostics of running `path` in `files`.
    fn messages(files: &[(&str, &str)], path: &str) -> Vec<String> {
        match run_file(files, path) {
            Ok(value) => panic!("{path}: unexpected value {value}"),
            Err(errors) => errors.into_iter().map(|(message, _)| message).collect(),
        }
    }

    #[test]
    fn the_summary_runs_each_file_to_its_own_value() {
        let mut files = SUMMARY.to_vec();
        // A file nothing needs is checked but never run: running it fails.
        files.push(("unused.gcsl", "def boom(): Int { [1][5] }\nnever = boom()"));
        assert_eq!(
            value(&files, "app.gcsl"),
            "{com = {x = {app = {name = \"app-1.2\"; deps = [{name = \"lib-1.2\"; \
             src = \"lib-1.2.tar\"}]}}}}"
        );
        assert_eq!(
            value(&files, "lib.gcsl"),
            "{com = {x = {lib = {name = \"lib-1.2\"; src = \"lib-1.2.tar\"}}}}"
        );
        // A `def` is a field of its file's record, as in one file.
        assert_eq!(
            value(&files, "common.gcsl"),
            "{version = \"1.2\"; tagged = fn(n: String): String}"
        );
        assert_eq!(value(&files, "workspace.gcsl"), "{}");
    }

    #[test]
    fn dotted_paths_merge_in_either_path_order() {
        for (a, b) in [("a.gcsl", "b.gcsl"), ("b.gcsl", "a.gcsl")] {
            let files = [
                ("workspace.gcsl", ""),
                (a, "com.x.one = 1"),
                (b, "com.x.two = 2"),
                ("c.gcsl", "all = com\nx = com.x"),
            ];
            // The merged node's fields come in path order of their files.
            let x = if a == "a.gcsl" {
                "{one = 1; two = 2}"
            } else {
                "{two = 2; one = 1}"
            };
            assert_eq!(
                value(&files, "c.gcsl"),
                format!("{{all = {{x = {x}}}; x = {x}}}")
            );
            assert_eq!(value(&files, a), "{com = {x = {one = 1}}}");
        }
    }

    #[test]
    fn defs_and_function_values_cross_files() {
        let files = [
            (
                "a.gcsl",
                "def twice(n: Int): Int { n * 2 }\nf = fn(n: Int): Int { n + 1 }",
            ),
            (
                "b.gcsl",
                "g = f\nh = twice(3)\nfn k(): Int { twice(4) }\nm = k()",
            ),
            ("c.gcsl", "r = g(10)"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "b.gcsl"),
            "{g = fn(n: Int): Int; h = 6; m = 8}"
        );
        assert_eq!(value(&files, "c.gcsl"), "{r = 11}");
    }

    #[test]
    fn a_contributor_reads_a_shared_node_whole() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "com.one = 1"),
            ("b.gcsl", "com.two = 2\nall = com"),
        ];
        assert_eq!(
            value(&files, "b.gcsl"),
            "{com = {two = 2}; all = {one = 1; two = 2}}"
        );
    }

    #[test]
    fn private_names_and_other_directories_are_not_shared() {
        let base = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "let p = 1\nfn q(): Int { 2 }\nx = 3\ndef d(): Int { 4 }\n...{ s = 5 }",
            ),
        ];
        for (text, message) in [
            ("y = p", "undefined variable `p`"),
            ("y = q()", "undefined variable `q`"),
            ("y = s", "undefined variable `s`"),
            ("let x = 1", "`x` is a field of this directory"),
            ("let mut x = 1", "`x` is a field of this directory"),
            ("x := 1", "`x` is a field of this directory"),
            ("let x.y = 1", "`x` is a field of this directory"),
            ("fn x(): Int { 1 }", "`x` is a field of this directory"),
            ("let d = 1", "`d` is a field of this directory"),
        ] {
            let mut files = base.to_vec();
            files.push(("b.gcsl", text));
            assert!(
                messages(&files, "b.gcsl").iter().any(|m| m == message),
                "{text}: {:?}",
                messages(&files, "b.gcsl")
            );
            // From a directory below, likewise.
            let mut files = base.to_vec();
            files.push(("sub/b.gcsl", text));
            assert!(
                messages(&files, "sub/b.gcsl").iter().any(|m| m == message),
                "sub: {text}"
            );
        }
        let mut files = base.to_vec();
        files.push(("sub/c.gcsl", "y = x + d()"));
        files.push(("other/d.gcsl", "z = 1"));
        assert_eq!(value(&files, "sub/c.gcsl"), "{y = 7}");
        files.push(("e.gcsl", "w = z"));
        assert_eq!(messages(&files, "e.gcsl"), ["undefined variable `z`"]);
        // The directory above does not see a subdirectory's declarations.
        files.pop();
        files.push(("e.gcsl", "w = y"));
        assert_eq!(messages(&files, "e.gcsl"), ["undefined variable `y`"]);
    }

    #[test]
    fn a_clash_within_a_file_is_reported_as_in_one_file() {
        // A root of the file itself: one file's errors only, never also "of
        // this directory", though the root is a declaration of the directory.
        let files = [
            ("a.gcsl", "x = 1\nlet x = 2\ny.z = 3\nfn y(): Int { 1 }"),
            ("b.gcsl", "w = 1"),
        ];
        assert_eq!(
            messages(&files, "a.gcsl"),
            [
                "`x` is a field of this file",
                "`y` is a function of this file"
            ]
        );
    }

    #[test]
    fn a_future_of_another_file_runs_once() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "p = await f\nq = await f"),
            ("b.gcsl", "r = { a = 20 }\nf = lazy { v = r.a + 1 }"),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        let index = workspace.file("a.gcsl").unwrap();
        let Value::Record(record) = workspace.run(index).unwrap().value else {
            panic!("expected a record");
        };
        // Both awaits give the one record its body built, reading the
        // field of `b.gcsl` from the frame.
        let (Some(Value::Record(p)), Some(Value::Record(q))) =
            (record.values.first(), record.values.get(1))
        else {
            panic!("expected records");
        };
        assert!(std::rc::Rc::ptr_eq(p, q));
        assert_eq!(Value::Record(p.clone()).to_string(), "{v = 21}");
    }

    #[test]
    fn spreads_and_paths_read_other_files() {
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "com.lib.name = \"n\"\ncom.lib.other = [1][0]\nr = { k = 1 }",
            ),
            ("b.gcsl", "n = com.lib.name\n...r\n...{ z = com.lib.name }"),
            ("c.gcsl", "...com.lib\nall = com"),
        ];
        assert_eq!(value(&files, "b.gcsl"), "{n = \"n\"; k = 1; z = \"n\"}");
        assert_eq!(
            value(&files, "c.gcsl"),
            "{name = \"n\"; other = 1; all = {lib = {name = \"n\"; other = 1}}}"
        );
    }

    #[test]
    fn empty_and_value_files() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x = 3"),
            ("b.gcsl", "= x + 1"),
            ("c.gcsl", "y = x"),
        ];
        assert_eq!(value(&files, "b.gcsl"), "4");
        assert_eq!(value(&files, "workspace.gcsl"), "{}");
        assert_eq!(value(&files, "c.gcsl"), "{y = 3}");
    }

    #[test]
    fn a_needed_file_runs_whole_first_and_its_error_names_it() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "def boom(): Int { [1][5] }\nx = boom()"),
            ("b.gcsl", "y = x"),
        ];
        let errors = run_file(&files, "b.gcsl").unwrap_err();
        // `a.gcsl` is input 0: the error names it, and `b.gcsl` never ran.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, Some(0));
        let erroneous = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x = 1"),
            ("z.gcsl", "y = 1 + \"\""),
        ];
        let errors = run_file(&erroneous, "a.gcsl").unwrap_err();
        // The error is `z.gcsl`'s, input 2.
        assert!(errors.iter().all(|error| error.1 == Some(2)), "{errors:?}");
        // Every failing file is reported, in path order; a file that needs
        // one that failed is not checked.
        let erroneous = [
            ("a.gcsl", "x = 1 + \"\""),
            ("b.gcsl", "y = x\nz = undefined"),
            ("c.gcsl", "w = 2 + \"\""),
            ("workspace.gcsl", ""),
        ];
        let errors = run_file(&erroneous, "workspace.gcsl").unwrap_err();
        let inputs: Vec<_> = errors.iter().map(|error| error.1).collect();
        assert_eq!(inputs, [Some(0), Some(2)], "{errors:?}");
        // A closure of another file names that file when it fails.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "def boom(): Int { [1][5] }"),
            ("b.gcsl", "y = boom()"),
        ];
        let errors = run_file(&files, "b.gcsl").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, Some(0), "{errors:?}");
    }

    /// The time to check a workspace of `count` files of ten fields each,
    /// every file reading a field of the one before, and to run its last
    /// file, which needs every other: linear in `count`. Run with
    /// `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_chain_of_files() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let sources: Vec<SourceFile> = (0..count)
                .map(|file| {
                    let mut text = if file == 0 {
                        String::from("f0_0 = 1\n")
                    } else {
                        format!("f{file}_0 = f{}_9 + 1\n", file - 1)
                    };
                    for field in 1..10 {
                        writeln!(text, "f{file}_{field} = f{file}_{} + 1", field - 1).unwrap();
                    }
                    SourceFile::new(format!("f{file:04}.gcsl"), text).unwrap()
                })
                .collect();
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let outcome = workspace.run(count - 1).unwrap();
            let total = start.elapsed();
            let last = format!("{}", outcome.value);
            assert!(last.contains(&format!("= {}", count * 10)), "{last}");
            writeln!(
                report,
                "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }
}
