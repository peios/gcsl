//! A workspace: the files of a directory tree, each directory's files
//! sharing one scope, and every file's dotted paths merged into one view.
//!
//! A *workspace* is the directory tree under a directory that holds a file
//! named `workspace.gcsl`; finding it and reading its files is the CLI's,
//! and this module reads no file: it is given the files as
//! [`SourceFile`]s named by their paths relative to the root, `/`
//! separated, with the root's absolute path, which only the `fileroot`
//! module tells (see [The standard library](#the-standard-library)), and
//! answers with diagnostics or values.
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
//! # The standard library
//!
//! Two kinds of module are not files of the workspace. `builtins` and
//! `fileroot` are modules implemented in Rust: `builtins` a record of the
//! built-in functions ([`Builtin`]): `len`, `int`, `float`, `exec`,
//! `panic`, `split`, `join`, `starts_with`, `ends_with`, `trim`,
//! `strip_prefix`, `strip_suffix`, `lines`, `fields`, `compare_versions`,
//! the file system's `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`,
//! `fs_temp_dir` and `fs_write`, and `sort_versions` and `show`;
//! `fileroot` where files are (below). The *standard files* are the
//! `.gcsl` files of the repository's `stdlib/` directory, compiled in
//! ([`crate::STANDARD`]): `archive.gcsl`, unpacking archives;
//! `toolchain.gcsl`, the tools a library asks for; `keychain.gcsl`, the
//! secrets it asks for; `libraries.gcsl`, what a host pinned of the
//! libraries a workspace imports; `packaging.gcsl`, the packagers a
//! workspace packs with; `fs.gcsl`, the
//! file system, whose members are
//! the file system's built-in functions, bound as the prelude binds the
//! others (`exists = builtins.fs_exists`, `write = builtins.fs_write`), so
//! that a call of one is a call of the built-in, its errors located at the
//! call; `path.gcsl`, lexical operations on paths; and `prelude.gcsl`,
//! which binds the other built-in functions, `show` and `len` by the alias
//! rule, and `sh`. Each is
//! importable by a bare name, `import builtins`, `import len from
//! builtins`, `import fileroot`, `import * from prelude`, `import path`,
//! `import fs`, from any file; the names are reserved roots of the path
//! namespace, and a file or directory at the workspace's root named like
//! one of them is an error of the scan, "`builtins` is the name of a
//! built-in module" (and so for `fileroot`), "`prelude` is the name of a
//! standard module" (and so for `archive`, `toolchain`, `path` and `fs`), reported at the start of
//! the file, or of the first file under the directory, whether or not any
//! file imports it. A path of more
//! than one segment never names one, and a file or directory below the
//! root has a path of its own.
//!
//! EACH STANDARD FILE IS A DIRECTORY SCOPE OF ITS OWN, a directory with no
//! directory above, whose path is [`STANDARD_DIR`]: it sees its own
//! declarations, the prelude's through its implicit `#[shadowable] import *
//! from prelude` (below, as for any file), and what it imports, by
//! bare name, `builtins` and the other standard files; nothing else of the
//! standard library and nothing of the workspace, and nothing of the
//! workspace sees its declarations but by an import. So two standard files
//! may declare one top-level name, as the prelude's string `join` and
//! `path`'s `join` do. One standard file importing another is an import
//! like any other, and standard files that need each other are the usual
//! error, which is a bug of the library.
//!
//! ONLY THE STANDARD FILES A RUN NEEDS ARE LOADED: checked, lowered and
//! run, given cells, slots and input numbers. A scan of a workspace or a
//! lone file loads the prelude, always, and each standard file that any
//! file it scans imports, whether or not the file run needs that file,
//! since every file of a workspace is checked: an `import` whose path is
//! one segment naming a standard file, in whatever form (`import path`,
//! `import path as p`, `import join from path`, `import * from path`);
//! and, transitively, each standard file that a standard file it loads
//! imports so. The scan computes the set as it parses (`parse_files`): it
//! parses the prelude, and what the prelude imports, then the workspace's
//! files, then reads the top-level `import` items of each file that parsed
//! and of each standard file as it is parsed in turn (`standard_imports`),
//! and parses each standard file they name that it has not, until none is
//! left; a file that fails to parse imports nothing. A standard file
//! nobody imports is never parsed. A session loads every standard file
//! instead (see [`crate::session`]).
//!
//! The standard files a scan loads are its first files, so they take the
//! first input numbers, in the order of [`crate::STANDARD`], which is path
//! order, each named `<stdlib>/STEM.gcsl`, the workspace's files following
//! them: so `path`, when it is loaded without `fs`, is input 0 and the
//! prelude input 1, `fs` with both is input 0 and they 1 and 2, and a run
//! that loads the prelude alone numbers its files as if there were no other
//! standard file. Since the caller cannot know which are
//! loaded before the scan has parsed the files, [`check_with_standard`] and
//! [`check_lone_with_standard`] return their sources with the check,
//! whether it succeeds or fails, for rendering diagnostics by input number;
//! [`standard_sources`] gives every one, as a session loads them. They are
//! checked and run with the workspace's files, before them in the order
//! (they need no file of the workspace), on the same frame and memo
//! tables; every run runs those loaded first ([`Scan::needed`]). An error
//! in one is reported like any other, and is a bug.
//!
//! The `builtins` module is a directory of its own too, with no file: its
//! tree is one root leaf per built-in function, nodes `0..` of
//! [`Scan::nodes`] in the order of
//! [`Builtin::ALL`], so their cells are the first slots of the frame, which
//! the driver writes, each a [`Value::Builtin`], as it lays the frame out,
//! before any file runs ([`check_with`]). No file computes them, and none
//! needs another for them. Their types are each built-in's: its function
//! type, or for one whose typing is special the opaque type of the alias
//! rule (see [`crate::check`], "Built-in functions"). As it lays the frame
//! out, the driver also builds the one layout of the records `exec` and
//! `exec_live` return (`exec_layout`), its field names interned in the
//! run's names, and gives it to every file it runs, beside the memo tables,
//! so that
//! every such record in the run shares it (see [`crate::eval`], "The
//! built-in functions"); a session gets it with the rest of the standard
//! library's run.
//!
//! A [`Workspace`] OWNS THE TEMPORARY DIRECTORIES that `fs.temp_dir` makes
//! in its runs, the files' and every [`Workspace::invoke`]'s alike (a
//! [`crate::eval::TempDirs`], given to every file it runs as the layout of
//! `exec`'s records is): one parent directory in the system's temporary
//! directory, made by the first call and never before, so that a run that
//! makes none touches nothing, each directory a numbered child of it. As
//! the workspace is dropped, the parent is removed, with everything in it,
//! whether its runs succeeded or failed: so when `gcsl run`, of a workspace
//! or of a lone file, whose run is a workspace's too, or `pekit` is done
//! with it. A process killed before then leaves the directory behind. Its
//! host may choose another directory than the system's temporary directory
//! for the parent, its TEMP ROOT ([`Workspace::set_temp_root`]), before it
//! runs anything: the root is made by the first call if it is missing, and
//! is never removed. A session owns those of its inputs, and of the
//! standard library's run as it starts, as a workspace does (see
//! [`crate::session`]), always in the system's temporary directory.
//!
//! The `fileroot` module has no file, no cell and no value: it is a target
//! of its own ([`Target::Fileroot`]), which only a whole import names,
//! `import fileroot` or `import fileroot as fr`, in a workspace's file, a
//! lone file or a session's input, and whose four members are read only as
//! `fileroot.NAME` through that name (each a target too, numbered after the
//! view's nodes, so that a reference resolves to it as to any member, and
//! mentions it, needing no file): `fileroot.this`, the directory of the
//! file the expression is written in; `fileroot.workspace`, the root;
//! `fileroot.caller`, the directory of the file whose code made the call
//! the running function runs for; and `fileroot.helper()`, after which the
//! running function's calls pass its own caller on (see [`crate::eval`],
//! "Callers and helpers", and the checker's "Imports"). A member or `*`
//! import of it is "`fileroot`'s members are read as `fileroot.caller`",
//! naming the member imported, `caller` for a `*`, and an import of a
//! name it lacks "`fileroot` has no `nope`". The scan looks the members'
//! names up, never interning them, once the files are parsed
//! (`Scan::fileroot_names`), since the names are lent to each file as it
//! is checked.
//!
//! Every path `fileroot` gives is absolute, without a trailing `/` (but `/`
//! itself), and is what the driver resolved for the files: the caller of
//! [`check_with_standard`] gives the workspace's root, which the CLI found
//! above the canonical path of the file it runs (`fs::canonicalize`, once,
//! so that it is canonical too), and [`check_lone_with_standard`] the lone
//! file's directory, which the CLI takes from the file's canonical path; a
//! session's root is the process's working directory as it starts,
//! canonical ([`working_directory`]), which [`check`], [`check_with`],
//! [`check_lone`] and [`crate::run`], told none, take too. Once a scan has
//! parsed its files, the driver *locates* them (`Scan::locate`): it builds
//! the run's table of directories, the root first, always, at index 0, then
//! each other directory with a file, once, by its absolute path, the root
//! joined to its path relative to the root, the standard files' `<stdlib>`
//! once; each file's `fileroot.this` is its directory's, the standard files'
//! `<stdlib>`, which nothing should rely on, and its `fileroot.workspace`
//! the root (a session's input: its root, which is both). The checker
//! gives each file's code its directory's index ([`crate::check::Place`],
//! [`crate::ir::Ir::dir`]), and every run its table, which
//! `fileroot.caller` reads. A session's inputs are each in its root, at
//! index 0 of the table its start built, which no input extends.
//!
//! Every file whose first items do not include `#![no_prelude]`, the standard
//! files included, and every session's input, begins as if with `#[shadowable]
//! import * from prelude`: the parse gives its tree the import, an item written
//! nowhere and kept apart from its body
//! ([`Ast::implicit_import`](crate::ast::Ast::implicit_import)), so that no
//! span or line moves; the scan takes it first of all in the static order (see
//! [Shadowable declarations](crate::workspace::parse#shadowable-declarations)),
//! its aliases last among the file's ([`Alias::implicit`]), so that the others
//! keep their numbers. So any declaration or import of one of its names, or of
//! a path below one, shadows the prelude's silently, wherever it is, and the
//! clash family ignores them; the aliases of those it shadows are gone from the
//! file. `prelude.gcsl` is marked `#![no_prelude]` itself, and a file so marked
//! has no prelude name but those it imports. An explicit `import * from
//! prelude` is an ordinary `*`, and conflicts as one.
//!
//! A file outside any workspace (`gcsl run` of a lone file, [`check_lone`])
//! is checked and run as a workspace of one directory holding just it,
//! beside the standard library, loading what it needs as a workspace does:
//! it may import `builtins` and the standard files, and an import of
//! anything else is the checker's error, "`lib` is not a built-in or
//! standard module, and there is no `workspace.gcsl` above this file", with
//! the file's other errors. A session's input is scanned so too
//! (`check_input`), though a session is not a workspace (see
//! [`crate::session`]), beside every standard file, as the session loaded
//! them when it started, so that their cells are numbered as they were
//! there; it inherits the imports of the session's earlier inputs, after
//! its own, last in the static order, but for each it imports again or
//! declares at its top level. An input is no file: one marked
//! `#![no_prelude]` is "`#![no_prelude]` applies only to a file"
//! (`no_prelude_in_input`). Its `#[shadowable]` marks are a file's within
//! it, and change nothing between inputs, a later one shadowing an earlier
//! one's names already.
//!
//! # Libraries
//!
//! A LIBRARY is a module apart from any workspace, named by its path, as a
//! string, `import "github.com/gcslib/gcc"`: the parser reads it as the
//! module path `<lib>/github.com/gcslib/gcc`, its first segment
//! [`LIBRARIES`], which no file of a workspace can name. Its files are given
//! with the workspace's by the host, which finds them (`gcsl-disk` in each of
//! its library roots, [`library_imports`] telling it which a file imports),
//! named `<lib>/PATH/FILE.gcsl`, so that a library is named the same however
//! it was found, on receipts among other places. Each library's directory is
//! a directory with no directory above, as a standard file's is: it sees its
//! own files' declarations, the prelude's and what it imports, which may be
//! other libraries and the standard library, but never a workspace's module,
//! "a library imports only libraries and the standard library, never a
//! workspace's modules", so a library means the same in every workspace. A
//! library's path names its directory whole, no directory above it being a
//! module; one the host did not give is "library `PATH` is not found", noted
//! with where hosts look for one. A library's path that is not segments of a
//! module path separated by `/` is the parser's error.

use std::mem;
use std::path::PathBuf;
use std::rc::Rc;

use crate::Outcome;

#[cfg(test)]
use crate::check::ViewRecord;
use crate::check::{Attribute, Builtin, KnownParams, Slot, Types};
use crate::diagnostic::{Diagnostic, InternalError, Mark};
use crate::eval::graph::{Graph, GraphHost, GraphRun, NodeId, NodeKind, RunExtras, Wait, run_node};
use crate::eval::plain::{Plain, from_plain, to_identity_plain, to_key_plain, to_plain};

/// A way to make a value plain data: [`to_plain`], [`to_key_plain`] or
/// [`to_identity_plain`].
type MakePlain = fn(&Value, &crate::names::Names) -> Result<Plain, String>;
use crate::eval::profile::{Profile, ProfileReport};
use crate::eval::{Direct, Effects, MemoTables, TempDirs};
use crate::ir::Ir;
use crate::names::{Name, Names};
use crate::session;
use crate::source::{SourceFile, Span};
use crate::types::{TyId, TypeTable};
use crate::value::{Layout, Value};

pub use files::{
    Alias, DeclaredContext, DeclaredType, DeclaredVariant, Dir, File, Function, Merged, MethodRef,
    Node, SeenVariant, Target, TypeImport, View, ViewNode, ViewPart,
};
pub(crate) use files::{
    CheckedMethod, ContextDefault, ForLater, ForeignMethod, InheritedContext, InheritedDefType,
    InheritedVariant, SeenContext,
};
use files::Inheriting;
pub use scan::Scan;
pub use scope::FileScope;
pub use scanning::{scan, scan_with};
use scanning::{Placing, count, parent_path, scan_into, scan_into_with};
use parse::{
    builtin_nodes, first_leaf, leaf_spans, leaves_under, no_prelude_in_input, parse_files,
    place_dirs, reserved, resolve, shadow, shadowable_nodes,
};
use views::{merge, views};
pub(crate) use imports::{Modules, module, target_roots};
use imports::{context_in, imports, value_cells};
use declared::{check_context_items, declare_types, define_types, method_views, type_views};
use needs::{clashes, find_needs, order, report_cycles, schedule};
pub(crate) use context::{AttributeLookup, Context};
use context::CellReads;
pub use checking::{
    Failed, Partial, check, check_lone, check_lone_with_standard, check_partial, check_with,
    check_with_standard, index_lone_with_standard, index_with_standard, working_directory,
};
pub(crate) use checking::{Recorded, check_lone_loading};
use checking::{
    check_declared_items, check_loading, checked_contexts, checked_methods, in_file, merge_files,
};
pub use requests::{DefType, Definition, Given, InvokeError, Referenced, RequestError};
use requests::{
    Argument, FoundDefType, INVOKED_DEFINITION, INVOKED_DEF_TYPE, Invocable, ResolvedDefinition,
    Scalar,
};
pub use invoke::{TopLevel, being_run};
pub(crate) use input::{Earlier, Standard, check_input, start};
#[cfg(test)]
pub(crate) use input::check_beside_prelude;
use input::quantity_literal;

/// The key of a root in [`Scan`]'s index of nodes by parent and name.
const ROOT: u32 = u32::MAX;

/// The path of the standard library's directories, which no path of a
/// workspace can name, `<` being in no name: each standard file is
/// `<stdlib>/STEM.gcsl`, which is how a diagnostic of one names it, in a
/// directory of its own with this path.
pub const STANDARD_DIR: &str = "<stdlib>";

/// The paths of the libraries `source` imports, `github.com/gcslib/gcc`,
/// each once, in source order: what a host loads before a workspace that has
/// the file is checked, and loads again for each library's own files, until
/// it has every library the workspace needs (see [Libraries](#libraries)).
/// A file that does not parse gives what it imports as far as it parses.
#[must_use]
pub fn library_imports(source: &SourceFile) -> Vec<String> {
    let (ast, _) = crate::parser::parse_source(source);
    let mut found: Vec<String> = Vec::new();
    for &item in ast.items(ast.root()) {
        let crate::ast::ItemKind::Import(id) = ast.item(item).kind else {
            continue;
        };
        let names: Vec<&str> = ast
            .path(ast.import_decl(id).path)
            .iter()
            .filter_map(crate::ast::PathSegment::name)
            .map(|name| ast.names().get(name))
            .collect();
        if let Some((&LIBRARIES, rest)) = names.split_first() {
            let path = rest.join("/");
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// The root of every library's path, which no file of a workspace can have:
/// `import "github.com/gcslib/gcc"` names the module `<lib>/github.com/gcslib/gcc`
/// (see [Libraries](#libraries)).
pub const LIBRARIES: &str = "<lib>";

/// The name of the module of built-in functions.
pub(crate) const BUILTINS: &str = "builtins";

/// The stem of the prelude, the standard file every scan loads.
const PRELUDE: &str = "prelude";

/// Every standard file ([`crate::STANDARD`]) as a source, in its order,
/// each named `<stdlib>/STEM.gcsl`: what a session, which loads them all,
/// renders a diagnostic of one against. A run of a workspace or a lone file
/// loads only those it needs: [`check_with_standard`] says which.
#[must_use]
pub fn standard_sources() -> Vec<SourceFile> {
    crate::STANDARD
        .iter()
        .filter_map(|&(stem, text)| standard_source(stem, text))
        .collect()
}

/// The standard file of stem `stem` and text `text` as a source, named
/// `<stdlib>/STEM.gcsl`.
fn standard_source(stem: &str, text: &str) -> Option<SourceFile> {
    SourceFile::new(format!("{STANDARD_DIR}/{stem}.gcsl"), text).ok()
}

/// The standard library a scan is given (see the module documentation,
/// "The standard library"): a table of standard files, and which of them
/// it loads.
#[derive(Clone, Copy, Debug)]
struct Library<'a> {
    /// The standard files, each with its stem, in path order: always
    /// [`crate::STANDARD`] but in tests.
    table: &'a [(&'a str, &'a str)],
    /// Whether the scan loads every standard file, as a session does, or
    /// only the prelude and those its files need.
    every: bool,
}

impl Library<'_> {
    /// The standard library of a workspace or a lone file: only the
    /// standard files it needs.
    const NEEDED: Library<'static> = Library {
        table: &crate::STANDARD,
        every: false,
    };

    /// The standard library of a session: every standard file.
    const EVERY: Library<'static> = Library {
        table: &crate::STANDARD,
        every: true,
    };

    /// The index in the table of the standard file of stem `stem`.
    fn find(&self, stem: &str) -> Option<usize> {
        self.table.iter().position(|&(name, _)| name == stem)
    }

    /// The standard files of the table at `loaded`, its indices, as
    /// sources, by input number.
    fn sources(&self, loaded: &[usize]) -> Vec<SourceFile> {
        loaded
            .iter()
            .filter_map(|&at| self.table.get(at))
            .filter_map(|&(stem, text)| standard_source(stem, text))
            .collect()
    }
}

/// The values of the cells of the built-in functions, the first slots of a
/// frame, in the order of [`Builtin::ALL`]: what the driver writes as it
/// lays the frame out, before any file runs.
fn builtin_cells() -> Vec<Value> {
    Builtin::ALL.into_iter().map(Value::Builtin).collect()
}

/// The layout of the record `exec` returns, and `exec_live`, its fields
/// ([`Builtin::EXEC_FIELDS`]) interned into `names`, the run's, as the
/// checker interns them for `exec`'s type, so that `r.status` reads it:
/// built once per run as the frame is laid out, and shared by every record
/// `exec` or `exec_live` builds in the run, whatever file, input or function
/// calls it (see [`crate::eval`]).
pub(crate) fn exec_layout(names: &mut Names) -> Rc<Layout> {
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

/// What a host needs of a step to key, stage and run it (see
/// [`Workspace::step_inputs`]).
#[derive(Clone, Debug, PartialEq)]
pub struct StepInputs {
    /// What the step is.
    pub kind: NodeKind,
    /// Its callee, as a closure's plain form: its code's input, its function's
    /// number and its captures; none for an effect. What it calls, which a
    /// host names its step by, apart from its code's contents.
    pub callee: Option<Plain>,
    /// Its arguments, in parameter order.
    pub args: Vec<Plain>,
    /// Its callee's captures, which its code reads; none for an effect.
    pub captures: Vec<Plain>,
    /// Each context bound where it was made that it can read, by its
    /// declaration's key's text, sorted, with its value: of a call, each
    /// with a default, and each with none that its callee's type needs
    /// ([`FnCode::needs`](crate::ir::FnCode::needs)); of an effect, each;
    /// and never the definition it is of
    /// ([`Workspace::step_definition`]).
    pub context: Vec<(String, Plain)>,
    /// Its callee's attributes, in source order; none for an effect.
    pub attributes: Vec<Attribute>,
    /// The files whose sources make its code, by number; none for an
    /// effect.
    pub code_files: Vec<u32>,
}

/// What running a unit in graph mode came to (see [`Workspace::run_graph`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// It finished with this value, which may hold results the host has yet
    /// to give.
    Done(Value),
    /// It stopped for the results of these nodes, which its tasks need,
    /// none of them given yet (see [Tasks](crate::eval::machine::tasks#tasks)).
    Paused(Vec<NodeId>),
    /// It stopped for the run of this unit, which computes a memoised
    /// call's value or a `lazy`'s that it needs, and has stopped itself: run
    /// it again once that unit has run (see [Graph
    /// mode](crate::eval::graph#graph-mode)).
    Waiting(u32),
}

impl Progress {
    /// What a run that stopped for `waits` is: the nodes it waits for, or,
    /// if it waits for none, the first unit it waits for. `None` if it
    /// waits for nothing.
    pub(crate) fn stopped(waits: &[Wait]) -> Option<Self> {
        let nodes: Vec<NodeId> = waits
            .iter()
            .filter_map(|wait| match *wait {
                Wait::Node(id) => Some(id),
                Wait::Unit(_) => None,
            })
            .collect();
        if !nodes.is_empty() {
            return Some(Progress::Paused(nodes));
        }
        waits.iter().find_map(|wait| match *wait {
            Wait::Unit(unit) => Some(Progress::Waiting(unit)),
            Wait::Node(_) => None,
        })
    }
}

/// A file runs at most once. A failed attempt must remain a failure for every
/// later request that needs it, without repeating any of its earlier effects.
/// Boxing keeps this state table compact after the large IR is released. A
/// file whose run in graph mode stopped for a step's result keeps its IR,
/// shared, to take up where it stopped, its closures' code the same (see
/// [Graph mode](crate::eval::graph#graph-mode)).
#[derive(Debug)]
enum FileRun {
    Pending(Box<Ir>),
    Paused(Rc<Ir>),
    Succeeded,
    Failed(Box<Diagnostic>),
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
    /// Each file's pending IR or completed outcome, by input number.
    runs: Vec<FileRun>,
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
    /// The temporary directories `fs.temp_dir` makes for its runs, none
    /// until the first is asked for, removed with everything in them as the
    /// workspace is dropped, in the temp root if one was set
    /// ([`Workspace::set_temp_root`]; see the module documentation, "The
    /// standard library").
    temp_dirs: TempDirs,
    /// How many calls have been prepared to run in graph mode, each a unit
    /// numbered after every file (see [Prepared
    /// calls](crate::workspace::graph#prepared-calls)).
    calls_prepared: u32,
    /// The evaluation of each unit in graph mode that stopped where it
    /// waits, by its number, for its next run to take up (see
    /// [Suspension](crate::eval::machine#suspension)).
    suspended: std::collections::HashMap<u32, crate::eval::machine::Suspended>,
    /// The profile being taken of its runs, if they are profiled
    /// ([`Workspace::start_profiling`]).
    profile: Option<Profile>,
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
    /// For each cell that is a `def`'s, what a call of it needs to know of
    /// its parameters (see [`Context::known`]), kept for a session's inputs
    /// and for the call that invokes a definition by name.
    cell_known: Vec<Option<Rc<KnownParams>>>,
    /// The methods and the contexts' defaults of the files, checked, kept for
    /// a session's inputs.
    for_later: ForLater,
    /// Each file's records of the nodes of its view with a leaf of it under
    /// them, by input number: kept for the tests' check that each reads
    /// only cells stored before it.
    #[cfg(test)]
    view_records: Vec<Vec<ViewRecord>>,
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

    /// Makes `root` the workspace's temp root: the directory in which the
    /// parent of the temporary directories `fs.temp_dir` makes in its runs
    /// is made, instead of the system's temporary directory (see the module
    /// documentation, "The standard library"). Everything else is as
    /// without it: the parent, `gcsl-PID-N`, is made by the first call and
    /// never before, and removed with everything in it as the workspace is
    /// dropped. `root` is made then, with its missing parents, if it is
    /// missing, and is never removed. A `root` that is not absolute fails
    /// every call: "`fs.temp_dir` could not create a temporary directory:
    /// the temporary directory root `ROOT` is not an absolute path".
    ///
    /// The root is read only as the parent is made, so a host calls this
    /// before running anything: once a temporary directory has been made,
    /// it changes nothing, and the parent made stays in use.
    pub fn set_temp_root(&mut self, root: PathBuf) {
        self.temp_dirs.set_root(root);
    }

    /// Profiles every run of the workspace's code from now on, its files',
    /// its calls' and its steps', afresh if it was profiled before (see
    /// [Profiling](crate::eval::profile#profiling)).
    pub fn start_profiling(&mut self) {
        self.profile = Some(Profile::new());
    }

    /// What the profile taken since [`Workspace::start_profiling`] found,
    /// if one is being taken.
    #[must_use]
    pub fn profile_report(&self) -> Option<ProfileReport> {
        self.profile
            .as_ref()
            .map(|profile| profile.report(&self.scan.names))
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
        let FileRun::Pending(ir) = self.runs.get_mut(index as usize)? else {
            return None;
        };
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

    /// Runs file `index` and, before it, the standard files the scan loaded
    /// and every file it needs that has not run, in the order
    /// ([`Scan::needed`]), on the frame whose first cells the driver wrote
    /// with the built-in functions, and returns its own value: its result,
    /// or the record of its own fields, with the warnings of the whole
    /// workspace.
    ///
    /// # Errors
    ///
    /// The runtime error of the first file that fails, naming its file,
    /// with the workspace's warnings. A failed file retains its diagnostic:
    /// later requests needing it return that error without running it or its
    /// dependents again. Requests for unrelated files can still run.
    pub fn run(&mut self, index: u32) -> Result<Outcome, Vec<Diagnostic>> {
        self.run_needed(index)?;
        Ok(Outcome {
            value: self
                .values
                .get(index as usize)
                .cloned()
                .unwrap_or(Value::Empty),
            warnings: self.warnings.clone(),
        })
    }

    /// Runs file `index` and every file it needs in graph mode, against
    /// `graph`, which the host keeps for the whole evaluation, as
    /// [`Workspace::run`] runs them otherwise (see [Graph
    /// mode](crate::eval::graph#graph-mode)): each file a unit, numbered as
    /// it is, `host` saying, of a call of a function with these attributes, a
    /// deferring one among them, whether it is a step, and how the files'
    /// other effects, the working directory among them, reach the system.
    /// [`Progress::Paused`] if a unit stopped for a step's result, to run
    /// again, from where the run stopped, once the host has given it; else
    /// [`Progress::Done`] with file `index`'s value, whose holes, the results
    /// the host has yet to give, [`Graph::holes`] finds.
    ///
    /// # Errors
    ///
    /// As [`Workspace::run`].
    pub fn run_graph(
        &mut self,
        index: u32,
        graph: &mut Graph,
        host: GraphHost<'_>,
    ) -> Result<Progress, Vec<Diagnostic>> {
        if let Some(progress) = self.run_files(index, Some((graph, host)))? {
            return Ok(progress);
        }
        Ok(Progress::Done(
            self.values
                .get(index as usize)
                .cloned()
                .unwrap_or(Value::Empty),
        ))
    }

    /// Runs the node `id` of `graph`, a step of a unit of this workspace, as
    /// plain GCSL runs it, its effects through `effects`, the host's effect
    /// context for it (see [Graph mode](crate::eval::graph#graph-mode)): a
    /// deferred call's callee with its arguments, in the context it was made
    /// in, or an effect's built-in. The host then gives the graph what it
    /// came to ([`Graph::resolve`]).
    ///
    /// # Errors
    ///
    /// The step's failure, in the input of the unit that made it if it is
    /// in none of its own; and an internal error if `graph` has no node `id`.
    pub fn evaluate(
        &mut self,
        graph: &Graph,
        id: NodeId,
        effects: &dyn Effects,
    ) -> Result<Value, Diagnostic> {
        let Some(node) = graph.node(id) else {
            return Err(InternalError {
                message: "a step the graph lacks",
                span: Span::new(0, 0),
            }
            .into());
        };
        let context = (&mut self.frame, &mut self.memo);
        let names = std::cell::RefCell::new(std::mem::take(&mut self.scan.names));
        let extras = RunExtras {
            exec_result: Some(&self.exec_result),
            dirs: &self.scan.dir_texts[..],
            effects,
            names: Some(&names),
            profile: self.profile.as_ref(),
        };
        let ran = run_node(node, context, extras);
        self.scan.names = names.into_inner();
        ran.map_err(|mut error| {
            error.input.get_or_insert(node.unit());
            error
        })
    }

    /// `value` as plain data, which a host can hash, keep and give back (see
    /// [Plain values](crate::eval::plain#plain-values)).
    ///
    /// # Errors
    ///
    /// What it holds that is no data: "a step's result must be plain data:
    /// it holds a function".
    pub fn to_plain(&self, value: &Value) -> Result<Plain, String> {
        to_plain(value, &self.scan.names)
    }

    /// The value `plain` is the plain form of, its names interned in the
    /// workspace's table (see [Plain values](crate::eval::plain#plain-values)).
    pub fn from_plain(&mut self, plain: &Plain) -> Value {
        from_plain(plain, &mut self.scan.names)
    }

    /// The text of each declared context's key, by the key: what a step's
    /// run tells its host each read it makes by.
    pub(crate) fn context_texts(&self) -> std::collections::HashMap<Name, Rc<str>> {
        self.scan
            .contexts
            .iter()
            .map(|context| (context.key, Rc::from(self.scan.names.get(context.key))))
            .collect()
    }

    /// What a host needs of the node `id` of `graph` to key, stage and run
    /// it as a step, once its data edges are given (see [Graph
    /// mode](crate::eval::graph#graph-mode)): its arguments as plain data,
    /// its callee's captures and the context it can read as keys, functions
    /// by their code
    /// ([Keys of functions](crate::eval::plain#keys-of-functions)), its
    /// callee's attributes, and
    /// the files whose sources make its code, by number, every file its
    /// callee's file needs and that file, standard ones among them; none for
    /// an effect, a built-in's.
    ///
    /// # Errors
    ///
    /// What of it is no data, or that `graph` has no node `id`.
    pub fn step_inputs(&self, graph: &Graph, id: NodeId) -> Result<StepInputs, String> {
        self.inputs_of(graph, id, (to_plain, to_key_plain))
    }

    /// What a host names the node `id` of `graph` by, a step, before its
    /// data edges are given: [`Workspace::step_inputs`], but each step's
    /// result it is given and that has not been given yet as
    /// [`Plain::Step`], by its node ([`to_identity_plain`]).
    ///
    /// # Errors
    ///
    /// As [`Workspace::step_inputs`].
    pub fn step_identity(&self, graph: &Graph, id: NodeId) -> Result<StepInputs, String> {
        self.inputs_of(graph, id, (to_identity_plain, to_identity_plain))
    }

    /// [`Workspace::step_inputs`], its arguments made plain by `plain` and
    /// its callee, captures and context by `key`.
    fn inputs_of(
        &self,
        graph: &Graph,
        id: NodeId,
        (plain, key): (MakePlain, MakePlain),
    ) -> Result<StepInputs, String> {
        let names = &self.scan.names;
        let node = graph.node(id).ok_or("a step the graph lacks")?;
        let plain_all = |values: &[Value]| -> Result<Vec<Plain>, String> {
            values.iter().map(|value| plain(value, names)).collect()
        };
        // What it can read, functions among them, which are keyed by their
        // code. A context with no default that its callee does not need is
        // one that nothing it calls reads, since needs are inferred through
        // every call: a context only `pekit run` binds, say, keys none of
        // the steps a script builds.
        // The definition the step is of, which a host names it by, keys it
        // no further: keyed by its closure, every step a definition reached
        // would change with its file.
        let definition = self.scan.names.lookup(crate::lower::DEFINITION_CONTEXT);
        let readable = |key: Name| {
            if Some(key) == definition {
                return false;
            }
            let Some(code) = node
                .closure()
                .and_then(|closure| closure.code.find_function(closure.function))
            else {
                return true;
            };
            let defaulted = self
                .scan
                .contexts
                .iter()
                .find(|context| context.key == key)
                .is_none_or(|context| context.binding.is_some());
            defaulted || code.needs.binary_search(&key).is_ok()
        };
        let context = node
            .context()
            .iter()
            .filter(|(key, _)| readable(*key))
            .map(|(name, value)| Ok((names.get(*name).to_owned(), key(value, names)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let (captures, attributes, code_files) = match node.closure() {
            Some(closure) => {
                let file = closure.code.input();
                let captures = closure
                    .captures
                    .iter()
                    .map(|value| key(value, names))
                    .collect::<Result<Vec<_>, String>>()?;
                (
                    captures,
                    closure.attributes().to_vec(),
                    self.scan.needed(file),
                )
            }
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        let callee = node
            .closure()
            .map(|closure| key(&Value::Fn(Rc::clone(closure)), names))
            .transpose()?;
        Ok(StepInputs {
            kind: node.kind(),
            callee,
            args: plain_all(node.args())?,
            captures,
            context,
            attributes,
            code_files,
        })
    }

    /// The files file `file` needs to run, by number, transitively, with
    /// itself and the standard files every run runs, in the order they run:
    /// every file whose change can change what it does.
    #[must_use]
    pub fn files_needed(&self, file: u32) -> Vec<u32> {
        self.scan.needed(file)
    }

    /// The files whose sources make the code of the node `id` of `graph`, a
    /// deferred call's, by number, as [`StepInputs::code_files`] gives them,
    /// but known before its data edges are given; none for an effect's.
    #[must_use]
    pub fn node_code_files(&self, graph: &Graph, id: NodeId) -> Vec<u32> {
        graph
            .node(id)
            .and_then(crate::eval::graph::Node::closure)
            .map_or_else(Vec::new, |closure| self.scan.needed(closure.code.input()))
    }

    /// The definition that made the node `id` of `graph`, a step or an
    /// effect: the one whose body was running where it was made, which binds
    /// itself wherever it is called from, or else the innermost call of a
    /// def-type reference, `DT PATH()`, around where it was made, whose
    /// definition every node it makes knows through a context of its own
    /// ([`DEFINITION_CONTEXT`](crate::lower::DEFINITION_CONTEXT)), named as
    /// [`Workspace::definition_of`] names a reference's. A prepared call
    /// binds its own definition so too ([`Workspace::run_prepared`]), so a
    /// node its own code makes is its definition's, and two prepared calls
    /// that make the same call make a node each. `None` for a node made
    /// outside every prepared call and reference, by a file's own run, say.
    /// So a host can place a step by the definition it is of, the same
    /// whichever target's build reaches it and whatever else it builds.
    #[must_use]
    pub fn step_definition(&self, graph: &Graph, id: NodeId) -> Option<Referenced> {
        let key = self.scan.names.lookup(crate::lower::DEFINITION_CONTEXT)?;
        let node = graph.node(id)?;
        let (_, Value::Fn(definition)) = node.context().into_iter().find(|(at, _)| *at == key)?
        else {
            return None;
        };
        self.definition_holding(&definition)
    }

    /// Runs file `index` and every file it needs, as [`Workspace::run`]
    /// does, but those that have run already, keeping each one's value.
    ///
    /// # Errors
    ///
    /// As [`Workspace::run`].
    fn run_needed(&mut self, index: u32) -> Result<(), Vec<Diagnostic>> {
        self.run_files(index, None).map(|_| ())
    }

    /// Runs file `index` and every file it needs, as
    /// [`Workspace::run_needed`] does, each in graph mode, as unit number its
    /// file's, if `graph` is given, the graph and which deferring calls the
    /// host takes as steps (see [Graph mode](crate::eval::graph#graph-mode)).
    /// Returns what the run stopped for, if it stopped, the file that
    /// stopped kept to take up where it stopped.
    ///
    /// # Errors
    ///
    /// As [`Workspace::run`].
    pub(crate) fn run_files(
        &mut self,
        index: u32,
        mut graph: Option<(&mut Graph, GraphHost<'_>)>,
    ) -> Result<Option<Progress>, Vec<Diagnostic>> {
        for at in self.scan.needed(index) {
            let Some(state) = self.runs.get_mut(at as usize) else {
                continue;
            };
            let ir = match mem::replace(state, FileRun::Succeeded) {
                FileRun::Pending(mut ir) => {
                    // The slots of the files run before it, which may end
                    // after its own, are kept (see [`Ir::reserve_frame`]).
                    ir.reserve_frame(count(self.frame.len()));
                    Rc::new(*ir)
                }
                FileRun::Paused(ir) => ir,
                FileRun::Succeeded => continue,
                FileRun::Failed(error) => {
                    let diagnostic = (*error).clone();
                    *state = FileRun::Failed(error);
                    return Err(merge_files(vec![diagnostic], self.warnings.clone()));
                }
            };
            let direct = Direct {
                temp_dirs: Some(&self.temp_dirs),
            };
            let effects: &dyn Effects = graph.as_ref().map_or(&direct, |(_, host)| host.effects);
            let names = std::cell::RefCell::new(std::mem::take(&mut self.scan.names));
            let context = (
                &mut self.memo,
                RunExtras {
                    exec_result: Some(&self.exec_result),
                    dirs: &self.scan.dir_texts[..],
                    effects,
                    names: Some(&names),
                    profile: self.profile.as_ref(),
                },
            );
            let suspended = self.suspended.remove(&at);
            let run = graph.as_mut().map(|(graph, host)| {
                GraphRun::new(mem::take(*graph), at, host.defers).resuming(suspended)
            });
            let outcome =
                session::run_input_in(&ir, &mut self.frame, context, (None, run.as_ref()));
            self.scan.names = names.into_inner();
            let paused = match (run, graph.as_mut()) {
                (Some(run), Some((graph, _))) => {
                    if let Some(suspended) = run.take_suspended() {
                        self.suspended.insert(at, suspended);
                    }
                    let (left, paused) = run.finish();
                    **graph = left;
                    paused
                }
                _ => Vec::new(),
            };
            let Some(state) = self.runs.get_mut(at as usize) else {
                continue;
            };
            match outcome {
                Ok(value) => {
                    if let Some(entry) = self.values.get_mut(at as usize) {
                        *entry = value;
                    }
                }
                // Stopped for a step's result: taken up where it stopped
                // once the host has given it.
                Err(error) if error.mark == Mark::PENDING => {
                    *state = FileRun::Paused(ir);
                    return match Progress::stopped(&paused) {
                        Some(progress) => Ok(Some(progress)),
                        None => Err(vec![
                            InternalError {
                                message: "a unit stopped for no step",
                                span: error.span,
                            }
                            .into(),
                        ]),
                    };
                }
                Err(mut error) => {
                    error.input.get_or_insert(at);
                    *state = FileRun::Failed(Box::new(error.clone()));
                    return Err(merge_files(vec![error], self.warnings.clone()));
                }
            }
        }
        Ok(None)
    }
}

pub(crate) mod checking;
pub(crate) mod context;
pub(crate) mod declared;
pub(crate) mod files;
pub mod graph;
pub(crate) mod imports;
pub(crate) mod input;
pub(crate) mod invoke;
pub(crate) mod needs;
pub(crate) mod parse;
pub(crate) mod requests;
pub(crate) mod scan;
pub(crate) mod scanning;
pub(crate) mod scope;
pub(crate) mod views;

#[cfg(test)]
mod tests;
