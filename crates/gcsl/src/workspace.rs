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
//!
//! # Imports
//!
//! An `import` gives a file the rest of the workspace, by name. Its path is
//! absolute from the root: each segment but the last a directory, the last
//! a directory or a file's stem (`import lib.x` is `lib/x.gcsl`); the root
//! directory itself has no path. The scan resolves every import of every
//! file, mentioned or not ([`scan_with`]): a path that names nothing is "no
//! module `a.b`", with a note listing what the last directory that exists
//! has (its subdirectories, with `/`, then its file stems), or saying that
//! the directory is a workspace of its own. The library cannot see that
//! from the files alone, so [`scan_with`] and [`check_with`] take the
//! root-relative directories the caller left out as nested workspaces;
//! [`scan`] and [`check`] pass none. A directory and a file of one name are
//! an error once per pair, at the first import that names them.
//!
//! An imported name is an [`Alias`] of the file ([`File::aliases`]), naming
//! a [`Target`]: a directory's merged tree without its subdirectories, a
//! file's own tree (or, for a value file, its value), or a root of either
//! (`import x from lib`, and each root of `import * from lib`). It is a
//! private top-level name of its file: in scope throughout it, shadowed by
//! nearer names, never in its tree or record, never seen by another file;
//! one clashing with another top-level name of the file, or with a root of
//! its directory chain, is an error found by the scan. The aliases are a
//! lookup of the [`FileScope`] apart from the merged trees
//! ([`FileScope::alias`]), so a reference through one means its longest
//! declared prefix in the target's tree, as every reference does: the
//! numbers past the merged nodes are targets ([`FileScope::target`]).
//!
//! The needs through an import are per leaf ([`Scan::target_files`]): a
//! leaf, a `def` or an interior node needs the files that own it, a file
//! read whole needs that file, and a directory read whole needs every file
//! of it. A reach into a node the file itself owns or contributes to, its
//! own file or directory read whole, and a whole read of a directory one of
//! whose files needs this one, are the one-file form of the cycle error,
//! "`app.gcsl` needs itself"; files importing each other are the cycle
//! error of [Order](#order).
//!
//! A reference that ends on a cell reads that cell's slot: a `def`'s is its
//! function's slot, since the merged node of a `def` is never written. A
//! record read whole (a directory, a file, or a file's part of a shared
//! node) is assembled by the importing file, once, into a private slot of
//! its own, before its first item ([`crate::check#imports`]). A value file
//! that some file imports has a cell of its own after the merged nodes
//! ([`Scan::value_slot`]), which the file stores its result into as the
//! last node of its top-level block.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::mem;

use crate::Outcome;
use crate::ast::{Ast, ImportDecl, ImportForm, ImportMember, ItemId, ItemKind, PathSegment};
use crate::check::{CheckOutput, Checker, Scope, Slot, Types};
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
    /// The names it imports, in source order (a `*` gives one per root).
    pub aliases: Vec<Alias>,
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

/// What an imported name aliases (see the module documentation,
/// "Imports").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A directory's merged tree, a synthetic node whose children are the
    /// directory's roots; read whole, the record of every root.
    Dir(u32),
    /// A file's own tree, a synthetic node whose children are its roots;
    /// read whole, its own record, or its value if it is a value file.
    File(u32),
    /// A node of a file's own tree that is not the whole of its merged node
    /// (an interior node other files contribute to): the file and the
    /// file's own node. Read whole, the file's part of it.
    Part {
        /// The file.
        file: u32,
        /// Its own node.
        node: u32,
    },
    /// A merged node, a cell of the workspace, reached through an import.
    Node(u32),
}

/// A name a file imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The name bound.
    pub name: Name,
    /// Where it is bound: the alias, the member, the `*`, or the path's
    /// last segment.
    pub span: Span,
    /// The `import` item.
    pub item: ItemId,
    /// What it names.
    pub target: Target,
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
    /// For each file, the number of the own nodes of the files before it:
    /// where its [`Target::Part`]s start among the targets' numbers.
    part_base: Vec<u32>,
    /// The value files some file imports, each with the slot of its value's
    /// cell and its name (the file's stem), in path order. Their slots
    /// follow the merged nodes'.
    values: Vec<(u32, Name)>,
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

    /// The number of `target` among every target of the workspace: the
    /// directories, then the files, then the files' own nodes, then the
    /// merged nodes.
    fn target_number(&self, target: Target) -> u32 {
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        let parts = self.parts();
        match target {
            Target::Dir(dir) => dir,
            Target::File(file) => dirs.saturating_add(file),
            Target::Part { file, node } => {
                let base = self.part_base.get(file as usize).copied().unwrap_or(0);
                dirs.saturating_add(files)
                    .saturating_add(base)
                    .saturating_add(node)
            }
            Target::Node(node) => dirs
                .saturating_add(files)
                .saturating_add(parts)
                .saturating_add(node),
        }
    }

    /// The target of number `number` (see [`Scan::target_number`]).
    fn target_of(&self, number: u32) -> Option<Target> {
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        let Some(rest) = number.checked_sub(dirs) else {
            return Some(Target::Dir(number));
        };
        let Some(rest) = rest.checked_sub(files) else {
            return Some(Target::File(rest));
        };
        let Some(node) = rest.checked_sub(self.parts()) else {
            // The last file whose parts start at or before `rest`.
            let after = self.part_base.partition_point(|&base| base <= rest);
            let file = count(after.checked_sub(1)?);
            let base = self.part_base.get(file as usize).copied()?;
            return Some(Target::Part {
                file,
                node: rest.checked_sub(base)?,
            });
        };
        ((node as usize) < self.nodes.len()).then_some(Target::Node(node))
    }

    /// The slot of the cell of the value of `file`, a value file some file
    /// imports: its owner stores its result there, and an importer reads
    /// it ([`Target::File`] of a value file). `None` for any other file.
    #[must_use]
    pub fn value_slot(&self, file: u32) -> Option<Slot> {
        self.value_cell(file).map(Slot::at)
    }

    /// The number of the cell of the value of `file`, a value file some
    /// file imports, among the workspace's cells ([`Scan::cells`]): after
    /// the merged nodes. Like a merged node's, its slot is [`Slot::at`] its
    /// number. `None` for any other file.
    fn value_cell(&self, file: u32) -> Option<u32> {
        let at = self.values.iter().position(|&(f, _)| f == file)?;
        Some(count(self.nodes.len()).saturating_add(count(at)))
    }

    /// The number of the workspace's cells: the merged nodes, then the
    /// imported value files' values. Every other slot is a file's own.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.nodes.len() + self.values.len()
    }

    /// The names of the cells' slots, by slot.
    fn cell_names(&self) -> Vec<Name> {
        self.nodes
            .iter()
            .map(|node| node.name)
            .chain(self.values.iter().map(|&(_, name)| name))
            .collect()
    }

    /// The number of every file's own nodes.
    fn parts(&self) -> u32 {
        self.part_base
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_add(self.files.last().map_or(0, |file| count(file.global.len())))
    }

    /// The child of the target `target` named `name`, as a reference
    /// through an import means it.
    fn target_child(&self, target: Target, name: Name) -> Option<Target> {
        match target {
            Target::Dir(dir) => self
                .by_name
                .get(&(dir, ROOT, name))
                .map(|&n| Target::Node(n)),
            Target::File(file) => {
                let dir = self.files.get(file as usize)?.dir;
                let merged = *self.by_name.get(&(dir, ROOT, name))?;
                self.in_file(file, merged)
            }
            Target::Part { file, node } => {
                let entry = self.files.get(file as usize)?;
                let parent = *entry.global.get(node as usize)?;
                let merged = *self.by_name.get(&(entry.dir, parent, name))?;
                self.in_file(file, merged)
            }
            Target::Node(node) => {
                let dir = self.nodes.get(node as usize)?.dir;
                self.by_name
                    .get(&(dir, node, name))
                    .map(|&n| Target::Node(n))
            }
        }
    }

    /// The merged node `merged` as a node of `file`'s own tree: the merged
    /// node when it is wholly `file`'s, else `file`'s part of it; `None`
    /// when `file` has nothing at it.
    fn in_file(&self, file: u32, merged: u32) -> Option<Target> {
        let entry = self.nodes.get(merged as usize)?;
        if entry.leaf == Some(file) || entry.contributors == [file] {
            return Some(Target::Node(merged));
        }
        let own = self
            .files
            .get(file as usize)?
            .global
            .iter()
            .position(|&g| g == merged)?;
        Some(Target::Part {
            file,
            node: count(own),
        })
    }

    /// The files a reference through an import to `target` needs: every
    /// file of a directory, the file of a file or a part, a merged node's
    /// leaf's file or its contributors.
    #[must_use]
    pub fn target_files(&self, target: Target) -> Vec<u32> {
        match target {
            Target::Dir(dir) => (0u32..)
                .zip(&self.files)
                .filter(|(_, file)| file.dir == dir)
                .map(|(index, _)| index)
                .collect(),
            Target::File(file) | Target::Part { file, .. } => vec![file],
            Target::Node(node) => self.nodes.get(node as usize).map_or_else(Vec::new, |n| {
                n.leaf
                    .map_or_else(|| n.contributors.clone(), |leaf| vec![leaf])
            }),
        }
    }

    /// How `target` is shown in a diagnostic: a directory's or a file's
    /// module path, a node's dotted path.
    fn show_target(&self, target: Target) -> String {
        let module = |path: &str| path.trim_end_matches(".gcsl").replace('/', ".");
        match target {
            Target::Dir(dir) => self
                .dirs
                .get(dir as usize)
                .map_or_else(String::new, |d| module(&d.path)),
            Target::File(file) => self
                .files
                .get(file as usize)
                .map_or_else(String::new, |f| module(&f.path)),
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.global.get(node as usize))
                .map_or_else(String::new, |&merged| self.show(merged)),
            Target::Node(node) => self.show(node),
        }
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
    /// What each name the file imports names.
    aliases: HashMap<Name, Target>,
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
            aliases: file.aliases.iter().map(|a| (a.name, a.target)).collect(),
        }
    }

    /// Where the numbers of targets reached through imports start: after
    /// the file's own nodes and the merged nodes.
    fn targets_start(&self) -> u32 {
        count(self.global.len()).saturating_add(count(self.scan.nodes.len()))
    }

    /// What the scope's number `node` is when it is reached through an
    /// import, `None` for the file's own nodes and the merged nodes of its
    /// directory chain ([`FileScope::merged`]). A reference through an
    /// import resolves to one of these, and so does a mention of it in
    /// [`Mentions::foreign`]; a [`Target::Node`] is a cell like any merged
    /// node, read by its slot.
    #[must_use]
    pub fn target(&self, node: u32) -> Option<Target> {
        self.scan.target_of(node.checked_sub(self.targets_start())?)
    }

    /// The import of the name `name`, if the file imports it.
    #[must_use]
    pub fn alias(&self, name: Name) -> Option<Target> {
        self.aliases.get(&name).copied()
    }

    /// The scope's number of `target`.
    fn target_number(&self, target: Target) -> u32 {
        self.targets_start()
            .saturating_add(self.scan.target_number(target))
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
            Some(other) => (node < self.targets_start()).then_some(other),
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
            .or_else(|| Some(self.target_number(self.alias(name)?)))
    }

    fn child(&self, node: u32, name: Name) -> Option<u32> {
        if let Some(target) = self.target(node) {
            let child = self.scan.target_child(target, name)?;
            return Some(self.target_number(child));
        }
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
    scan_with(sources, &[])
}

/// [`scan`], told the directories left out of the workspace as workspaces
/// of their own, each by its path relative to the root without a trailing
/// `/`: an import of one is "no module" with a note saying so.
///
/// # Errors
///
/// As [`scan`]; with the conflicts, every import that names nothing or
/// clashes with a name of its file or directory chain, and with the cycles,
/// every reference through an import that reaches back into its own file.
pub fn scan_with(sources: &[SourceFile], nested: &[String]) -> Result<Scan, Vec<Diagnostic>> {
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
        part_base: Vec::new(),
        values: Vec::new(),
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
            aliases: Vec::new(),
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
    let mut base = 0u32;
    scan.part_base = scan
        .files
        .iter()
        .map(|file| {
            let start = base;
            base = base.saturating_add(count(file.global.len()));
            start
        })
        .collect();
    imports(&mut scan, &dirs, nested, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    value_cells(&mut scan);
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
    let mentioned = find_needs(&mut scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
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

/// What [`imports`] looks modules up in.
struct Modules<'a> {
    /// The directories by path.
    dirs: &'a HashMap<String, u32>,
    /// The files by path.
    files: HashMap<&'a str, u32>,
    /// The directories that are workspaces of their own.
    nested: &'a [String],
}

/// Resolves every file's imports into [`File::aliases`], reporting each
/// path that names nothing, each directory and file of one name (once per
/// pair, at the first import naming it), each member that is not a root,
/// and each imported name that clashes with a name of its file or a root of
/// its directory chain.
fn imports(
    scan: &mut Scan,
    dirs: &HashMap<String, u32>,
    nested: &[String],
    errors: &mut Vec<Diagnostic>,
) {
    let modules = Modules {
        dirs,
        files: (0u32..)
            .zip(&scan.files)
            .map(|(index, file)| (file.path.as_str(), index))
            .collect(),
        nested,
    };
    let mut both: Vec<String> = Vec::new();
    let mut found = Vec::with_capacity(scan.files.len());
    for (index, file) in (0u32..).zip(&scan.files) {
        let mut aliases = Vec::new();
        let mut own_errors = Vec::new();
        for &item in file.ast.items(file.ast.root()) {
            let ItemKind::Import(id) = file.ast.item(item).kind else {
                continue;
            };
            let decl = file.ast.import_decl(id);
            let path = file.ast.path(decl.path);
            let target = match module(scan, &modules, path, &mut both) {
                Ok(target) => target,
                Err(error) => {
                    own_errors.extend(error);
                    continue;
                }
            };
            members(
                scan,
                file,
                item,
                decl,
                target,
                &mut aliases,
                &mut own_errors,
            );
        }
        clashes(scan, index, &aliases, &mut own_errors);
        errors.extend(own_errors.into_iter().map(|e| e.in_input(index)));
        found.push(aliases);
    }
    for (file, aliases) in scan.files.iter_mut().zip(found) {
        file.aliases = aliases;
    }
}

/// What the module path `path` names, or the error that it names nothing
/// (`None` for a directory and file of one name already reported).
fn module(
    scan: &Scan,
    modules: &Modules,
    path: &[PathSegment],
    both: &mut Vec<String>,
) -> Result<Target, Option<Diagnostic>> {
    let written = match (path.first(), path.last()) {
        (Some(first), Some(last)) => first.span.join(last.span),
        _ => return Err(None),
    };
    let shown: Vec<&str> = path
        .iter()
        .filter_map(|segment| Some(scan.names.get(segment.name()?)))
        .collect();
    let mut at = String::new();
    for (position, name) in shown.iter().enumerate() {
        let candidate = if at.is_empty() {
            (*name).to_owned()
        } else {
            format!("{at}/{name}")
        };
        let dir = modules.dirs.get(&candidate).copied();
        let last = position + 1 == shown.len();
        let file = last
            .then(|| {
                modules
                    .files
                    .get(format!("{candidate}.gcsl").as_str())
                    .copied()
            })
            .flatten();
        match (dir, file) {
            (Some(_), Some(_)) => {
                if both.contains(&candidate) {
                    return Err(None);
                }
                both.push(candidate);
                return Err(Some(Diagnostic::error(
                    format!("`{name}` is both a directory and a file"),
                    written,
                )));
            }
            (None, Some(file)) => return Ok(Target::File(file)),
            (Some(dir), None) if last => return Ok(Target::Dir(dir)),
            (Some(_), None) => at = candidate,
            (None, None) => {
                let note = if modules.nested.contains(&candidate) {
                    format!("`{candidate}/` is a workspace of its own")
                } else {
                    let listed = contents(scan, modules.dirs.get(&at).copied());
                    if at.is_empty() {
                        format!("the workspace has {listed}")
                    } else {
                        format!("`{at}/` has {listed}")
                    }
                };
                return Err(Some(
                    Diagnostic::error(format!("no module `{}`", shown.join(".")), written)
                        .with_note(note),
                ));
            }
        }
    }
    Err(None)
}

/// Gives a cell to the value of each value file (one whose outermost block
/// ends in a result) that some file imports whole ([`Scan::value_slot`]).
fn value_cells(scan: &mut Scan) {
    let mut imported: Vec<u32> = scan
        .files
        .iter()
        .flat_map(|file| &file.aliases)
        .filter_map(|alias| match alias.target {
            Target::File(file) => Some(file),
            _ => None,
        })
        .filter(|&file| {
            scan.files.get(file as usize).is_some_and(|f| {
                f.ast
                    .items(f.ast.root())
                    .last()
                    .is_some_and(|&item| matches!(f.ast.item(item).kind, ItemKind::Result(_)))
            })
        })
        .collect();
    imported.sort_unstable();
    imported.dedup();
    for file in imported {
        let stem = scan.files.get(file as usize).map_or("", |f| {
            let name = f.path.rsplit('/').next().unwrap_or("");
            name.strip_suffix(".gcsl").unwrap_or(name)
        });
        let name = scan.names.intern(stem);
        scan.values.push((file, name));
    }
}

/// What directory `dir` holds, as a "no module" note lists it: its
/// directories with `/`, then its files' stems, each in path order.
fn contents(scan: &Scan, dir: Option<u32>) -> String {
    let Some(dir) = dir else {
        return "nothing".to_owned();
    };
    let last = |path: &str| path.rsplit('/').next().unwrap_or("").to_owned();
    let mut subdirs: Vec<String> = (0u32..)
        .zip(&scan.dirs)
        .filter(|(_, d)| d.parent == Some(dir))
        .map(|(_, d)| format!("`{}/`", last(&d.path)))
        .collect();
    subdirs.sort_unstable();
    let stems = scan
        .files
        .iter()
        .filter(|file| file.dir == dir)
        .map(|file| format!("`{}`", last(&file.path).trim_end_matches(".gcsl")));
    subdirs.extend(stems);
    if subdirs.is_empty() {
        "nothing".to_owned()
    } else {
        subdirs.join(", ")
    }
}

/// Binds the names the import `item` of `file`, `decl`, makes of `target`,
/// what its path names, into `aliases`.
fn members(
    scan: &Scan,
    file: &File,
    item: ItemId,
    decl: &ImportDecl,
    target: Target,
    aliases: &mut Vec<Alias>,
    errors: &mut Vec<Diagnostic>,
) {
    let path = file.ast.path(decl.path);
    match decl.form {
        ImportForm::Whole(alias) => {
            let bound = alias.map(|a| (a.name, a.span)).or_else(|| {
                let last = path.last()?;
                Some((last.name()?, last.span))
            });
            if let Some((name, span)) = bound {
                aliases.push(Alias {
                    name,
                    span,
                    item,
                    target,
                });
            }
        }
        ImportForm::Members(range) => {
            for member in file.ast.import_members(range) {
                match *member {
                    ImportMember::Named { name, alias } => {
                        let bound = alias.unwrap_or(name);
                        match scan.target_child(target, name.name) {
                            Some(child) => aliases.push(Alias {
                                name: bound.name,
                                span: bound.span,
                                item,
                                target: child,
                            }),
                            None => errors.push(Diagnostic::error(
                                format!(
                                    "`{}` has no `{}`",
                                    scan.show_target(target),
                                    scan.names.get(name.name)
                                ),
                                name.span,
                            )),
                        }
                    }
                    ImportMember::All(star) => {
                        for root in target_roots(scan, target) {
                            if let Some(child) = scan.target_child(target, root) {
                                aliases.push(Alias {
                                    name: root,
                                    span: star,
                                    item,
                                    target: child,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The names of the roots of `target`, a directory or a file, in order.
fn target_roots(scan: &Scan, target: Target) -> Vec<Name> {
    match target {
        Target::Dir(dir) => scan
            .nodes
            .iter()
            .filter(|node| node.dir == dir && node.parent.is_none())
            .map(|node| node.name)
            .collect(),
        Target::File(file) => scan.files.get(file as usize).map_or_else(Vec::new, |f| {
            f.mentions
                .tree
                .roots
                .iter()
                .filter_map(|&root| f.mentions.tree.nodes.get(root as usize).map(|n| n.name))
                .collect()
        }),
        Target::Part { .. } | Target::Node(_) => Vec::new(),
    }
}

/// Reports each name of `aliases`, the imports of file `index`, that
/// clashes with another import, a top-level `fn`, `let`, `let mut`, `:=` or
/// field root of the file, or a root of its directory chain.
fn clashes(scan: &Scan, index: u32, aliases: &[Alias], errors: &mut Vec<Diagnostic>) {
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    let ast = &file.ast;
    // The file's own top-level names, first declaration of each.
    let mut declared: Vec<(Name, Span)> = Vec::new();
    for &item in ast.items(ast.root()) {
        let (ItemKind::Let { path, .. }
        | ItemKind::Rebind { path, .. }
        | ItemKind::Field { path, .. }) = ast.item(item).kind
        else {
            continue;
        };
        if let Some(first) = ast.path(path).first()
            && let Some(name) = first.name()
            && !declared.iter().any(|&(n, _)| n == name)
        {
            declared.push((name, first.span));
        }
    }
    let star = |alias: &Alias| {
        let ItemKind::Import(id) = ast.item(alias.item).kind else {
            return false;
        };
        matches!(ast.import_decl(id).form, ImportForm::Members(range)
            if ast.import_members(range).iter().any(|m| m == &ImportMember::All(alias.span)))
    };
    for (position, alias) in aliases.iter().enumerate() {
        let shown = scan.names.get(alias.name);
        let earlier = aliases
            .get(..position)
            .and_then(|before| before.iter().find(|a| a.name == alias.name))
            .map(|a| (a.span, "imported here"));
        let other = earlier.or_else(|| {
            declared
                .iter()
                .find(|&&(n, _)| n == alias.name)
                .map(|&(_, span)| (span, "declared here"))
        });
        if let Some((span, label)) = other {
            let message = format!("`{shown}` is imported in this file");
            errors.push(if star(alias) || label == "imported here" {
                Diagnostic::error(message, alias.span).with_context(span, label)
            } else {
                Diagnostic::error(message, span).with_context(alias.span, "imported here")
            });
            continue;
        }
        let above = scan
            .chain(file.dir)
            .find_map(|dir| scan.by_name.get(&(dir, ROOT, alias.name)));
        if above.is_some() {
            errors.push(Diagnostic::error(
                format!("`{shown}` is a field of this directory"),
                alias.span,
            ));
        }
    }
}

/// The error that file `index` reaches `target`, which it computes itself,
/// through an import at `at`: the one-file form of a cycle.
fn needs_itself(scan: &Scan, index: u32, target: Target, at: Span) -> Diagnostic {
    let label = format!(
        "`{}` is computed when this file runs",
        scan.show_target(target)
    );
    needs_itself_because(scan, index, label, at)
}

/// The error that file `index` needs itself through the reference at `at`,
/// with `label` saying why.
fn needs_itself_because(scan: &Scan, index: u32, label: String, at: Span) -> Diagnostic {
    let path = scan
        .files
        .get(index as usize)
        .map_or("", |f| f.path.as_str());
    Diagnostic::error(format!("`{path}` needs itself"), at)
        .with_label(label)
        .with_note(
            "a file runs as a whole before a file that needs it, so a file cannot need itself",
        )
        .in_input(index)
}

/// Reports each whole read of a directory through an import (`whole_dirs`:
/// the reading file, the directory's files, the target, the reference) one
/// of whose files needs the reading file back, in the needs `all` found so
/// far: the one-file form of a cycle too, "`app.gcsl` needs itself", at the
/// read, once per file (`itselves` saying which files have had theirs). The
/// needs that would close a cycle of two files with it are dropped, so that
/// the cycle is reported once.
fn whole_reads_needing_back(
    scan: &Scan,
    whole_dirs: Vec<(u32, Vec<u32>, Target, Span)>,
    all: &mut [HashMap<u32, (Span, String)>],
    itselves: &mut [bool],
    errors: &mut Vec<Diagnostic>,
) {
    for (index, owners, target, span) in whole_dirs {
        let back: Vec<u32> = owners
            .into_iter()
            .filter(|&o| all.get(o as usize).is_some_and(|f| f.contains_key(&index)))
            .collect();
        if back.is_empty() {
            continue;
        }
        if let Some(itself) = itselves.get_mut(index as usize)
            && !*itself
        {
            *itself = true;
            let file = back
                .first()
                .and_then(|&o| scan.files.get(o as usize))
                .map_or("", |f| f.path.as_str());
            let label = format!(
                "`{}` is read whole, and its file `{file}` needs this file",
                scan.show_target(target)
            );
            errors.push(needs_itself_because(scan, index, label, span));
        }
        if let Some(first) = all.get_mut(index as usize) {
            for owner in back {
                first.remove(&owner);
            }
        }
    }
}

/// Fills [`Scan::needs`], and returns for each file the first mention, in
/// source order, of a declaration of each file it needs, with what it
/// shows.
fn find_needs(scan: &mut Scan, errors: &mut Vec<Diagnostic>) -> Vec<HashMap<u32, (Span, String)>> {
    let mut all = Vec::with_capacity(scan.files.len());
    let mut itselves = Vec::with_capacity(scan.files.len());
    // Each whole read of a directory through an import: the reading file,
    // the directory's files, the target and the reference.
    let mut whole_dirs: Vec<(u32, Vec<u32>, Target, Span)> = Vec::new();
    for (index, file) in (0u32..).zip(&scan.files) {
        let scope = FileScope::new(scan, file);
        let mut itself = false;
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
                    // Through an import: what it reaches needs its files,
                    // and reaching into this file is a cycle of one file.
                    Need::Field(n) => {
                        let Some(target) = scope.target(n) else {
                            nodes.push((n.saturating_sub(own_nodes), span));
                            continue;
                        };
                        let owners = scan.target_files(target);
                        if owners.contains(&index) {
                            if !itself {
                                itself = true;
                                errors.push(needs_itself(scan, index, target, span));
                            }
                            continue;
                        }
                        for &owner in &owners {
                            record(owner, span, &|| scan.show_target(target));
                        }
                        if matches!(target, Target::Dir(_)) {
                            whole_dirs.push((index, owners, target, span));
                        }
                    }
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
        itselves.push(itself);
    }
    whole_reads_needing_back(scan, whole_dirs, &mut all, &mut itselves, errors);
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
    /// The file's index.
    index: u32,
    /// The type of each cell of the workspace, by its slot: `Error` until
    /// the file that computes it is checked.
    cell_types: &'a [TyId],
    /// The slot each cell of the workspace is read from, by its number: its
    /// own, but a `def`'s function's slot, known once its file is checked.
    cell_slots: &'a [Slot],
    /// For each cell that is a `def`'s, by its number, which parameters of
    /// its function are present because its body measures them, known once
    /// its file is checked: what a call of it through an import needs for
    /// the call-site form of the implicit-await error.
    cell_measured: &'a [Option<Box<[bool]>>],
}

/// What a [`Context`] reads cells by: the slot each cell is read from, and
/// for each `def`'s cell which parameters its body measures.
type CellReads<'a> = (&'a [Slot], &'a [Option<Box<[bool]>>]);

impl<'a> Context<'a> {
    /// The context of file `index` of `scan`.
    fn new(
        scan: &'a Scan,
        index: u32,
        cell_types: &'a [TyId],
        (cell_slots, cell_measured): CellReads<'a>,
    ) -> Option<Self> {
        let file = scan.files.get(index as usize)?;
        Some(Self {
            scan,
            file,
            scope: FileScope::new(scan, file),
            index,
            cell_types,
            cell_slots,
            cell_measured,
        })
    }

    /// Which parameters of the `def` whose merged node is `node` its body
    /// measures, if `node` is a `def`'s cell of a file already checked.
    pub(crate) fn measured(&self, node: u32) -> Option<&[bool]> {
        self.cell_measured.get(node as usize)?.as_deref()
    }

    /// The slot the cell of the merged node `node` is read from: its own,
    /// or for a `def` its function's (whose merged node's slot is never
    /// written).
    pub(crate) fn cell_slot(&self, node: u32) -> Slot {
        self.cell_slots
            .get(node as usize)
            .copied()
            .unwrap_or(Slot::at(node))
    }

    /// The cell the file's result is stored in, if it is a value file some
    /// file imports.
    pub(crate) fn value_slot(&self) -> Option<Slot> {
        self.scan.value_slot(self.index)
    }

    /// The number of the cell of the value of `target`, among the
    /// workspace's cells, if it is a value file's: its slot is [`Slot::at`]
    /// that number, and [`Context::cell_type`] of it is its type.
    pub(crate) fn value_of(&self, target: Target) -> Option<u32> {
        match target {
            Target::File(file) => self.scan.value_cell(file),
            _ => None,
        }
    }

    /// The fields of the record of `target`, a directory, a file or a part
    /// read whole through an import: each child's name and target, in the
    /// order of the directory's or the file's tree.
    pub(crate) fn target_fields(&self, target: Target) -> Vec<(Name, Target)> {
        let names = match target {
            Target::Part { file, node } => self
                .scan
                .files
                .get(file as usize)
                .and_then(|f| {
                    let tree = &f.mentions.tree;
                    let children = &tree.nodes.get(node as usize)?.children;
                    Some(
                        children
                            .iter()
                            .filter_map(|&c| tree.nodes.get(c as usize).map(|n| n.name))
                            .collect(),
                    )
                })
                .unwrap_or_default(),
            other => target_roots(self.scan, other),
        };
        names
            .into_iter()
            .filter_map(|name| Some((name, self.scan.target_child(target, name)?)))
            .collect()
    }

    /// A name for the slot of the record of `target`: the name the file
    /// imports it as, or a part's last name.
    pub(crate) fn target_name(&self, target: Target) -> Option<Name> {
        if let Some(alias) = self.file.aliases.iter().find(|a| a.target == target) {
            return Some(alias.name);
        }
        match target {
            Target::Part { file, node } => self
                .scan
                .files
                .get(file as usize)
                .and_then(|f| f.mentions.tree.nodes.get(node as usize))
                .map(|n| n.name),
            _ => self.file.aliases.first().map(|a| a.name),
        }
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

    /// The type of the cell numbered `node` (a merged node, or an imported
    /// value file's value, [`Context::value_of`]), as its file's check
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
    /// The slot each cell is read from, by merged node: its own, or for a
    /// `def` its function's (see [`Context::cell_slot`]). Kept for the
    /// tests' check that every cell an import reaches is stored by its
    /// owner.
    #[cfg(test)]
    cell_slots: Vec<Slot>,
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
    check_with(sources, &[])
}

/// [`check`], told the directories left out of the workspace as
/// workspaces of their own (see [`scan_with`]).
///
/// # Errors
///
/// As [`check`].
pub fn check_with(sources: &[SourceFile], nested: &[String]) -> Result<Workspace, Vec<Diagnostic>> {
    let mut scan = scan_with(sources, nested)?;
    let mut names = mem::take(&mut scan.names);
    let mut types = TypeTable::new();
    let mut cell_types = vec![TyId::ERROR; scan.cells()];
    let mut cell_slots: Vec<Slot> = (0..count(scan.cells())).map(Slot::at).collect();
    let mut cell_measured: Vec<Option<Box<[bool]>>> = vec![None; scan.cells()];
    let mut slot_names: Vec<Name> = scan.cell_names();
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
        let reads = (&cell_slots[..], &cell_measured[..]);
        let compiled = Context::new(&scan, index, &cell_types, reads).map(|context| {
            let output = Checker::in_workspace(&file.ast, scope, &mut types, &context).check();
            output.and_then(|output| session::lower_checked(&file.ast, output, &types, index))
        });
        // What the file declares, committed for the files after it.
        let committed = match &compiled {
            Some(Ok((_, output))) => computed_cells(&scan, index, output),
            _ => Vec::new(),
        };
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
                slot_names = scan.cell_names();
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
        for (global, slot, ty, measured) in committed {
            set(&mut cell_types, global, ty);
            set(&mut cell_slots, global, slot);
            set(&mut cell_measured, global, measured);
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
        #[cfg(test)]
        cell_slots,
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

/// The cells file `index` computes, now that its check gave `output`: each
/// one's number, the slot it is read from (a `def`'s is its function's) and
/// its type, with the cell of its value if it is an imported value file.
fn computed_cells(scan: &Scan, index: u32, output: &CheckOutput) -> Vec<ComputedCell> {
    let file = scan.files.get(index as usize);
    let global = file.map_or(&[][..], |file| &file.global);
    let slots = output.checked.resolution.field_cells();
    let mut cells = Vec::new();
    for (own, (&global, &ty)) in global.iter().zip(&output.cell_types).enumerate() {
        if scan.computed_by(global) == Some(index) {
            // A `def`'s cell is read from its function's slot.
            let slot = slots.get(own).copied().unwrap_or(Slot::at(global));
            let measured = output.scope.measured_at(slot).map(Box::from);
            cells.push((global, slot, ty, measured));
        }
    }
    // An imported value file's value is a cell of the workspace.
    if let Some(cell) = scan.value_cell(index) {
        cells.push((cell, Slot::at(cell), output.ty, None));
    }
    cells
}

/// A cell a file computes, as [`computed_cells`] gives it: its number, the
/// slot it is read from, its type, and for a `def` which parameters its body
/// measures.
type ComputedCell = (u32, Slot, TyId, Option<Box<[bool]>>);

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
    use crate::ir::IrKind;

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
        assert_imports_stored(&workspace);
        let index = workspace.file(path).unwrap();
        let outcome = workspace.run(index).map_err(shown)?;
        Ok(outcome.value.to_string())
    }

    /// Asserts that every cell an import of `workspace` reaches, directly
    /// or as a field of a record read whole, is stored by a node of its
    /// owner's top-level block: slice 24's schedule test sees one file
    /// only, and a cell no owner writes would read as `{}`.
    fn assert_imports_stored(workspace: &Workspace) {
        let scan = &workspace.scan;
        let types = vec![TyId::ERROR; scan.cells()];
        for (index, file) in (0u32..).zip(&scan.files) {
            let reads = (&workspace.cell_slots[..], &[][..]);
            let context = Context::new(scan, index, &types, reads).unwrap();
            let mut pending: Vec<Target> = file
                .mentions
                .foreign
                .iter()
                .flatten()
                .filter_map(|&(need, _)| match need {
                    Need::Field(n) => context.scope.target(n),
                    Need::Function(_) => None,
                })
                .collect();
            while let Some(target) = pending.pop() {
                let cell = match target {
                    Target::Node(node) => Some(context.cell_slot(node)),
                    other => context.value_of(other).map(Slot::at),
                };
                let Some(slot) = cell else {
                    pending.extend(context.target_fields(target).into_iter().map(|(_, t)| t));
                    continue;
                };
                let owners = scan.target_files(target);
                let stored = owners.iter().any(|&owner| {
                    let Some(Some(ir)) = workspace.irs.get(owner as usize) else {
                        return false;
                    };
                    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                        return false;
                    };
                    ir.items(items)
                        .iter()
                        .any(|&item| matches!(ir.get(item).kind, IrKind::Store(s, _) if s == slot))
                });
                assert!(
                    stored,
                    "{}: `{}` is not stored by {owners:?}",
                    file.path,
                    scan.show_target(target)
                );
            }
        }
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

    /// [`benchmark_a_chain_of_files`] through imports: each file in a
    /// directory of its own, importing the previous file's directory and
    /// reading one leaf through it. Linear in `count`, as the chain is.
    /// Run with `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_chain_of_imports() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let sources: Vec<SourceFile> = (0..count)
                .map(|file| {
                    let mut text = if file == 0 {
                        String::from("f0_0 = 1\n")
                    } else {
                        let before = file - 1;
                        format!("import d{before:04}\nf{file}_0 = d{before:04}.f{before}_9 + 1\n")
                    };
                    for field in 1..10 {
                        writeln!(text, "f{file}_{field} = f{file}_{} + 1", field - 1).unwrap();
                    }
                    SourceFile::new(format!("d{file:04}/f.gcsl"), text).unwrap()
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

    /// The files of the imports summary, in path order.
    const IMPORTS: [(&str, &str); 6] = [
        (
            "app/main.gcsl",
            "import lib\nimport tools.build as builder\nimport com as libs from lib\n\
             import lib.y\ncom.x.app.deps = [lib.com.x.lib, libs.y.lib]\n\
             com.x.app.tool = builder\ncom.x.app.all = y.com.y.lib.name",
        ),
        ("common.gcsl", "version = \"1.2\""),
        ("lib/x.gcsl", "com.x.lib.name = \"lib-{version}\""),
        ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
        ("tools/build.gcsl", "= \"build-{version}\""),
        ("workspace.gcsl", ""),
    ];

    /// The scan of `files` told that `nested` are workspaces of their own.
    fn ws_nested(files: &[(&str, &str)], nested: &[&str]) -> Result<Scan, Vec<Diagnostic>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let nested: Vec<String> = nested.iter().map(|&n| n.to_owned()).collect();
        scan_with(&sources, &nested)
    }

    /// The message and note of the single error of scanning `files`.
    fn noted(files: &[(&str, &str)], nested: &[&str]) -> (String, Option<String>) {
        let errors = ws_nested(files, nested).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        (errors[0].message.clone(), errors[0].notes.first().cloned())
    }

    #[test]
    fn imports_resolve_to_their_targets() {
        let scan = ws(&IMPORTS).unwrap();
        let main = &scan.files[0];
        let named: Vec<(&str, Target)> = main
            .aliases
            .iter()
            .map(|a| (scan.names.get(a.name), a.target))
            .collect();
        let lib = scan.dirs.iter().position(|d| d.path == "lib").unwrap();
        let com = scan.by_name[&(count(lib), ROOT, scan.names.lookup("com").unwrap())];
        assert_eq!(
            named,
            [
                ("lib", Target::Dir(count(lib))),
                ("builder", Target::File(4)),
                ("libs", Target::Node(com)),
                ("y", Target::File(3)),
            ]
        );
        // Needs per leaf: x, y and build, each through an import; `common`
        // only through them.
        assert_eq!(scan.needs[0], [2, 3, 4]);
        assert_eq!(scan.needed(0), [1, 2, 3, 4, 0]);
        assert_eq!(scan.value_slot(4), Some(Slot::at(count(scan.nodes.len()))));
        assert_eq!(scan.value_slot(1), None);
    }

    #[test]
    fn a_reference_through_an_import_needs_only_its_leaf() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport lib.y\nimport tools\na = lib.com.x.lib.name\nb = y";
        let scan = ws(&files).unwrap();
        // `lib.com.x.lib.name` is x's leaf; `y` whole is y; nothing reads
        // `tools`, so it needs nothing.
        assert_eq!(scan.needs[0], [2, 3]);
        files[0].1 = "import lib\na = lib";
        assert_eq!(ws(&files).unwrap().needs[0], [2, 3]);
        files[0].1 = "import lib\na = lib.com.y";
        assert_eq!(ws(&files).unwrap().needs[0], [3]);
    }

    #[test]
    fn a_file_part_and_a_whole_node_are_told_apart() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib.y\na = y.com\nb = y.com.y";
        let scan = ws(&files).unwrap();
        let main = &scan.files[0];
        let scope = FileScope::new(&scan, main);
        let targets: Vec<Target> = main
            .mentions
            .foreign
            .iter()
            .flatten()
            .filter_map(|&(need, _)| match need {
                Need::Field(n) => scope.target(n),
                Need::Function(_) => None,
            })
            .collect();
        // `com` of `lib/` is shared with x: y's part of it. `com.y` is y's
        // alone: the merged node.
        assert!(
            matches!(targets[0], Target::Part { file: 3, .. }),
            "{targets:?}"
        );
        assert!(matches!(targets[1], Target::Node(_)), "{targets:?}");
    }

    #[test]
    fn no_module_says_what_there_is() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import nope";
        let (message, note) = noted(&files, &["vendor"]);
        assert_eq!(message, "no module `nope`");
        assert_eq!(
            note.as_deref(),
            Some("the workspace has `app/`, `lib/`, `tools/`, `common`, `workspace`")
        );
        files[0].1 = "import lib.z";
        let (message, note) = noted(&files, &[]);
        assert_eq!(message, "no module `lib.z`");
        assert_eq!(note.as_deref(), Some("`lib/` has `x`, `y`"));
        files[0].1 = "import lib.x.z";
        assert_eq!(noted(&files, &[]).0, "no module `lib.x.z`");
        files[0].1 = "import vendor.pkg";
        let (_, note) = noted(&files, &["vendor"]);
        assert_eq!(note.as_deref(), Some("`vendor/` is a workspace of its own"));
    }

    /// An import that nothing mentions needs nothing, but is still resolved:
    /// a broken one stops the run of every file, even one that does not
    /// import anything.
    #[test]
    fn an_import_nothing_mentions_is_still_resolved() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport nope\na = 1";
        let errors = run_file(&files, "lib/x.gcsl").unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].0, "no module `nope`", "{errors:?}");
        files[0].1 = "import lib\na = 1";
        assert!(run_file(&files, "app/main.gcsl").is_ok());
    }

    #[test]
    fn a_directory_and_a_file_of_one_name_are_reported_once() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport lib as again";
        files.push(("lib.gcsl", "z = 1"));
        let (message, _) = noted(&files, &[]);
        assert_eq!(message, "`lib` is both a directory and a file");
        files[0].1 = "a = 1";
        assert!(ws(&files).is_ok());
    }

    #[test]
    fn imported_names_clash() {
        let mut files = IMPORTS.to_vec();
        let imported = "is imported in this file";
        for (text, message) in [
            ("import lib\nimport tools as lib", imported),
            ("import lib\nfn lib(): Int { 1 }", imported),
            ("import lib\nlet lib = 1", imported),
            ("import lib\nlib.a = 1", imported),
            ("import com from lib\ncom.a = 1", imported),
            ("import * from lib\ncom.a = 1", imported),
            ("import tools as version", "is a field of this directory"),
            ("import x from lib.x", "has no `x`"),
        ] {
            files[0].1 = text;
            assert!(noted(&files, &[]).0.ends_with(message), "{text}");
        }
        files[0].1 = "import * from lib\na = com.x.lib.name";
        assert!(ws(&files).is_ok());
    }

    #[test]
    fn reaching_into_your_own_file_through_an_import_needs_itself() {
        let mut files = IMPORTS.to_vec();
        files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nall = lib";
        let (message, _) = noted(&files, &[]);
        assert_eq!(message, "`lib/x.gcsl` needs itself");
        files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nother = lib.com.y.lib.name";
        assert!(ws(&files).is_ok());
    }

    #[test]
    fn files_importing_each_other_are_a_cycle() {
        let (message, _) = noted(
            &[
                ("a.gcsl", "import b\nx = b.y"),
                ("b.gcsl", "import a\ny = a.x"),
                ("workspace.gcsl", ""),
            ],
            &[],
        );
        assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
        // A whole read of a directory one of whose files needs this one is
        // the one-file form, at the read, and no cycle of two files.
        let files = [
            ("app.gcsl", "import lib\nall = lib"),
            ("lib/y.gcsl", "import app\ny = app.all"),
            ("workspace.gcsl", ""),
        ];
        let errors = match ws(&files) {
            Ok(_) => Vec::new(),
            Err(errors) => errors,
        };
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["`app.gcsl` needs itself"]);
    }

    #[test]
    fn the_imports_summary_runs() {
        assert_eq!(
            value(&IMPORTS, "app/main.gcsl"),
            "{com = {x = {app = {deps = [{name = \"lib-1.2\"}, {name = \"ylib\"}]; \
             tool = \"build-1.2\"; all = \"ylib\"}}}}"
        );
    }

    /// A workspace of a library directory with a `def`, a leaf and an
    /// interior node in one file, a file failing at runtime if it runs, and
    /// `main` as given.
    fn library(main: &str) -> Vec<(&str, &str)> {
        vec![
            ("app/main.gcsl", main),
            ("lib/bad.gcsl", "def bad(): Int { [1][5] }\nboom = bad()"),
            (
                "lib/f.gcsl",
                "def twice(n: Int): Int { n * 2 }\nleafy = 3\ncom.k.v = 1",
            ),
            ("lib/sub/s.gcsl", "import lib\nv = lib.leafy"),
            ("workspace.gcsl", ""),
        ]
    }

    #[test]
    fn every_import_form_binds_its_names() {
        let files = library("import lib.f as lf\nimport lib.f\na = lf.leafy\nb = f.com.k.v");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3; b = 1}");
        let main = "import twice, leafy as l, com from lib\na = twice(l)\nb = com.k\nc = com.k.v";
        let files = library(main);
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{a = 6; b = {v = 1}; c = 1}"
        );
        let files = library("import * from lib.f\na = twice(leafy)\nb = com");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 6; b = {k = {v = 1}}}");
        // Each root of `*` is a member: `lib/bad.gcsl`'s `boom` is not read.
        let files = library("import * from lib\na = twice(leafy)");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 6}");
    }

    #[test]
    fn a_def_is_read_from_its_functions_slot() {
        let files = library("import lib\nimport twice from lib.f\n= lib.twice(1) + twice(2)");
        assert_eq!(value(&files, "app/main.gcsl"), "6");
        let main = "import lib.f\nfn g(): Int { let r = f\nr.twice(5) }\nr = f.com\nn = g()";
        assert_eq!(
            value(&library(main), "app/main.gcsl"),
            "{r = {k = {v = 1}}; n = 10}"
        );
        let main = "import lib.f\nlet r = f\n= r.twice(4) + r.leafy";
        assert_eq!(value(&library(main), "app/main.gcsl"), "11");
    }

    /// A file mentioning a record-valued leaf through an import from every
    /// kind of position, reading the directory whole and calling a `def`
    /// out of the record, at the top level and inside functions, a nested
    /// `def` and lazies.
    const POSITIONS: &str = r#"import lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := lib.com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy lib.com.x.lib.r
let lw = lazy lib
let g = fn(): {a: Int} { lib.com.x.lib.r }
let gw = fn(): Int { let w = lib; = w.twice(lib.com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = lib
    = lib.com.x.lib.r.a + w.com.x.lib.r.a
  }
}
let w = lib
p01 = "{lib.com.x.lib.r}"
p02 = xs[lib.com.x.lib.r.a]
p03 = m
p04 = for v in [lib.com.x.lib.r] { v.a }
p05 = for v in [1] if lib.com.x.lib.r.a == 42 { v }
p06 = for let i = lib.com.x.lib.r.a; i < lib.com.x.lib.r.a + 1; i := i + lib.com.x.lib.r.a { i }
p07 = for k < lib.com.x.lib.r.a { k := k + 50; lib.com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < lib.com.x.lib.r.a)
p09 = true and lib.com.x.lib.r.a == 42
p10 = lib.com.x.lib.r is {a: Int}
p11 = (lib.com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue lib.com.x.lib.r }; break lib.com.x.lib.r }
p13 = get(config = lib.com.x.lib.r)
p14 = wrap(lib.com.x.lib.r) { ...lib.com.x.lib.r }
p15 = { ...lib.com.x.lib.r }
p16 = len(0..lib.com.x.lib.r.a)
p17 = [lib.com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).com.x.lib.r.a
p20 = g()
p21 = { let q = lib.com.x.lib.r; = q }
p22 = w.twice(w.com.x.lib.r.a)
p23 = gw()
p24 = w.com"#;

    #[test]
    fn records_read_whole_have_the_types_of_their_roots() {
        let files = [
            ("app/main.gcsl", "import lib\nimport lib.x\nd = lib\nf = x"),
            (
                "lib/x.gcsl",
                "com.x.lib.n = 1\ndef twice(n: Int): Int { n * 2 }",
            ),
            ("lib/y.gcsl", "top = \"y\""),
            ("workspace.gcsl", ""),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        let index = workspace.file("app/main.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        // A directory's record has every file's roots, a `def`'s its
        // function's type; a file's, its own roots.
        assert!(dump.contains(
            "(name lib : {com: {x: {lib: {n: Int}}}, top: String, twice: fn(n: Int): Int}"
        ));
        assert!(dump.contains("(name x : {com: {x: {lib: {n: Int}}}, twice: fn(n: Int): Int}"));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{d = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int; top = \"y\"}; \
             f = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int}}"
        );
    }

    #[test]
    fn a_call_of_an_imported_def_is_of_a_known_callee() {
        // The call-site form of the implicit-await error, as for a `def` of
        // the directory, through a member and through a path.
        let files = [
            (
                "app/main.gcsl",
                "import inc from lib\nimport lib\n\
                 fn p(f: Future<Int>): Int { inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.inc(f) }",
            ),
            ("lib/x.gcsl", "def inc(n: Int): Int { n + 1 }"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            messages(&files, "app/main.gcsl"),
            [
                "argument `f` would be awaited inside `inc`, which is pure",
                "argument `f` would be awaited inside `lib.inc`, which is pure",
            ]
        );
    }

    #[test]
    fn an_import_is_read_from_every_position() {
        let files = [
            ("app/main.gcsl", POSITIONS),
            (
                "lib/x.gcsl",
                "let base = 40\ncom.x.lib.r = { a = base + 2 }\ndef twice(n: Int): Int { n * 2 }",
            ),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{outer = fn(): {inner: fn(): Int}; p01 = \"{{a = 42}}\"; p02 = 42; \
             p03 = {a = 42}; p04 = [42]; p05 = [1]; p06 = [42]; p07 = [{a = 42}]; \
             p08 = 42; p09 = true; p10 = true; p11 = 42; p12 = [{a = 42}, {a = 42}]; \
             p13 = 42; p14 = 84; p15 = {a = 42}; p16 = 42; p17 = [{a = 42}]; p18 = 84; \
             p19 = 84; p20 = {a = 42}; p21 = {a = 42}; p22 = 84; p23 = 84; \
             p24 = {x = {lib = {r = {a = 42}}}}}"
        );
    }

    #[test]
    fn needs_through_an_import_are_per_leaf() {
        let files = library("import lib\na = lib.leafy");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
        let files = library("import lib\na = lib.com.k");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = {v = 1}}");
        let files = library("import lib\nfn g(): Int { lib.leafy }\na = g()");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
        let main =
            "import lib.f\ndef d(): Int { f.com.k.v + (fn(): Int { f.twice(1) })() }\na = d()";
        assert_eq!(
            value(&library(main), "app/main.gcsl"),
            "{d = fn(): Int; a = 3}"
        );
        // Read whole, every file of `lib/` runs, `lib/bad.gcsl` too.
        let messages = messages(
            &library("import lib\nlet r = lib\na = r.leafy"),
            "app/main.gcsl",
        );
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(value(&library(""), "lib/sub/s.gcsl"), "{v = 3}");
    }

    #[test]
    fn a_value_file_is_imported_by_its_value() {
        let files = [
            ("a.gcsl", "import b\nimport c\nx = b + 1\ny = c.z"),
            ("b.gcsl", "= 41"),
            ("c.gcsl", "z = 2"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "a.gcsl"), "{x = 42; y = 2}");
        assert_eq!(value(&files, "b.gcsl"), "41");
    }

    #[test]
    fn imported_names_are_the_files_own() {
        let main =
            "a = leafy + g(10)\nfn g(leafy: Int): Int { leafy + 1 }\nimport leafy from lib.f";
        assert_eq!(value(&library(main), "app/main.gcsl"), "{a = 14}");
        let mut files = library("import leafy from lib.f\na = leafy");
        files.push(("app/other.gcsl", "b = leafy"));
        assert_eq!(
            messages(&files, "app/main.gcsl"),
            ["undefined variable `leafy`"]
        );
        files.pop();
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
    }

    #[test]
    fn an_importing_files_items_are_in_any_order() {
        let lines = [
            "import lib.f",
            "a = f.leafy + b",
            "b = twice(2)",
            "import twice from lib.f",
        ];
        // Every rotation, and (as slice 23's helper in lowering, which runs
        // a file alone, compares) the reverse order.
        let reversed: Vec<&str> = lines.iter().rev().copied().collect();
        let orders = (0..lines.len())
            .map(|start| {
                let mut rotated = lines.to_vec();
                rotated.rotate_left(start);
                rotated
            })
            .chain([reversed]);
        for order in orders {
            let main = order.join("\n");
            let mut record: Vec<String> = value(&library(&main), "app/main.gcsl")
                .trim_matches(['{', '}'])
                .split("; ")
                .map(str::to_owned)
                .collect();
            record.sort();
            assert_eq!(record, ["a = 7", "b = 4"], "{main}");
        }
    }
}
