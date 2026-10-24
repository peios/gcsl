//! The view of a file: the nodes of the merged tree it reads, their conflicts,
//! and the order of shadowing among them.
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
//! [`FileScope`](super::FileScope), whose numbering keeps the file's own nodes and functions
//! as [`crate::mentions::find`] numbers them and puts every other node
//! after them.
//!
//! One leaf path declared in two files of a directory, and a leaf under a
//! leaf or over an interior node of another file of it, are errors, found
//! while the directory's tree is merged ([`scan`](super::scan)), unless the earlier is
//! shadowable (see [Shadowable declarations](crate::workspace::parse#shadowable-declarations)):
//! the directory's tree has the winner of each leaf, which every file of
//! the directory sees.
//!
//! A `def` named by a dotted path, `def dev.peios.hello()`, is a leaf like
//! any dotted field's (see [`crate::mentions`], "Dotted `def`s"): it merges
//! under its prefixes with every other file's leaves, in its directory, in
//! every view and through every import, conflicts only as a leaf does, and
//! is shadowable as any `def`. Its merged node's cell is the one its file
//! stores its closure into, so it is read as any cell is, by its path,
//! from another file or through an import, a record of a prefix read whole
//! holding it among the other leaves. It has no name: it is none of a
//! directory's `def`s by name ([`FileScope`](super::FileScope)'s `function` never finds it),
//! so another file's reference to it mentions its node, which makes that
//! file need its own, and never its function; within its own file, a
//! reference to it, or to a node above it, mentions its function too
//! (the file's own tree, [`FileScope`](super::FileScope)'s `dotted_defs_under`). Once its
//! file is checked, the workspace carries which of its parameters its body
//! measures, as for a `def` of one name (from its function's resolution,
//! since it has no binding: `computed_cells`), so a call of it from another
//! file is of a known callee.
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
//! or a file's part of a node also reached whole): merging is idempotent;
//! and except that a leaf shadows what it overlaps, or is shadowed, when
//! the earlier of the two is shadowable (see [Shadowable
//! declarations](crate::workspace::parse#shadowable-declarations)). A whole import (`import PATH
//! [as NAME]`) is not merged: it is a name of its own (see
//! [Imports](crate::workspace::imports#imports)), and one named like a root of the view is still an
//! error, but for a root that is shadowable, which it shadows.
//!
//! Only what several sources have is stored ([`File::view`]): a node one
//! source alone has is that source's node, read as it always was, so a
//! view costs what its overlaps cost, the largest source of each level
//! only looked up in. A node several sources have is a [`ViewNode`] of
//! [`Scan::views`], its parts the sources' nodes, root directory first,
//! the order its children are in (by first definition, each part in its own
//! order); its children several parts have are view nodes too, and a child
//! one part has is that part's child. The file's [`FileScope`](super::FileScope) answers a
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

use std::mem;
use std::collections::{HashMap, HashSet};

use crate::ast::ItemId;
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;

use super::{
    File, Function, Merged, Node, ROOT, Scan, Target, View, ViewNode, ViewPart, context_in, count,
    first_leaf, leaf_spans, leaves_under,
};

/// Adds `file`'s own tree to its directory's merged tree, reporting the
/// conflicts with the files before it, and numbers its functions. The
/// declarations of each conflict of the file, its leaves at or under its
/// node that conflicts, are added to `dropped`, by file and position (see
/// [`merge_trees`](super::scanning::merge_trees)).
pub(super) fn merge(
    scan: &mut Scan,
    file: u32,
    errors: &mut Vec<Diagnostic>,
    dropped: &mut Vec<(u32, usize)>,
) {
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
                dropped.extend(leaves_under(entry, own).into_iter().map(|at| (file, at)));
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
                shadowable: false,
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
                target.shadowable = node.shadowable;
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
        // A `def` named by a dotted path is found by its path, through the
        // merged tree, never by a name.
        if is_def && let Some(name) = function.name {
            scan.defs.entry((dir, name)).or_insert(id);
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
pub(super) fn conflict(scan: &Scan, input: u32, own: u32, existing: u32) -> Option<Diagnostic> {
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
pub(super) enum Level {
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
pub(super) type Conflict = (u32, Name, Vec<ViewPart>);

/// A file's view as [`file_view`] finds it: its nodes, numbered from 0,
/// what the merge made of each name several sources have, the conflicts,
/// and the imports gone from it, by their indices among the file's
/// aliases.
pub(super) type FileView = (
    Vec<ViewNode>,
    HashMap<(u32, Name), Merged>,
    Vec<Conflict>,
    Vec<u32>,
);

/// What identifies one side of a conflict, so that a conflict seen from
/// several files' views is reported once: its node, and for an import the
/// importing file and where the import binds it.
pub(super) type Side = (Target, Option<(u32, u32)>);

impl Scan {
    /// The number of names `level` has.
    pub(super) fn level_size(&self, level: Level) -> usize {
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
    pub(super) fn level_entries(&self, level: Level) -> Vec<(Name, ViewPart)> {
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
    pub(super) fn level_lookup(&self, level: Level, name: Name) -> Option<ViewPart> {
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
    pub(super) fn overlaps(&self, levels: &[Level]) -> Vec<(Name, Vec<ViewPart>)> {
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
    /// of a merged node also met whole. Merging is idempotent. A cell
    /// reached both through a `#[shadowable]` import and plainly is the
    /// plain one, which stands (see the module documentation, "Shadowable
    /// declarations"): of two reachings of one node, the one kept is the
    /// first but for a marked one met again plainly, which the plain one
    /// replaces, its place in the order with it; and the node a file's part
    /// is met as whole is marked only if the part is too.
    pub(super) fn same_cells(&self, parts: &[ViewPart]) -> Vec<ViewPart> {
        let whole = |part: &ViewPart| match part.target {
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.global.get(node as usize))
                .copied(),
            _ => None,
        };
        let mut kept: Vec<ViewPart> = Vec::with_capacity(parts.len());
        for part in parts {
            if let Some(same) = kept.iter_mut().find(|k| k.target == part.target) {
                if same.marked && !part.marked {
                    *same = *part;
                }
                continue;
            }
            if let Some(merged) = whole(part)
                && parts
                    .iter()
                    .any(|other| other.target == Target::Node(merged))
            {
                continue;
            }
            kept.push(*part);
        }
        // A part met whole too leaves its plainness to the whole node.
        for part in parts {
            if !part.marked
                && let Some(merged) = whole(part)
                && let Some(node) = kept.iter_mut().find(|k| k.target == Target::Node(merged))
            {
                node.marked = false;
            }
        }
        kept
    }

    /// Whether `target` is shadowable as its declarations say: a merged
    /// node or a file's part of one all of whose leaves are marked
    /// `#[shadowable]` (see the module documentation, "Shadowable
    /// declarations").
    pub(super) fn target_shadowable(&self, target: Target) -> bool {
        match target {
            Target::Node(node) => self.nodes.get(node as usize).is_some_and(|n| n.shadowable),
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.mentions.tree.nodes.get(node as usize))
                .is_some_and(|n| n.shadowable),
            Target::View(view) => self
                .views
                .get(view as usize)
                .is_some_and(|v| v.parts.iter().all(|&part| self.part_shadowable(part))),
            Target::Dir(_) | Target::File(_) | Target::Fileroot(_) => false,
        }
    }

    /// Whether the part `part` of a node of a file's view is shadowable:
    /// reached through a `#[shadowable]` import, or shadowable as its
    /// declarations say.
    pub(super) fn part_shadowable(&self, part: ViewPart) -> bool {
        part.marked || self.target_shadowable(part.target)
    }

    /// The position among its file's items of the item declaring the leaf
    /// `node`, a merged node whose leaf is file `owner`'s.
    pub(super) fn leaf_position(&self, owner: u32, node: u32) -> u32 {
        self.files
            .get(owner as usize)
            .and_then(|file| {
                let own = file.global.iter().position(|&g| g == node)?;
                let leaf = file.mentions.tree.nodes.get(own)?.leaf?;
                Some(count(leaf.position))
            })
            .unwrap_or(0)
    }

    /// Whether `target` is a leaf: a merged node declared by an item, or a
    /// built-in function's.
    pub(super) fn is_leaf(&self, target: Target) -> bool {
        matches!(target, Target::Node(node)
            if self.nodes.get(node as usize).is_some_and(|n| n.leaf.is_some())
                || self.builtin(node).is_some())
    }

    /// The file that declares the first leaf at or under `target`, a merged
    /// node or a file's part of one, and the spans of that leaf's path and
    /// whole item.
    pub(super) fn part_spans(&self, target: Target) -> Option<(u32, Span, Span)> {
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
/// [`Scan::views`] and [`File::view`], each leaf shadowing what it overlaps
/// or shadowed by it as the static order says (see the module
/// documentation, "Shadowable declarations"), and the imports something
/// shadows taken from [`File::aliases`]. Reports each conflict, two
/// sources with a leaf and anything else of one path neither of which
/// shadows the other, once per pair however many files' views it is in.
pub(super) fn views(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    let mut reported: HashSet<(Side, Side)> = HashSet::new();
    let mut found = Vec::with_capacity(scan.files.len());
    // Each file's imports gone from its view, and the roots of its conflicts,
    // which it reads as `Error` (see [Scan
    // errors](crate::workspace::scanning#scan-errors)).
    let mut gone_from: Vec<(Vec<u32>, Vec<Name>)> = Vec::with_capacity(scan.files.len());
    for index in 0..count(scan.files.len()) {
        let (mut nodes, mut merged, conflicts, gone) = file_view(scan, index);
        let mut roots = Vec::new();
        for (parent, name, parts) in conflicts {
            report_conflicts(
                scan,
                index,
                &nodes,
                (parent, name, &parts),
                &mut reported,
                errors,
            );
            roots.push(conflict_root(&nodes, parent, name));
        }
        // The imports gone from the view are no aliases of the file: the
        // others are renumbered.
        if !gone.is_empty() {
            let depth = scan
                .files
                .get(index as usize)
                .map_or(0, |file| count(scan.chain(file.dir).count()));
            let renumber = |part: &mut ViewPart| {
                if !part.chain {
                    let at = part.source.saturating_sub(depth);
                    let before = count(gone.partition_point(|&g| g < at));
                    part.source = part.source.saturating_sub(before);
                }
            };
            for node in &mut nodes {
                node.parts.iter_mut().for_each(renumber);
            }
            for value in merged.values_mut() {
                if let Merged::One(part) = value {
                    renumber(part);
                }
            }
        }
        gone_from.push((gone, roots));
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
    for ((file, view), (gone, roots)) in scan.files.iter_mut().zip(found).zip(gone_from) {
        for root in roots {
            file.poison(root);
        }
        file.view = view;
        if !gone.is_empty() {
            let aliases = mem::take(&mut file.aliases);
            file.aliases = (0u32..)
                .zip(aliases)
                .filter(|(at, _)| gone.binary_search(at).is_err())
                .map(|(_, alias)| alias)
                .collect();
        }
    }
}

/// The root of the path of a conflict of a file's view at the name `name`
/// under `parent` ([`ROOT`] for a root, else one of `nodes`, the view's
/// nodes numbered from 0): what the file reads as `Error` (see [Scan
/// errors](crate::workspace::scanning#scan-errors)).
pub(super) fn conflict_root(nodes: &[ViewNode], parent: u32, name: Name) -> Name {
    let mut root = name;
    let mut at = (parent != ROOT).then_some(parent);
    while let Some(view) = at
        && let Some(node) = nodes.get(view as usize)
    {
        root = node.name;
        at = node.parent;
    }
    root
}

/// A place in the static order of the declarations and imports reaching a
/// file's view (see the module documentation, "Shadowable declarations"),
/// as a key: the implicit prelude's roots `(0, 0, 0)`; then a directory
/// above the file's own, `(1, its position in the chain from the root, 0)`;
/// then a declaration of the file's own directory, or an import of the
/// file, `(2, its file, its position among its file's items)`; then an
/// import a session's input inherits, `(3, its index, 0)`.
pub(super) type Rank = (u8, u32, u32);

/// The static order of the parts of one file's view (see the module
/// documentation, "Shadowable declarations").
pub(super) struct Order<'a> {
    /// The workspace.
    pub(super) scan: &'a Scan,
    /// The file.
    pub(super) file: &'a File,
    /// Its index.
    pub(super) index: u32,
    /// The number of directories of its chain.
    pub(super) depth: u32,
}

impl<'a> Order<'a> {
    /// The order of the view of `file`, file `index` of `scan`, whose chain
    /// has `depth` directories.
    pub(super) const fn new(scan: &'a Scan, (file, index): (&'a File, u32), depth: u32) -> Self {
        Self {
            scan,
            file,
            index,
            depth,
        }
    }

    /// The position among the file's items of its item `item`; found only
    /// where a leaf meets an import, which is rare.
    pub(super) fn position(&self, item: ItemId) -> u32 {
        let items = self.file.ast.items(self.file.ast.root());
        count(items.iter().position(|&at| at == item).unwrap_or(0))
    }

    /// The earliest and the latest place of the declarations of `part`: an
    /// import's place, a directory above's, or the places of the leaves at
    /// or under a node of the file's own directory, which may come before
    /// and after the file's imports.
    pub(super) fn ranks(&self, part: ViewPart) -> (Rank, Rank) {
        if !part.chain {
            let at = part.source.saturating_sub(self.depth);
            let rank = match self.file.aliases.get(at as usize) {
                Some(alias) if alias.implicit => (0, 0, 0),
                Some(alias) if !alias.inherited => {
                    let position = alias.item.map_or(0, |item| self.position(item));
                    (2, self.index, position)
                }
                _ => (3, at, 0),
            };
            return (rank, rank);
        }
        if part.source.saturating_add(1) < self.depth {
            let rank = (1, part.source, 0);
            return (rank, rank);
        }
        let Target::Node(node) = part.target else {
            return ((2, 0, 0), (2, 0, 0));
        };
        let (mut earliest, mut latest): (Option<Rank>, Option<Rank>) = (None, None);
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            let Some(entry) = self.scan.nodes.get(at as usize) else {
                continue;
            };
            if let Some(owner) = entry.leaf {
                let rank = (2, owner, self.scan.leaf_position(owner, at));
                earliest = Some(earliest.map_or(rank, |known| known.min(rank)));
                latest = Some(latest.map_or(rank, |known| known.max(rank)));
            }
            stack.extend(&entry.children);
        }
        (earliest.unwrap_or((2, 0, 0)), latest.unwrap_or((2, 0, 0)))
    }
}

/// Of `parts`, the nodes several sources have at one name of a file's
/// view, at least one of them a leaf: those that stand once each, in the
/// static order `order`, has shadowed every earlier one it overlaps (a
/// leaf overlaps every other part; two interior nodes do not, at this
/// name), which it may only if every one of those is shadowable (see the
/// module documentation, "Shadowable declarations"), in source order, with
/// those shadowed; or, when one overlaps one that is not, the parts that
/// conflict, as they always did, those shadowed before it left out. An
/// interior node of the file's own directory with leaves both before and
/// after a leaf of another source, an import of the file, would stand only
/// in part, which a view cannot hold: that is the conflict, conservatively.
pub(super) fn shadow_parts(
    scan: &Scan,
    order: &Order,
    parts: &[ViewPart],
) -> Result<(Vec<ViewPart>, Vec<ViewPart>), Vec<ViewPart>> {
    let mut ranked: Vec<(Rank, Rank, ViewPart)> = parts
        .iter()
        .map(|&part| {
            let (earliest, latest) = order.ranks(part);
            (earliest, latest, part)
        })
        .collect();
    ranked.sort_by_key(|&(earliest, _, part)| (earliest, part.source));
    let interleaved = ranked.iter().any(|&(at, _, leaf)| {
        scan.is_leaf(leaf.target)
            && ranked.iter().any(|&(earliest, latest, other)| {
                !scan.is_leaf(other.target) && earliest < at && at < latest
            })
    });
    if interleaved {
        // The implicit prelude's, first of all, is shadowed whatever the
        // rest does.
        return Err(ranked
            .iter()
            .filter(|&&(earliest, _, _)| earliest.0 != 0)
            .map(|&(_, _, part)| part)
            .collect());
    }
    let mut live: Vec<ViewPart> = Vec::with_capacity(parts.len());
    let mut gone: Vec<ViewPart> = Vec::new();
    for (_, _, part) in ranked {
        let overlaps =
            scan.is_leaf(part.target) || live.iter().any(|stands| scan.is_leaf(stands.target));
        if live.is_empty() || !overlaps {
            live.push(part);
        } else if live.iter().all(|&stands| scan.part_shadowable(stands)) {
            gone.append(&mut live);
            live.push(part);
        } else {
            return Err(parts
                .iter()
                .filter(|part| !gone.contains(part))
                .copied()
                .collect());
        }
    }
    live.sort_by_key(|part| part.source);
    Ok((live, gone))
}

/// Lets each whole import of `file` (`import PATH [as NAME]`), whose chain
/// has `depth` directories, take its name in the view, `merged` (by
/// parent, [`ROOT`] for a root, and name), from whatever the view has of
/// it there, if that is shadowable (see the module documentation,
/// "Shadowable declarations"): a root of the directory chain, a member or
/// `*` import, or a node of the view, `nodes` being the view's so far. One
/// of a name that is not is the clash [`clashes`](super::clashes) reports. Each member or
/// `*` import it shadows is added to `gone`, by its index among the file's
/// aliases.
pub(super) fn whole_imports_shadow(
    scan: &Scan,
    (file, depth): (&File, u32),
    (nodes, merged): (&[ViewNode], &mut HashMap<(u32, Name), Merged>),
    gone: &mut Vec<u32>,
) {
    for (at, alias) in (0u32..).zip(&file.aliases) {
        if !alias.whole {
            continue;
        }
        let shadowable = match merged.get(&(ROOT, alias.name)) {
            Some(&Merged::One(part)) => Some(scan.part_shadowable(part)),
            Some(&Merged::View(view)) => nodes
                .get(view as usize)
                .map(|node| node.parts.iter().all(|&part| scan.part_shadowable(part))),
            None => scan
                .chain(file.dir)
                .find_map(|dir| scan.by_name.get(&(dir, ROOT, alias.name)))
                .map(|&node| scan.target_shadowable(Target::Node(node)))
                .or_else(|| {
                    file.aliases
                        .iter()
                        .find(|other| !other.whole && other.name == alias.name)
                        .map(|other| file.marked(other) || scan.target_shadowable(other.target))
                }),
        };
        if shadowable == Some(true) {
            let part = ViewPart {
                source: depth.saturating_add(at),
                target: alias.target,
                chain: false,
                marked: false,
            };
            merged.insert((ROOT, alias.name), Merged::One(part));
            gone.extend(
                (0u32..)
                    .zip(&file.aliases)
                    .filter(|(_, other)| !other.whole && other.name == alias.name)
                    .map(|(other, _)| other),
            );
        }
    }
}

/// The view of file `index`: its view nodes, numbered from 0 in the order
/// found, each a parent before its children; what the merge made of each
/// name several sources have, by parent (a local number or [`ROOT`]); the
/// conflicts; and the member and `*` imports of the file that something
/// shadows, gone from the view, by their indices among its aliases.
pub(super) fn file_view(scan: &Scan, index: u32) -> FileView {
    let (mut nodes, mut merged, mut conflicts) = (Vec::new(), HashMap::new(), Vec::new());
    let mut gone: Vec<u32> = Vec::new();
    let Some(file) = scan.files.get(index as usize) else {
        return (nodes, merged, conflicts, gone);
    };
    let mut chain: Vec<u32> = scan.chain(file.dir).collect();
    chain.reverse();
    let depth = count(chain.len());
    let order = Order::new(scan, (file, index), depth);
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
                marked: file.marked(alias),
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
        for (name, all) in scan.overlaps(&levels) {
            let parts = scan.same_cells(&all);
            // A leaf among them shadows, or is shadowed by, what it
            // overlaps, or conflicts with it.
            let (parts, shadowed) =
                if parts.len() > 1 && parts.iter().any(|part| scan.is_leaf(part.target)) {
                    match shadow_parts(scan, &order, &parts) {
                        Ok(stands) => stands,
                        Err(conflicting) => {
                            conflicts.push((parent, name, conflicting));
                            continue;
                        }
                    }
                } else {
                    (parts, Vec::new())
                };
            // The imports gone from the view at a root: those shadowed, and
            // a marked one whose cell a plain one reaches too.
            if parent == ROOT {
                let replaced = all.iter().filter(|part| {
                    part.marked
                        && parts
                            .iter()
                            .any(|kept| kept.target == part.target && kept.source != part.source)
                });
                gone.extend(
                    shadowed
                        .iter()
                        .chain(replaced)
                        .filter(|part| !part.chain)
                        .map(|part| part.source.saturating_sub(depth)),
                );
            }
            if let [one] = parts.as_slice() {
                merged.insert((parent, name), Merged::One(*one));
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
    whole_imports_shadow(scan, (file, depth), (&nodes, &mut merged), &mut gone);
    gone.sort_unstable();
    gone.dedup();
    (nodes, merged, conflicts, gone)
}

/// Reports the conflict `conflict` of file `index`'s view, whose view
/// nodes so far are `nodes`: each part, in the order the sources are
/// reported in (its own directory, then each directory upward, then its
/// imports in source order), against the first before it that it conflicts
/// with, once per pair (`reported`), at the later of the two.
pub(super) fn report_conflicts(
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
pub(super) fn conflict_between(
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
