//! A workspace: the files of a directory tree, each directory's files
//! sharing one scope, and every file's dotted paths merged into one view.
//!
//! A *workspace* is the directory tree under a directory that holds a file
//! named `workspace.gcsl`; finding it and reading its files is the CLI's,
//! and this module reads no file: it is given the files as
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
//! sibling's or a subdirectory's, but by an import. A reference means its
//! longest declared prefix in the file's view (below), never in the file's
//! own tree alone: [`crate::mentions::Mentions::resolve`] is given a
//! [`FileScope`], whose numbering keeps the file's own nodes and functions
//! as [`crate::mentions::find`] numbers them and puts every other node
//! after them.
//!
//! One leaf path declared in two files of a directory, and a leaf under a
//! leaf or over an interior node of another file of it, are errors, found
//! while the directory's tree is merged ([`scan`]).
//!
//! # The view of a file
//!
//! Dotted paths always merge. Every file has a *view*: one tree, merged
//! node by node from its *sources*, the tree of its own directory, the
//! tree of each directory above it up to the root, the tree of each
//! `import * from PATH` it has (a directory's merged tree or a file's own
//! tree), and each member import's node at its bound name. An interior
//! node and an interior node of one name merge, their children merged in
//! turn; a leaf (a `def` among them) and anything else of one name is a
//! conflict, except that one cell reached twice is one node (`*` from the
//! file's own directory, from a directory above, from one directory twice,
//! or a file's part of a node also reached whole): merging is idempotent.
//! A whole import (`import PATH [as NAME]`) is not merged: it is a name of
//! its own (see [Imports](#imports)), and one named like a root of the
//! view is still an error.
//!
//! Only what several sources have is stored ([`File::view`]): a node one
//! source alone has is that source's node, read as it always was, so a
//! view costs what its overlaps cost, the largest source of each level
//! only looked up in. A node several sources have is a [`ViewNode`] of
//! [`Scan::views`], its parts the sources' nodes, root directory first,
//! the order its children are in (by first definition, each part in its own
//! order); its children several parts have are view nodes too, and a child
//! one part has is that part's child. The file's [`FileScope`] answers a
//! root by the view's node of that name if several sources have it, else
//! the root of the one directory of its chain or the one import that has
//! it; a child of a view node likewise, a child of any other node in that
//! node's own tree. So a leaf of the view is always one cell, of one file,
//! and an interior node of it is either a node of one source, read from its
//! cell as before, or a view node, assembled by the file itself.
//!
//! A view node with a leaf of the file under it is numbered among the
//! file's own field nodes ([`crate::mentions::Mentions::views`]): read
//! whole, it is a node of the file's schedule under slice 24's rule for an
//! interior node, computed at the earlier of "after its last child of the
//! file" and its first mention, the other sources' children computed
//! already, since the file needs their owners. Any other view node is a
//! [`Target::View`], read whole as a record the file assembles as it starts,
//! as an imported record is. Reading a view node whole needs every owner of
//! every leaf under it, in whichever directory or import, but never makes a
//! file need itself; only an import that reaches the file's own nodes by
//! another path (a member renamed, `import com as c from app` inside
//! `app/`) is "`app/main.gcsl` needs itself", as through a whole import.
//! The view is the file's alone: the directory tree committed for the
//! files after it is unchanged, and a file's own record is still built
//! from its own cells only.
//!
//! The scan finds every conflict of every view before any file is
//! checked, once per pair however many views have it, reported at the
//! later of the two sources in the order: the file's own directory first,
//! then each directory upward, then its imports in source order. Two
//! leaves are "field `com.x.lib.name` is defined in two files", "defined
//! here" at both; a leaf and an interior node, "field `com.x` is already
//! defined", the earlier side's whole definition labelled. An import's side
//! is labelled "imported here" at its `*` or member, with an excerpt of the
//! imported definition, "defined here"; a directory above the file's own
//! among the two adds the note "one is in a directory above".
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
//! number of nodes are the workspace's cells, the built-in functions' the
//! first and the standard files' the next (see [The standard
//! library](#the-standard-library)), and every other slot (a
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
//! its index among the files, the standard files first and the workspace's
//! after them in path order ([`Diagnostic::input`]). After a file is
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
//! (`import x from lib`, and each root of `import * from lib`). A whole
//! import (`import lib`, `import lib.x as f`) is a private top-level name
//! of its file: in scope throughout it, shadowed by nearer names, never in
//! its tree or record, never seen by another file; one clashing with
//! another top-level name of the file, with a member or `*` root of it, or
//! with a root of its directory chain, is an error found by the scan. A
//! member and a root of a `*` merge into the file's view instead ([The
//! view of a file](#the-view-of-a-file)), private to the file too, and
//! clash only with a whole import or a top-level `fn`, `let`, `let mut` or
//! `:=` of their name. `import * from` a value file is "a value file has
//! no members to import". The aliases are a lookup of the [`FileScope`]
//! apart from the merged trees ([`FileScope::alias`]), so a reference
//! through one means its longest declared prefix in the target's tree, as
//! every reference does: the numbers past the merged nodes are targets
//! ([`FileScope::target`]).
//!
//! The needs through an import are per leaf ([`Scan::target_files`]): a
//! leaf, a `def` or an interior node needs the files that own it, a file
//! read whole needs that file, a directory read whole needs every file of
//! it, and a view node read whole every owner of every leaf under it. A
//! reach through an import into a node the file itself owns or contributes
//! to, its own file or directory read whole through a whole import, and a
//! whole read of a directory one of whose files needs this one, are the
//! one-file form of the cycle error, "`app.gcsl` needs itself"; files
//! importing each other are the cycle error of [Order](#order).
//!
//! A reference that ends on a cell reads that cell's slot: a `def`'s is its
//! function's slot, since the merged node of a `def` is never written. A
//! record read whole (a directory, a file, a file's part of a shared node,
//! or a view node without a leaf of the file under it) is assembled by the
//! reading file, once, into a private slot of its own, before its first
//! item ([`crate::check#imports`]). A value file that some file imports has
//! a cell of its own after the merged nodes ([`Scan::value_slot`]), which
//! the file stores its result into as the last node of its top-level
//! block.
//!
//! # The standard library
//!
//! Two kinds of module are not files of the workspace. `builtins` is a
//! module implemented in Rust, a record of the built-in functions
//! ([`Builtin`]): `len`, `int`, `float`, `exec` and `panic`. The *standard
//! files* are the `.gcsl` files of the repository's `stdlib/` directory,
//! compiled in ([`crate::STANDARD`], [`standard_sources`]): today only
//! `prelude.gcsl`, which binds the built-in functions and `sh`. Each is
//! importable by a bare name, `import builtins`,
//! `import len from builtins`, `import * from prelude`, from any file; the
//! names are reserved roots of the path namespace, and a file or directory
//! at the workspace's root named like one of them is an error of the scan,
//! "`builtins` is the name of a built-in module", "`prelude` is the name of
//! a standard module", reported at the start of the file, or of the first
//! file under the directory. A path of more than one segment never names
//! one, and a file or directory below the root has a path of its own.
//!
//! Every scan holds them. The standard files are its first files, so they
//! take the first input numbers, each named `<stdlib>/STEM.gcsl`
//! ([`STANDARD_DIR`]), in a directory of their own with no directory
//! above: one directory scope among themselves, a standard file seeing the
//! other standard files' declarations and nothing of the workspace, and
//! nothing of the workspace seeing theirs but by an import. They are
//! checked and run with the workspace's files, before them in the order
//! (they need no file of the workspace), on the same frame and memo
//! tables; every run runs them first ([`Scan::needed`]). An error in one is
//! reported like any other, and is a bug. The `builtins` module is a
//! directory of its own too, with no file: its tree is one root leaf per
//! built-in function, nodes `0..` of [`Scan::nodes`] in the order of
//! [`Builtin::ALL`], so their cells are the first slots of the frame, which
//! the driver writes, each a [`Value::Builtin`], as it lays the frame out,
//! before any file runs ([`check_with`]). No file computes them, and none
//! needs another for them. Their types are each built-in's: its function
//! type, or for one whose typing is special the opaque type of the alias
//! rule (see [`crate::check`], "Built-in functions"). As it lays the frame
//! out, the driver also builds the one layout of the records `exec`
//! returns (`exec_layout`), its field names interned in the run's names,
//! and gives it to every file it runs, beside the memo tables, so that
//! every such record in the run shares it (see [`crate::eval`], "The
//! built-in functions"); a session gets it with the rest of the standard
//! library's run.
//!
//! Every file but `prelude.gcsl` itself, the standard files included,
//! begins as if with `import * from prelude`, as the last source of its
//! view, after its directory chain and its own imports: an *implicit*
//! member alias for each root of the prelude ([`Alias::implicit`]), except
//! each that any earlier source has (a root of its directory chain, its
//! own declarations among them, or a name it imports), which is dropped,
//! shadowed silently rather than a conflict. Its names are invisible to the
//! clash family: an implicit alias clashes with nothing, and a name it
//! binds yields to any binding of the file of its name, a top-level `let`
//! or `fn` included ([`FileScope::yields`]). An explicit `import * from
//! prelude` is an ordinary `*`, and conflicts as one.
//!
//! A file outside any workspace (`gcsl run` of a lone file, [`check_lone`])
//! is checked and run as a workspace of one directory holding just it,
//! beside the standard library: it may import `builtins` and the standard
//! files, and an import of anything else is the checker's error,
//! "`lib` is not a built-in or standard module, and there is no
//! `workspace.gcsl` above this file", with the file's other errors. A
//! session's input is scanned so too (`check_input`), though a session is
//! not a workspace (see [`crate::session`]); it inherits the imports of the
//! session's earlier inputs, after its own and before the implicit prelude,
//! but for each it imports again or declares at its top level.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::mem;
use std::rc::Rc;

use crate::Outcome;
use crate::ast::{Ast, ImportDecl, ImportForm, ImportMember, ItemId, ItemKind, PathSegment};
#[cfg(test)]
use crate::check::ViewRecord;
use crate::check::{Builtin, CheckOutput, Checker, Scope, Slot, Types};
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
use crate::value::{Layout, Value};

/// The key of a root in [`Scan`]'s index of nodes by parent and name.
const ROOT: u32 = u32::MAX;

/// The path of the standard library's directory, which no path of a
/// workspace can name, `<` being in no name: each standard file is
/// `<stdlib>/STEM.gcsl`, which is how a diagnostic of one names it.
pub const STANDARD_DIR: &str = "<stdlib>";

/// The name of the module of built-in functions.
const BUILTINS: &str = "builtins";

/// The standard files ([`crate::STANDARD`]) as the sources every scan
/// begins with, in their order, each named `<stdlib>/STEM.gcsl`: what a
/// caller renders a diagnostic of a standard file against.
#[must_use]
pub fn standard_sources() -> Vec<SourceFile> {
    crate::STANDARD
        .iter()
        .filter_map(|&(stem, text)| {
            SourceFile::new(format!("{STANDARD_DIR}/{stem}.gcsl"), text).ok()
        })
        .collect()
}

/// The values of the cells of the built-in functions, the first slots of a
/// frame, in the order of [`Builtin::ALL`]: what the driver writes as it
/// lays the frame out, before any file runs.
fn builtin_cells() -> Vec<Value> {
    Builtin::ALL.into_iter().map(Value::Builtin).collect()
}

/// The layout of the record `exec` returns, its fields
/// ([`Builtin::EXEC_FIELDS`]) interned into `names`, the run's, as the
/// checker interns them for `exec`'s type, so that `r.status` reads it:
/// built once per run as the frame is laid out, and shared by every record
/// `exec` builds in the run, whatever file, input or function calls it (see
/// [`crate::eval`]).
fn exec_layout(names: &mut Names) -> Rc<Layout> {
    Rc::new(Layout {
        names: Builtin::EXEC_FIELDS
            .iter()
            .map(|text| names.intern(text))
            .collect(),
        texts: Builtin::EXEC_FIELDS
            .iter()
            .map(|&text| Box::from(text))
            .collect(),
    })
}

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
    /// against its view ([`FileScope`]).
    pub mentions: Mentions,
    /// For each node of its own field tree, the merged node of its path.
    pub global: Vec<u32>,
    /// For each of its own top-level functions, its index in
    /// [`Scan::functions`].
    pub functions: Vec<u32>,
    /// The names it imports, in source order (a `*` gives one per root).
    pub aliases: Vec<Alias>,
    /// Its view: the nodes several of its sources have, merged.
    pub view: View,
    /// When its field nodes are computed: its own nodes that another file
    /// computes (shared interior nodes it is not the last contributor of)
    /// are not in it; the nodes of its view with a leaf of it under them
    /// are, if it reads them whole ([`Mentions::views`]).
    pub schedule: Schedule,
}

impl File {
    /// The file at `path` in directory `dir`, parsed to `ast`, spelling
    /// `spelled`, with its declarations found and nothing else yet.
    fn new(path: &str, dir: u32, ast: Ast, spelled: Vec<Name>) -> Self {
        let mentions = mentions::scan(&ast);
        Self {
            path: path.to_owned(),
            dir,
            ast,
            spelled,
            mentions,
            global: Vec::new(),
            functions: Vec::new(),
            aliases: Vec::new(),
            view: View::default(),
            schedule: Schedule::default(),
        }
    }
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
    /// A node of a file's view that several sources have and no leaf of
    /// the file is under, by its index in [`Scan::views`]: read whole, the
    /// record its file assembles of every source's children.
    View(u32),
}

/// A name a file imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The name bound.
    pub name: Name,
    /// Where it is bound: the alias, the member, the `*`, or the path's
    /// last segment; nowhere (`0..0`) for the implicit `import * from
    /// prelude` and for an import a session's input inherits from an
    /// earlier input, whose source is not this one.
    pub span: Span,
    /// The `import` item; `None` for the implicit `import * from prelude`,
    /// and for an import a session's input inherits from an earlier input.
    pub item: Option<ItemId>,
    /// What it names.
    pub target: Target,
    /// Whether it is a whole import (`import PATH`, `import PATH as NAME`),
    /// a name of its own; a member or a root of a `*` is merged into the
    /// file's view instead.
    pub whole: bool,
    /// Whether it is a root of the implicit `import * from prelude`, which
    /// yields to every other source and binding of its name (see the module
    /// documentation, "The standard library").
    pub implicit: bool,
    /// Whether a session's input inherits it from an earlier input, which
    /// the input's declarations shadow and which clashes with nothing (see
    /// [`inherit_imports`]).
    pub inherited: bool,
}

/// One source's node at a node of a file's view (see the module
/// documentation, "The view of a file").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewPart {
    /// The source, root directory first: the directories of the file's
    /// chain from the root (0) down to its own, then its imported members
    /// and `*` roots, by their index in [`File::aliases`] after those.
    pub source: u32,
    /// The node: a merged node, or a file's part of one.
    pub target: Target,
    /// Whether it is a node of a directory of the file's chain, reached by
    /// its own path (an import of such a node at its own path is the same
    /// cell, merged into it).
    pub chain: bool,
}

impl ViewPart {
    /// The merged node `node` of the directory of the file's chain at
    /// position `source`.
    const fn chain(source: u32, node: u32) -> Self {
        Self {
            source,
            target: Target::Node(node),
            chain: true,
        }
    }
}

/// A node of a file's view that several sources have, the same cell
/// reached twice taken once (see the module documentation, "The view of a
/// file").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNode {
    /// Its last segment.
    pub name: Name,
    /// The view node it is a child of, by its index in [`Scan::views`];
    /// `None` for a root of the view.
    pub parent: Option<u32>,
    /// The file whose view it is of.
    pub file: u32,
    /// What it merges, root directory first, each an interior node.
    pub parts: Vec<ViewPart>,
    /// The file's own node of its path, when a leaf of the file is under
    /// it: it is then a node of the file's schedule.
    pub own: Option<u32>,
}

/// What a file's view makes of a name that several sources have at one
/// node: a node of the view, or the one node left once the same cell
/// reached twice is taken once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merged {
    /// A node of the view, by its index in [`Scan::views`].
    View(u32),
    /// A single node.
    One(ViewPart),
}

/// A file's view: its nodes that several sources have (see the module
/// documentation, "The view of a file").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// Where its nodes start in [`Scan::views`].
    pub start: u32,
    /// The number of its nodes with a leaf of the file under them, which
    /// come first.
    pub own: u32,
    /// Where its nodes end in [`Scan::views`].
    pub end: u32,
    /// What it makes of each name several sources have, by the node it is
    /// under ([`ROOT`] for a root, else its index in [`Scan::views`]) and
    /// the name.
    merged: HashMap<(u32, Name), Merged>,
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
    /// The nodes of every file's view that several sources have, each
    /// file's together ([`File::view`]).
    pub views: Vec<ViewNode>,
    /// For each file, the files it needs, each once, in index order.
    pub needs: Vec<Vec<u32>>,
    /// The files in the order they are checked and run.
    pub order: Vec<u32>,
    /// The names of every file.
    pub names: Names,
    /// The nodes by directory, parent ([`ROOT`] for a root) and name.
    by_name: HashMap<(u32, u32, Name), u32>,
    /// The roots of each directory's tree, by directory, in order.
    roots: Vec<Vec<u32>>,
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
    /// The number of standard files, which are files `0..standard` (see
    /// the module documentation, "The standard library").
    standard: u32,
    /// The standard library's directory.
    standard_dir: u32,
    /// The directory of the `builtins` module, whose nodes are the first
    /// [`Builtin::ALL`]`.len()` of `nodes`.
    builtins_dir: u32,
    /// Whether the files given are one file outside any workspace (a lone
    /// file, a session's input), whose imports of anything but `builtins`
    /// and the standard files the checker reports.
    lone: bool,
    /// For a session's input ([`check_input`]): the number of cells of the
    /// standard library, which keep their slots, and the slot of each cell
    /// after them, past the slots of the session's earlier inputs (see
    /// [`Scan::slot`]).
    relocation: Option<(u32, Vec<Slot>)>,
}

impl Scan {
    /// The directories from `dir` up to the root, `dir` first.
    pub fn chain(&self, dir: u32) -> impl Iterator<Item = u32> + '_ {
        std::iter::successors(Some(dir), |&at| {
            self.dirs.get(at as usize).and_then(|d| d.parent)
        })
    }

    /// The files `file` needs, transitively, with `file` and the standard
    /// files, which every run runs, in the order: the files to run to run
    /// `file`.
    #[must_use]
    pub fn needed(&self, file: u32) -> Vec<u32> {
        let mut wanted = vec![false; self.files.len()];
        let mut stack: Vec<u32> = (0..self.standard).collect();
        stack.push(file);
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
    /// merged nodes, then the views' nodes.
    fn target_number(&self, target: Target) -> u32 {
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        let parts = self.parts();
        let nodes = dirs.saturating_add(files).saturating_add(parts);
        match target {
            Target::Dir(dir) => dir,
            Target::File(file) => dirs.saturating_add(file),
            Target::Part { file, node } => {
                let base = self.part_base.get(file as usize).copied().unwrap_or(0);
                dirs.saturating_add(files)
                    .saturating_add(base)
                    .saturating_add(node)
            }
            Target::Node(node) => nodes.saturating_add(node),
            Target::View(view) => nodes
                .saturating_add(count(self.nodes.len()))
                .saturating_add(view),
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
        let Some(view) = node.checked_sub(count(self.nodes.len())) else {
            return Some(Target::Node(node));
        };
        ((view as usize) < self.views.len()).then_some(Target::View(view))
    }

    /// The slot of the cell of the value of `file`, a value file some file
    /// imports: its owner stores its result there, and an importer reads
    /// it ([`Target::File`] of a value file). `None` for any other file.
    #[must_use]
    pub fn value_slot(&self, file: u32) -> Option<Slot> {
        self.value_cell(file).map(|cell| self.slot(cell))
    }

    /// The slot of the cell numbered `cell` ([`Scan::cells`]): its number;
    /// in a session's input (`check_input`), the cells after the standard
    /// library's take the slots after the session's instead, in order, but
    /// a `def`'s, which is never written, which takes none
    /// ([`Slot::NONE`]), as a `def` of a session's input never did.
    #[must_use]
    pub fn slot(&self, cell: u32) -> Slot {
        match &self.relocation {
            Some((from, slots)) if cell >= *from => slots
                .get((cell - from) as usize)
                .copied()
                .unwrap_or(Slot::NONE),
            _ => Slot::at(cell),
        }
    }

    /// Whether the merged node `node` is a `def`'s, whose cell is its
    /// function's slot and never written itself.
    fn is_def(&self, node: u32) -> bool {
        self.nodes.get(node as usize).is_some_and(|entry| {
            entry.parent.is_none()
                && entry.leaf.is_some_and(|file| {
                    self.defs
                        .get(&(entry.dir, entry.name))
                        .and_then(|&id| self.functions.get(id as usize))
                        .is_some_and(|function| function.file == file)
                })
        })
    }

    /// The built-in function whose cell is the merged node `node`, if it is
    /// one: the first nodes are the `builtins` module's.
    #[must_use]
    pub fn builtin(&self, node: u32) -> Option<Builtin> {
        Builtin::ALL.get(node as usize).copied()
    }

    /// The number of standard files: files `0..` this are the standard
    /// library's (see the module documentation, "The standard library").
    #[must_use]
    pub fn standard(&self) -> u32 {
        self.standard
    }

    /// The standard file whose stem is `stem`, if there is one.
    fn standard_file(&self, stem: &str) -> Option<u32> {
        (0..self.standard).find(|&file| {
            self.files.get(file as usize).is_some_and(|f| {
                f.path
                    .strip_prefix(STANDARD_DIR)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .and_then(|rest| rest.strip_suffix(".gcsl"))
                    == Some(stem)
            })
        })
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
            Target::View(view) => match self.view_child(view, name)? {
                Merged::View(child) => Some(Target::View(child)),
                Merged::One(part) => Some(part.target),
            },
        }
    }

    /// The child named `name` of the view node `view`: a view node of its
    /// own when several of its parts have one, else the one part's child.
    fn view_child(&self, view: u32, name: Name) -> Option<Merged> {
        let node = self.views.get(view as usize)?;
        let file = self.files.get(node.file as usize)?;
        if let Some(&merged) = file.view.merged.get(&(view, name)) {
            return Some(merged);
        }
        node.parts.iter().find_map(|part| {
            let target = self.target_child(part.target, name)?;
            Some(Merged::One(ViewPart { target, ..*part }))
        })
    }

    /// The names of the children of the view node `view`, in the view's
    /// order: by first definition, its parts root directory first, each in
    /// its own order.
    fn view_names(&self, view: u32) -> Vec<Name> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        let parts = self.views.get(view as usize).map_or(&[][..], |v| &v.parts);
        for part in parts {
            for name in self.child_names(part.target) {
                if seen.insert(name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// The names of the children of `target`, a merged node or a file's
    /// part of one, in its order.
    fn child_names(&self, target: Target) -> Vec<Name> {
        match target {
            Target::Node(node) => self.nodes.get(node as usize).map_or_else(Vec::new, |n| {
                n.children
                    .iter()
                    .filter_map(|&c| self.nodes.get(c as usize).map(|c| c.name))
                    .collect()
            }),
            Target::Part { file, node } => self
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
            Target::View(view) => self.view_names(view),
            Target::Dir(_) | Target::File(_) => target_roots(self, target),
        }
    }

    /// The files with a leaf at or under the part `part` of a view node.
    fn part_owners(&self, part: ViewPart) -> Vec<u32> {
        self.target_files(part.target)
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
    /// leaf's file or its contributors, every owner of every part of a view
    /// node.
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
            Target::View(view) => {
                let parts = self.views.get(view as usize).map_or(&[][..], |v| &v.parts);
                let mut files: Vec<u32> = parts
                    .iter()
                    .flat_map(|&part| self.part_owners(part))
                    .collect();
                files.sort_unstable();
                files.dedup();
                files
            }
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
            Target::View(view) => self.show_view(view),
        }
    }

    /// The name of the top-level function `function`.
    fn show_function(&self, function: Function) -> String {
        self.files
            .get(function.file as usize)
            .and_then(|file| file.mentions.functions.get(function.index as usize))
            .map_or_else(String::new, |top| self.names.get(top.name).to_owned())
    }

    /// The dotted path in its file's view of the view node `view`.
    fn show_view(&self, view: u32) -> String {
        let mut names = Vec::new();
        let mut at = Some(view);
        while let Some(index) = at
            && let Some(entry) = self.views.get(index as usize)
        {
            names.push(self.names.get(entry.name));
            at = entry.parent;
        }
        names.reverse();
        names.join(".")
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

/// The scope a file's references are resolved in: its view, the merged
/// trees of its directory chain and what it imports (see the module
/// documentation, "The view of a file"), and the `def`s of its chain,
/// numbered as [`crate::mentions::Resolve`] asks. The file's own field
/// nodes come first, by their own numbers, then the nodes of its view with
/// a leaf of the file under them ([`Mentions::views`]), then every merged
/// node at its index in [`Scan::nodes`] after those, then every target an
/// import or the view reaches (a [`Target::View`] among them); its own
/// functions come first, then every other one at its index in
/// [`Scan::functions`] after them.
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
    /// What each name the file imports names; of a name several sources
    /// have, the view's node says.
    aliases: HashMap<Name, Target>,
    /// The names only the implicit `import * from prelude` binds, which
    /// yield to any binding of the file ([`FileScope::yields`]).
    implicit: Vec<Name>,
    /// The names of the imports a session's input inherits from earlier
    /// inputs ([`FileScope::inherited`]).
    inherited: Vec<Name>,
    /// The file's view.
    view: &'a View,
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
            implicit: file
                .aliases
                .iter()
                .filter(|a| a.implicit)
                .map(|a| a.name)
                .collect(),
            inherited: file
                .aliases
                .iter()
                .filter(|a| a.inherited)
                .map(|a| a.name)
                .collect(),
            view: &file.view,
        }
    }

    /// Whether the root `name` is one an import a session's input inherits
    /// from an earlier input binds, which a top-level `let` of the input
    /// shadows from its point on, as it would an earlier input's variable,
    /// and clashes with nothing (see [`inherit_imports`]).
    #[must_use]
    pub fn inherited(&self, name: Name) -> bool {
        self.inherited.contains(&name)
    }

    /// Whether the root `name` is one only the implicit `import * from
    /// prelude` binds, which yields to any binding of the file of its name,
    /// a top-level `let` or `fn` included, and clashes with none (see the
    /// module documentation, "The standard library").
    #[must_use]
    pub fn yields(&self, name: Name) -> bool {
        self.implicit.contains(&name)
    }

    /// Where the numbers of the merged nodes start: after the file's own
    /// nodes and its view's own nodes.
    fn merged_start(&self) -> u32 {
        count(self.global.len()).saturating_add(self.view.own)
    }

    /// Where the numbers of targets reached through imports or the view
    /// start: after the merged nodes.
    fn targets_start(&self) -> u32 {
        self.merged_start()
            .saturating_add(count(self.scan.nodes.len()))
    }

    /// What the scope's number `node` is when it is reached through an
    /// import or a node of the view without a leaf of the file under it,
    /// `None` for the file's own nodes, its view's own nodes and the merged
    /// nodes of its directory chain ([`FileScope::merged`]). A reference
    /// through an import resolves to one of these, and so does a mention of
    /// it in [`Mentions::foreign`]; a [`Target::Node`] is a cell like any
    /// merged node, read by its slot.
    #[must_use]
    pub fn target(&self, node: u32) -> Option<Target> {
        self.scan.target_of(node.checked_sub(self.targets_start())?)
    }

    /// The import of the name `name`, if the file imports it.
    #[must_use]
    pub fn alias(&self, name: Name) -> Option<Target> {
        self.aliases.get(&name).copied()
    }

    /// Whether a reference to the root `name` reads through the file's
    /// view rather than a binding: when the root is imported, or several
    /// sources have it. A root of the file's own or of one directory of its
    /// chain alone is read through its binding, as before imports.
    #[must_use]
    pub fn reads_through(&self, name: Name) -> bool {
        // Only an imported name or a root of the view can: two lookups
        // settle most names, without the walk of the directory chain.
        if !self.aliases.contains_key(&name) && !self.view.merged.contains_key(&(ROOT, name)) {
            return false;
        }
        self.root(name)
            .is_some_and(|node| node >= count(self.global.len()) && self.merged(node).is_none())
    }

    /// The index in [`Scan::views`] of the scope's number `node`, if it is
    /// a node of the view with a leaf of the file under it.
    #[must_use]
    pub fn own_view(&self, node: u32) -> Option<u32> {
        let at = node.checked_sub(count(self.global.len()))?;
        (at < self.view.own).then(|| self.view.start.saturating_add(at))
    }

    /// The scope's number of `target`.
    fn target_number(&self, target: Target) -> u32 {
        if let Target::View(view) = target
            && let Some(at) = view.checked_sub(self.view.start)
            && at < self.view.own
        {
            return count(self.global.len()).saturating_add(at);
        }
        self.targets_start()
            .saturating_add(self.scan.target_number(target))
    }

    /// The scope's number of the merged node `node`.
    fn number(&self, node: u32) -> u32 {
        self.local
            .get(&node)
            .copied()
            .unwrap_or_else(|| self.merged_start().saturating_add(node))
    }

    /// The scope's number of what the view makes of a name.
    fn merged_number(&self, merged: Merged) -> u32 {
        match merged {
            Merged::View(view) => self.target_number(Target::View(view)),
            Merged::One(ViewPart {
                target: Target::Node(node),
                chain: true,
                ..
            }) => self.number(node),
            Merged::One(part) => self.target_number(part.target),
        }
    }

    /// The merged node of the scope's number `node`, if it is one of the
    /// file's own nodes or a merged node of its directory chain.
    #[must_use]
    pub fn merged(&self, node: u32) -> Option<u32> {
        if let Some(own) = self.global.get(node as usize) {
            return Some(*own);
        }
        let other = node.checked_sub(self.merged_start())?;
        (node < self.targets_start()).then_some(other)
    }

    /// The children of the view node `node` (a scope's number), each with
    /// its name and its scope's number, in the view's order: by first
    /// definition, its sources root directory first.
    #[must_use]
    pub fn view_children(&self, node: u32) -> Vec<(Name, u32)> {
        let view = match self.own_view(node) {
            Some(view) => view,
            None => match self.target(node) {
                Some(Target::View(view)) => view,
                _ => return Vec::new(),
            },
        };
        self.scan
            .view_names(view)
            .into_iter()
            .filter_map(|name| Some((name, self.child(node, name)?)))
            .collect()
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
        if let Some(&merged) = self.view.merged.get(&(ROOT, name)) {
            return Some(self.merged_number(merged));
        }
        self.scan
            .chain(self.dir)
            .find_map(|dir| self.scan.by_name.get(&(dir, ROOT, name)))
            .map(|&node| self.number(node))
            .or_else(|| Some(self.target_number(self.alias(name)?)))
    }

    fn child(&self, node: u32, name: Name) -> Option<u32> {
        let view = self.own_view(node).or(match self.target(node) {
            Some(Target::View(view)) => Some(view),
            _ => None,
        });
        if let Some(view) = view {
            return Some(self.merged_number(self.scan.view_child(view, name)?));
        }
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
        (self.merged_start(), count(self.own.len()))
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
/// relative to the root, beside the standard library: parses every file,
/// merges each directory's trees and each file's view, resolves every
/// file's references against its view, and finds the order and each file's
/// schedule (see the module documentation). The standard files are files
/// `0..` [`Scan::standard`], and the workspace's follow them in path order.
///
/// # Errors
///
/// Every syntax error of every file; else every file or directory of the
/// root named like a module of the standard library; else every conflict
/// between files' declarations in a directory; else every conflict of
/// every file's view; else every cycle of files that need each other. Each
/// names its file in [`Diagnostic::input`], its index among the files.
pub fn scan(sources: &[SourceFile]) -> Result<Scan, Vec<Diagnostic>> {
    scan_with(sources, &[])
}

/// [`scan`], told the directories left out of the workspace as workspaces
/// of their own, each by its path relative to the root without a trailing
/// `/`: an import of one is "no module" with a note saying so.
///
/// # Errors
///
/// As [`scan`]; before the view's conflicts, every import that names
/// nothing or clashes with a name of its file or directory chain, and with
/// the cycles, every reference through an import that reaches back into
/// its own file.
pub fn scan_with(sources: &[SourceFile], nested: &[String]) -> Result<Scan, Vec<Diagnostic>> {
    scan_into(sources, Placing::Workspace(nested), &mut Names::new())
}

/// Where the files a scan is given are placed (see the module
/// documentation, "The standard library").
#[derive(Clone, Copy, Debug)]
enum Placing<'a> {
    /// The files of a workspace, by their paths relative to its root, with
    /// the directories left out as workspaces of their own.
    Workspace(&'a [String]),
    /// One file outside any workspace, a lone file or a session's input, in
    /// the root directory by the last part of its name, with the imports of
    /// the session's earlier inputs it inherits (see [`inherit_imports`]),
    /// none for a lone file.
    Lone(&'a [Alias]),
}

/// [`scan_with`] of `sources` placed as `placing` says, interning into
/// `names`, which the scan holds once it succeeds ([`Scan::names`]) and
/// which are given back to `names` if it fails.
fn scan_into(
    sources: &[SourceFile],
    placing: Placing,
    names: &mut Names,
) -> Result<Scan, Vec<Diagnostic>> {
    let mut scan = Scan {
        files: Vec::with_capacity(sources.len() + crate::STANDARD.len()),
        dirs: Vec::new(),
        nodes: Vec::new(),
        functions: Vec::new(),
        views: Vec::new(),
        needs: Vec::new(),
        order: Vec::new(),
        names: mem::take(names),
        by_name: HashMap::new(),
        roots: Vec::new(),
        defs: HashMap::new(),
        rank: Vec::new(),
        part_base: Vec::new(),
        values: Vec::new(),
        standard: 0,
        standard_dir: 0,
        builtins_dir: 0,
        lone: matches!(placing, Placing::Lone(_)),
        relocation: None,
    };
    match scan_files(&mut scan, sources, placing) {
        Ok(()) => Ok(scan),
        Err(errors) => {
            *names = mem::take(&mut scan.names);
            Err(errors)
        }
    }
}

/// Fills `scan`, empty but for its names, with the standard library and
/// `sources` placed as `placing` says (see [`scan_with`]).
fn scan_files(
    scan: &mut Scan,
    sources: &[SourceFile],
    placing: Placing,
) -> Result<(), Vec<Diagnostic>> {
    let mut sorted: Vec<&SourceFile> = sources.iter().collect();
    sorted.sort_by(|a, b| a.name().as_bytes().cmp(b.name().as_bytes()));
    let standard = standard_sources();
    scan.standard = count(standard.len());
    let (dirs, user_dirs) = place_dirs(scan, &sorted, placing);
    let placed = standard
        .iter()
        .map(|source| (source, scan.standard_dir, source.name()))
        .chain(sorted.iter().zip(&user_dirs).map(|(&source, &dir)| {
            let path = match placing {
                Placing::Workspace(_) => source.name(),
                Placing::Lone(_) => source.name().rsplit('/').next().unwrap_or(""),
            };
            (source, dir, path)
        }));
    let mut errors = Vec::new();
    for (index, (source, dir, path)) in (0u32..).zip(placed) {
        match parse(source, &mut scan.names) {
            Ok((ast, spelled)) => scan.files.push(File::new(path, dir, ast, spelled)),
            Err(error) => errors.push(error.in_input(index)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if matches!(placing, Placing::Workspace(_)) {
        reserved(scan, &mut errors);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    scan.roots = vec![Vec::new(); scan.dirs.len()];
    builtin_nodes(scan);
    for file in 0..count(scan.files.len()) {
        merge(scan, file, &mut errors);
    }
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
    let (nested, inherited) = match placing {
        Placing::Workspace(nested) => (nested, &[][..]),
        Placing::Lone(inherited) => (&[][..], inherited),
    };
    imports(scan, &dirs, (nested, inherited), &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    views(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    value_cells(scan);
    resolve(scan);
    let mentioned = find_needs(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    report_cycles(scan, &mentioned, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    scan.order = order(&scan.needs);
    scan.rank = scan.ranks();
    schedule(scan);
    Ok(())
}

/// Adds the directories of `scan`: the workspace's in path order, the root
/// first, as its files are, `sorted`, placed as `placing` says (a lone
/// file's is the root); then the standard library's and the `builtins`
/// module's, with no directory above, apart from the workspace's tree.
/// Returns the workspace's directories by path, and each file's.
fn place_dirs(
    scan: &mut Scan,
    sorted: &[&SourceFile],
    placing: Placing,
) -> (HashMap<String, u32>, Vec<u32>) {
    let mut dirs: HashMap<String, u32> = HashMap::new();
    intern_dir(&mut scan.dirs, &mut dirs, "");
    let user_dirs: Vec<u32> = sorted
        .iter()
        .map(|source| match placing {
            Placing::Workspace(_) => {
                intern_dir(&mut scan.dirs, &mut dirs, parent_path(source.name()))
            }
            Placing::Lone(_) => 0,
        })
        .collect();
    for path in [STANDARD_DIR, BUILTINS] {
        let dir = count(scan.dirs.len());
        scan.dirs.push(Dir {
            path: path.to_owned(),
            parent: None,
        });
        if path == STANDARD_DIR {
            scan.standard_dir = dir;
        } else {
            scan.builtins_dir = dir;
        }
    }
    (dirs, user_dirs)
}

/// Adds the nodes of the `builtins` module's tree, a root leaf for each
/// built-in function in the order of [`Builtin::ALL`], before any file's:
/// they are nodes `0..`, so their cells are the first slots of the frame.
/// No file owns or contributes to them.
fn builtin_nodes(scan: &mut Scan) {
    let dir = scan.builtins_dir;
    for builtin in Builtin::ALL {
        let name = scan.names.intern(builtin.name());
        let node = count(scan.nodes.len());
        scan.nodes.push(Node {
            name,
            parent: None,
            children: Vec::new(),
            dir,
            leaf: None,
            contributors: Vec::new(),
        });
        scan.by_name.insert((dir, ROOT, name), node);
        if let Some(roots) = scan.roots.get_mut(dir as usize) {
            roots.push(node);
        }
    }
}

/// Reports each file or directory of the workspace's root named like a
/// module of the standard library, the names it reserves: "`builtins` is
/// the name of a built-in module", "`prelude` is the name of a standard
/// module", once per name, at the start of the first file that has it (the
/// file, or the first file under the directory), in path order.
fn reserved(scan: &Scan, errors: &mut Vec<Diagnostic>) {
    let mut reported: Vec<&str> = Vec::new();
    for (index, file) in (0u32..).zip(&scan.files).skip(scan.standard as usize) {
        let first = file.path.split('/').next().unwrap_or("");
        let name = if file.path.contains('/') {
            first
        } else {
            first.strip_suffix(".gcsl").unwrap_or(first)
        };
        let message = if name == BUILTINS {
            format!("`{name}` is the name of a built-in module")
        } else if scan.standard_file(name).is_some() {
            format!("`{name}` is the name of a standard module")
        } else {
            continue;
        };
        if reported.contains(&name) {
            continue;
        }
        reported.push(name);
        errors.push(Diagnostic::error(message, Span::new(0, 0)).in_input(index));
    }
}

/// Resolves every file's references against its view ([`FileScope`]).
fn resolve(scan: &mut Scan) {
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
            let mut scope = FileScope::new(scan, file);
            scope.own = &functions;
            mentions.resolve(&file.ast, &scope);
            mentions.functions = functions;
            mentions.tree = tree;
        }
        if let Some(file) = scan.files.get_mut(index) {
            file.mentions = mentions;
        }
    }
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
            } else if let Some(roots) = scan.roots.get_mut(dir as usize) {
                roots.push(merged);
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

/// One source of a level of a file's view as it is merged: what the
/// source has at that level.
#[derive(Clone, Copy, Debug)]
enum Level {
    /// The roots of a directory of the file's chain, with the source's
    /// position ([`ViewPart::source`]).
    Dir(u32, u32),
    /// An imported member or root of a `*`: its bound name and its node.
    Alias(Name, ViewPart),
    /// The children of a part of a node of the view.
    Of(ViewPart),
}

/// A conflict found merging a file's view: the view node it is under
/// ([`ROOT`] for a root, else a view node by its index among the file's),
/// its name, and the parts, at least one a leaf.
type Conflict = (u32, Name, Vec<ViewPart>);

/// A file's view as [`file_view`] finds it: its nodes, numbered from 0,
/// what the merge made of each name several sources have, and the
/// conflicts.
type FileView = (Vec<ViewNode>, HashMap<(u32, Name), Merged>, Vec<Conflict>);

/// What identifies one side of a conflict, so that a conflict seen from
/// several files' views is reported once: its node, and for an import the
/// importing file and where the import binds it.
type Side = (Target, Option<(u32, u32)>);

impl Scan {
    /// The number of names `level` has.
    fn level_size(&self, level: Level) -> usize {
        match level {
            Level::Dir(dir, _) => self.roots.get(dir as usize).map_or(0, Vec::len),
            Level::Alias(..) => 1,
            Level::Of(part) => match part.target {
                Target::Node(node) => self
                    .nodes
                    .get(node as usize)
                    .map_or(0, |n| n.children.len()),
                Target::Part { file, node } => self
                    .files
                    .get(file as usize)
                    .and_then(|f| f.mentions.tree.nodes.get(node as usize))
                    .map_or(0, |n| n.children.len()),
                _ => 0,
            },
        }
    }

    /// Each name `level` has, with its node, in the source's order.
    fn level_entries(&self, level: Level) -> Vec<(Name, ViewPart)> {
        match level {
            Level::Dir(dir, source) => {
                let roots = self.roots.get(dir as usize).map_or(&[][..], Vec::as_slice);
                roots
                    .iter()
                    .filter_map(|&root| {
                        let name = self.nodes.get(root as usize)?.name;
                        Some((name, ViewPart::chain(source, root)))
                    })
                    .collect()
            }
            Level::Alias(name, part) => vec![(name, part)],
            Level::Of(part) => self
                .child_names(part.target)
                .into_iter()
                .filter_map(|name| Some((name, self.level_lookup(level, name)?)))
                .collect(),
        }
    }

    /// The node `level` has of the name `name`, if any.
    fn level_lookup(&self, level: Level, name: Name) -> Option<ViewPart> {
        match level {
            Level::Dir(dir, source) => self
                .by_name
                .get(&(dir, ROOT, name))
                .map(|&root| ViewPart::chain(source, root)),
            Level::Alias(bound, part) => (bound == name).then_some(part),
            Level::Of(part) => Some(ViewPart {
                target: self.target_child(part.target, name)?,
                ..part
            }),
        }
    }

    /// Each name that more than one of `levels`, the sources of one node of
    /// a view, have, with their nodes, sources root directory first. The
    /// largest source is never walked, only looked up in, so that a level
    /// costs the size of the others.
    fn overlaps(&self, levels: &[Level]) -> Vec<(Name, Vec<ViewPart>)> {
        if levels.len() < 2 {
            return Vec::new();
        }
        let largest = (0..levels.len())
            .max_by_key(|&at| levels.get(at).map_or(0, |&level| self.level_size(level)));
        let mut found: HashMap<Name, Vec<ViewPart>> = HashMap::new();
        let mut order = Vec::new();
        for (at, &level) in levels.iter().enumerate() {
            if Some(at) == largest {
                continue;
            }
            for (name, part) in self.level_entries(level) {
                found
                    .entry(name)
                    .or_insert_with(|| {
                        order.push(name);
                        Vec::new()
                    })
                    .push(part);
            }
        }
        let largest = largest.and_then(|at| levels.get(at)).copied();
        let mut overlapping = Vec::new();
        for name in order {
            let Some(mut parts) = found.remove(&name) else {
                continue;
            };
            if let Some(level) = largest
                && let Some(part) = self.level_lookup(level, name)
            {
                parts.push(part);
            }
            if parts.len() > 1 {
                parts.sort_by_key(|part| part.source);
                overlapping.push((name, parts));
            }
        }
        overlapping
    }

    /// `parts`, the nodes several sources have at one name, with the same
    /// cell reached twice taken once: a node met again, and a file's part
    /// of a merged node also met whole. Merging is idempotent.
    fn same_cells(&self, parts: &[ViewPart]) -> Vec<ViewPart> {
        let whole = |part: &ViewPart| match part.target {
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.global.get(node as usize))
                .is_some_and(|&merged| {
                    parts
                        .iter()
                        .any(|other| other.target == Target::Node(merged))
                }),
            _ => false,
        };
        let mut kept: Vec<ViewPart> = Vec::with_capacity(parts.len());
        for part in parts {
            if !kept.iter().any(|k| k.target == part.target) && !whole(part) {
                kept.push(*part);
            }
        }
        kept
    }

    /// Whether `target` is a leaf: a merged node declared by an item, or a
    /// built-in function's.
    fn is_leaf(&self, target: Target) -> bool {
        matches!(target, Target::Node(node)
            if self.nodes.get(node as usize).is_some_and(|n| n.leaf.is_some())
                || self.builtin(node).is_some())
    }

    /// The file that declares the first leaf at or under `target`, a merged
    /// node or a file's part of one, and the spans of that leaf's path and
    /// whole item.
    fn part_spans(&self, target: Target) -> Option<(u32, Span, Span)> {
        let (file, own) = match target {
            Target::Node(node) => {
                let entry = self.nodes.get(node as usize)?;
                let file = entry.leaf.or_else(|| entry.contributors.first().copied())?;
                let own = self
                    .files
                    .get(file as usize)?
                    .global
                    .iter()
                    .position(|&g| g == node)?;
                (file, count(own))
            }
            Target::Part { file, node } => (file, node),
            _ => return None,
        };
        let (path, item) = first_leaf(self.files.get(file as usize)?, own)?;
        Some((file, path, item))
    }
}

/// Finds every file's view (see the module documentation, "The view of a
/// file"): the nodes several of its sources have, merged, into
/// [`Scan::views`] and [`File::view`]. Reports each conflict, two sources
/// with a leaf and anything else of one path, once per pair however many
/// files' views it is in.
fn views(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    let mut reported: HashSet<(Side, Side)> = HashSet::new();
    let mut found = Vec::with_capacity(scan.files.len());
    for index in 0..count(scan.files.len()) {
        let (mut nodes, merged, conflicts) = file_view(scan, index);
        for (parent, name, parts) in conflicts {
            report_conflicts(
                scan,
                index,
                &nodes,
                (parent, name, &parts),
                &mut reported,
                errors,
            );
        }
        // Its own nodes first, each after its parent as before: a node with
        // a leaf of the file under it has only such nodes above it.
        let start = count(scan.views.len());
        let mut renumbered = vec![0u32; nodes.len()];
        let mut at = start;
        for own in [true, false] {
            for (local, node) in nodes.iter().enumerate() {
                if node.own.is_some() == own
                    && let Some(entry) = renumbered.get_mut(local)
                {
                    *entry = at;
                    at = at.saturating_add(1);
                }
            }
        }
        let own = count(nodes.iter().filter(|node| node.own.is_some()).count());
        let global = |local: u32| renumbered.get(local as usize).copied().unwrap_or(ROOT);
        for node in &mut nodes {
            node.parent = node.parent.map(global);
        }
        let mut ordered: Vec<Option<ViewNode>> = vec![None; nodes.len()];
        for (local, node) in nodes.into_iter().enumerate() {
            let slot = global(count(local)).saturating_sub(start);
            if let Some(entry) = ordered.get_mut(slot as usize) {
                *entry = Some(node);
            }
        }
        scan.views.extend(ordered.into_iter().flatten());
        let merged = merged
            .into_iter()
            .map(|((parent, name), value)| {
                let parent = if parent == ROOT { ROOT } else { global(parent) };
                let value = match value {
                    Merged::View(local) => Merged::View(global(local)),
                    one @ Merged::One(_) => one,
                };
                ((parent, name), value)
            })
            .collect();
        found.push(View {
            start,
            own,
            end: count(scan.views.len()),
            merged,
        });
    }
    for (file, view) in scan.files.iter_mut().zip(found) {
        file.view = view;
    }
}

/// The view of file `index`: its view nodes, numbered from 0 in the order
/// found, each a parent before its children; what the merge made of each
/// name several sources have, by parent (a local number or [`ROOT`]); and
/// the conflicts.
fn file_view(scan: &Scan, index: u32) -> FileView {
    let (mut nodes, mut merged, mut conflicts) = (Vec::new(), HashMap::new(), Vec::new());
    let Some(file) = scan.files.get(index as usize) else {
        return (nodes, merged, conflicts);
    };
    let mut chain: Vec<u32> = scan.chain(file.dir).collect();
    chain.reverse();
    let depth = count(chain.len());
    let mut levels: Vec<Level> = (0u32..)
        .zip(&chain)
        .map(|(source, &dir)| Level::Dir(dir, source))
        .collect();
    for (at, alias) in (0u32..).zip(&file.aliases) {
        if !alias.whole {
            let part = ViewPart {
                source: depth.saturating_add(at),
                target: alias.target,
                chain: false,
            };
            levels.push(Level::Alias(alias.name, part));
        }
    }
    let local: HashMap<u32, u32> = (0u32..)
        .zip(&file.global)
        .map(|(own, &global)| (global, own))
        .collect();
    let mut stack = vec![(ROOT, levels)];
    while let Some((parent, levels)) = stack.pop() {
        for (name, parts) in scan.overlaps(&levels) {
            let parts = scan.same_cells(&parts);
            if let [one] = parts.as_slice() {
                merged.insert((parent, name), Merged::One(*one));
                continue;
            }
            if parts.iter().any(|part| scan.is_leaf(part.target)) {
                conflicts.push((parent, name, parts));
                continue;
            }
            let own = parts.iter().find_map(|part| match part.target {
                Target::Node(node) if part.chain => scan
                    .nodes
                    .get(node as usize)
                    .filter(|n| n.contributors.contains(&index))
                    .and_then(|_| local.get(&node).copied()),
                _ => None,
            });
            let view = count(nodes.len());
            nodes.push(ViewNode {
                name,
                parent: (parent != ROOT).then_some(parent),
                file: index,
                parts: parts.clone(),
                own,
            });
            merged.insert((parent, name), Merged::View(view));
            stack.push((view, parts.into_iter().map(Level::Of).collect()));
        }
    }
    (nodes, merged, conflicts)
}

/// Reports the conflict `conflict` of file `index`'s view, whose view
/// nodes so far are `nodes`: each part, in the order the sources are
/// reported in (its own directory, then each directory upward, then its
/// imports in source order), against the first before it that it conflicts
/// with, once per pair (`reported`), at the later of the two.
fn report_conflicts(
    scan: &Scan,
    index: u32,
    nodes: &[ViewNode],
    (parent, name, parts): (u32, Name, &[ViewPart]),
    reported: &mut HashSet<(Side, Side)>,
    errors: &mut Vec<Diagnostic>,
) {
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    let depth = count(scan.chain(file.dir).count());
    // Its own directory first (the last of the chain), then upward, then
    // the imports.
    let rank = |part: &ViewPart| {
        if part.chain {
            depth.saturating_sub(1).saturating_sub(part.source)
        } else {
            part.source
        }
    };
    let mut parts = parts.to_vec();
    parts.sort_by_key(rank);
    let mut path = Vec::new();
    let mut at = (parent != ROOT).then_some(parent);
    while let Some(view) = at
        && let Some(node) = nodes.get(view as usize)
    {
        path.push(scan.names.get(node.name));
        at = node.parent;
    }
    path.reverse();
    path.push(scan.names.get(name));
    let shown = path.join(".");
    for (later_at, later) in parts.iter().enumerate() {
        let earlier = parts
            .get(..later_at)
            .into_iter()
            .flatten()
            .find(|earlier| scan.is_leaf(earlier.target) || scan.is_leaf(later.target));
        let Some(earlier) = earlier else { continue };
        let side = |part: &ViewPart| -> Side {
            let import = (!part.chain).then(|| {
                let alias = part.source.saturating_sub(depth);
                let start = file.aliases.get(alias as usize).map_or(0, |a| a.span.start);
                (index, start)
            });
            (part.target, import)
        };
        if !reported.insert((side(later), side(earlier))) {
            continue;
        }
        if let Some(error) = conflict_between(scan, index, &shown, (*later, *earlier), depth) {
            errors.push(error);
        }
    }
}

/// The conflict at the view path `shown` of file `index` (whose chain has
/// `depth` directories) between the later part and the earlier one: two
/// leaves, "defined in two files", or a leaf and an interior node, "already
/// defined", at the later, labelled as the module documentation says.
fn conflict_between(
    scan: &Scan,
    index: u32,
    shown: &str,
    (later, earlier): (ViewPart, ViewPart),
    depth: u32,
) -> Option<Diagnostic> {
    let file = scan.files.get(index as usize)?;
    let leaves = scan.is_leaf(later.target) && scan.is_leaf(earlier.target);
    let message = if leaves {
        format!("field `{shown}` is defined in two files")
    } else {
        format!("field `{shown}` is already defined")
    };
    let imported_at = |part: &ViewPart| {
        file.aliases
            .get(part.source.saturating_sub(depth) as usize)
            .map(|alias| alias.span)
    };
    // A built-in function's side, which only an import reaches, has no
    // definition to show: a note says what it is instead.
    let builtin = |part: &ViewPart| match part.target {
        Target::Node(node) => scan.builtin(node),
        _ => None,
    };
    let mut error = if later.chain {
        let (later_file, later_path, _) = scan.part_spans(later.target)?;
        let error = Diagnostic::error(message, later_path).in_input(later_file);
        if leaves {
            error.with_label("defined here")
        } else {
            error
        }
    } else {
        let error = Diagnostic::error(message, imported_at(&later)?)
            .with_label("imported here")
            .in_input(index);
        match scan.part_spans(later.target) {
            Some((later_file, later_path, _)) => {
                context_in(error, later_file, later_path, "defined here")
            }
            None => error,
        }
    };
    if !earlier.chain {
        error = context_in(error, index, imported_at(&earlier)?, "imported here");
    }
    if let Some((earlier_file, earlier_path, earlier_item)) = scan.part_spans(earlier.target) {
        let earlier_span = if leaves { earlier_path } else { earlier_item };
        error = context_in(error, earlier_file, earlier_span, "defined here");
    }
    for part in [later, earlier] {
        if let Some(builtin) = builtin(&part) {
            error = error.with_note(format!("`{}` is a built-in function", builtin.name()));
        }
    }
    // A directory above the file's own is one side.
    let own_dir = depth.saturating_sub(1);
    if [later, earlier]
        .iter()
        .any(|part| part.chain && part.source != own_dir)
    {
        error = error.with_note("one is in a directory above");
    }
    Some(error)
}

/// `error` with the label `text` at `span` of file `input`: a label of the
/// primary span's own file when it is that file, else an excerpt of its
/// own.
fn context_in(error: Diagnostic, input: u32, span: Span, text: &str) -> Diagnostic {
    if error.input == Some(input) {
        error.with_context(span, text)
    } else {
        error.with_context_in(input, span, text)
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
/// its directory chain. `nested` are the directories that are workspaces of
/// their own, and `inherited` the imports of a session's earlier inputs,
/// which a session's input, the scan's one file of its own, inherits.
fn imports(
    scan: &mut Scan,
    dirs: &HashMap<String, u32>,
    (nested, inherited): (&[String], &[Alias]),
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
        if index >= scan.standard {
            inherit_imports(file, inherited, &mut aliases);
        }
        implicit_prelude(scan, index, &mut aliases);
        clashes(scan, index, &aliases, &mut own_errors);
        errors.extend(own_errors.into_iter().map(|e| e.in_input(index)));
        found.push(aliases);
    }
    for (file, aliases) in scan.files.iter_mut().zip(found) {
        file.aliases = aliases;
    }
}

/// Adds to `aliases`, the imports of `file`, a session's input, each of
/// `inherited`, the imports of the session's earlier inputs, that the input
/// neither imports again nor declares in the whole of its top level (by a
/// `fn`, field or `def` whose path starts with its name, or a `:=` of it,
/// which means the variable of its name): a name of an earlier input is
/// shadowed by a later input's, whether it was imported or declared (see
/// [`crate::session`]). A `let` shadows one only from its point on, as it
/// does an earlier input's variable: the checker reads an inherited import
/// through the view until a binding of its name is found
/// ([`FileScope::inherited`]). They follow the input's own imports, before
/// the implicit prelude, which yields to them, and clash with nothing.
fn inherit_imports(file: &File, inherited: &[Alias], aliases: &mut Vec<Alias>) {
    if inherited.is_empty() {
        return;
    }
    let ast = &file.ast;
    let declared: HashSet<Name> = ast
        .items(ast.root())
        .iter()
        .filter_map(|&item| match ast.item(item).kind {
            ItemKind::Let {
                path,
                function: true,
                ..
            }
            | ItemKind::Rebind { path, .. }
            | ItemKind::Field { path, .. } => ast.path(path).first().and_then(PathSegment::name),
            _ => None,
        })
        .collect();
    let own = aliases.len();
    for alias in inherited {
        let imported = aliases.iter().take(own).any(|a| a.name == alias.name);
        if !imported && !declared.contains(&alias.name) {
            aliases.push(*alias);
        }
    }
}

/// Adds to `aliases`, the imports of file `index`, the implicit `import *
/// from prelude` of every file but the prelude itself: an implicit member
/// alias for each root of the prelude that no earlier source of the file's
/// view has, neither a root of its directory chain (its own declarations
/// among them) nor a name it imports; each that one has is dropped, shadowed
/// silently (see the module documentation, "The standard library").
fn implicit_prelude(scan: &Scan, index: u32, aliases: &mut Vec<Alias>) {
    let Some(prelude) = scan.standard_file("prelude") else {
        return;
    };
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    if index == prelude {
        return;
    }
    let explicit = aliases.len();
    for root in target_roots(scan, Target::File(prelude)) {
        let shadowed = scan
            .chain(file.dir)
            .any(|dir| scan.by_name.contains_key(&(dir, ROOT, root)))
            || aliases
                .get(..explicit)
                .into_iter()
                .flatten()
                .any(|a| a.name == root);
        if shadowed {
            continue;
        }
        if let Some(target) = scan.target_child(Target::File(prelude), root) {
            aliases.push(Alias {
                name: root,
                span: Span::new(0, 0),
                item: None,
                target,
                whole: false,
                implicit: true,
                inherited: false,
            });
        }
    }
}

/// What the module path `path` names, or the error that it names nothing
/// (`None` for a directory and file of one name already reported, and for
/// any path of a lone file but the standard library's, which the checker
/// reports). A single segment naming the `builtins` module or a standard
/// file names it, before anything of the workspace.
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
    if let [only] = shown.as_slice() {
        if *only == BUILTINS {
            return Ok(Target::Dir(scan.builtins_dir));
        }
        if let Some(file) = scan.standard_file(only) {
            return Ok(Target::File(file));
        }
    }
    if scan.lone {
        return Err(None);
    }
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
        .filter(|&file| scan.files.get(file as usize).is_some_and(is_value_file))
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

/// Whether `file` is a value file: its outermost block ends in a result.
fn is_value_file(file: &File) -> bool {
    file.ast
        .items(file.ast.root())
        .last()
        .is_some_and(|&item| matches!(file.ast.item(item).kind, ItemKind::Result(_)))
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
                    item: Some(item),
                    target,
                    whole: true,
                    implicit: false,
                    inherited: false,
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
                                item: Some(item),
                                target: child,
                                whole: false,
                                implicit: false,
                                inherited: false,
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
                        if let Target::File(of) = target
                            && scan.files.get(of as usize).is_some_and(is_value_file)
                        {
                            errors.push(Diagnostic::error(
                                "a value file has no members to import",
                                star,
                            ));
                            continue;
                        }
                        for root in target_roots(scan, target) {
                            if let Some(child) = scan.target_child(target, root) {
                                aliases.push(Alias {
                                    name: root,
                                    span: star,
                                    item: Some(item),
                                    target: child,
                                    whole: false,
                                    implicit: false,
                                    inherited: false,
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
        Target::Part { .. } | Target::Node(_) | Target::View(_) => Vec::new(),
    }
}

/// Reports each name of `aliases`, the imports of file `index`, that
/// clashes: a whole import with another import, a top-level `fn`, `let`,
/// `let mut`, `:=` or field root of the file, or a root of its directory
/// chain; a member or a root of a `*`, which merges into the file's view
/// with the file's fields, its directory chain and the other members, with
/// a whole import or a top-level `fn`, `let`, `let mut` or `:=`. The
/// implicit prelude's aliases and those a session's input inherits clash
/// with nothing.
fn clashes(scan: &Scan, index: u32, aliases: &[Alias], errors: &mut Vec<Diagnostic>) {
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    // The implicit `import * from prelude` and the imports a session's input
    // inherits alone clash with nothing.
    if aliases
        .iter()
        .all(|alias| alias.implicit || alias.inherited)
    {
        return;
    }
    let ast = &file.ast;
    // The file's own top-level names, first declaration of each, and
    // whether it is a field's.
    let mut declared: Vec<(Name, Span, bool)> = Vec::new();
    let mut seen: HashSet<(Name, bool)> = HashSet::new();
    for &item in ast.items(ast.root()) {
        let (field, path) = match ast.item(item).kind {
            ItemKind::Let { path, .. } | ItemKind::Rebind { path, .. } => (false, path),
            ItemKind::Field { path, .. } => (true, path),
            _ => continue,
        };
        if let Some(first) = ast.path(path).first()
            && let Some(name) = first.name()
            && seen.insert((name, field))
        {
            declared.push((name, first.span, field));
        }
    }
    let star = |alias: &Alias| {
        let Some(ItemKind::Import(id)) = alias.item.map(|item| ast.item(item).kind) else {
            return false;
        };
        matches!(ast.import_decl(id).form, ImportForm::Members(range)
            if ast.import_members(range).iter().any(|m| m == &ImportMember::All(alias.span)))
    };
    // The implicit `import * from prelude` clashes with nothing: its names
    // yield to every other (see the module documentation, "The standard
    // library"). Nor does an inherited import, which the input's
    // declarations shadow ([`inherit_imports`]).
    for (position, alias) in aliases.iter().enumerate() {
        if alias.implicit || alias.inherited {
            continue;
        }
        let shown = scan.names.get(alias.name);
        let earlier = aliases
            .get(..position)
            .and_then(|before| {
                before
                    .iter()
                    .find(|a| !a.implicit && a.name == alias.name && (alias.whole || a.whole))
            })
            .map(|a| (a.span, "imported here"));
        let other = earlier.or_else(|| {
            declared
                .iter()
                .filter(|&&(n, _, field)| n == alias.name && (alias.whole || !field))
                .min_by_key(|&&(_, span, _)| span.start)
                .map(|&(_, span, _)| (span, "declared here"))
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
        if alias.whole && above.is_some() {
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

/// The files a whole read by file `index` of its view node `view` needs:
/// every owner of every leaf under it, in whichever directory or import,
/// but `index` itself, whose own leaves are computed in its block; with the
/// node of an import that reaches into `index`, if one does, which is a
/// cycle of one file, as a reach through a whole import is.
fn view_owners(scan: &Scan, index: u32, view: u32) -> (Vec<u32>, Option<Target>) {
    let mut owners = Vec::new();
    let mut reached = None;
    let parts = scan.views.get(view as usize).map_or(&[][..], |v| &v.parts);
    for &part in parts {
        let files = scan.part_owners(part);
        if !part.chain && files.contains(&index) {
            reached.get_or_insert(part.target);
        } else {
            owners.extend(files.into_iter().filter(|&file| file != index));
        }
    }
    (owners, reached)
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
        for (position, fields) in file.mentions.fields.iter().enumerate() {
            let spans = file.mentions.field_spans.get(position);
            let foreign = file.mentions.foreign.get(position).into_iter().flatten();
            let mut nodes: Vec<(u32, Span)> = Vec::new();
            // The nodes of the view it reads whole, each with the reference.
            let mut views: Vec<(u32, Span)> = Vec::new();
            for (&n, &span) in fields.iter().zip(spans.into_iter().flatten()) {
                if let Some(&global) = file.global.get(n as usize) {
                    nodes.push((global, span));
                } else if let Some(view) = scope.own_view(n) {
                    views.push((view, span));
                }
            }
            for &(need, span) in foreign {
                match need {
                    // Through an import: what it reaches needs its files,
                    // and reaching into this file is a cycle of one file.
                    Need::Field(n) => {
                        let target = match scope.target(n) {
                            None => {
                                nodes.extend(scope.merged(n).map(|merged| (merged, span)));
                                continue;
                            }
                            Some(Target::View(view)) => {
                                views.push((view, span));
                                continue;
                            }
                            Some(target) => target,
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
                        if let Some(function) = scope
                            .foreign_function(f)
                            .and_then(|id| scan.functions.get(id as usize))
                        {
                            record(function.file, span, &|| scan.show_function(*function));
                        }
                    }
                }
            }
            for (merged, span) in nodes {
                for owner in scan.target_files(Target::Node(merged)) {
                    record(owner, span, &|| scan.show(merged));
                }
            }
            for (view, span) in views {
                let (owners, reached) = view_owners(scan, index, view);
                if let Some(target) = reached
                    && !itself
                {
                    itself = true;
                    errors.push(needs_itself(scan, index, target, span));
                }
                for owner in owners {
                    record(owner, span, &|| scan.show_view(view));
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
/// file in the order computes left out, and the nodes of its view with a
/// leaf of the file under them that it reads whole in: those mentioned and
/// the view nodes under them, each needing its own children (see
/// [`Mentions::views`]).
fn schedule(scan: &mut Scan) {
    let edges: Vec<(Vec<mentions::ViewNode>, Vec<bool>)> = scan
        .files
        .iter()
        .map(|file| view_edges(scan, file))
        .collect();
    let mut needs = Vec::with_capacity(edges.len());
    for (file, (views, needed)) in scan.files.iter_mut().zip(edges) {
        file.mentions.views = views;
        needs.push(needed);
    }
    let rank = &scan.rank;
    let mut schedules = Vec::with_capacity(scan.files.len());
    for ((index, file), needed) in (0u32..).zip(&scan.files).zip(needs) {
        let mut done: Vec<bool> = file
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
        done.extend(needed.iter().map(|&needed| !needed));
        schedules.push(file.mentions.schedule_except(&done));
    }
    for (file, schedule) in scan.files.iter_mut().zip(schedules) {
        file.schedule = schedule;
    }
}

/// The nodes of `file`'s view with a leaf of the file under them, as the
/// nodes of its graph of what needs what ([`Mentions::views`]), placed by
/// the file's own node of their path, with whether the file reads each
/// whole: those it mentions, and the view nodes under them. Only those
/// have their children, the file's own nodes or such view nodes, as edges:
/// finding every one's would cost every file of a directory the children of
/// its directory's nodes, which a whole read costs only the file that
/// reads them.
fn view_edges(scan: &Scan, file: &File) -> (Vec<mentions::ViewNode>, Vec<bool>) {
    let scope = FileScope::new(scan, file);
    let own = count(file.global.len());
    let start = file.view.start;
    let nodes = scan
        .views
        .get(start as usize..start.saturating_add(file.view.own) as usize)
        .unwrap_or_default();
    let mut views: Vec<mentions::ViewNode> = nodes
        .iter()
        .map(|node| {
            let tree = node
                .own
                .and_then(|own| file.mentions.tree.nodes.get(own as usize));
            mentions::ViewNode {
                name: node.name,
                parent: node
                    .parent
                    .map(|parent| own.saturating_add(parent.saturating_sub(start))),
                children: Vec::new(),
                first: tree.map_or(0, |tree| tree.first),
                last: tree.map_or(0, |tree| tree.last),
            }
        })
        .collect();
    let mut needed = vec![false; views.len()];
    let mut stack: Vec<u32> = file
        .mentions
        .fields
        .iter()
        .flatten()
        .copied()
        .filter(|&n| scope.own_view(n).is_some())
        .collect();
    while let Some(number) = stack.pop() {
        let at = number.saturating_sub(own) as usize;
        if needed.get(at) != Some(&false) {
            continue;
        }
        let children: Vec<u32> = scope
            .view_children(number)
            .into_iter()
            .map(|(_, child)| child)
            .filter(|&child| child < scope.merged_start())
            .collect();
        stack.extend(children.iter().filter(|&&c| scope.own_view(c).is_some()));
        if let (Some(seen), Some(view)) = (needed.get_mut(at), views.get_mut(at)) {
            *seen = true;
            view.children = children;
        }
    }
    (views, needed)
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
    /// Whether the file is a session's input, whose scope is the session's:
    /// its variables are read as a session's input reads them, captured
    /// by a function, not read through the top level's frame as a
    /// directory's committed declarations are (see [`check_input`]).
    session: bool,
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
            session: false,
        })
    }

    /// Whether the file is a session's input (see [`Context::session`]'s
    /// field).
    pub(crate) fn session(&self) -> bool {
        self.session
    }

    /// Whether the import item `item` of a lone file, or of a session's
    /// input, names nothing it can: a path of anything but `builtins` and
    /// the standard files, which the checker reports as needing a
    /// workspace (see the module documentation, "The standard library").
    pub(crate) fn needs_workspace(&self, item: ItemId) -> bool {
        self.scan.lone && !self.file.aliases.iter().any(|a| a.item == Some(item))
    }

    /// The slot of the cell numbered `cell` (see [`Scan::slot`]).
    pub(crate) fn slot(&self, cell: u32) -> Slot {
        self.scan.slot(cell)
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
            .unwrap_or_else(|| self.scan.slot(node))
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

    /// The fields of the record of `target`, a directory, a file, a part or
    /// a view node read whole: each child's name and target, in the order of
    /// the directory's or the file's tree, or the view's.
    pub(crate) fn target_fields(&self, target: Target) -> Vec<(Name, Target)> {
        let names = match target {
            Target::Part { .. } | Target::View(_) => self.scan.child_names(target),
            other => target_roots(self.scan, other),
        };
        names
            .into_iter()
            .filter_map(|name| Some((name, self.scan.target_child(target, name)?)))
            .collect()
    }

    /// A name for the slot of the record of `target`: the name the file
    /// imports it as, or a part's or a view node's last name.
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
            Target::View(view) => self.scan.views.get(view as usize).map(|v| v.name),
            _ => self.file.aliases.first().map(|a| a.name),
        }
    }

    /// Whether a field access in the file may read through its view rather
    /// than a binding: it imports something that has children, or several
    /// of its sources share a node (see [`FileScope::reads_through`]). The
    /// implicit `import * from prelude` of leaves alone does not count: a
    /// field access on a leaf reads the leaf's value.
    pub(crate) fn has_view(&self) -> bool {
        self.file
            .aliases
            .iter()
            .any(|a| !a.implicit || !self.scan.is_leaf(a.target))
            || self.file.view.end > self.file.view.start
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
                Some((
                    entry.name,
                    own,
                    self.scan.slot(child),
                    self.cell_type(child),
                ))
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
    /// The layout of the record `exec` returns, built once for the run as
    /// the frame is laid out ([`exec_layout`]).
    exec_result: Rc<Layout>,
    /// The table the files' types are interned into.
    types: TypeTable,
    /// The name of every slot of the frame, the last variable's in it, for
    /// [`Workspace::dump_ir`]: one table for every file, each file's IR
    /// keeping none (a copy of the whole frame's in each would make the
    /// workspace's memory quadratic in its files).
    slot_names: Vec<Name>,
    /// For each file, by input number, the number of slots it committed,
    /// and the names of its slots past them, its temporaries', which a later
    /// file may take and name otherwise in `slot_names`: what
    /// [`Workspace::dump_ir`] names them by.
    slot_tails: Vec<(usize, Vec<Name>)>,
    /// The type of each cell, by its number, as its file's check found it:
    /// what a session's inputs read the standard library's cells as.
    cell_types: Vec<TyId>,
    /// The slot each cell is read from, by merged node: its own, or for a
    /// `def` its function's (see [`Context::cell_slot`]). Kept for a
    /// session's inputs, and for the tests' check that every cell an import
    /// reaches is stored by its owner.
    cell_slots: Vec<Slot>,
    /// For each cell that is a `def`'s, which parameters its body measures
    /// (see [`Context::measured`]), kept for a session's inputs.
    cell_measured: Vec<Option<Box<[bool]>>>,
    /// Each file's records of the nodes of its view with a leaf of it under
    /// them, by input number: kept for the tests' check that each reads
    /// only cells stored before it.
    #[cfg(test)]
    view_records: Vec<Vec<ViewRecord>>,
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
    check_scan(scan_with(sources, nested)?)
}

/// Scans, checks and lowers `source`, a file outside any workspace (`gcsl
/// run` of a lone file), as the workspace of one directory holding just
/// it, beside the standard library (see the module documentation, "The
/// standard library"): it is file [`Scan::standard`], after the standard
/// files.
///
/// # Errors
///
/// As [`check`]; an import of anything but `builtins` and the standard
/// files is the checker's error, "`lib` is not a built-in or standard
/// module, and there is no `workspace.gcsl` above this file".
pub fn check_lone(source: &SourceFile) -> Result<Workspace, Vec<Diagnostic>> {
    check_scan(scan_into(
        std::slice::from_ref(source),
        Placing::Lone(&[]),
        &mut Names::new(),
    )?)
}

/// The tables of the cells of `scan` before any file is checked: each
/// one's type, `Error` but for the built-in functions', the first
/// ([`builtin_nodes`]), each built-in's (see [`Builtin::ty`]), interned
/// into `types` with `names`; each one's slot; and no `def`'s measured
/// parameters.
fn cell_tables(scan: &Scan, types: &mut TypeTable, names: &mut Names) -> CellTables {
    let mut cell_types = vec![TyId::ERROR; scan.cells()];
    for (cell, builtin) in cell_types.iter_mut().zip(Builtin::ALL) {
        *cell = builtin.ty(types, names);
    }
    let cell_slots = (0..count(scan.cells()))
        .map(|cell| scan.slot(cell))
        .collect();
    (cell_types, cell_slots, vec![None; scan.cells()])
}

/// Takes the names of the slots of `ir`, a file's: its slots' names are the
/// workspace's (see [`Workspace::dump_ir`]), but for those of its
/// temporaries past the `committed` slots it committed, which a later file
/// may take and name otherwise, which are kept, with `committed`.
fn own_slot_names(ir: &mut Ir, committed: usize) -> (usize, Vec<Name>) {
    let own = ir.replace_slot_names(Vec::new());
    (committed, own.get(committed..).unwrap_or_default().to_vec())
}

/// The tables of a workspace's cells, by number: each one's type, the slot
/// it is read from, and for a `def`'s which parameters its body measures.
type CellTables = (Vec<TyId>, Vec<Slot>, Vec<Option<Box<[bool]>>>);

/// Checks and lowers every file of `scan` in the order (see [`check`]).
fn check_scan(mut scan: Scan) -> Result<Workspace, Vec<Diagnostic>> {
    let mut names = mem::take(&mut scan.names);
    let mut types = TypeTable::new();
    let (mut cell_types, mut cell_slots, mut cell_measured) =
        cell_tables(&scan, &mut types, &mut names);
    let mut slot_names: Vec<Name> = scan.cell_names();
    let mut scopes = vec![Committed::default(); scan.dirs.len()];
    let mut irs: Vec<Option<Ir>> = (0..scan.files.len()).map(|_| None).collect();
    let mut tails: Vec<(usize, Vec<Name>)> = vec![(0, Vec::new()); scan.files.len()];
    let mut checked: Vec<Option<Types>> = (0..scan.files.len()).map(|_| None).collect();
    let mut warnings = vec![Vec::new(); scan.files.len()];
    let mut errors = Vec::new();
    // The files whose check failed, or that need one that did: those are
    // not checked, since their errors would follow from the first.
    let mut failed = vec![false; scan.files.len()];
    #[cfg(test)]
    let mut view_records: Vec<Vec<ViewRecord>> = vec![Vec::new(); scan.files.len()];
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
        let keep = |name: Name| commits(&scan, index, name);
        #[cfg(test)]
        if let Some(Ok((_, output))) = &compiled {
            let records = output.checked.resolution.view_records();
            set(&mut view_records, index, records.to_vec());
        }
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
                // Every file of the order is a file of the workspace.
                Err(failed_check.and_then(Result::err).unwrap_or_default())
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
                set(&mut tails, index, own_slot_names(&mut ir, slot_names.len()));
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
        // Built before `scan` moves; its names are interned already.
        exec_result: exec_layout(&mut scan.names),
        scan,
        irs,
        checked,
        warnings: warnings.into_iter().flatten().collect(),
        // The driver writes the built-in functions' cells, the first slots,
        // as it lays the frame out, before any file runs.
        frame: builtin_cells(),
        slot_names,
        slot_tails: tails,
        memo: MemoTables::new(),
        types,
        cell_types,
        cell_slots,
        cell_measured,
        #[cfg(test)]
        view_records,
    })
}

/// Whether file `index` commits its declaration `name` for the files
/// checked after it: a root it computes, being its last contributor, or a
/// `def`.
fn commits(scan: &Scan, index: u32, name: Name) -> bool {
    let Some(file) = scan.files.get(index as usize) else {
        return false;
    };
    file.mentions.tree.root(name).is_some_and(|root| {
        let global = file.global.get(root as usize).copied();
        global.and_then(|global| scan.computed_by(global)) == Some(index)
    }) || file.mentions.functions.iter().any(|top| {
        top.name == name && matches!(file.ast.item(top.item).kind, ItemKind::Field { .. })
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
    let measured_by_slot = output.scope.measured_by_slot();
    let mut cells = Vec::new();
    for (own, (&global, &ty)) in global.iter().zip(&output.cell_types).enumerate() {
        if scan.computed_by(global) == Some(index) {
            // A `def`'s cell is read from its function's slot.
            let slot = slots.get(own).copied().unwrap_or_else(|| scan.slot(global));
            let measured = measured_by_slot
                .get(&slot)
                .copied()
                .flatten()
                .map(Box::from);
            cells.push((global, slot, ty, measured));
        }
    }
    // An imported value file's value is a cell of the workspace.
    if let Some(cell) = scan.value_cell(index) {
        cells.push((cell, scan.slot(cell), output.ty, None));
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
        // The names of the slots it committed, of which its frame is a
        // prefix, then its own temporaries'.
        let (committed, tail) = self.slot_tails.get(index as usize)?;
        let mut names = self
            .slot_names
            .get(..*committed)
            .unwrap_or_default()
            .to_vec();
        names.extend_from_slice(tail);
        ir.replace_slot_names(names);
        let dump = ir.pretty(&self.scan.names).to_string();
        ir.replace_slot_names(Vec::new());
        Some(dump)
    }

    /// Runs file `index` and, before it, the standard files and every file
    /// it needs that has not run, in the order ([`Scan::needed`]), on the
    /// frame whose first cells the driver wrote with the built-in
    /// functions, and returns its own value: its result, or the
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
            let context = (&mut self.memo, Some(&self.exec_result));
            match session::run_input(ir, &mut self.frame, context) {
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

/// `diagnostics` of the file `index` of a scan of one file beside the
/// standard library, as the diagnostics of the input being run name it: by
/// no input, as a label in it does, a label in a standard file keeping that
/// file's number, which is its input number in a session too.
#[must_use]
pub fn being_run(mut diagnostics: Vec<Diagnostic>, index: u32) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        if diagnostic.input == Some(index) {
            diagnostic.input = None;
        }
        for label in &mut diagnostic.labels {
            if label.input == Some(index) {
                label.input = None;
            }
        }
    }
    diagnostics
}

/// The standard library as a session ran it once, as it started: what each
/// of its inputs is scanned and checked beside ([`check_input`]).
#[derive(Clone, Debug)]
pub(crate) struct Standard {
    /// The number of its cells, the built-in functions' and the standard
    /// files' nodes, which are the first nodes of every scan, an input's
    /// after them. (A standard file that is a value file has no cell of its
    /// value here; none is.)
    cells: u32,
    /// The type of each of its cells.
    cell_types: Vec<TyId>,
    /// The slot each of its cells is read from: its own, or for a `def` its
    /// function's.
    cell_slots: Vec<Slot>,
    /// For each of its cells that is a `def`'s, which parameters its body
    /// measures.
    cell_measured: Vec<Option<Box<[bool]>>>,
}

/// What a session continues from once it has run the standard library
/// ([`start`]): the standard library's cells, and the names, types, frame,
/// memo tables and names of slots of the run. (The standard library has
/// no warnings to keep.)
#[derive(Debug)]
pub(crate) struct Started {
    /// The standard library's cells.
    pub(crate) standard: Standard,
    /// The names interned.
    pub(crate) names: Names,
    /// The types interned.
    pub(crate) types: TypeTable,
    /// The top level's frame, the standard library's cells and slots
    /// written.
    pub(crate) frame: Vec<Value>,
    /// The memo tables of the run.
    pub(crate) memo: MemoTables,
    /// The layout of the record `exec` returns, built once for the run.
    pub(crate) exec_result: Rc<Layout>,
    /// The name of each slot of the frame.
    pub(crate) slot_names: Vec<Name>,
}

/// Checks and runs the standard library as a session starts: through the
/// driver of a workspace with no file of its own, the built-in functions'
/// cells written as the frame is laid out and every standard file run.
///
/// # Errors
///
/// The diagnostics of the standard library, which are a bug.
pub(crate) fn start() -> Result<Started, Vec<Diagnostic>> {
    let mut workspace = check_with(&[], &[])?;
    for file in 0..workspace.scan.standard {
        workspace.run(file)?;
    }
    let Workspace {
        scan,
        frame,
        memo,
        exec_result,
        types,
        slot_names,
        cell_types,
        cell_slots,
        cell_measured,
        ..
    } = workspace;
    Ok(Started {
        standard: Standard {
            cells: count(scan.cells()),
            cell_types,
            cell_slots,
            cell_measured,
        },
        names: scan.names,
        types,
        frame,
        memo,
        exec_result,
        slot_names,
    })
}

/// Checks and lowers `source`, session input number `input`, beside the
/// standard library `standard`, from `scope`, the session's: scanned as a
/// lone file is ([`check_lone`]), its names interned into `names` and its
/// types into `types`, so that it may import `builtins` and the standard
/// files and has the prelude's names as the yielding last source of its
/// view; its cells, after the standard library's, take the slots after the
/// scope's ([`Scan::slot`]), and its check is a session input's, which
/// reads the scope's variables as a session's input does (see
/// [`crate::session`]). It inherits `imports`, those of the session's
/// earlier inputs, as the imports of its own that follow its `import`s
/// ([`inherit_imports`]). Its diagnostics name it as the input being run,
/// with no input number; a label in a standard file names that file's.
/// Returns, with its IR and its check's output, the imports it has, its own
/// and those it inherited and did not shadow, each with no item or span of
/// this input's: what a later input inherits, once it has run.
///
/// # Errors
///
/// The diagnostics of the scan, the check or the lowering.
pub(crate) fn check_input(
    standard: &Standard,
    (source, imports): (&SourceFile, &[Alias]),
    (names, types): (&mut Names, &mut TypeTable),
    scope: Scope,
    input: u32,
) -> Result<(Ir, CheckOutput, Vec<Alias>), Vec<Diagnostic>> {
    let (lowered, imports) = check_input_then(
        standard,
        (source, imports),
        (names, types),
        scope,
        |ast, checked, types| {
            checked.and_then(|output| session::lower_checked(ast, output, types, input))
        },
    )?;
    let (ir, output) = lowered?;
    Ok((ir, output, imports))
}

/// [`check_input`] up to its check, whose outcome `then` is given with the
/// input's tree, its names in it, and the types: what lowering is done by,
/// or a test looks at. Returns what `then` does, with the input's imports
/// as [`check_input`] does.
///
/// # Errors
///
/// The diagnostics of the scan.
fn check_input_then<R>(
    standard: &Standard,
    (source, imports): (&SourceFile, &[Alias]),
    (names, types): (&mut Names, &mut TypeTable),
    scope: Scope,
    then: impl FnOnce(&Ast, Result<CheckOutput, Vec<Diagnostic>>, &mut TypeTable) -> R,
) -> Result<(R, Vec<Alias>), Vec<Diagnostic>> {
    let mut scan = scan_into(std::slice::from_ref(source), Placing::Lone(imports), names)
        .map_err(|errors| being_run(errors, count(crate::STANDARD.len())))?;
    let index = scan.standard;
    // The input's cells, but its `def`s', take the slots past the scope's,
    // in order.
    let cells = scan.cells();
    let cell_names = scan.cell_names();
    let mut next = count(scope.slot_count());
    let mut taken = Vec::new();
    let relocated = (standard.cells..count(cells))
        .map(|cell| {
            if scan.is_def(cell) {
                return Slot::NONE;
            }
            if let Some(&name) = cell_names.get(cell as usize) {
                taken.push(name);
            }
            next = next.saturating_add(1);
            Slot::at(next - 1)
        })
        .collect();
    scan.relocation = Some((standard.cells, relocated));
    let mut cell_types = standard.cell_types.clone();
    cell_types.resize(cells, TyId::ERROR);
    let mut cell_slots = standard.cell_slots.clone();
    cell_slots.extend((count(cell_slots.len())..count(cells)).map(|cell| scan.slot(cell)));
    let mut cell_measured = standard.cell_measured.clone();
    cell_measured.resize(cells, None);
    let scope = scope.with_slots(taken);
    let file_names = mem::take(&mut scan.names);
    let Some(file) = scan.files.get_mut(index as usize) else {
        *names = file_names;
        return Err(Vec::new());
    };
    file.ast.replace_names(file_names);
    let finished = scan.files.get(index as usize).and_then(|file| {
        let reads = (&cell_slots[..], &cell_measured[..]);
        let context = Context::new(&scan, index, &cell_types, reads)?;
        let context = Context {
            session: true,
            ..context
        };
        let checked = Checker::in_workspace(&file.ast, scope, types, &context).check();
        Some(then(&file.ast, checked, types))
    });
    let mut imports = Vec::new();
    if let Some(file) = scan.files.get_mut(index as usize) {
        *names = file.ast.replace_names(Names::new());
        imports.extend(file.aliases.iter().filter(|a| !a.implicit).map(|a| Alias {
            span: Span::new(0, 0),
            item: None,
            inherited: true,
            ..*a
        }));
    }
    finished.map(|r| (r, imports)).ok_or_else(Vec::new)
}

/// For the tests of the other modules, which check a text alone: `text`
/// checked as a session's first input is, beside the standard library, so
/// that the prelude's names, `len` among them, are its, with the tree it
/// was parsed to, which holds every name, and the types. Its top-level
/// slots come after the standard library's.
#[cfg(test)]
pub(crate) fn check_beside_prelude(
    text: &str,
) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    let started = start().unwrap();
    let (mut names, mut types) = (started.names, started.types);
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let scope = Scope::of_slots(started.slot_names);
    let checked = check_input_then(
        &started.standard,
        (&source, &[]),
        (&mut names, &mut types),
        scope,
        |ast, checked, _| (ast.clone(), checked),
    );
    match checked {
        Ok(((ast, checked), _)) => (ast, types, checked),
        Err(errors) => panic!("{text}: {errors:?}"),
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
            [
                "<stdlib>/prelude.gcsl",
                "app.gcsl",
                "common.gcsl",
                "lib.gcsl",
                "workspace.gcsl"
            ]
        );
        assert_eq!(scan.needs, [vec![], vec![2, 3], vec![], vec![2], vec![]]);
        assert_eq!(scan.order, [0, 2, 3, 1, 4]);
        assert_eq!(scan.needed(1), [0, 2, 3, 1]);
        assert_eq!(scan.needed(3), [0, 2, 3]);
        assert_eq!(scan.needed(4), [0, 4]);
    }

    #[test]
    fn a_reference_means_its_prefix_in_the_merged_tree() {
        let scan = ws(&SUMMARY).unwrap();
        let app = &scan.files[1];
        // `com.x.lib` in app's second item is lib's interior node, not
        // app's own `com.x`.
        let foreign = &app.mentions.foreign[1];
        assert_eq!(foreign.len(), 1);
        let Need::Field(node) = foreign[0].0 else {
            panic!("{foreign:?}")
        };
        let merged = FileScope::new(&scan, app).merged(node).unwrap();
        assert_eq!(scan.show(merged), "com.x.lib");
        assert_eq!(scan.nodes[merged as usize].contributors, [3]);
    }

    #[test]
    fn dotted_paths_merge_and_the_last_contributor_assembles() {
        let scan = ws(&[("a.gcsl", "com.a = 1"), ("b.gcsl", "all = com\ncom.b = 2")]).unwrap();
        assert_eq!(scan.order, [0, 1, 2]);
        let com = scan.by_name[&(0, ROOT, scan.files[1].mentions.tree.nodes[0].name)];
        assert_eq!(scan.nodes[com as usize].contributors, [1, 2]);
        assert_eq!(scan.computed_by(com), Some(2));
        // a leaves `com` out; b computes it before its first mention.
        assert!(!scan.files[1].schedule.order.contains(&0));
        let b = &scan.files[2];
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
        assert_eq!(scan.needs, [vec![], vec![], vec![], vec![1, 2]]);
        // b, the last contributor, assembles `com` after its last child.
        let b = &scan.files[2];
        assert_eq!(b.schedule.point[0], 1);
        assert!(!scan.files[1].schedule.order.contains(&0));
    }

    #[test]
    fn a_def_of_one_file_called_from_another_is_a_need() {
        let scan = ws(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "x = f()")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![1]]);
    }

    #[test]
    fn siblings_and_subdirectories_are_hidden() {
        let scan = ws(&[("x/a.gcsl", "v = 1"), ("y/b.gcsl", "w = v")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![]]);
        let scan = ws(&[("a.gcsl", "w = v"), ("sub/b.gcsl", "v = 1")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![]]);
        let scan = ws(&[("a.gcsl", "v = 1"), ("sub/b.gcsl", "w = v")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![1]]);
    }

    #[test]
    fn conflicts_between_files() {
        assert_eq!(
            error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x.y = 2")]),
            (
                "field `x.y` is defined in two files".to_owned(),
                Some(2),
                vec![Some(1)]
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
        // A root of a directory above merges; the same leaf in both is a
        // conflict, reported at the directory above.
        assert_eq!(
            error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
            (
                "field `version` is defined in two files".to_owned(),
                Some(1),
                vec![Some(2)]
            )
        );
    }

    #[test]
    fn files_that_need_each_other() {
        let (message, input, labels) = error(&[("a.gcsl", "p = q"), ("b.gcsl", "q = p")]);
        assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
        assert_eq!((input, labels), (Some(1), vec![Some(2)]));
        let (message, _, labels) = error(&[
            ("a.gcsl", "p = q"),
            ("b.gcsl", "q = r"),
            ("c.gcsl", "r = p"),
        ]);
        assert_eq!(message, "`a.gcsl`, `b.gcsl` and `c.gcsl` need each other");
        assert_eq!(labels, [Some(2), Some(3)]);
    }

    #[test]
    fn path_order_breaks_ties_and_errors_name_their_file() {
        let scan = ws(&[("b.gcsl", "x = 1"), ("a.gcsl", "y = 2")]).unwrap();
        assert_eq!(scan.order, [0, 1, 2]);
        assert_eq!(scan.files[1].path, "a.gcsl");
        let errors = ws(&[("a.gcsl", "x = 1"), ("b.gcsl", "y = (")]).unwrap_err();
        assert_eq!(errors[0].input, Some(2));
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
        assert_assemblies_stored(&workspace);
        let index = workspace.file(path).unwrap();
        let outcome = workspace.run(index).map_err(shown)?;
        Ok(outcome.value.to_string())
    }

    /// Asserts that the driver wrote the cell of every built-in function
    /// as it laid the frame out, before any file runs: a `Value::Builtin`,
    /// never `Value::Empty`, which would read as a record-kinded `{}` that
    /// nothing else can see. Returns their slots, which no file stores.
    fn assert_builtins_written(workspace: &Workspace) -> Vec<Slot> {
        Builtin::ALL
            .into_iter()
            .zip(0u32..)
            .map(|(builtin, node)| {
                let slot = workspace.cell_slots[node as usize];
                assert_eq!(workspace.scan.builtin(node), Some(builtin));
                assert_eq!(
                    workspace.frame.get(slot.index()),
                    Some(&Value::Builtin(builtin)),
                    "the cell of `{}` is not written by the driver",
                    builtin.name()
                );
                slot
            })
            .collect()
    }

    /// Asserts that every cell an import of `workspace` reaches, directly
    /// or as a field of a record read whole, is stored by a node of its
    /// owner's top-level block: slice 24's schedule test sees one file
    /// only, and a cell no owner writes would read as `{}`. A built-in
    /// function's cell has no owner: the driver wrote it before any file
    /// ran ([`assert_builtins_written`]).
    fn assert_imports_stored(workspace: &Workspace) {
        let scan = &workspace.scan;
        let types = vec![TyId::ERROR; scan.cells()];
        let builtins = assert_builtins_written(workspace);
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
                if builtins.contains(&slot) {
                    continue;
                }
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

    /// The slots the top-level block of file `file` of `workspace` stores,
    /// in order.
    fn top_stores(workspace: &Workspace, file: u32) -> Vec<Slot> {
        let Some(Some(ir)) = workspace.irs.get(file as usize) else {
            return Vec::new();
        };
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            return Vec::new();
        };
        ir.items(items)
            .iter()
            .filter_map(|&item| match ir.get(item).kind {
                IrKind::Store(slot, _) => Some(slot),
                _ => None,
            })
            .collect()
    }

    /// Asserts that every record a file of `workspace` assembles of a node
    /// of its view with a leaf of the file under it reads only cells stored
    /// before it: the file's own by an earlier node of its top-level block,
    /// every other one by the top-level block of a file it needs, which runs
    /// whole before it. And that every record any file assembles at its top
    /// level (an interior node's, a shared node's, an imported one) reads
    /// only cells stored earlier in its block or by a file earlier in the
    /// order: slice 25's shared nodes read the cells of the files that
    /// contribute to them, which come earlier but need not be needed. A cell
    /// nobody stored first would read as `{}`. A built-in function's cell
    /// counts as stored before any file: the driver wrote it as it laid the
    /// frame out ([`assert_builtins_written`]).
    fn assert_assemblies_stored(workspace: &Workspace) {
        let scan = &workspace.scan;
        let builtins = assert_builtins_written(workspace);
        for (index, file) in (0u32..).zip(&scan.files) {
            let own = top_stores(workspace, index);
            let needed: Vec<Slot> = scan
                .needed(index)
                .into_iter()
                .filter(|&other| other != index)
                .flat_map(|other| top_stores(workspace, other))
                .chain(builtins.iter().copied())
                .collect();
            let rank = scan.order.iter().position(|&f| f == index).unwrap_or(0);
            let earlier: Vec<Slot> = scan.order[..rank]
                .iter()
                .flat_map(|&other| top_stores(workspace, other))
                .chain(builtins.iter().copied())
                .collect();
            let records = workspace.view_records.get(index as usize);
            for (slot, _, fields) in records.into_iter().flatten() {
                let Some(at) = own.iter().position(|s| s == slot) else {
                    let path = &file.path;
                    assert_eq!(*slot, Slot::NONE, "{path}: a view record is not stored");
                    continue;
                };
                for &(name, cell, _) in fields {
                    assert!(
                        own[..at].contains(&cell) || needed.contains(&cell),
                        "{}: the view record in {slot:?} reads `{}` from {cell:?}, stored by \
                         nothing before it",
                        file.path,
                        scan.names.get(name)
                    );
                }
            }
            let Some(Some(ir)) = workspace.irs.get(index as usize) else {
                continue;
            };
            let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                continue;
            };
            let mut stored: Vec<Slot> = Vec::new();
            for &item in ir.items(items) {
                let IrKind::Store(slot, value) = ir.get(item).kind else {
                    continue;
                };
                if let IrKind::MakeRecord { values, .. } = ir.get(value).kind {
                    for &field in ir.items(values) {
                        if let IrKind::Load(cell, _) = ir.get(field).kind {
                            assert!(
                                stored.contains(&cell) || earlier.contains(&cell),
                                "{}: a record stored in {slot:?} reads {cell:?}, stored by \
                                 nothing before it",
                                file.path
                            );
                        }
                    }
                }
                stored.push(slot);
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
        // `a.gcsl` is input 1, after the prelude: the error names it, and
        // `b.gcsl` never ran.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, Some(1));
        let erroneous = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x = 1"),
            ("z.gcsl", "y = 1 + \"\""),
        ];
        let errors = run_file(&erroneous, "a.gcsl").unwrap_err();
        // The error is `z.gcsl`'s, input 3.
        assert!(errors.iter().all(|error| error.1 == Some(3)), "{errors:?}");
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
        assert_eq!(inputs, [Some(1), Some(3)], "{errors:?}");
        // A closure of another file names that file when it fails.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "def boom(): Int { [1][5] }"),
            ("b.gcsl", "y = boom()"),
        ];
        let errors = run_file(&files, "b.gcsl").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, Some(1), "{errors:?}");
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
            let last = workspace.file(&format!("f{:04}.gcsl", count - 1)).unwrap();
            let outcome = workspace.run(last).unwrap();
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

    /// The time to check and run a lone file of `count` fields, each reading
    /// the one before, beside the standard library ([`check_lone`]), against
    /// checking, lowering and running it alone as [`crate::check::check`]
    /// does, with no standard library: what the prelude and the path of a
    /// workspace cost a lone file. Run with `cargo test --release --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_lone_file_beside_the_prelude() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [1_u32, 1000, 2000, 4000, 8000] {
            let mut text = String::from("f0 = 1\n");
            for field in 1..count {
                writeln!(text, "f{field} = f{} + 1", field - 1).unwrap();
            }
            let source = SourceFile::new("lone.gcsl", text).unwrap();
            let start = std::time::Instant::now();
            let (tokens, names, strings) = lexer::tokenize(&source).unwrap();
            let ast = parser::parse(&tokens, names, strings, &source).unwrap();
            let mut types = TypeTable::new();
            let (checked, _) = crate::check::check(&ast, &mut types).unwrap();
            let ir = std::rc::Rc::new(crate::lower::lower(&ast, &checked, &types, 0).unwrap());
            let alone = crate::eval::eval(&ir).unwrap();
            let bare = start.elapsed();
            // Its scan and its check, apart.
            let start = std::time::Instant::now();
            let scanned = scan_into(
                std::slice::from_ref(&source),
                Placing::Lone(&[]),
                &mut Names::new(),
            )
            .unwrap();
            let scanning = start.elapsed();
            let start = std::time::Instant::now();
            check_scan(scanned).unwrap();
            let checking = start.elapsed();
            let start = std::time::Instant::now();
            let lone = crate::run(&source).unwrap().value;
            let beside = start.elapsed();
            assert_eq!(alone.to_string(), lone.to_string());
            writeln!(
                report,
                "{count} fields: alone {bare:?}; beside the standard library {beside:?} \
                 (scan {scanning:?}, check {checking:?})"
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
            let last = workspace
                .file(&format!("d{:04}/f.gcsl", count - 1))
                .unwrap();
            let outcome = workspace.run(last).unwrap();
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
        let main = &scan.files[1];
        // Its own imports; the implicit prelude's names follow them.
        let named: Vec<(&str, Target)> = main
            .aliases
            .iter()
            .filter(|a| !a.implicit)
            .map(|a| (scan.names.get(a.name), a.target))
            .collect();
        let lib = scan.dirs.iter().position(|d| d.path == "lib").unwrap();
        let com = scan.by_name[&(count(lib), ROOT, scan.names.lookup("com").unwrap())];
        assert_eq!(
            named,
            [
                ("lib", Target::Dir(count(lib))),
                ("builder", Target::File(5)),
                ("libs", Target::Node(com)),
                ("y", Target::File(4)),
            ]
        );
        // Needs per leaf: x, y and build, each through an import; `common`
        // only through them.
        assert_eq!(scan.needs[1], [3, 4, 5]);
        assert_eq!(scan.needed(1), [0, 2, 3, 4, 5, 1]);
        assert_eq!(scan.value_slot(5), Some(Slot::at(count(scan.nodes.len()))));
        assert_eq!(scan.value_slot(2), None);
    }

    #[test]
    fn a_reference_through_an_import_needs_only_its_leaf() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport lib.y\nimport tools\na = lib.com.x.lib.name\nb = y";
        let scan = ws(&files).unwrap();
        // `lib.com.x.lib.name` is x's leaf; `y` whole is y; nothing reads
        // `tools`, so it needs nothing.
        assert_eq!(scan.needs[1], [3, 4]);
        files[0].1 = "import lib\na = lib";
        assert_eq!(ws(&files).unwrap().needs[1], [3, 4]);
        files[0].1 = "import lib\na = lib.com.y";
        assert_eq!(ws(&files).unwrap().needs[1], [4]);
    }

    #[test]
    fn a_file_part_and_a_whole_node_are_told_apart() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib.y\na = y.com\nb = y.com.y";
        let scan = ws(&files).unwrap();
        let main = &scan.files[1];
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
            matches!(targets[0], Target::Part { file: 4, .. }),
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
            ("import tools as version", "is a field of this directory"),
            ("import x from lib.x", "has no `x`"),
        ] {
            files[0].1 = text;
            assert!(noted(&files, &[]).0.ends_with(message), "{text}");
        }
        // A member or a `*` root merges with the file's own root of its
        // name.
        for text in [
            "import com from lib\ncom.a = 1",
            "import * from lib\ncom.a = 1",
        ] {
            files[0].1 = text;
            assert!(ws(&files).is_ok(), "{text}");
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

    /// The files of the merging summary (slice 27), in path order: `pkgs/`
    /// is the root.
    const MERGE: [(&str, &str); 5] = [
        (
            "app/main.gcsl",
            "import * from lib\ncom.x.app.deps = [com.x.lib, com.y.lib]\n\
             com.x.app.base = com.base.name",
        ),
        ("app/vendor/v.gcsl", "com.vendor.thing = 1\nall = com"),
        ("lib/x.gcsl", "com.x.lib.name = \"lib\""),
        ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
        ("workspace.gcsl", "com.base.name = \"base\""),
    ];

    /// The single error of scanning `files`, shown: its message and file,
    /// its label, each other label's file and text, and its notes.
    fn shown_error(files: &[(&str, &str)]) -> String {
        let errors = ws(files).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
        paths.sort_unstable();
        // The standard files take the first input numbers.
        let standard = standard_sources();
        let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
        all.extend(paths);
        let path = |input: Option<u32>| {
            input
                .and_then(|i| all.get(i as usize))
                .copied()
                .unwrap_or("?")
        };
        let error = &errors[0];
        let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
        if let Some(label) = &error.label {
            parts[0] = format!("{} ({label})", parts[0]);
        }
        for label in &error.labels {
            let file = path(label.input.or(error.input));
            parts.push(format!("{file}: {}", label.text));
        }
        for note in &error.notes {
            parts.push(format!("note: {note}"));
        }
        parts.join("; ")
    }

    /// The checked workspace of `files`.
    fn checked(files: &[(&str, &str)]) -> Workspace {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        check(&sources).unwrap()
    }

    /// Whether the first item of the top-level block of the file at `path`
    /// of `workspace` stores a record: one assembled as the file starts.
    fn assembled_first(workspace: &Workspace, path: &str) -> bool {
        let index = workspace.file(path).unwrap();
        let ir = workspace.irs[index as usize].as_ref().unwrap();
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("expected a block")
        };
        let first = ir.items(items)[0];
        matches!(ir.get(first).kind, IrKind::Store(_, value)
            if matches!(ir.get(value).kind, IrKind::MakeRecord { .. }))
    }

    #[test]
    fn the_merging_summary_runs_both_files() {
        assert_eq!(
            value(&MERGE, "app/main.gcsl"),
            "{com = {x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; \
             base = \"base\"}}}}"
        );
        // `all` has the root's part, `app/`'s and v's own, root directory
        // first, but not `lib/`'s, which v does not import.
        assert_eq!(
            value(&MERGE, "app/vendor/v.gcsl"),
            "{com = {vendor = {thing = 1}}; all = {base = {name = \"base\"}; \
             x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; base = \"base\"}}; \
             vendor = {thing = 1}}}"
        );
        // v owns a leaf of `com`: its view's `com` is a node of its own
        // schedule, after its own child and before `all`.
        let scan = ws(&MERGE).unwrap();
        let v = &scan.files[2];
        assert_eq!((v.view.own, v.view.end - v.view.start), (1, 1));
        let number = count(v.global.len());
        assert!(v.schedule.order.contains(&number));
        assert_eq!(v.schedule.point[number as usize], 1);
        // main reads no node of its view whole: nothing is assembled.
        let main = &scan.files[1];
        let own = count(main.global.len());
        assert!(main.schedule.order.iter().all(|&n| n < own));
        // Its `com` is the root's, `app/`'s and `lib/`'s, root first.
        let com = &scan.views[main.view.start as usize];
        let sources: Vec<u32> = com.parts.iter().map(|p| p.source).collect();
        assert_eq!(sources, [0, 1, 2]);
        // v's type of `all` is the record of the merged children.
        let mut workspace = checked(&MERGE);
        let dump = workspace.dump_types(2).unwrap();
        assert!(
            dump.contains(
                "(name com : {base: {name: String}, vendor: {thing: Int}, x: {app: {base: \
                 String, deps: List<{name: String}>}}}"
            ),
            "{dump}"
        );
    }

    /// [`POSITIONS`] through a merged node: `com` is the file's own and
    /// `lib/`'s, imported with `*`, and `com.x.lib.r` is `lib/`'s leaf, read
    /// from every kind of position; `com` is read whole too (a node of the
    /// file's schedule, the file owning `com.x.app.name`), and a function
    /// out of that record called. A `def` is a root, never a leaf under
    /// `com`: `com.x.lib.f` is a field holding one, and `twice` itself is
    /// called through the `*`.
    const MERGED_POSITIONS: &str = r#"import * from lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy com.x.lib.r
let lw = lazy com
let g = fn(): {a: Int} { com.x.lib.r }
let gw = fn(): Int { let w = com; = w.x.lib.f(com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = com
    = com.x.lib.r.a + w.x.lib.r.a
  }
}
let w = com
com.x.app.name = "app"
p01 = "{com.x.lib.r}"
p02 = xs[com.x.lib.r.a]
p03 = m
p04 = for v in [com.x.lib.r] { v.a }
p05 = for v in [1] if com.x.lib.r.a == 42 { v }
p06 = for let i = com.x.lib.r.a; i < com.x.lib.r.a + 1; i := i + com.x.lib.r.a { i }
p07 = for k < com.x.lib.r.a { k := k + 50; com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < com.x.lib.r.a)
p09 = true and com.x.lib.r.a == 42
p10 = com.x.lib.r is {a: Int}
p11 = (com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue com.x.lib.r }; break com.x.lib.r }
p13 = get(config = com.x.lib.r)
p14 = wrap(com.x.lib.r) { ...com.x.lib.r }
p15 = { ...com.x.lib.r }
p16 = len(0..com.x.lib.r.a)
p17 = [com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).x.lib.r.a
p20 = g()
p21 = { let q = com.x.lib.r; = q }
p22 = w.x.lib.f(w.x.lib.r.a)
p23 = gw()
p24 = twice(21)
p25 = com
p26 = com.x"#;

    #[test]
    fn a_merged_node_is_read_from_every_position() {
        let files = [
            ("app/main.gcsl", MERGED_POSITIONS),
            (
                "lib/x.gcsl",
                "let base = 40\ncom.x.lib.r = { a = base + 2 }\ncom.x.lib.f = twice\n\
                 def twice(n: Int): Int { n * 2 }",
            ),
            ("workspace.gcsl", ""),
        ];
        let lib = "lib = {r = {a = 42}; f = fn(n: Int): Int}";
        assert_eq!(
            value(&files, "app/main.gcsl"),
            format!(
                "{{outer = fn(): {{inner: fn(): Int}}; com = {{x = {{app = {{name = \"app\"}}}}}}; \
                 p01 = \"{{{{a = 42}}}}\"; p02 = 42; p03 = {{a = 42}}; p04 = [42]; p05 = [1]; \
                 p06 = [42]; p07 = [{{a = 42}}]; p08 = 42; p09 = true; p10 = true; p11 = 42; \
                 p12 = [{{a = 42}}, {{a = 42}}]; p13 = 42; p14 = 84; p15 = {{a = 42}}; \
                 p16 = 42; p17 = [{{a = 42}}]; p18 = 84; p19 = 84; p20 = {{a = 42}}; \
                 p21 = {{a = 42}}; p22 = 84; p23 = 84; p24 = 42; \
                 p25 = {{x = {{app = {{name = \"app\"}}; {lib}}}}}; \
                 p26 = {{app = {{name = \"app\"}}; {lib}}}}}"
            )
        );
    }

    #[test]
    fn star_and_member_imports_merge_with_the_files_own_root() {
        let lib = [
            ("lib/x.gcsl", "com.x.lib.name = \"x\""),
            ("lib/y.gcsl", "com.y.lib.name = \"y\""),
            ("workspace.gcsl", ""),
        ];
        // A directory's tree, a file's own tree, and a member, each with an
        // own `com`: the import side's children after the file's own.
        for (main, all) in [
            (
                "import * from lib\ncom.app = 1\nall = com",
                "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
            ),
            (
                "import * from lib.y\ncom.app = 1\nall = com",
                "{app = 1; y = {lib = {name = \"y\"}}}",
            ),
            (
                "import com from lib\ncom.app = 1\nall = com",
                "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
            ),
        ] {
            let mut files = lib.to_vec();
            files.push(("app/main.gcsl", main));
            assert_eq!(
                value(&files, "app/main.gcsl"),
                format!("{{com = {{app = 1}}; all = {all}}}"),
                "{main}"
            );
        }
        // A member that is a leaf, and one that is a `def`.
        let mut files = lib.to_vec();
        files.push(("lib/z.gcsl", "n = 1\ndef twice(k: Int): Int { k * 2 }"));
        files.push((
            "app/main.gcsl",
            "import n, twice from lib\nm = twice(n + 1)",
        ));
        assert_eq!(value(&files, "app/main.gcsl"), "{m = 4}");
    }

    #[test]
    fn a_subdirectory_merges_under_a_root_above_two_levels_deep() {
        let files = [
            ("workspace.gcsl", "com.a = 1"),
            ("x/b.gcsl", "com.b = 2"),
            ("x/y/c.gcsl", "com.c = 3\nall = com\nb = com.b"),
        ];
        assert_eq!(
            value(&files, "x/y/c.gcsl"),
            "{com = {c = 3}; all = {a = 1; b = 2; c = 3}; b = 2}"
        );
        assert_eq!(value(&files, "x/b.gcsl"), "{com = {b = 2}}");
    }

    /// A whole read of each kind of node of the view: who assembles it, and
    /// when, with its contents and their order.
    #[test]
    fn a_view_node_read_whole_of_each_kind() {
        // One directory's tree, without an import: slice 25's rule, the
        // last contributor's schedule; the view has no node of its own.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "com.one = 1"),
            ("b.gcsl", "com.two = 2\nall = com"),
        ];
        let scan = ws(&files).unwrap();
        assert!(scan.views.is_empty());
        assert_eq!(
            value(&files, "b.gcsl"),
            "{com = {two = 2}; all = {one = 1; two = 2}}"
        );
        // Across directories, with no leaf of the reading file: assembled
        // as the file starts, the root directory's part first.
        let files = [
            ("workspace.gcsl", "com.a = 1"),
            ("sub/b.gcsl", "com.b = 2"),
            ("sub/c.gcsl", "x = 0\nall = com"),
        ];
        assert_eq!(value(&files, "sub/c.gcsl"), "{x = 0; all = {a = 1; b = 2}}");
        assert!(assembled_first(&checked(&files), "sub/c.gcsl"));
        // Across directories, with a leaf of the reading file: a node of
        // its schedule, after its own leaf, so not first.
        let files = [
            ("workspace.gcsl", "com.a = 1"),
            ("sub/b.gcsl", "com.b = 2\nall = com"),
        ];
        assert_eq!(
            value(&files, "sub/b.gcsl"),
            "{com = {b = 2}; all = {a = 1; b = 2}}"
        );
        assert!(!assembled_first(&checked(&files), "sub/b.gcsl"));
        // Across imports only, with no leaf of the reading file: as the
        // file starts, in the imports' source order.
        let files = [
            (
                "app/main.gcsl",
                "import * from tools\nimport * from lib\nall = com",
            ),
            ("lib/x.gcsl", "com.x = 1"),
            ("tools/t.gcsl", "com.t = 2"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "app/main.gcsl"), "{all = {t = 2; x = 1}}");
        assert!(assembled_first(&checked(&files), "app/main.gcsl"));
        // Mixed: a directory above, the file's own directory, an import and
        // a leaf of the file, two levels of the view merged; `com.x` is
        // read whole too, and a part of it alone.
        let files = [
            (
                "app/main.gcsl",
                "import * from lib\ncom.x.app = 1\nall = com\nx = com.x",
            ),
            ("app/other.gcsl", "com.other = 2"),
            ("lib/x.gcsl", "com.x.lib = 3"),
            ("workspace.gcsl", "com.x.root = 4\ncom.base = 5"),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{com = {x = {app = 1}}; all = {x = {root = 4; app = 1; lib = 3}; base = 5; \
             other = 2}; x = {root = 4; app = 1; lib = 3}}"
        );
        let scan = ws(&files).unwrap();
        let main = &scan.files[1];
        // `com` and `com.x`, both with a leaf of main under them.
        assert_eq!((main.view.own, main.view.end - main.view.start), (2, 2));
    }

    #[test]
    fn the_needs_of_a_view_node_are_per_leaf() {
        let base = [
            ("workspace.gcsl", ""),
            ("bad.gcsl", "def bad(): Int { [1][5] }\ncom.bad.v = bad()"),
            ("ok.gcsl", "com.ok.v = 1"),
        ];
        // A leaf read alone needs its own file only: `bad.gcsl` never runs.
        let mut files = base.to_vec();
        files.push(("app/main.gcsl", "com.app.v = 2\na = com.ok.v"));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{com = {app = {v = 2}}; a = 1}"
        );
        // Read whole, every owner of every leaf runs, `bad.gcsl` too.
        files.pop();
        files.push(("app/main.gcsl", "com.app.v = 2\na = com"));
        assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
        files.pop();
        files.push(("app/main.gcsl", "a = com"));
        assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
    }

    #[test]
    fn conflicts_across_directories() {
        let above = "one is in a directory above";
        // Two leaves, at the directory above, whichever path is first.
        assert_eq!(
            shown_error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
            format!(
                "field `version` is defined in two files at a.gcsl (defined here); \
                 sub/b.gcsl: defined here; note: {above}"
            )
        );
        assert_eq!(
            shown_error(&[("z.gcsl", "version = 1"), ("a/b.gcsl", "version = 2")]),
            format!(
                "field `version` is defined in two files at z.gcsl (defined here); \
                 a/b.gcsl: defined here; note: {above}"
            )
        );
        // A whole value extended by a path below, and the other way.
        let defined = format!(
            "field `com` is already defined at a.gcsl; sub/b.gcsl: defined here; note: {above}"
        );
        assert_eq!(
            shown_error(&[("a.gcsl", "com = 1"), ("sub/b.gcsl", "com.x = 2")]),
            defined
        );
        assert_eq!(
            shown_error(&[("a.gcsl", "com.x = 1"), ("sub/b.gcsl", "com = 2")]),
            defined
        );
        // A `def` is a leaf: with a field, either way, and with a `def`.
        for (a, b) in [
            ("def f(): Int { 1 }", "f = 2"),
            ("f = 2", "def f(): Int { 1 }"),
            ("def f(): Int { 1 }", "def f(): Int { 2 }"),
        ] {
            assert_eq!(
                shown_error(&[("a.gcsl", a), ("sub/b.gcsl", b)]),
                format!(
                    "field `f` is defined in two files at a.gcsl (defined here); \
                     sub/b.gcsl: defined here; note: {above}"
                ),
                "{a} / {b}"
            );
        }
        // Once per pair, however many files' views have it.
        assert_eq!(
            ws(&[
                ("a.gcsl", "version = 1"),
                ("sub/b.gcsl", "version = 2"),
                ("sub/c.gcsl", "x = 1"),
                ("sub/d/e.gcsl", "y = 1"),
            ])
            .unwrap_err()
            .len(),
            1
        );
    }

    #[test]
    fn conflicts_through_imports() {
        let lib = [
            ("lib/x.gcsl", "com.x.lib.name = \"x\""),
            ("workspace.gcsl", ""),
        ];
        let with = |main: &'static str, more: &[(&'static str, &'static str)]| {
            let mut files = lib.to_vec();
            files.push(("app/main.gcsl", main));
            files.extend_from_slice(more);
            shown_error(&files)
        };
        // Two leaves, an own leaf and an imported interior node, and an own
        // interior node and an imported leaf: at the import.
        assert_eq!(
            with("import * from lib\ncom.x.lib.name = \"own\"", &[]),
            "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
        );
        assert_eq!(
            with("import * from lib\ncom.x.lib = 1", &[]),
            "field `com.x.lib` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
        );
        assert_eq!(
            with("import * from lib\ncom.x.lib.name.deep = 1", &[]),
            "field `com.x.lib.name` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
        );
        // A member whose bound name is a plain root of the file, a leaf and
        // an interior node of it.
        let version = [("lib/v.gcsl", "version = 1")];
        assert_eq!(
            with("import version from lib\nversion = 2", &version),
            "field `version` is defined in two files at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
        );
        assert_eq!(
            with("import version from lib\nversion.minor = 2", &version),
            "field `version` is already defined at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
        );
        // Against a directory above the file (not above `lib/`), and
        // between two imports, at the later.
        let mut files = lib.to_vec();
        files.push(("app/sub/main.gcsl", "import * from lib"));
        files.push(("app/up.gcsl", "com.x.lib.name = \"up\""));
        assert_eq!(
            shown_error(&files),
            "field `com.x.lib.name` is defined in two files at app/sub/main.gcsl (imported \
             here); lib/x.gcsl: defined here; app/up.gcsl: defined here; note: one is in a \
             directory above"
        );
        assert_eq!(
            with(
                "import * from lib\nimport * from tools",
                &[("tools/t.gcsl", "com.x.lib.name = \"t\"")]
            ),
            "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             tools/t.gcsl: defined here; app/main.gcsl: imported here; lib/x.gcsl: defined here"
        );
    }

    #[test]
    fn a_view_is_the_files_own() {
        // b does not see what a imported with `*`, nor its own directory's
        // tree `lib/`'s part.
        let mut files = vec![
            ("app/a.gcsl", "import * from lib\nx = com.lib.v"),
            ("lib/l.gcsl", "com.lib.v = 1"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "app/a.gcsl"), "{x = 1}");
        files.push(("app/b.gcsl", "y = com.lib.v"));
        assert_eq!(messages(&files, "app/b.gcsl"), ["undefined variable `com`"]);
        files.pop();
        files.push(("app/b.gcsl", "com.app.v = 2\ny = com.lib.v"));
        assert_eq!(
            messages(&files, "app/b.gcsl"),
            ["no field `lib` in `{app: {v: Int}}`"]
        );
    }

    #[test]
    fn star_from_a_value_file_is_an_error() {
        let files = [
            ("app/main.gcsl", "import * from tools.build"),
            ("tools/build.gcsl", "= 1"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            shown_error(&files),
            "a value file has no members to import at app/main.gcsl"
        );
    }

    #[test]
    fn a_whole_import_still_clashes_with_a_root_of_the_view() {
        let files = |main: &'static str| {
            vec![
                ("app/main.gcsl", main),
                ("lib/x.gcsl", "com.x = 1"),
                ("tools/t.gcsl", "t = 1"),
                ("workspace.gcsl", ""),
            ]
        };
        for main in [
            "import * from lib\nimport tools as com",
            "import tools as com\nimport * from lib",
            "import tools as com\ncom.y = 1",
        ] {
            assert!(
                shown_error(&files(main)).starts_with("`com` is imported in this file"),
                "{main}"
            );
        }
    }

    #[test]
    fn merging_the_same_cell_twice_is_idempotent() {
        let lib = [("lib/x.gcsl", "com.x.lib.name = 1"), ("workspace.gcsl", "")];
        // `*` from one directory twice.
        let mut files = lib.to_vec();
        files.push((
            "app/main.gcsl",
            "import * from lib\nimport * from lib\na = com.x.lib.name\nall = com",
        ));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{a = 1; all = {x = {lib = {name = 1}}}}"
        );
        assert!(ws(&files).unwrap().views.is_empty());
        // `*` from the file's own directory (`app` inside `app/`), from a
        // file of it, and from a directory above, with its own leaves.
        for main in [
            "import * from app\ncom.main.v = 1\nall = com",
            "import * from app.other\ncom.main.v = 1\nall = com",
            "import * from app.main\ncom.main.v = 1\nall = com",
            "import com from app\ncom.main.v = 1\nall = com",
        ] {
            let mut files = lib.to_vec();
            files.push(("app/main.gcsl", main));
            files.push(("app/other.gcsl", "com.other.v = 2"));
            assert_eq!(
                value(&files, "app/main.gcsl"),
                "{com = {main = {v = 1}}; all = {main = {v = 1}; other = {v = 2}}}",
                "{main}"
            );
            assert!(ws(&files).unwrap().views.is_empty(), "{main}");
        }
        let mut files = lib.to_vec();
        files.push((
            "app/sub/main.gcsl",
            "import * from app\ncom.sub = 1\nall = com",
        ));
        files.push(("app/other.gcsl", "com.other = 2"));
        assert_eq!(
            value(&files, "app/sub/main.gcsl"),
            "{com = {sub = 1}; all = {other = 2; sub = 1}}"
        );
    }

    /// A member of the file's own directory bound by another name is not
    /// the same cell at the same path: reaching the file's own nodes through
    /// it is still the one-file cycle, as through a whole import.
    #[test]
    fn a_renamed_member_reaching_the_files_own_nodes_needs_itself() {
        let files = [
            (
                "app/main.gcsl",
                "import com as c from app\ncom.x = 1\ny = c.x",
            ),
            ("workspace.gcsl", ""),
        ];
        assert!(shown_error(&files).starts_with(
            "`app/main.gcsl` needs itself at app/main.gcsl (`com.x` is computed when this \
             file runs)"
        ));
    }

    #[test]
    fn an_importing_files_items_merge_in_any_order() {
        let lines = [
            "import * from lib",
            "com.x.app.a = com.x.lib.name + b",
            "b = 2",
            "all = com.x",
        ];
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
            let files = [
                ("app/main.gcsl", main.as_str()),
                ("lib/x.gcsl", "com.x.lib.name = 1"),
                ("workspace.gcsl", ""),
            ];
            let shown = value(&files, "app/main.gcsl");
            for field in [
                "com = {x = {app = {a = 3}}}",
                "b = 2",
                "all = {app = {a = 3}; lib = {name = 1}}",
            ] {
                assert!(shown.contains(field), "{main}: {shown}");
            }
        }
    }

    /// The time to check a workspace of `count` sibling directories of the
    /// root, each with one file of ten leaves under the root's `com`, each
    /// reading one leaf of the root's file through the merged `com`, and to
    /// run the last: linear in `count`. Run with `cargo test --release --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_sibling_directories_merging() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let mut sources = vec![
                SourceFile::new(
                    "workspace.gcsl",
                    (0..10).fold(String::new(), |mut text, leaf| {
                        writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                        text
                    }),
                )
                .unwrap(),
            ];
            for dir in 0..count {
                let mut text = format!("com.d{dir}.l0 = com.base.l3 + 1\n");
                for leaf in 1..10 {
                    writeln!(text, "com.d{dir}.l{leaf} = com.d{dir}.l{} + 1", leaf - 1).unwrap();
                }
                sources.push(SourceFile::new(format!("d{dir:04}/f.gcsl"), text).unwrap());
            }
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace
                .file(&format!("d{:04}/f.gcsl", count - 1))
                .unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            assert!(format!("{}", outcome.value).contains("l9 = 13"));
            writeln!(
                report,
                "{count} directories: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// [`benchmark_sibling_directories_merging`] with every file in one
    /// directory below the root: each file's view merges the root's `com`
    /// with the directory's, which has every file's part, so a file's view
    /// must not cost the size of its directory's tree. Linear in `count`.
    /// Run with `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_one_directory_merging_with_the_root() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let mut sources = vec![
                SourceFile::new(
                    "workspace.gcsl",
                    (0..10).fold(String::new(), |mut text, leaf| {
                        writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                        text
                    }),
                )
                .unwrap(),
            ];
            for file in 0..count {
                let mut text = format!("com.f{file}.l0 = com.base.l3 + 1\n");
                for leaf in 1..10 {
                    let before = leaf - 1;
                    writeln!(text, "com.f{file}.l{leaf} = com.f{file}.l{before} + 1").unwrap();
                }
                sources.push(SourceFile::new(format!("app/f{file:04}.gcsl"), text).unwrap());
            }
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace
                .file(&format!("app/f{:04}.gcsl", count - 1))
                .unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            assert!(format!("{}", outcome.value).contains("l9 = 13"));
            writeln!(
                report,
                "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// The cost of a file's view in a chain of `depth` directories, each
    /// below the one before and with one file of ten leaves under `com`,
    /// each reading a leaf of the directory above through the merged `com`:
    /// a file's view merges every directory above it, so the time per file
    /// grows with its depth. Run with `cargo test --release -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_deep_chain_of_directories() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for depth in [10_u32, 25, 50, 100] {
            let mut path = String::new();
            let mut sources = Vec::new();
            for level in 0..depth {
                let mut text = if level == 0 {
                    String::from("com.d0.l0 = 0\n")
                } else {
                    format!("com.d{level}.l0 = com.d{}.l9 + 1\n", level - 1)
                };
                for leaf in 1..10 {
                    let before = leaf - 1;
                    writeln!(text, "com.d{level}.l{leaf} = com.d{level}.l{before} + 1").unwrap();
                }
                let name = if level == 0 {
                    String::from("workspace.gcsl")
                } else {
                    write!(path, "d{level}/").unwrap();
                    format!("{path}f.gcsl")
                };
                sources.push(SourceFile::new(name, text).unwrap());
            }
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace.file(&format!("{path}f.gcsl")).unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            assert!(format!("{}", outcome.value).contains(&format!("l9 = {}", depth * 10 - 1)));
            let per_file = total / depth;
            writeln!(
                report,
                "{depth} deep: scan {scanned:?}; check {checked:?}, check and run {total:?}, \
                 {per_file:?} a file"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    // ----- The standard library -----

    /// The summary of slice 28: the prelude's names, `builtins` and the
    /// prelude imported by name.
    const STANDARD_SUMMARY: &str = "xs = [1, 2, 3]\n\
                                    n = len(xs)\n\
                                    h = half(n)\n\
                                    import builtins\n\
                                    same = builtins.len(xs) == n\n\
                                    import len as count from prelude\n\
                                    c = count(xs)";

    /// The value of `text` run as a lone file, beside the standard library,
    /// or the messages of its diagnostics.
    fn lone(text: &str) -> Result<String, Vec<String>> {
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        crate::run(&source)
            .map(|outcome| outcome.value.to_string())
            .map_err(|errors| errors.into_iter().map(|e| e.message).collect())
    }

    #[test]
    fn the_standard_summary_runs_in_a_workspace_and_alone() {
        let expected = "{xs = [1, 2, 3]; n = 3; h = 1.5; same = true; c = 3}";
        let files = [("workspace.gcsl", ""), ("app/main.gcsl", STANDARD_SUMMARY)];
        assert_eq!(value(&files, "app/main.gcsl"), expected);
        assert_eq!(lone(STANDARD_SUMMARY).unwrap(), expected);
        // The standard files are the first files, by input number, in a
        // directory of their own.
        let scan = ws(&files).unwrap();
        assert_eq!(scan.standard(), 1);
        assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
        assert_eq!(scan.dirs[scan.files[0].dir as usize].parent, None);
        assert_eq!(scan.order[0], 0);
        // The built-in functions' cells are the first.
        let builtins: Vec<Option<Builtin>> = (0..6).map(|node| scan.builtin(node)).collect();
        assert_eq!(
            builtins,
            [
                Some(Builtin::Len),
                Some(Builtin::Int),
                Some(Builtin::Float),
                Some(Builtin::Exec),
                Some(Builtin::Panic),
                None
            ]
        );
    }

    #[test]
    fn len_is_reached_by_every_road() {
        for text in [
            "n = len([1, 2])",
            "import len from builtins\nn = len([1, 2])",
            "import builtins\nn = builtins.len([1, 2])",
            "import * from builtins\nn = len([1, 2])",
            "import prelude\nn = prelude.len([1, 2])",
            "let l = len\nn = l([1, 2])",
            "let l = len\nlet m = l\nn = m([1, 2])",
            "l = len\nn = l([1, 2])",
            "import len as count from prelude\nlet l = count\nn = l([1, 2])",
            "import builtins\nl = builtins.len\nn = l([1, 2])",
        ] {
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            let shown = value(&files, "main.gcsl");
            assert!(shown.contains("n = 2"), "{text}: {shown}");
            assert!(lone(text).unwrap().contains("n = 2"), "{text}");
        }
        // An alias of `len` is of its opaque type; the callee of a call of
        // it has the function type of the call.
        let mut workspace =
            checked(&[("workspace.gcsl", ""), ("main.gcsl", "l = len\nn = l([1])")]);
        let index = workspace.file("main.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(dump.contains("(name len : builtin len @4..7)"), "{dump}");
        assert!(
            dump.contains("(name l : fn(List<Int>): Int @12..13)"),
            "{dump}"
        );
    }

    #[test]
    fn an_alias_of_len_crosses_files_directories_and_imports() {
        // A field of the root's, a member import's alias of an alias in
        // another directory, and a `let` of that in a third file.
        let files = [
            ("workspace.gcsl", "measure = len"),
            (
                "lib/a.gcsl",
                "import len as count from prelude\nsize = count",
            ),
            (
                "app/main.gcsl",
                "import size from lib\nlet l = size\nn = l([1, 2]) + measure([1])\n\
                 def twice(v: Int[]): Int { l(v) + size(v) }",
            ),
            ("app/other.gcsl", "m = measure([1, 2, 3]) + twice([1])"),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{n = 3; twice = fn(v: List<Int>): Int}"
        );
        assert_eq!(value(&files, "app/other.gcsl"), "{m = 5}");
        assert_eq!(value(&files, "lib/a.gcsl"), "{size = builtin len}");
        // Read whole, a record holding one is a use of it as a value.
        let mut whole = files.to_vec();
        whole.push(("app/whole.gcsl", "import lib\nall = lib"));
        assert_eq!(
            messages(&whole, "app/whole.gcsl"),
            ["`len` is a built-in function and cannot be used as a value yet"]
        );
    }

    #[test]
    fn int_and_float_are_values_of_their_function_types() {
        for (text, expected) in [
            ("= int(2.7)", "2"),
            ("= int(-2.7)", "-2"),
            ("= int(x = 9.99)", "9"),
            ("= float(3)", "3.0"),
            ("= float(-3)", "-3.0"),
            ("= int", "fn(x: Float): Int"),
            ("import builtins\n= builtins.float", "fn(x: Int): Float"),
            ("import int from builtins\n= int(1.5)", "1"),
            // Passed, stored and called through a variable.
            ("let i = int\n= i(-0.5)", "0"),
            (
                "fn apply(f: fn(x: Float): Int, v: Float): Int { f(v) }\n= apply(int, 7.5)",
                "7",
            ),
            ("let fs = [float, float]\n= fs[1](2)", "2.0"),
            ("let r = { f = float }\n= r.f(4)", "4.0"),
            ("let mut f = int\nf := int\n= f(3.5)", "3"),
            ("= half(3)", "1.5"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // Truncation toward zero fails when the result is no `Int`.
        for (text, message) in [
            ("= int(1e300)", "integer overflow"),
            ("= int(-1e300)", "integer overflow"),
            ("= int(1.0 / 0.0)", "integer overflow"),
            ("= int(0.0 / 0.0)", "cannot truncate NaN to an Int"),
            ("let i = int\n= i(1e300)", "integer overflow"),
        ] {
            assert_eq!(lone(text).unwrap_err(), [message], "{text}");
        }
        assert_eq!(
            lone("= int(9223372036854774784.0)").unwrap(),
            "9223372036854774784"
        );
        assert_eq!(
            lone("= int(-9223372036854775808.0)").unwrap(),
            "-9223372036854775808"
        );
        // Functions have no equality, built-in ones included.
        assert_eq!(
            lone("= int === int").unwrap_err(),
            ["cannot compare functions"]
        );
    }

    #[test]
    fn a_file_s_own_names_shadow_the_prelude_s_silently() {
        // A field, a `def`, a `let` and a `fn`: no clash, and the built-in
        // is gone there.
        for (text, expected) in [
            ("len = 1\nn = len + 1", "{len = 1; n = 2}"),
            (
                "def len(v: Int[]): Int { v[0] + 41 }\nn = len([1])",
                "{len = fn(v: List<Int>): Int; n = 42}",
            ),
            ("let len = 5\nn = len", "{n = 5}"),
            ("fn len(): Int { 7 }\nn = len()", "{n = 7}"),
            ("let int = 2\nn = int * 3", "{n = 6}"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // A `let` shadows from its item on, as it always has.
        assert_eq!(
            lone("n = len([1])\nlet len = 5\nm = len").unwrap(),
            "{n = 1; m = 5}"
        );
        // A field of the directory, or of a directory above, shadows it in
        // every file of the directory, and an explicit import does too.
        let files = [
            ("workspace.gcsl", "len = 3"),
            ("sub/a.gcsl", "n = len + 1"),
            ("sub/b.gcsl", "import int from builtins\nm = len + int(1.5)"),
        ];
        assert_eq!(value(&files, "sub/a.gcsl"), "{n = 4}");
        assert_eq!(value(&files, "sub/b.gcsl"), "{m = 4}");
        // Nothing of the implicit import is an alias the scan knows of.
        let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "len = 1")]).unwrap();
        let names: Vec<&str> = scan.files[2]
            .aliases
            .iter()
            .map(|alias| scan.names.get(alias.name))
            .collect();
        assert_eq!(names, ["int", "float", "half", "exec", "panic", "sh"]);
        assert!(scan.files[2].aliases.iter().all(|alias| alias.implicit));
        assert!(scan.files[0].aliases.iter().all(|alias| !alias.implicit));
    }

    #[test]
    fn an_explicit_star_from_the_prelude_conflicts_as_any() {
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import * from prelude\nlen = 3"),
        ];
        assert_eq!(
            shown_error(&files),
            "field `len` is defined in two files at main.gcsl (imported here); \
             <stdlib>/prelude.gcsl: defined here; main.gcsl: defined here"
        );
        // The same for the `builtins` module, whose side has no definition.
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import * from builtins\nlen = 3"),
        ];
        assert_eq!(
            shown_error(&files),
            "field `len` is defined in two files at main.gcsl (imported here); \
             main.gcsl: defined here; note: `len` is a built-in function"
        );
        // An explicit import of the prelude's name and the implicit one are
        // one cell: no conflict.
        assert_eq!(
            lone("import * from prelude\nn = len([1, 2])").unwrap(),
            "{n = 2}"
        );
        // A `let` of an imported name is the clash it always was.
        assert_eq!(
            lone("import len from prelude\nlet len = 1").unwrap_err(),
            ["`len` is imported in this file"]
        );
    }

    #[test]
    fn the_standard_modules_names_are_reserved() {
        for (files, message, input) in [
            (
                &[("workspace.gcsl", ""), ("prelude.gcsl", "x = 1")][..],
                "`prelude` is the name of a standard module",
                Some(1),
            ),
            (
                &[("builtins/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
                "`builtins` is the name of a built-in module",
                Some(1),
            ),
            (
                &[("builtins.gcsl", ""), ("workspace.gcsl", "")][..],
                "`builtins` is the name of a built-in module",
                Some(1),
            ),
        ] {
            assert_eq!(
                error(files),
                (message.to_owned(), input, vec![]),
                "{files:?}"
            );
        }
        // Below the root, a name of a path of its own.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/prelude.gcsl", "x = 1"),
            ("main.gcsl", "import lib.prelude\nn = prelude.x + len([1])"),
        ];
        assert_eq!(value(&files, "main.gcsl"), "{n = 2}");
        // A path of more than one segment never names a standard module.
        let files = [("workspace.gcsl", ""), ("main.gcsl", "import prelude.len")];
        assert_eq!(error(&files).0, "no module `prelude.len`");
    }

    #[test]
    fn a_lone_file_imports_only_the_standard_library() {
        assert_eq!(
            lone("import nothing").unwrap_err(),
            [
                "`nothing` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
            ]
        );
        assert_eq!(
            lone("x = 1 + true\nimport a from c.d").unwrap_err(),
            [
                "cannot add Int and Bool",
                "`c.d` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
            ]
        );
        assert_eq!(lone("import builtins\n= builtins.len([1])").unwrap(), "1");
        assert_eq!(lone("import prelude\n= prelude.half(1)").unwrap(), "0.5");
        // Its diagnostics name no input; a standard file's names its own.
        let source = SourceFile::new("lone.gcsl", "x = 1 + true").unwrap();
        assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
        let source = SourceFile::new("lone.gcsl", "import * from prelude\nlen = 1").unwrap();
        let error = &crate::run(&source).unwrap_err()[0];
        assert_eq!(error.input, None);
        let inputs: Vec<Option<u32>> = error.labels.iter().map(|l| l.input).collect();
        assert_eq!(inputs, [Some(0), None]);
    }

    #[test]
    fn the_table_of_standard_files_is_the_stdlib_directory() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib");
        let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "gcsl"))
            .map(|path| {
                let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
                (stem, std::fs::read_to_string(&path).unwrap())
            })
            .collect();
        found.sort();
        let table: Vec<(String, String)> = crate::STANDARD
            .iter()
            .map(|&(stem, text)| (stem.to_owned(), text.to_owned()))
            .collect();
        assert_eq!(table, found);
        // Each is a standard file, in path order, named under `<stdlib>/`.
        let names: Vec<String> = standard_sources()
            .iter()
            .map(|source| source.name().to_owned())
            .collect();
        let expected: Vec<String> = found
            .iter()
            .map(|(stem, _)| format!("<stdlib>/{stem}.gcsl"))
            .collect();
        assert_eq!(names, expected);
    }

    // ----- `exec`, `panic` and `sh` -----

    /// The summary of slice 29, its `ls` of a directory every machine has.
    const SHELL_SUMMARY: &str = "r = exec([\"ls\", \"-1d\", \"/\"])\n\
                                 lines = r.stdout\n\
                                 ok = r.status == 0\n\
                                 out = sh(\"echo hi\")\n\
                                 fn pkg(name: String): { build: Future<String> } {\n\
                                     { build = lazy { sh(\"echo make {name}\") } }\n\
                                 }\n\
                                 built = await pkg(\"lib\").build\n\
                                 fn guard(n: Int): Int {\n\
                                     if n < 0 { panic(\"negative size {n}\") }\n\
                                     n\n\
                                 }\n\
                                 size = guard(3)";

    /// The one diagnostic of running `text` as a lone file.
    fn lone_error(text: &str) -> Diagnostic {
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let mut errors = crate::run(&source).unwrap_err();
        assert_eq!(errors.len(), 1, "{text}: {errors:?}");
        errors.remove(0)
    }

    #[test]
    fn the_shell_summary_runs_in_a_workspace_and_alone() {
        let expected = "{r = {status = 0; stdout = \"/\\n\"; stderr = \"\"}; lines = \"/\\n\"; \
                        ok = true; out = \"hi\\n\"; built = \"make lib\\n\"; size = 3}";
        assert_eq!(lone(SHELL_SUMMARY).unwrap(), expected);
        let files = [("workspace.gcsl", ""), ("app/main.gcsl", SHELL_SUMMARY)];
        assert_eq!(value(&files, "app/main.gcsl"), expected);
        assert_eq!(
            lone_error("fn guard(n: Int): Int {\nif n < 0 { panic(\"negative size {n}\") }\nn\n}\n= guard(-2)")
                .message,
            "negative size -2"
        );
    }

    #[test]
    fn exec_reports_what_its_program_did() {
        for (text, expected) in [
            (
                "= exec([\"true\"])",
                "{status = 0; stdout = \"\"; stderr = \"\"}",
            ),
            (
                "= exec([\"false\"])",
                "{status = 1; stdout = \"\"; stderr = \"\"}",
            ),
            (
                "= exec([\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"])",
                "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
            ),
            // A non-zero status is data, never an error.
            ("= exec([\"false\"]).status != 0", "true"),
            // Killed by a signal: 128 plus its number.
            ("= exec([\"sh\", \"-c\", \"kill -9 $$\"]).status", "137"),
            // Its standard input is the null device: `cat` ends at once.
            (
                "= exec([\"cat\"])",
                "{status = 0; stdout = \"\"; stderr = \"\"}",
            ),
            // Invalid UTF-8 is replaced.
            (
                "= exec([\"sh\", \"-c\", \"printf '\\\\377ok'\"]).stdout",
                "\u{FFFD}ok",
            ),
            // The arguments are passed as they are, with no shell.
            (
                "= exec([\"printf\", \"%s|\", \"a b\", \"$HOME\", \"*\"]).stdout",
                "a b|$HOME|*|",
            ),
            // Named, through a variable, a field, a `->` function.
            ("= exec(argv = [\"echo\", \"n\"]).stdout", "n\n"),
            ("let e = exec\n= e([\"echo\", \"v\"]).stdout", "v\n"),
            (
                "let r = { run = exec }\n= r.run([\"echo\", \"f\"]).stdout",
                "f\n",
            ),
            (
                "fn run(a: String) -> String { exec([\"echo\", a]).stdout }\n= run(\"i\")",
                "i\n",
            ),
            (
                "fn pick(f: fn(argv: Present<List<String>>) -> {status: Int}): \
                 fn(Present<List<String>>) -> {status: Int} { f }\n\
                 = pick(exec)([\"false\"]).status",
                "1",
            ),
            (
                "import exec as run from builtins\n= run([\"true\"]).status",
                "0",
            ),
            // A future argument is awaited, as any call's.
            (
                "let xs = lazy [\"echo\", \"later\"]\n= exec(xs).stdout",
                "later\n",
            ),
            (
                "let xs = [lazy \"echo\", \"deep\"]\n= exec(xs).stdout",
                "deep\n",
            ),
            // The value shows as its type.
            (
                "= exec",
                "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}",
            ),
            ("= panic", "fn(message: String): Never"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }

    #[test]
    fn exec_fails_only_when_its_program_cannot_run() {
        let text = "let none: List<String> = []\nr = exec(none)";
        let error = lone_error(text);
        assert_eq!(error.message, "`exec` needs a program to run");
        assert_eq!(error.span, Span::new(32, 42));
        assert!(error.notes.is_empty());
        let text = "r = exec([\"gcsl-no-such-program-slice-29\", \"x\"])";
        let error = lone_error(text);
        assert_eq!(error.message, "cannot run `gcsl-no-such-program-slice-29`");
        assert_eq!(error.span, Span::new(4, 48));
        assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
        assert_eq!(error.input, None);
        // A directory is not a program.
        let error = lone_error("r = exec([\"/\"])");
        assert_eq!(error.message, "cannot run `/`");
        assert_eq!(error.notes, ["Permission denied (os error 13)"]);
    }

    #[test]
    fn panic_ends_the_run_with_its_message_at_the_call() {
        let text = "x = 1\ny = panic(\"x\")";
        let error = lone_error(text);
        assert_eq!(error.message, "x");
        assert_eq!(error.span, Span::new(10, 20));
        assert!(error.labels.is_empty() && error.notes.is_empty());
        assert_eq!(lone_error("panic(\"one\\ntwo\")").message, "one\ntwo");
        // Wherever it stands, whichever walker meets it: a statement, a
        // `let`, a branch, an operand of each kind, an element, a field, an
        // argument, a callee, an interpolation, a function's result, a
        // `lazy`, a loop.
        for text in [
            "panic(\"p\")",
            "let x = panic(\"p\")",
            "let c = true\nx = if c { panic(\"p\") } else { 1 }",
            "x = 1 + panic(\"p\")",
            "x = 1.5 * panic(\"p\")",
            "x = true and panic(\"p\")",
            "x = \"a\" + panic(\"p\")",
            "x = \"a{panic(\"p\")}\"",
            "x = [1, panic(\"p\")]",
            "x = len(panic(\"p\"))",
            "x = { a = panic(\"p\") }",
            "let r: {a: Int} = panic(\"p\")",
            "x = float(panic(\"p\"))",
            "x = panic(\"p\")(1)",
            "fn f(n: Int): Int { if n > 0 { panic(\"p\") }\nn }\nx = f(1)",
            "fn f(): String { panic(\"p\") }\nx = f() + \"!\"",
            "x = await lazy panic(\"p\")",
            "x = for i in [1] { panic(\"p\") }",
            "def f(n: Int) ->> Int { panic(\"p\") }\nx = f(1)",
            // As a value of a parameter of type `fn(String): Int`.
            "fn apply(f: fn(String): Int): Int { f(\"p\") }\nx = apply(panic)",
            "let p = panic\nx = p(\"p\")",
        ] {
            assert_eq!(lone_error(text).message, "p", "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(messages(&files, "main.gcsl"), ["p"], "{text}");
        }
        // What follows a panic never runs.
        assert_eq!(
            lone_error("let c = true\nif c { panic(\"first\") }\n= 1 // 0").message,
            "first"
        );
        // A message of the program's own wording, built at runtime.
        assert_eq!(
            lone_error("let n = 3\n= panic(\"{n} is {n * 2 // 2}\")").message,
            "3 is 3"
        );
    }

    #[test]
    fn sh_returns_its_output_or_panics_in_the_prelude() {
        assert_eq!(lone("= sh(\"echo hi\")").unwrap(), "hi\n");
        assert_eq!(lone("= sh(\"printf hi\")").unwrap(), "hi");
        // The panic is the prelude's, located in it: input 0.
        let error = lone_error("x = sh(\"echo nope >&2; exit 2\")");
        assert_eq!(
            error.message,
            "`echo nope >&2; exit 2` exited with status 2: nope\n"
        );
        assert_eq!(error.input, Some(0));
        let prelude = crate::STANDARD[0].1;
        let at = u32::try_from(prelude.find("panic(\"`").unwrap()).unwrap();
        assert_eq!(error.span.start, at);
        // With no standard error, the message ends in ": ".
        assert_eq!(
            lone_error("x = sh(\"exit 2\")").message,
            "`exit 2` exited with status 2: "
        );
        let files = [("workspace.gcsl", ""), ("main.gcsl", "x = sh(\"exit 2\")")];
        assert_eq!(
            run_file(&files, "main.gcsl").unwrap_err(),
            [("`exit 2` exited with status 2: ".to_owned(), Some(0))]
        );
    }

    #[test]
    fn a_memoised_function_around_exec_runs_its_program_once() {
        let path = std::env::temp_dir().join(format!("gcsl-slice-29-{}", std::process::id()));
        let path = path.to_str().unwrap();
        let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
        let _ = std::fs::remove_file(path);
        let memo = format!(
            "fn mark(p: String) ->> Int {{ exec([\"sh\", \"-c\", \"echo x >> \" + p]).status }}\n\
             = [mark(\"{path}\"), mark(\"{path}\")]"
        );
        assert_eq!(lone(&memo).unwrap(), "[0, 0]");
        assert_eq!(count(), 1);
        // An impure function that is not memoised runs it each time.
        let impure = memo.replace("->>", "->");
        assert_eq!(lone(&impure).unwrap(), "[0, 0]");
        assert_eq!(count(), 3);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn exec_panic_and_sh_are_shadowed_silently() {
        for (text, expected) in [
            ("exec = 1\nn = exec + 1", "{exec = 1; n = 2}"),
            ("let panic = \"no\"\nn = panic", "{n = \"no\"}"),
            (
                "def sh(n: Int): Int { n * 2 }\nn = sh(2)",
                "{sh = fn(n: Int): Int; n = 4}",
            ),
            (
                "fn panic(m: String): Int { 7 }\nn = panic(\"x\")",
                "{n = 7}",
            ),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }

    #[test]
    fn every_record_exec_builds_shares_the_runs_one_layout() {
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "ra = exec([\"true\"])\n\
                 def run() -> {status: Int, stdout: String, stderr: String} { exec([\"true\"]) }",
            ),
            (
                "b.gcsl",
                "rb = exec([\"false\"])\nlet e = exec\ns = e([\"true\"])\nt = run()",
            ),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        // The driver wrote the five built-in functions' cells.
        assert_eq!(assert_builtins_written(&workspace).len(), 5);
        let a = workspace.file("a.gcsl").unwrap();
        let b = workspace.file("b.gcsl").unwrap();
        let first = workspace.run(a).unwrap().value;
        let second = workspace.run(b).unwrap().value;
        let layout = |value: &Value, field: &str| {
            let Value::Record(record) = value else {
                panic!("not a record: {value}");
            };
            let name = workspace.scan.names.lookup(field).unwrap();
            let Some(Value::Record(result)) = record.get(name) else {
                panic!("no record {field} in {value}");
            };
            Rc::clone(&result.layout)
        };
        for (value, field) in [
            (&first, "ra"),
            (&second, "rb"),
            (&second, "s"),
            (&second, "t"),
        ] {
            assert!(
                Rc::ptr_eq(&layout(value, field), &workspace.exec_result),
                "{field} of {value}"
            );
        }
        assert_eq!(
            &*workspace.exec_result.texts,
            ["status", "stdout", "stderr"].map(Box::from)
        );
    }
}
