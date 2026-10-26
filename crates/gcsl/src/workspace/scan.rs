//! The scan of a workspace: its files, directories and merged trees, and what
//! each file sees, before anything is checked.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{ItemId, ItemKind, PathSegment, Syntax, TypeStore};
use crate::check::{Builtin, Fileroot, Place, Slot};
use crate::diagnostic::{Diagnostic, Severity};
use crate::names::{self, Name, Names};

use super::parse::is_foreign_path;
use super::{
    DeclaredContext, DeclaredType, DeclaredVariant, Dir, File, Function, InheritedContext,
    InheritedDefType, InheritedVariant, Merged, Node, ROOT, STANDARD_DIR, Target, ViewNode,
    ViewPart, count, in_file, merge_files, target_roots,
};

/// The merged nodes seen through a leaf below them that a scan's imports
/// bind or reach ([`Target::Through`]), each a node and its leaf.
#[derive(Debug, Default)]
pub(super) struct Throughs {
    /// Each, by its number among them.
    pub(super) list: Vec<(u32, u32)>,
    /// The number of each.
    pub(super) numbers: HashMap<(u32, u32), u32>,
}

/// The scanned workspace: what [`scan`](super::scan) finds before anything is checked.
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
    /// Each merged node seen through one leaf below it that an import binds,
    /// or that is on the way down from one to its leaf ([`Target::Through`]),
    /// by its number among them, and the numbers by node and leaf.
    pub(super) throughs: Throughs,
    /// For each file, the files it needs, each once, in index order.
    pub needs: Vec<Vec<u32>>,
    /// The files in the order they are checked and run.
    pub order: Vec<u32>,
    /// The names of every file.
    pub names: Names,
    /// The nodes by directory, parent ([`ROOT`] for a root) and name.
    pub(super) by_name: HashMap<(u32, u32, Name), u32>,
    /// The roots of each directory's tree, by directory, in order.
    pub(super) roots: Vec<Vec<u32>>,
    /// The `def`s by directory and name, as indices in `functions`.
    pub(super) defs: HashMap<(u32, Name), u32>,
    /// The position of each file in `order`.
    pub(super) rank: Vec<usize>,
    /// For each file, the number of the own nodes of the files before it:
    /// where its [`Target::Part`]s start among the targets' numbers.
    pub(super) part_base: Vec<u32>,
    /// The value files some file imports, each with the slot of its value's
    /// cell and its name (the file's stem), in path order. Their slots
    /// follow the merged nodes'.
    pub(super) values: Vec<(u32, Name)>,
    /// The number of standard files the scan loaded, which are files
    /// `0..standard`, each in a directory of its own (see the module
    /// documentation, "The standard library").
    pub(super) standard: u32,
    /// The directory of the `builtins` module, whose nodes are the first
    /// [`Builtin::ALL`]`.len()` of `nodes`.
    pub(crate) builtins_dir: u32,
    /// The name of each member of the `fileroot` module, in the order of
    /// [`Fileroot::ALL`], if some file spells it: looked up, never interned,
    /// once the files are parsed, since the names are lent to each file as
    /// it is checked, while its references are resolved.
    pub(super) fileroot_names: [Option<Name>; 4],
    /// Whether the files given are one file outside any workspace (a lone
    /// file, a session's input), whose imports of anything but `builtins`
    /// and the standard files the checker reports.
    pub(super) lone: bool,
    /// For a session's input ([`check_input`](super::check_input)): the number of cells of the
    /// standard library, which keep their slots, and the slot of each cell
    /// after them, past the slots of the session's earlier inputs (see
    /// [`Scan::slot`]).
    pub(super) relocation: Option<(u32, Vec<Slot>)>,
    /// The run's table of directories, the root's first ([`Scan::locate`]),
    /// by which the code of each file names its directory: what
    /// `fileroot.caller` reads (see [`crate::eval`], "Callers and
    /// helpers"). Empty until the scan is located.
    pub(super) dir_texts: Vec<Rc<str>>,
    /// Each file's directory, by its index in `dir_texts`, by input number.
    pub(super) file_dirs: Vec<u32>,
    /// The `type` declarations of every file, in path order, each file's in
    /// source order: but one named like a built-in type, and a second of one
    /// name in one file, which the checker reports (see
    /// [Types](crate::workspace::declared#types)).
    pub types: Vec<DeclaredType>,
    /// The variants of enums every file declares, in path order, each
    /// file's in source order (see [Enums](crate::workspace::declared#enums)).
    pub variants: Vec<DeclaredVariant>,
    /// The variants a session's input inherits from the earlier inputs, each
    /// by its enum's key and its name, with its own key (see
    /// [Enums](crate::workspace::declared#enums)).
    pub(super) inherited_variants: Vec<InheritedVariant>,
    /// The contexts every file declares, in path order, each file's in
    /// source order (see [Context](crate::workspace::declared#context)).
    pub contexts: Vec<DeclaredContext>,
    /// The contexts a session's input inherits from the earlier inputs (see
    /// [Context](crate::workspace::declared#context)).
    pub(super) inherited_contexts: Vec<InheritedContext>,
    /// The types of each directory, by its index: its files' declarations,
    /// as indices in `types`, in order, the first of each name.
    pub(super) dir_types: Vec<Vec<u32>>,
    /// The definitions of the types: each of `types` is the declaration
    /// numbered `type_base` more than its index. A session's input's scan
    /// has the session's, its earlier inputs' declarations first.
    pub type_store: TypeStore,
    /// The number in `type_store` of the first of `types`.
    pub(crate) type_base: u32,
    /// The types a session's input inherits from the earlier inputs, each
    /// with its declaration's number in `type_store`: the last source of its
    /// view, shadowed by anything (see [`crate::session`]).
    pub(super) inherited_types: Vec<(Name, u32)>,
    /// The def-types a session's input inherits from the earlier inputs,
    /// each by its path, with its namespace's key: what a `DT` names that
    /// nothing of the input does, or only a shadowable import (see
    /// [Def-types](crate::workspace::declared#def-types)); and each name an
    /// earlier input bound to something else, with none.
    pub(super) inherited_def_types: Vec<InheritedDefType>,
    /// For a session's input, its number among the session's inputs, which
    /// the keys of the def-types it declares are made with, in place of its
    /// index among the scan's files, which every input shares.
    pub(super) input_number: Option<u32>,
    /// The lexical and syntax errors of every file, each naming its file,
    /// in path then source order (see [Syntax errors](crate::workspace::parse#syntax-errors)): a
    /// file with some is scanned as its parser recovered it.
    pub(super) syntax_errors: Vec<Diagnostic>,
    /// The errors of the scan itself, each naming its file, which it recovered
    /// from (see [Scan errors](crate::workspace::scanning#scan-errors)): but
    /// those of a file that may see a file with syntax errors.
    pub(super) recovered: Vec<Diagnostic>,
    /// The declarations dropped as later definitions of a field defined
    /// first elsewhere, each by its file and its position among the file's
    /// items (see [Scan errors](crate::workspace::scanning#scan-errors)).
    pub(super) conflict_items: Vec<(u32, usize)>,
    /// The CONFLICTED merged nodes, each with the declarations dropped as
    /// later definitions of it, by file and item, in path then source order:
    /// a node at which a dropped declaration's path meets the tree (see
    /// [Scan errors](crate::workspace::scanning#scan-errors)).
    pub(super) conflicted: HashMap<u32, Vec<(u32, ItemId)>>,
    /// The conflicted nodes and every node above one, whose cells are read
    /// as `Error` (see [Scan errors](crate::workspace::scanning#scan-errors)).
    pub(super) poisoned_nodes: HashSet<u32>,
    /// The `type` declarations, by their indices in `types`, that a declaration
    /// of their name in another file of their directory conflicts with: each is
    /// erroneous (see [Scan errors](crate::workspace::scanning#scan-errors)).
    pub(super) poisoned_types: Vec<u32>,
    /// Whether each file is in a cycle of files that need each other, or
    /// needs itself: such a file is not checked (see [Scan
    /// errors](crate::workspace::scanning#scan-errors)).
    pub(super) cyclic: Vec<bool>,
}

impl Scan {
    /// The directories below `dir`: those directly in it, or, if `deep`,
    /// those at any depth below it, in path order, a hidden one, whose name
    /// starts with `.`, and everything below it left out (see [Imports of
    /// definitions](crate::workspace::imports#imports-of-definitions)).
    #[must_use]
    pub fn dirs_below(&self, dir: u32, deep: bool) -> Vec<u32> {
        let hidden = |at: &Dir| {
            at.path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with('.'))
        };
        let mut found: Vec<u32> = (0u32..)
            .zip(&self.dirs)
            .filter(|(_, below)| {
                if hidden(below) {
                    return false;
                }
                let mut at = below.parent;
                while let Some(parent) = at {
                    if parent == dir {
                        return true;
                    }
                    let Some(above) = self.dirs.get(parent as usize) else {
                        return false;
                    };
                    if !deep || hidden(above) {
                        return false;
                    }
                    at = above.parent;
                }
                false
            })
            .map(|(index, _)| index)
            .collect();
        found.sort_by(|&a, &b| {
            let path = |at: u32| self.dirs.get(at as usize).map_or("", |d| d.path.as_str());
            path(a).cmp(path(b))
        });
        found
    }

    /// The lexical and syntax errors of every file, each naming its file by
    /// its input number, in path then source order (see [Syntax
    /// errors](crate::workspace::parse#syntax-errors)).
    #[must_use]
    pub fn syntax_errors(&self) -> &[Diagnostic] {
        &self.syntax_errors
    }

    /// The errors of the scan itself, which it recovered from, each naming
    /// its file by its input number, in path then source order (see [Scan
    /// errors](crate::workspace::scanning#scan-errors)).
    #[must_use]
    pub fn scan_errors(&self) -> &[Diagnostic] {
        &self.recovered
    }

    /// The syntax errors and the errors of the scan, in path then source
    /// order: what a scan with either reports, and why nothing of it runs.
    pub(super) fn errors(&self) -> Vec<Diagnostic> {
        merge_files(
            self.syntax_errors
                .iter()
                .chain(&self.recovered)
                .cloned()
                .collect(),
            Vec::new(),
        )
    }

    /// Whether the merged node `node` is conflicted, or above a node that is,
    /// so that it is read as `Error` (see [Scan
    /// errors](crate::workspace::scanning#scan-errors)).
    #[must_use]
    pub fn is_poisoned(&self, node: u32) -> bool {
        !self.poisoned_nodes.is_empty() && self.poisoned_nodes.contains(&node)
    }

    /// The declarations the scan dropped as later definitions of the merged
    /// node `node`, each by its file and item, if it is conflicted (see
    /// [Scan errors](crate::workspace::scanning#scan-errors)).
    #[must_use]
    pub fn conflicting(&self, node: u32) -> &[(u32, ItemId)] {
        self.conflicted.get(&node).map_or(&[], Vec::as_slice)
    }

    /// Whether file `index` is in a cycle of files that need each other, or
    /// needs itself: it is not checked (see [Scan errors](crate::workspace::scanning#scan-errors)).
    #[must_use]
    pub fn is_cyclic(&self, index: u32) -> bool {
        self.cyclic.get(index as usize) == Some(&true)
    }

    /// Whether file `index` has syntax errors, which the scan and the
    /// checker take the tree its parser recovered for.
    #[must_use]
    pub fn is_broken(&self, index: u32) -> bool {
        self.files
            .get(index as usize)
            .is_some_and(|file| file.ast.syntax() != Syntax::Clean)
    }

    /// Whether each file has syntax errors or is in a cycle, by input
    /// number: such a file always fails, and is not checked if in a cycle.
    pub(super) fn broken_files(&self) -> Vec<bool> {
        (0..count(self.files.len()))
            .map(|index| self.is_broken(index) || self.is_cyclic(index))
            .collect()
    }

    /// The diagnostics `found` of the failed check of file `index`, as they
    /// are reported, each naming the file: the errors alone of a file with
    /// syntax errors, a warning of which may be of source that could not be
    /// parsed.
    pub(super) fn failure(&self, index: u32, mut found: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.is_broken(index) {
            found.retain(|diagnostic| diagnostic.severity == Severity::Error);
        }
        in_file(found, index)
    }

    /// Whether file `index` is not checked: its tree is unbalanced, or it is
    /// in a cycle (see [Scan errors](crate::workspace::scanning#scan-errors)).
    pub(super) fn unchecked(&self, index: u32) -> bool {
        self.is_unbalanced(index) || self.is_cyclic(index)
    }

    /// Whether file `index`'s tree is [`Syntax::Unbalanced`], its braces not
    /// matching, which is not checked.
    pub(super) fn is_unbalanced(&self, index: u32) -> bool {
        self.files
            .get(index as usize)
            .is_some_and(|file| file.ast.syntax() == Syntax::Unbalanced)
    }

    /// Whether the view of file `index` sees a file with syntax errors, in
    /// which a declaration may be missing or misplaced (see [Syntax
    /// errors](crate::workspace::parse#syntax-errors)): it has them, a file of its directory or of
    /// one above it has, or it imports a name, a type, a file or a
    /// directory of a file that has. A name it reads that names nothing is
    /// no error then; nor is it in a file that imports names the scan
    /// cannot know ([`File::blind`]).
    #[must_use]
    pub fn sees_broken(&self, index: u32) -> bool {
        if self.syntax_errors.is_empty() && self.recovered.is_empty() {
            return false;
        }
        let Some(file) = self.files.get(index as usize) else {
            return false;
        };
        file.blind || self.sees_syntax_errors(index)
    }

    /// Whether the view of file `index` sees a file with syntax errors (see
    /// [`Scan::sees_broken`]), or an import of it that failed names a module
    /// with one, whose member it lacks may be declared in source that could
    /// not be parsed: the errors of the scan of such a file are not reported
    /// (see [Syntax errors](crate::workspace::parse#syntax-errors)).
    pub(super) fn sees_syntax_errors(&self, index: u32) -> bool {
        if self.syntax_errors.is_empty() {
            return false;
        }
        let Some(file) = self.files.get(index as usize) else {
            return false;
        };
        let chain: Vec<u32> = self.chain(file.dir).collect();
        let in_chain = (0u32..)
            .zip(&self.files)
            .any(|(at, other)| chain.contains(&other.dir) && self.is_broken(at));
        // A type by its index in `types`; one of a session's earlier input
        // is none of them.
        let broken_type = |at: u32| {
            self.types
                .get(at as usize)
                .is_some_and(|declared| self.is_broken(declared.file))
        };
        let broken_target = |target: Target| {
            self.target_files(target)
                .into_iter()
                .any(|at| self.is_broken(at))
        };
        in_chain
            || file.aliases.iter().any(|alias| broken_target(alias.target))
            || file
                .failed_imports
                .iter()
                .any(|&(_, target)| target.is_some_and(broken_target))
            || file
                .type_imports
                .iter()
                .any(|import| broken_type(import.decl))
            || file
                .type_view
                .values()
                .any(|&decl| decl.checked_sub(self.type_base).is_some_and(broken_type))
    }

    /// Of `errors`, the errors the stages of the scan found, those it
    /// reports: those of files that see no file with syntax errors
    /// ([`Scan::sees_syntax_errors`]), since in any other a declaration may
    /// be missing or misplaced (see [Syntax errors](crate::workspace::parse#syntax-errors)).
    pub(super) fn stage_errors(&self, errors: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.syntax_errors.is_empty() {
            return errors;
        }
        errors
            .into_iter()
            .filter(|error| {
                error
                    .input
                    .is_some_and(|input| !self.sees_syntax_errors(input))
            })
            .collect()
    }
    /// The directories from `dir` up to the root, `dir` first.
    pub fn chain(&self, dir: u32) -> impl Iterator<Item = u32> + '_ {
        std::iter::successors(Some(dir), |&at| {
            self.dirs.get(at as usize).and_then(|d| d.parent)
        })
    }

    /// The files `file` needs, transitively, with `file` and the standard
    /// files the scan loaded, which every run runs (the prelude and those
    /// some scanned file imports, see the module documentation, "The
    /// standard library"), in the order: the files to run to run `file`.
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
    pub(super) fn ranks(&self) -> Vec<usize> {
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
    /// merged nodes, then the views' nodes, then the `fileroot` module and
    /// its members, in the order of [`Fileroot::ALL`], then the merged
    /// nodes seen through a leaf, in the order they were registered
    /// ([`Scan::register_through`]).
    pub(super) fn target_number(&self, target: Target) -> u32 {
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
            Target::Fileroot(member) => self
                .fileroot_number()
                .saturating_add(member.map_or(0, |member| member.index() + 1)),
            Target::Through { node, leaf } => self.through_base().saturating_add(
                self.throughs
                    .numbers
                    .get(&(node, leaf))
                    .copied()
                    .unwrap_or(u32::MAX),
            ),
        }
    }

    /// The number of the first merged node seen through a leaf among every
    /// target, after the `fileroot` module's members (see
    /// [`Scan::target_number`]).
    fn through_base(&self) -> u32 {
        self.fileroot_number()
            .saturating_add(1)
            .saturating_add(count(Fileroot::ALL.len()))
    }

    /// Registers `node` seen through `leaf`, a leaf below it, and each node
    /// on the way down from it to `leaf` so seen, for [`Scan::target_number`]
    /// to number: what an import of one definition binds and reaches
    /// ([`Target::Through`]).
    pub(super) fn register_through(&mut self, node: u32, leaf: u32) {
        let mut at = node;
        while at != leaf {
            let Throughs { list, numbers } = &mut self.throughs;
            numbers.entry((at, leaf)).or_insert_with(|| {
                list.push((at, leaf));
                count(list.len() - 1)
            });
            let Some(next) = self.through_child(at, leaf) else {
                return;
            };
            at = next;
        }
    }

    /// The child of `node` on the way down to `leaf`, a node below it:
    /// `leaf` itself, or the node above it that is `node`'s child. `None`
    /// if `leaf` is not below `node`.
    pub(super) fn through_child(&self, node: u32, leaf: u32) -> Option<u32> {
        let mut at = leaf;
        loop {
            let parent = self.nodes.get(at as usize)?.parent?;
            if parent == node {
                return Some(at);
            }
            at = parent;
        }
    }

    /// The number of the `fileroot` module among every target, after the
    /// views' nodes; its members' follow it (see [`Scan::target_number`]).
    pub(super) fn fileroot_number(&self) -> u32 {
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        dirs.saturating_add(files)
            .saturating_add(self.parts())
            .saturating_add(count(self.nodes.len()))
            .saturating_add(count(self.views.len()))
    }

    /// The target of number `number` (see [`Scan::target_number`]).
    pub(super) fn target_of(&self, number: u32) -> Option<Target> {
        if let Some(at) = number.checked_sub(self.through_base()) {
            let &(node, leaf) = self.throughs.list.get(at as usize)?;
            return Some(Target::Through { node, leaf });
        }
        if let Some(at) = number.checked_sub(self.fileroot_number()) {
            return match at.checked_sub(1) {
                None => Some(Target::Fileroot(None)),
                Some(member) => Fileroot::ALL
                    .get(member as usize)
                    .map(|&member| Target::Fileroot(Some(member))),
            };
        }
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
    pub(super) fn is_def(&self, node: u32) -> bool {
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

    /// The number of standard files the scan loaded: files `0..` this are
    /// the standard library's (see the module documentation, "The standard
    /// library").
    #[must_use]
    pub fn standard(&self) -> u32 {
        self.standard
    }

    /// Whether the scan is of one file outside any workspace, a lone file
    /// or a session's input, which imports nothing but the standard
    /// library's modules.
    pub(crate) fn is_lone(&self) -> bool {
        self.lone
    }

    /// The standard file whose stem is `stem`, if the scan loaded one.
    pub(super) fn standard_file(&self, stem: &str) -> Option<u32> {
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

    /// Locates the files at `root`, the absolute path of the workspace's
    /// root directory, canonical and without a trailing `/` (but `/`
    /// itself), for the `fileroot` module (see the module documentation,
    /// "The standard library"): builds the run's table of directories,
    /// [`Scan::dir_texts`], and each file's index in it. The root is first,
    /// always, so that its index is 0 in every run, as a session's inputs,
    /// each in its root, need; then each other directory with a file, once,
    /// in the order of [`Scan::dirs`], each by its absolute path, its path
    /// relative to the root joined to `root` with a `/` (no other after a
    /// `root` of `/`); the standard files' `<stdlib>` once, as its own
    /// path, though each standard file is a directory of its own.
    pub(super) fn locate(&mut self, root: &str) {
        let mut texts: Vec<Rc<str>> = vec![Rc::from(root)];
        let mut found: HashMap<Rc<str>, u32> = HashMap::new();
        found.insert(Rc::from(root), 0);
        // The index of each directory of the scan, once it has one; the
        // root, dir 0 (`place_dirs` interns it first), has the first.
        let mut indices: Vec<Option<u32>> = vec![None; self.dirs.len()];
        if let Some(entry) = indices.first_mut() {
            *entry = Some(0);
        }
        let mut file_dirs = Vec::with_capacity(self.files.len());
        for file in &self.files {
            if let Some(&Some(at)) = indices.get(file.dir as usize) {
                file_dirs.push(at);
                continue;
            }
            let path = self
                .dirs
                .get(file.dir as usize)
                .map_or("", |d| d.path.as_str());
            let text: Rc<str> = if path.is_empty() {
                Rc::from(root)
            } else if path == STANDARD_DIR {
                Rc::from(path)
            } else if root.ends_with('/') {
                Rc::from(format!("{root}{path}"))
            } else {
                Rc::from(format!("{root}/{path}"))
            };
            let at = *found.entry(Rc::clone(&text)).or_insert_with(|| {
                texts.push(text);
                count(texts.len() - 1)
            });
            if let Some(entry) = indices.get_mut(file.dir as usize) {
                *entry = Some(at);
            }
            file_dirs.push(at);
        }
        self.dir_texts = texts;
        self.file_dirs = file_dirs;
    }

    /// Where file `index` is, as [`Scan::locate`] found it: its directory's
    /// index in the run's table, which is its `fileroot.this`, and the
    /// root's, its `fileroot.workspace`. Directory 0 and empty texts for a
    /// scan not located, which no run has.
    pub(super) fn place(&self, index: u32) -> Place {
        let dir = self.file_dirs.get(index as usize).copied().unwrap_or(0);
        let text = |at: u32| {
            self.dir_texts
                .get(at as usize)
                .map_or_else(|| Rc::from(""), Rc::clone)
        };
        Place {
            dir,
            this: text(dir),
            workspace: text(0),
            foreign: self
                .files
                .get(index as usize)
                .is_some_and(|file| is_foreign_path(&file.path)),
        }
    }

    /// The number of the cell of the value of `file`, a value file some
    /// file imports, among the workspace's cells ([`Scan::cells`]): after
    /// the merged nodes. Like a merged node's, its slot is [`Slot::at`] its
    /// number. `None` for any other file.
    pub(crate) fn value_cell(&self, file: u32) -> Option<u32> {
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
    pub(super) fn cell_names(&self) -> Vec<Name> {
        self.nodes
            .iter()
            .map(|node| node.name)
            .chain(self.values.iter().map(|&(_, name)| name))
            .collect()
    }

    /// The number of every file's own nodes.
    pub(super) fn parts(&self) -> u32 {
        self.part_base
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_add(self.files.last().map_or(0, |file| count(file.global.len())))
    }

    /// The child of the target `target` named `name`, as a reference
    /// through an import means it.
    pub(crate) fn target_child(&self, target: Target, name: Name) -> Option<Target> {
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
            Target::Fileroot(None) => Fileroot::ALL
                .into_iter()
                .zip(self.fileroot_names)
                .find(|&(_, named)| named == Some(name))
                .map(|(member, _)| Target::Fileroot(Some(member))),
            Target::Fileroot(Some(_)) => None,
            Target::Through { node, leaf } => {
                let child = self.through_child(node, leaf)?;
                (self.nodes.get(child as usize)?.name == name).then_some(if child == leaf {
                    Target::Node(leaf)
                } else {
                    Target::Through { node: child, leaf }
                })
            }
        }
    }

    /// The child named `name` of the view node `view`: a view node of its
    /// own when several of its parts have one, else the one part's child.
    pub(super) fn view_child(&self, view: u32, name: Name) -> Option<Merged> {
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
    pub(crate) fn view_names(&self, view: u32) -> Vec<Name> {
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
    pub(crate) fn child_names(&self, target: Target) -> Vec<Name> {
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
            Target::Through { node, leaf } => self
                .through_child(node, leaf)
                .and_then(|child| self.nodes.get(child as usize))
                .map(|child| vec![child.name])
                .unwrap_or_default(),
            Target::Dir(_) | Target::File(_) | Target::Fileroot(_) => target_roots(self, target),
        }
    }

    /// The files with a leaf at or under the part `part` of a view node.
    pub(super) fn part_owners(&self, part: ViewPart) -> Vec<u32> {
        self.target_files(part.target)
    }

    /// The merged node `merged` as a node of `file`'s own tree: the merged
    /// node when it is wholly `file`'s, else `file`'s part of it; `None`
    /// when `file` has nothing at it.
    pub(super) fn in_file(&self, file: u32, merged: u32) -> Option<Target> {
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
    /// leaf's file or its contributors, the file of the leaf a node is seen
    /// through, every owner of every part of a view node; none for the
    /// `fileroot` module, which no file computes.
    #[must_use]
    pub fn target_files(&self, target: Target) -> Vec<u32> {
        match target {
            Target::Fileroot(_) => Vec::new(),
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
            // Its leaf's file alone: nothing else below it is seen.
            Target::Through { leaf, .. } => self.target_files(Target::Node(leaf)),
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
    /// module path, a node's dotted path, `fileroot` or `fileroot.caller`.
    pub(super) fn show_target(&self, target: Target) -> String {
        let module = |path: &str| path.trim_end_matches(".gcsl").replace('/', ".");
        match target {
            Target::Fileroot(None) => Fileroot::MODULE.to_owned(),
            Target::Fileroot(Some(member)) => format!("{}.{}", Fileroot::MODULE, member.name()),
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
            Target::Node(node) | Target::Through { node, .. } => self.show(node),
            Target::View(view) => self.show_view(view),
        }
    }

    /// The name of the top-level function `function`, or its path if it is
    /// a `def` named by a dotted path.
    pub(super) fn show_function(&self, function: Function) -> String {
        let Some(file) = self.files.get(function.file as usize) else {
            return String::new();
        };
        let Some(top) = file.mentions.functions.get(function.index as usize) else {
            return String::new();
        };
        match (top.name, file.ast.item(top.item).kind) {
            (Some(name), _) => self.names.get(name).to_owned(),
            (None, ItemKind::Field { path, .. }) => {
                let path: Vec<Name> = file
                    .ast
                    .path(path)
                    .iter()
                    .filter_map(PathSegment::name)
                    .collect();
                names::show_path(&self.names, &path)
            }
            // A definition made with a def-type, as it is written.
            (None, ItemKind::DefTypeDef { def, .. }) => {
                let def = file.ast.def_type(def);
                let path: Vec<Name> = def
                    .key
                    .into_iter()
                    .chain(file.ast.path(def.path).iter().filter_map(PathSegment::name))
                    .collect();
                names::show_path(&self.names, &path)
            }
            // A definition's meta, as `defmeta` and its def-type and path.
            (None, ItemKind::DefMeta { def, .. }) => {
                let path: Vec<Name> = file
                    .ast
                    .def_type(def)
                    .meta
                    .map(|(key, leaf)| vec![key, leaf])
                    .unwrap_or_default();
                names::show_path(&self.names, &path)
            }
            (None, _) => String::new(),
        }
    }

    /// The dotted path in its file's view of the view node `view`.
    pub(super) fn show_view(&self, view: u32) -> String {
        let mut path = Vec::new();
        let mut at = Some(view);
        while let Some(index) = at
            && let Some(entry) = self.views.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        names::show_path(&self.names, &path)
    }

    /// The dotted path of the merged node `node`, a definition's as it is
    /// written (see [`crate::names`], "Namespace keys").
    pub(super) fn show(&self, node: u32) -> String {
        let mut path = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.nodes.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        names::show_path(&self.names, &path)
    }
}
