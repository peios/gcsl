//! Which top-level functions each top-level item of an input mentions, and
//! the groups of mutually recursive top-level functions.
//!
//! A *top-level function* of an input is a named function item (`fn
//! NAME`), a `def` (`def NAME`, or `def PATH` named by a dotted path) or a
//! definition made with a def-type (`def DT PATH`, see
//! [Def-types](#def-types)) that is directly an item of the input's
//! outermost block. Its name, or its
//! path, is in scope in the whole input, so the checker must know, before
//! it checks anything, which functions each top-level item and each
//! function body may use: that is what decides the order the bodies are
//! checked in (see the checker's module documentation, "Top-level
//! functions").
//!
//! An item or a body *mentions* a top-level function when a name in it,
//! read as a variable, is the function's and no declaration the tree shows
//! shadows it there: a parameter, a local `let` or `fn` item, a field of an
//! inner block, a `for` variable or a header variable, each from where it
//! is in scope, and a function's name for itself in its own body. The
//! first segment of a dotted path, and the variable a `:=` rebinds, count
//! as mentions too. A function named by a dotted path is mentioned by a
//! reference to its path, or to a prefix of it (see [Dotted
//! `def`s](#dotted-defs)), shadowed as the reference's first name is.
//!
//! The relation over-approximates, and only ever that way: a name that a
//! *spread* inside a nested block defines is not known before checking, so
//! it does not shadow here, and a name read there counts as a mention
//! although it reads the spread's field. So does a dotted `let` that would
//! declare its first segment. A mention that is not a use only makes a
//! body be checked earlier, or an item be held to the rule that a function
//! it mentions exists by then; it never hides a use.
//!
//! The groups are the strongly connected components of "mentions" between
//! the top-level functions, found by Tarjan's algorithm. They come out
//! callee first: every group a group mentions, other than itself, comes
//! before it. The order is deterministic, a function of the tree alone.
//!
//! # Dotted `def`s
//!
//! A `def` may be named by a dotted path, `def dev.peios.count(...)`: it is
//! then exactly a `def` whose name is its path. Its item is a top-level
//! function ([`TopLevel`], its `name` `None`) and the leaf of its path in
//! the tree of top-level fields (below), whose cell holds its closure; the
//! leaf of a `def` the tree leaves out, which today's rules reject (a leaf
//! twice, a leaf under a leaf or over an interior node), is no top-level
//! function, but the field it is written as, which the checker reports.
//! There is no name a variable could read it by, and it does not see
//! itself by one: inside `def dev.peios.count`, `count` is whatever else
//! binds it, if anything. A use of it is a read of its path, a reference
//! (below) whose longest declared prefix is its leaf, so a reference
//! mentions both the leaf and the function, as a plain `def`'s name
//! mentions both its root and its function, wherever it is: in an item, an
//! initialiser, its own body (recursion is by path) or another function's
//! body (mutual recursion). A reference to a node above such leaves, a
//! prefix read whole (`dev.peios`, `dev`), mentions each of those
//! functions too, all at the reference ([`FieldTree::dotted_defs_under`],
//! [`Resolve::dotted_defs_under`]): its record holds their closures, so an
//! item reading it whole can only run once they exist, and its type is the
//! record of their types, which their bodies settle. A function named by a
//! dotted path of another file is no function of this input: a reference to
//! it mentions the other file's node, which makes this file need that one,
//! as for any field. A `def` of one name is unchanged: its leaf is a root,
//! never under another node.
//!
//! # Def-types
//!
//! A definition made with a def-type, `def DT PATH(PARAMS) BODY` (see the
//! checker's "Def-types"), binds nothing a variable could read, so it
//! declares no name here. Once the workspace has given it its def-type's
//! namespace's key (see [`crate::names`], "Namespace keys"), it is exactly a
//! `def` named by the path of the key and PATH ([`top_level_function`],
//! [`declarations`]): a leaf of the tree under the key's root, order-free, a
//! top-level function with no name, in its groups, swept for shadowing as
//! any declaration is. One whose namespace is not known, its DT no def-type,
//! is in no tree and no top-level function: the checker reports it. Its item
//! mentions what its body does and what its DT reads, the def-type, a
//! function or a field node (another file's, if the def-type is), so that
//! the def-type is checked, or its file run, before it.
//!
//! A reference, `DT PATH`, mentions what its DT reads and, once its
//! namespace is known, what its read of the definition, the key's root with
//! a field access for each segment of PATH, means: the definition's leaf and
//! function, as a reference to a dotted `def` does. No declaration can
//! shadow the key, which no source can write.
//!
//! A definition's meta, `defmeta DT PATH { ... }`, is a top-level function
//! as a definition is, once the workspace has given it its leaf: the meta
//! key of its def-type and directory, then its path as one name
//! ([`top_level_function`], [`declarations`]; see [`crate::names`],
//! "Namespace keys"). Its item mentions what its body does and what its DT
//! reads. A definition's item also mentions, once its metas' namespace is
//! known, what its read of its meta, that meta key with a field access for
//! its path as one name, means: its meta's leaf and function, if its
//! directory declares one, of its own file or another (whose file it then
//! needs), so that the meta's closure exists before anything that uses the
//! definition, and nothing if not.
//!
//! # Top-level fields
//!
//! The field items directly at the top level of an input, plain (`a = e`),
//! quoted (`"a b" = e`), dotted (`a.b.c = e`) and `def` (`def f`, `def
//! a.b.f`), form a tree by their paths ([`FieldTree`]): each item is a
//! *leaf*, and every proper
//! prefix of a leaf's path is an *interior* node, a record assembled from
//! the nodes below it, whose children are in the order of their first item
//! in the source. An item that today's rules reject (a second leaf with one
//! path, a leaf under a leaf, a leaf over an interior node) is left out of
//! the tree, whatever the order of the two items: the checker reports it.
//!
//! # Shadowed declarations
//!
//! A field item marked `#[shadowable]` (see the parser's
//! [Attributes](crate::parser#attributes)) may be shadowed by a later one
//! that overlaps it (see [`crate::workspace`], "Shadowable declarations"):
//! its leaf is shadowable ([`FieldLeaf::shadowable`]), and so is a node all
//! of whose leaves are ([`FieldNode::shadowable`]). Before the tree is
//! built, the declarations are swept in source order ([`Sweep`],
//! [`shadowed_items`]): each leaf path is declared after the earlier ones,
//! and one that overlaps earlier leaves (the same path, a prefix of it, or a
//! path under it) all of which are shadowable shadows every one of them; one
//! that overlaps a leaf that is not is what it always was, left out here
//! and reported by the checker, and shadows nothing. A shadowed item is in
//! no tree ([`FieldTree::shadowed`]), and so the last of a run of shadowable
//! declarations is the leaf for the whole input, a read above it included;
//! a `def` among them is no top-level function either. It is still an item:
//! its references are found as any item's are, but it needs nothing and
//! nothing needs it, so the schedule passes it by, as it does a top-level
//! function, and the checker checks it apart, as it is written (see the
//! checker's "Shadowed declarations"). A workspace sweeps the declarations
//! of every file of a directory together, and gives each file its shadowed
//! items ([`scan_except`]).
//!
//! A *reference* to a top-level field is a variable naming a root, with a
//! chain of field accesses directly on it, `a`, `a.b`, `a.b.c`: it means the
//! longest prefix of its path that the tree declares, a leaf or an interior
//! node, and the segments after that are ordinary accesses on its value
//! ([`FieldTree::reference`]). A root named any other way, as in `(a).b`,
//! means the whole root. The mentions of the field nodes are found in the
//! same walk as those of the functions, with the same shadowing, and the
//! same over-approximation: an item's, an initialiser's or a body's
//! mentions are the nodes its references mean. A top-level field item's own
//! path is not a mention of its root (it declares it); a dotted path in a
//! nested block, which declares a field of that block, is, as it is of a
//! function.
//!
//! # The schedule and cycles
//!
//! What needs what is a graph over the field nodes and the functions
//! ([`Need`], [`Mentions::needs`]): a leaf needs what its initialiser
//! mentions (a `def`'s leaf nothing: its function carries the body), an
//! interior node its children, a function what its body mentions.
//! [`Mentions::schedule`] gives each field node its *point*, the item of
//! the outermost block before which it is computed, and the order to
//! compute them in: for each item that is not a top-level declaration, in
//! source order, the nodes it needs that are not yet scheduled, closure
//! first and dependencies before their users; a leaf nothing needs earlier
//! falls to its own item, an interior node to the item after its last
//! child's; the rest go after the last item. A function's own place never
//! pulls a field earlier. The walk keeps its own stack and visits each node
//! once, so it is linear in the size of the graph and does not recurse on
//! the native stack however long a chain of fields is.
//! [`Mentions::cycles`] gives the strongly connected components of the same
//! graph that hold a field node and either several nodes or an edge to
//! itself: each is a field that would be computed from itself, which the
//! checker reports. Both are found from the tree alone, before checking;
//! whether a leaf can be computed at its point (what it reads is bound by
//! then) is the checker's question.
//!
//! # Imports
//!
//! An `import` declares private top-level names of its file, but nothing
//! here: it has no expression to walk and adds nothing to the tree, so a
//! file's own tree and record never hold an imported name. The names are
//! the [`Resolve`] scope's. In a workspace that scope
//! ([`crate::workspace::FileScope`]) is the file's view: the file's tree
//! merged with its directory chain's and with each member and root of a
//! `*` it imports, a whole import a root of its own, with which the scan
//! has already made sure nothing shares a name, and the implicit
//! `#[shadowable] import * from prelude`'s roots, but those a declaration
//! shadows (see [`crate::workspace`], "Shadowable declarations"); a lone
//! file and a session's input are scanned as
//! a workspace's file, beside the standard library, and only a file found
//! with [`find`], with no scope of imports, has none (its checker rejects
//! its `import`s). The over-approximation holds here too: a name the
//! prelude binds that a top-level `let` or `fn` of the file shadows is
//! still a mention of the prelude's node. A reference through
//! an imported name is then resolved as any reference is, to the longest
//! prefix the scope declares, and the same shadowing applies: a parameter
//! or a nearer `let` of the name hides the import. Such a reference is
//! another file's node, so it is a mention in [`Mentions::foreign`], never
//! a need of this file's schedule, unless it is a node of the view that
//! other sources share and that has a leaf of the file under it: those the
//! scope numbers among the file's own nodes, after its tree's
//! ([`Mentions::views`]), and they are scheduled as interior nodes, each
//! needing its children of the file.
//!
//! # Types
//!
//! A `type` item declares a name in the input's namespace of types, which
//! is not its variables': like a top-level function's, the name is the
//! whole input's, above and below the item, so the scan lists the input's
//! `type` items ([`Mentions::types`], [`TypeDecl`]) as it does its
//! functions, and a workspace merges them across files apart from the
//! field tree (see [`crate::workspace`], "Types"). A type is no
//! declaration of the field tree, and nothing here mentions one: a type has
//! no value, so it is never computed, needs nothing and is needed by
//! nothing, and a file reading another's types does not need that file. A
//! name read in a type position is no variable: the walk reads no type
//! expression, and a variable, field or `def` named like a type is unrelated
//! to it. The reads of types by name, in annotations, ascriptions,
//! parameters, function types and `type` items alike, are listed apart
//! ([`type_reads`]), for the workspace to resolve and expand.

use std::collections::HashMap;
use std::mem;

use crate::ast::{
    Ast, DefTypeId, ExprId, ExprKind, FnId, ForHead, ForId, ItemId, ItemKind, ItemRange, PathRange,
    PathSegment, SegmentKind, StrPart, TypeExprId, TypeExprKind,
};
use crate::names::Name;
use crate::source::Span;

/// What [`find`] found: the input's top-level functions, what each
/// top-level item mentions, and the groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mentions {
    /// The top-level functions, in source order. A function is referred to
    /// by its index here.
    pub functions: Vec<TopLevel>,
    /// For each item of the outermost block, by its position among them,
    /// the functions it mentions, each once, in index order. For a
    /// top-level function, those its body mentions.
    pub items: Vec<Vec<u32>>,
    /// Parallel to `items`: where each item first mentions each function
    /// it mentions, in source order the first identifier naming it.
    pub spans: Vec<Vec<Span>>,
    /// The groups, callee first, each one's members in source order.
    pub groups: Vec<Vec<u32>>,
    /// The group of each function, an index in `groups`.
    pub group_of: Vec<u32>,
    /// The tree of the input's top-level fields.
    pub tree: FieldTree,
    /// For each item of the outermost block, by its position among them,
    /// the field nodes it mentions, each once, in index order: for a field
    /// item those its initialiser mentions, for a top-level function those
    /// its body mentions.
    pub fields: Vec<Vec<u32>>,
    /// Parallel to `fields`: where each item first mentions each node, the
    /// span of the reference's longest declared prefix.
    pub field_spans: Vec<Vec<Span>>,
    /// For each item of the outermost block, by its position among them,
    /// what it mentions that is not this input's own: the functions and
    /// field nodes of the scope [`Mentions::resolve`] was given numbered at
    /// or above this input's own counts, each once, in source order of its
    /// first mention, with where. Always empty for an input alone.
    pub foreign: Vec<Vec<(Need, Span)>>,
    /// In a workspace, the nodes of the file's view that other sources
    /// share and that have a leaf of the file under them (see
    /// [`crate::workspace`], "The view of a file"): field nodes numbered
    /// after the tree's, `tree.nodes.len() + index`, each needing its
    /// children as an interior node does. Set by the workspace before the
    /// schedule is found; always empty for an input alone.
    pub views: Vec<ViewNode>,
    /// For each item of the outermost block, by its position, whether a
    /// later declaration shadows it (see [Shadowed
    /// declarations](#shadowed-declarations)), as the tree has it too
    /// ([`FieldTree::shadowed`]).
    pub shadowed: Vec<bool>,
    /// The input's `type` declarations, in source order (see
    /// [Types](#types)).
    pub types: Vec<TypeDecl>,
}

/// A `type` item of an input's outermost block (see [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    /// The name it declares.
    pub name: Name,
    /// Where the name is written.
    pub span: Span,
    /// The item.
    pub item: ItemId,
    /// The item's position among the outermost block's items.
    pub position: usize,
    /// The type it stands for.
    pub ty: TypeExprId,
}

/// A read of a type by its name, in a type expression of a tree
/// ([`type_reads`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeRead {
    /// A name, written bare: a built-in type's, a declared type's, a
    /// module's or nothing's.
    Named(Name),
    /// A qualified name, its path in the tree's path pool.
    Path(PathRange),
    /// A name applied to a type argument, `NAME<TYPE>`.
    Applied(Name),
}

/// A node of a file's view that other sources share and that has a leaf of
/// the file under it: an interior node of the graph [`Mentions::schedule`]
/// walks, though not of the file's own tree (see [`Mentions::views`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNode {
    /// Its last segment.
    pub name: Name,
    /// The view node it is a child of, by its field node number; `None` for
    /// a root.
    pub parent: Option<u32>,
    /// Its children that are field nodes of the file (its own tree's nodes
    /// and other view nodes), by number, in the view's order; its other
    /// children are other files' cells, computed before the file runs.
    pub children: Vec<u32>,
    /// The position of the first item at or under it of the file.
    pub first: usize,
    /// The position of the last item at or under it of the file.
    pub last: usize,
}

/// The tree of an input's top-level fields (see the module documentation).
/// A node is referred to by its index in `nodes`; a parent always comes
/// before its children.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldTree {
    /// The nodes, in the order their paths first appear in the source.
    pub nodes: Vec<FieldNode>,
    /// The roots, in order of their first item.
    pub roots: Vec<u32>,
    /// For each item of the outermost block, by its position, the node it
    /// is the leaf of, if it is a field item the tree took.
    pub item_leaf: Vec<Option<u32>>,
    /// For each item of the outermost block, by its position, whether it is
    /// a field item a later declaration shadows, which the tree did not take
    /// (see [Shadowed declarations](#shadowed-declarations)).
    pub shadowed: Vec<bool>,
    /// Each node by its parent (`u32::MAX` for a root) and its name.
    by_name: HashMap<(u32, Name), u32>,
}

/// One node of a [`FieldTree`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldNode {
    /// Its last segment.
    pub name: Name,
    /// The node it is a child of; `None` for a root.
    pub parent: Option<u32>,
    /// Its children, in order of their first item.
    pub children: Vec<u32>,
    /// The position of the first item at or under it.
    pub first: usize,
    /// The position of the last item at or under it.
    pub last: usize,
    /// Its item, if it is a leaf; `None` for an interior node.
    pub leaf: Option<FieldLeaf>,
    /// Whether it is shadowable: a leaf whose item is marked so, or an
    /// interior node all of whose leaves are (see [Shadowed
    /// declarations](#shadowed-declarations)).
    pub shadowable: bool,
    /// Whether it is the leaf of a top-level function named by a dotted
    /// path, or an interior node with one under it (see [Dotted
    /// `def`s](#dotted-defs)).
    pub dotted_defs: bool,
}

/// The item of a leaf of a [`FieldTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLeaf {
    /// The field item, `PATH = EXPR` or `def PATH ...`.
    pub item: ItemId,
    /// The item's position among the outermost block's items.
    pub position: usize,
    /// Its initialiser.
    pub value: ExprId,
    /// Whether the item is marked `#[shadowable]`.
    pub shadowable: bool,
    /// For a `def`'s leaf, its top-level function, by its index among the
    /// input's ([`Mentions::functions`]).
    pub function: Option<u32>,
}

impl FieldTree {
    /// The tree of the field items of `root`, the items of an outermost
    /// block, but those `shadowed` marks (by position).
    fn build(ast: &Ast, root: &[ItemId], shadowed: &[bool]) -> Self {
        let mut tree = Self {
            item_leaf: vec![None; root.len()],
            shadowed: (0..root.len())
                .map(|position| shadowed.get(position).copied().unwrap_or(false))
                .collect(),
            ..Self::default()
        };
        for (position, &id) in root.iter().enumerate() {
            let Some((names, value)) = leaf_path(ast, id) else {
                continue;
            };
            if tree.shadowed.get(position) == Some(&true) {
                continue;
            }
            let Some(names) = names else { continue };
            if tree.accepts(&names) {
                let node = tree.insert(&names, position);
                if let Some(entry) = tree.nodes.get_mut(node as usize) {
                    entry.leaf = Some(FieldLeaf {
                        item: id,
                        position,
                        value,
                        shadowable: ast.is_shadowable(id),
                        function: None,
                    });
                }
                if let Some(entry) = tree.item_leaf.get_mut(position) {
                    *entry = Some(node);
                }
            }
        }
        // A child comes after its parent, so the children are known first.
        for index in (0..tree.nodes.len()).rev() {
            let shadowable = tree.nodes.get(index).is_some_and(|node| match node.leaf {
                Some(leaf) => leaf.shadowable,
                None => node.children.iter().all(|&child| {
                    tree.nodes
                        .get(child as usize)
                        .is_some_and(|child| child.shadowable)
                }),
            });
            if let Some(node) = tree.nodes.get_mut(index) {
                node.shadowable = shadowable;
            }
        }
        tree
    }

    /// Gives each `def`'s leaf its top-level function, of `functions`, and
    /// marks the leaf of each function named by a dotted path, and every
    /// node above it, as having one (see [Dotted `def`s](#dotted-defs)).
    fn link(&mut self, functions: &[TopLevel]) {
        for (index, function) in (0u32..).zip(functions) {
            let Some(&Some(node)) = self.item_leaf.get(function.position) else {
                continue;
            };
            if let Some(leaf) = self
                .nodes
                .get_mut(node as usize)
                .and_then(|entry| entry.leaf.as_mut())
            {
                leaf.function = Some(index);
            }
            if function.name.is_some() {
                continue;
            }
            let mut at = Some(node);
            while let Some(entry) = at.and_then(|at| self.nodes.get_mut(at as usize)) {
                if entry.dotted_defs {
                    break;
                }
                entry.dotted_defs = true;
                at = entry.parent;
            }
        }
    }

    /// Calls `each` with the top-level function of every leaf of a function
    /// named by a dotted path at or under `node`, in no particular order
    /// (see [Dotted `def`s](#dotted-defs)). Visits only the nodes that have
    /// one, so it costs nothing for a node with none.
    pub fn dotted_defs_under(&self, node: u32, each: &mut dyn FnMut(u32)) {
        if !self
            .nodes
            .get(node as usize)
            .is_some_and(|entry| entry.dotted_defs)
        {
            return;
        }
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            let Some(entry) = self.nodes.get(at as usize).filter(|n| n.dotted_defs) else {
                continue;
            };
            match entry.leaf {
                Some(leaf) => {
                    if let Some(function) = leaf.function {
                        each(function);
                    }
                }
                None => stack.extend(&entry.children),
            }
        }
    }

    /// Whether a leaf of the path `names` may be added: no node along it is
    /// a leaf, and it is not a node already.
    fn accepts(&self, names: &[Name]) -> bool {
        let mut parent = u32::MAX;
        for &name in names {
            let Some(&node) = self.by_name.get(&(parent, name)) else {
                return true;
            };
            if self
                .nodes
                .get(node as usize)
                .is_none_or(|n| n.leaf.is_some())
            {
                return false;
            }
            parent = node;
        }
        false
    }

    /// Adds the nodes of the path `names` that are missing, for the item at
    /// `position`, and returns the last.
    fn insert(&mut self, names: &[Name], position: usize) -> u32 {
        let mut parent = u32::MAX;
        for &name in names {
            let node = if let Some(&node) = self.by_name.get(&(parent, name)) {
                node
            } else {
                let node = index_u32(self.nodes.len());
                self.nodes.push(FieldNode {
                    name,
                    parent: (parent != u32::MAX).then_some(parent),
                    children: Vec::new(),
                    first: position,
                    last: position,
                    leaf: None,
                    shadowable: false,
                    dotted_defs: false,
                });
                self.by_name.insert((parent, name), node);
                match self.nodes.get_mut(parent as usize) {
                    Some(up) => up.children.push(node),
                    None => self.roots.push(node),
                }
                node
            };
            if let Some(entry) = self.nodes.get_mut(node as usize) {
                entry.last = position;
            }
            parent = node;
        }
        parent
    }

    /// The root named `name`, if the input has one.
    #[must_use]
    pub fn root(&self, name: Name) -> Option<u32> {
        self.by_name.get(&(u32::MAX, name)).copied()
    }

    /// The child of `node` named `name`, if it has one.
    #[must_use]
    pub fn child(&self, node: u32, name: Name) -> Option<u32> {
        self.by_name.get(&(node, name)).copied()
    }

    /// The names of `node`'s path, from its root.
    #[must_use]
    pub fn path(&self, node: u32) -> Vec<Name> {
        let mut names = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.nodes.get(index as usize)
        {
            names.push(entry.name);
            at = entry.parent;
        }
        names.reverse();
        names
    }

    /// What the expression `id` refers to, if it is a reference to a
    /// top-level field: a variable naming a root, or a chain of field
    /// accesses directly on one. The node is the longest prefix of the path
    /// the tree declares, and the expression the part of `id` that names
    /// it (the variable, or one of the accesses). Whether the variable is
    /// shadowed where it is read is the caller's to know.
    #[must_use]
    pub fn reference(&self, ast: &Ast, id: ExprId) -> Option<(u32, ExprId)> {
        Own {
            tree: self,
            functions: &[],
        }
        .reference(ast, id)
    }
}

/// What the names of an input's declarations mean where its references are
/// resolved: its own tree and functions for an input alone ([`Own`]), the
/// merged trees of its directory and the directories above in a workspace
/// (see [`crate::workspace`]). Node and function numbers are the scope's:
/// [`Mentions::resolve`] keeps those below the input's own counts as its
/// own and sets the rest apart ([`Mentions::foreign`]).
pub trait Resolve {
    /// The field node that is the root named `name`, if any.
    fn root(&self, name: Name) -> Option<u32>;

    /// The child of the field node `node` named `name`, if it has one.
    fn child(&self, node: u32, name: Name) -> Option<u32>;

    /// The top-level function named `name`, if any: one named by a dotted
    /// path has no name, and is found by its path's references instead
    /// ([`Resolve::dotted_defs_under`]).
    fn function(&self, name: Name) -> Option<u32>;

    /// Calls `each` with every top-level function of the input's own named
    /// by a dotted path whose leaf is the field node `node` or under it
    /// (see [Dotted `def`s](#dotted-defs)); with none for a node that is
    /// not the input's own.
    fn dotted_defs_under(&self, node: u32, each: &mut dyn FnMut(u32));

    /// The number of the input's own field nodes and of its own top-level
    /// functions: the scope's numbers below them are the input's own.
    fn own(&self) -> (u32, u32);

    /// What the expression `id` refers to, if it is a reference to a
    /// top-level field, as [`FieldTree::reference`] says, by this scope's
    /// nodes.
    fn reference(&self, ast: &Ast, id: ExprId) -> Option<(u32, ExprId)> {
        // The accesses, outermost first, then the base.
        let mut chain = Vec::new();
        let mut base = id;
        while let ExprKind::Field { expr, name } = ast.get(base).kind {
            chain.push((base, name));
            base = expr;
        }
        let ExprKind::Name(root) = ast.get(base).kind else {
            return None;
        };
        let mut node = self.root(root)?;
        let mut at = base;
        for &(access, name) in chain.iter().rev() {
            let Some(child) = self.child(node, name) else {
                break;
            };
            (node, at) = (child, access);
        }
        Some((node, at))
    }
}

/// The scope of an input alone: its own field tree and top-level functions.
#[derive(Clone, Copy, Debug)]
pub struct Own<'a> {
    /// The input's field tree.
    pub tree: &'a FieldTree,
    /// The input's top-level functions.
    pub functions: &'a [TopLevel],
}

impl Resolve for Own<'_> {
    fn root(&self, name: Name) -> Option<u32> {
        self.tree.root(name)
    }

    fn child(&self, node: u32, name: Name) -> Option<u32> {
        self.tree.child(node, name)
    }

    fn function(&self, name: Name) -> Option<u32> {
        self.functions
            .iter()
            .position(|f| f.name == Some(name))
            .map(index_u32)
    }

    fn dotted_defs_under(&self, node: u32, each: &mut dyn FnMut(u32)) {
        self.tree.dotted_defs_under(node, each);
    }

    fn own(&self) -> (u32, u32) {
        (
            index_u32(self.tree.nodes.len()),
            index_u32(self.functions.len()),
        )
    }
}

/// One top-level function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopLevel {
    /// Its item, `fn NAME ...`, `def NAME ...` or `def PATH ...`.
    pub item: ItemId,
    /// The item's position among the outermost block's items.
    pub position: usize,
    /// Its name; `None` for a `def` named by a dotted path, which has none
    /// a variable could be read by (see [Dotted `def`s](#dotted-defs)).
    pub name: Option<Name>,
    /// Its function expression.
    pub expr: ExprId,
    /// Its function.
    pub function: FnId,
}

/// Finds the top-level functions of `ast`'s outermost block, what each of
/// its items mentions, and the groups (see the module documentation).
#[must_use]
pub fn find(ast: &Ast) -> Mentions {
    let mut mentions = scan(ast);
    // The functions stay: the groups are found among them.
    let (functions, tree) = (mentions.functions.clone(), mem::take(&mut mentions.tree));
    mentions.resolve(
        ast,
        &Own {
            tree: &tree,
            functions: &functions,
        },
    );
    mentions.tree = tree;
    mentions
}

/// The declarations of `ast`'s outermost block, its top-level functions and
/// its field tree, with nothing resolved yet: the first half of [`find`]. A
/// workspace merges the trees of its files before it resolves any of them
/// ([`Mentions::resolve`]).
#[must_use]
pub fn scan(ast: &Ast) -> Mentions {
    scan_except(ast, &shadowed_items(ast))
}

/// [`scan`], with the items of the outermost block that `shadowed` marks
/// (by position) shadowed, as a workspace finds them among the files of a
/// directory (see [Shadowed declarations](#shadowed-declarations)): in no
/// tree, and no top-level function.
#[must_use]
pub fn scan_except(ast: &Ast, shadowed: &[bool]) -> Mentions {
    let root = ast.items(ast.root());
    let shadowed: Vec<bool> = (0..root.len())
        .map(|position| shadowed.get(position).copied().unwrap_or(false))
        .collect();
    let mut tree = FieldTree::build(ast, root, &shadowed);
    let mut functions = Vec::new();
    for (position, &id) in root.iter().enumerate() {
        if shadowed.get(position) == Some(&true) {
            continue;
        }
        let Some((name, expr, function)) = top_level_function(ast, id) else {
            continue;
        };
        // A dotted `def` the tree left out, which today's rules reject, is
        // the field it is written as, which the checker reports.
        if name.is_none() && tree.item_leaf.get(position).is_none_or(Option::is_none) {
            continue;
        }
        functions.push(TopLevel {
            item: id,
            position,
            name,
            expr,
            function,
        });
    }
    tree.link(&functions);
    let types = (0_usize..)
        .zip(root)
        .filter_map(|(position, &item)| match ast.item(item).kind {
            ItemKind::Type {
                name,
                name_span,
                ty,
            } => Some(TypeDecl {
                name,
                span: name_span,
                item,
                position,
                ty,
            }),
            _ => None,
        })
        .collect();
    Mentions {
        functions,
        tree,
        shadowed,
        types,
        ..Mentions::default()
    }
}

/// Every type expression of `ast` that reads a type by its name, in order
/// (by id), as it is written (see [Types](#types)): a name written bare, a
/// qualified name and a name applied to a type argument, in annotations,
/// ascriptions, parameters, function types and `type` items alike. Which
/// are of declared types, and which of those it can name, is the
/// workspace's to say.
#[must_use]
pub fn type_reads(ast: &Ast) -> Vec<(TypeExprId, TypeRead)> {
    ast.type_ids()
        .filter_map(|id| {
            let read = match ast.written_type(id) {
                TypeExprKind::Named(name) => TypeRead::Named(name),
                TypeExprKind::Path(path) => TypeRead::Path(path),
                TypeExprKind::Applied { name, .. } => TypeRead::Applied(name),
                _ => return None,
            };
            Some((id, read))
        })
        .collect()
}

/// The field items of `ast`'s outermost block that a later one shadows (see
/// [Shadowed declarations](#shadowed-declarations)), by position: those
/// its declarations, swept in source order, shadow.
#[must_use]
pub fn shadowed_items(ast: &Ast) -> Vec<bool> {
    let mut shadowed = vec![false; ast.items(ast.root()).len()];
    // Only a marked declaration is ever shadowed.
    if !ast.marks_any() {
        return shadowed;
    }
    let mut sweep = Sweep::new();
    for (position, names, shadowable) in declarations(ast) {
        for killed in sweep
            .declare(&names, position, shadowable)
            .unwrap_or_default()
        {
            if let Some(entry) = shadowed.get_mut(killed) {
                *entry = true;
            }
        }
    }
    shadowed
}

/// The declarations of `ast`'s outermost block that a later one may shadow
/// or be shadowed by, in source order: each field item's (a `def`'s
/// included) position, the names of its path, and whether it is marked
/// `#[shadowable]`.
pub fn declarations(ast: &Ast) -> impl Iterator<Item = (usize, Vec<Name>, bool)> + '_ {
    ast.items(ast.root())
        .iter()
        .enumerate()
        .filter_map(|(position, &id)| {
            let (names, _) = leaf_path(ast, id)?;
            Some((position, names?, ast.is_shadowable(id)))
        })
}

/// The path of the leaf the item `id` of an outermost block is, if it is a
/// field item or a definition made with a def-type whose namespace is
/// known, the definition's under its namespace's key, or a definition's
/// meta whose namespace is known, the meta key and the meta's path as one
/// name (see [Def-types](#def-types)), with its value: the path's names, or
/// `None` for a path with a segment that is not one.
fn leaf_path(ast: &Ast, id: ItemId) -> Option<(Option<Vec<Name>>, ExprId)> {
    match ast.item(id).kind {
        ItemKind::Field { path, value, .. } => Some((
            ast.path(path).iter().map(PathSegment::name).collect(),
            value,
        )),
        ItemKind::DefTypeDef { def, value } => {
            let def = ast.def_type(def);
            let names = std::iter::once(def.key)
                .chain(ast.path(def.path).iter().map(PathSegment::name))
                .collect::<Option<Vec<Name>>>();
            // A definition whose def-type is not known is in no tree.
            Some((Some(names?), value))
        }
        ItemKind::DefMeta { def, value } => {
            let (key, leaf) = ast.def_type(def).meta?;
            Some((Some(vec![key, leaf]), value))
        }
        _ => None,
    }
}

/// Leaf declarations swept in one order, each declared after those before
/// it, to find which shadows which (see [Shadowed
/// declarations](#shadowed-declarations)): the declarations of one input,
/// or of every file of a directory (see [`crate::workspace`]), each known by
/// its key `K`. A trie of their paths, holding only the leaves that stand.
#[derive(Clone, Debug)]
pub struct Sweep<K> {
    /// The nodes; a node of no leaf that stands is removed from `by_name`.
    nodes: Vec<SweepNode<K>>,
    /// Each node that stands by its parent (`u32::MAX` for a root) and its
    /// name.
    by_name: HashMap<(u32, Name), u32>,
}

/// One node of a [`Sweep`].
#[derive(Clone, Debug)]
struct SweepNode<K> {
    /// Its last segment.
    name: Name,
    /// The leaf declared at it that stands, with whether it is shadowable.
    leaf: Option<(K, bool)>,
    /// Its children that stand.
    children: Vec<u32>,
}

impl<K: Copy> Default for Sweep<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy> Sweep<K> {
    /// No declaration yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            by_name: HashMap::new(),
        }
    }

    /// Declares the leaf `key` of the path `names`, shadowable or not, after
    /// every leaf declared so far. It overlaps each leaf that stands at its
    /// path, at a prefix of it or under it: if every one of those is
    /// shadowable, it shadows them all, and returns their keys; if one is
    /// not, it is the conflict it always was, which others report, and it is
    /// not declared (`None`).
    pub fn declare(&mut self, names: &[Name], key: K, shadowable: bool) -> Option<Vec<K>> {
        let mut parent = u32::MAX;
        let mut walked = 0;
        let mut above = None;
        for &name in names {
            let Some(&node) = self.by_name.get(&(parent, name)) else {
                break;
            };
            parent = node;
            walked += 1;
            if self
                .nodes
                .get(node as usize)
                .is_some_and(|n| n.leaf.is_some())
            {
                above = Some(node);
                break;
            }
        }
        // The leaves it overlaps: one at its path or a prefix of it, or every
        // one under its path, if its path is a node already.
        let overlapped: Vec<(K, bool)> = match above {
            Some(node) => self
                .nodes
                .get(node as usize)
                .and_then(|n| n.leaf)
                .into_iter()
                .collect(),
            None if walked == names.len() && parent != u32::MAX => self.leaves_under(parent),
            None => Vec::new(),
        };
        if overlapped.iter().any(|&(_, marked)| !marked) {
            return None;
        }
        match above {
            Some(node) => {
                if let Some(entry) = self.nodes.get_mut(node as usize) {
                    entry.leaf = None;
                }
            }
            None if !overlapped.is_empty() => self.remove_under(parent),
            None => {}
        }
        for &name in names.get(walked..).unwrap_or_default() {
            let node = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
            self.nodes.push(SweepNode {
                name,
                leaf: None,
                children: Vec::new(),
            });
            self.by_name.insert((parent, name), node);
            if let Some(up) = self.nodes.get_mut(parent as usize) {
                up.children.push(node);
            }
            parent = node;
        }
        if let Some(entry) = self.nodes.get_mut(parent as usize) {
            entry.leaf = Some((key, shadowable));
        }
        Some(overlapped.into_iter().map(|(key, _)| key).collect())
    }

    /// The leaves that stand under the node `node`.
    fn leaves_under(&self, node: u32) -> Vec<(K, bool)> {
        let mut leaves = Vec::new();
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            let Some(entry) = self.nodes.get(at as usize) else {
                continue;
            };
            leaves.extend(entry.leaf);
            stack.extend(entry.children.iter().rev());
        }
        leaves
    }

    /// Removes every node under the node `node`, whose leaves are shadowed.
    fn remove_under(&mut self, node: u32) {
        let mut stack: Vec<(u32, u32)> = self
            .nodes
            .get_mut(node as usize)
            .map(|entry| mem::take(&mut entry.children))
            .unwrap_or_default()
            .into_iter()
            .map(|child| (node, child))
            .collect();
        while let Some((parent, at)) = stack.pop() {
            let Some(entry) = self.nodes.get_mut(at as usize) else {
                continue;
            };
            entry.leaf = None;
            let name = entry.name;
            stack.extend(mem::take(&mut entry.children).into_iter().map(|c| (at, c)));
            self.by_name.remove(&(parent, name));
        }
    }
}

impl Mentions {
    /// Resolves the references of `ast`, the tree [`scan`] found these
    /// declarations in, against `scope`, and finds what each item mentions
    /// and the groups: the second half of [`find`]. A function or node
    /// number of `scope` below the count of this input's own is its own;
    /// the rest are kept apart, in [`Mentions::foreign`], and nothing else
    /// here (the groups, the schedule, the cycles, [`Mentions::needs`])
    /// sees them. The groups are found among [`Mentions::functions`], which
    /// must be [`scan`]'s.
    pub fn resolve(&mut self, ast: &Ast, scope: &dyn Resolve) {
        let root = ast.items(ast.root());
        let (own_nodes, own_functions) = scope.own();
        let mut walker = Walker {
            ast,
            scope,
            shadows: Vec::new(),
            found: Vec::new(),
            found_fields: Vec::new(),
        };
        let mut items = Vec::with_capacity(root.len());
        let mut spans = Vec::with_capacity(root.len());
        let mut fields = Vec::with_capacity(root.len());
        let mut field_spans = Vec::with_capacity(root.len());
        let mut foreign = Vec::with_capacity(root.len());
        for &id in root {
            // A top-level item's own declaration shadows nothing for the
            // items after it: the names of the top level that clash with a
            // function's or a field's are errors of their own.
            walker.found.clear();
            walker.found_fields.clear();
            walker.shadows.clear();
            walker.top_item(id);
            let mut others = Vec::new();
            let (found, at) = first_of_each(std::mem::take(&mut walker.found));
            let (found, at) = split_own(found, at, own_functions, |f, span| {
                others.push((Need::Function(f), span));
            });
            items.push(found);
            spans.push(at);
            let (found, at) = first_of_each(std::mem::take(&mut walker.found_fields));
            let (found, at) = split_own(found, at, own_nodes, |n, span| {
                others.push((Need::Field(n), span));
            });
            fields.push(found);
            field_spans.push(at);
            others.sort_by_key(|&(_, span)| span.start);
            foreign.push(others);
        }
        // The edges of the top-level functions, as [`scan`] found them: a
        // shadowed `def`, and a dotted one the tree left out, are none.
        let edges: Vec<&[u32]> = self
            .functions
            .iter()
            .map(|function| items.get(function.position).map_or(&[][..], Vec::as_slice))
            .collect();
        (self.groups, self.group_of) = components(&edges);
        self.items = items;
        self.spans = spans;
        self.fields = fields;
        self.field_spans = field_spans;
        self.foreign = foreign;
    }
}

/// Splits `found`, with its spans, into the targets below `own`, returned,
/// and the rest, given to `other`.
fn split_own(
    found: Vec<u32>,
    at: Vec<Span>,
    own: u32,
    mut other: impl FnMut(u32, Span),
) -> (Vec<u32>, Vec<Span>) {
    let mut kept = (
        Vec::with_capacity(found.len()),
        Vec::with_capacity(at.len()),
    );
    for (target, span) in found.into_iter().zip(at) {
        if target < own {
            kept.0.push(target);
            kept.1.push(span);
        } else {
            other(target, span);
        }
    }
    kept
}

/// When each top-level field node is computed (see the checker's module
/// documentation, "Top-level fields"): what [`Mentions::schedule`] finds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    /// For each field node, its point: the position of the item of the
    /// outermost block before which it is computed, or the number of items
    /// for after the last.
    pub point: Vec<usize>,
    /// The field nodes in the order they are computed: by point, and at one
    /// point dependencies first, otherwise in source order.
    pub order: Vec<u32>,
}

/// A node of the graph [`Mentions::schedule`] walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// A field node.
    Field(u32),
    /// A top-level function, which needs pass through.
    Function(u32),
}

impl Mentions {
    /// The number of field nodes of the graph: the tree's, then the view's
    /// ([`Mentions::views`]).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.tree.nodes.len() + self.views.len()
    }

    /// The names of the path of the field node `node`, a node of the tree
    /// or of the view, from its root.
    #[must_use]
    pub fn path(&self, node: u32) -> Vec<Name> {
        let Some(view) = (node as usize).checked_sub(self.tree.nodes.len()) else {
            return self.tree.path(node);
        };
        let mut names = Vec::new();
        let mut at = self.views.get(view);
        while let Some(entry) = at {
            names.push(entry.name);
            at = entry.parent.and_then(|parent| {
                let index = (parent as usize).checked_sub(self.tree.nodes.len())?;
                self.views.get(index)
            });
        }
        names.reverse();
        names
    }

    /// The view node of the field node number `node`, if it is one.
    fn view(&self, node: u32) -> Option<&ViewNode> {
        self.views
            .get((node as usize).checked_sub(self.tree.nodes.len())?)
    }

    /// The points of the field nodes, and the order they are computed in.
    ///
    /// Before each item that is not a top-level declaration, the nodes it
    /// needs that are not computed yet are computed; before a field item,
    /// its leaf, if not computed yet; before the item after an interior
    /// node's last item, that node; after the last item, the rest. A leaf
    /// needs what its initialiser mentions, an interior node its children,
    /// a function what its body mentions (a `def`'s leaf needs nothing: its
    /// value is the closure). Each walk is a depth-first search on an
    /// explicit stack, dependencies first; a node is visited once over all
    /// of them, since a finished walk leaves everything it reached computed,
    /// so the whole is linear in the size of the graph. An edge back to a
    /// node still on the stack (a cycle, which the checker reports) is
    /// skipped.
    #[must_use]
    pub fn schedule(&self) -> Schedule {
        self.schedule_except(&[])
    }

    /// [`Mentions::schedule`], with the field nodes marked in `done` (by
    /// index; a node past its end is not marked) computed elsewhere: they
    /// are never scheduled (their point stays `usize::MAX`) and a walk
    /// stops at them, as at a node already computed. A workspace marks a
    /// file's nodes that another file computes (see [`crate::workspace`]).
    #[must_use]
    pub fn schedule_except(&self, done: &[bool]) -> Schedule {
        let nodes = &self.tree.nodes;
        let fields = self.node_count();
        let count = self.items.len();
        let mut is_function = vec![false; count];
        for function in &self.functions {
            if let Some(entry) = is_function.get_mut(function.position) {
                *entry = true;
            }
        }
        let mut state = Scheduler {
            mentions: self,
            is_function,
            point: vec![usize::MAX; fields],
            visited: vec![false; fields + self.functions.len()],
            order: Vec::with_capacity(fields),
        };
        for (seen, &marked) in state.visited.iter_mut().zip(done) {
            *seen = marked;
        }
        // The interior nodes, the view's among them, by the position after
        // their last item.
        let mut interior_at: Vec<Vec<u32>> = vec![Vec::new(); count + 1];
        let tree_lasts = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.leaf.is_none())
            .map(|(index, node)| (node.last, index));
        let view_lasts = (nodes.len()..)
            .zip(&self.views)
            .map(|(index, view)| (view.last, index));
        for (last, index) in tree_lasts.chain(view_lasts) {
            if let Some(at) = interior_at.get_mut(last.saturating_add(1).min(count)) {
                at.push(index_u32(index));
            }
        }
        for position in 0..count {
            for &node in interior_at.get(position).map_or(&[][..], Vec::as_slice) {
                state.visit(Need::Field(node), position);
            }
            match self.tree.item_leaf.get(position).copied().flatten() {
                Some(leaf) => state.visit(Need::Field(leaf), position),
                // A shadowed declaration needs nothing (see [Shadowed
                // declarations](#shadowed-declarations)).
                None if self.shadowed.get(position) == Some(&true) => {}
                None if state.is_function.get(position) == Some(&false) => {
                    for need in self.needs_of_item(position) {
                        state.visit(need, position);
                    }
                }
                None => {}
            }
        }
        for node in 0..fields {
            state.visit(Need::Field(index_u32(node)), count);
        }
        Schedule {
            point: state.point,
            order: state.order,
        }
    }

    /// The cycles through field nodes: the strongly connected components of
    /// the graph [`Mentions::schedule`] walks that hold a field node and
    /// more than one node or an edge from a node to itself, callee first,
    /// each one's members field nodes first, then functions, each in index
    /// order. Mutual recursion between functions alone is not one.
    #[must_use]
    pub fn cycles(&self) -> Vec<Vec<Need>> {
        let fields = self.node_count();
        let scheduler = Scheduler {
            mentions: self,
            is_function: {
                let mut is_function = vec![false; self.items.len()];
                for function in &self.functions {
                    if let Some(entry) = is_function.get_mut(function.position) {
                        *entry = true;
                    }
                }
                is_function
            },
            point: Vec::new(),
            visited: Vec::new(),
            order: Vec::new(),
        };
        let need = |index: usize| match index.checked_sub(fields) {
            None => Need::Field(index_u32(index)),
            Some(function) => Need::Function(index_u32(function)),
        };
        let edges: Vec<Vec<u32>> = (0..fields + self.functions.len())
            .map(|index| {
                scheduler
                    .needs_of(need(index))
                    .into_iter()
                    .map(|target| index_u32(scheduler.index(target)))
                    .collect()
            })
            .collect();
        let slices: Vec<&[u32]> = edges.iter().map(Vec::as_slice).collect();
        let (groups, _) = components(&slices);
        groups
            .into_iter()
            .filter(|group| {
                group.iter().any(|&member| (member as usize) < fields)
                    && (group.len() > 1
                        || group.iter().any(|&member| {
                            edges
                                .get(member as usize)
                                .is_some_and(|out| out.contains(&member))
                        }))
            })
            .map(|group| {
                group
                    .into_iter()
                    .map(|member| need(member as usize))
                    .collect()
            })
            .collect()
    }

    /// What the item at `position` mentions, as nodes of the graph:
    /// functions, then field nodes, each in index order.
    fn needs_of_item(&self, position: usize) -> Vec<Need> {
        let functions = self.items.get(position).map_or(&[][..], Vec::as_slice);
        let fields = self.fields.get(position).map_or(&[][..], Vec::as_slice);
        functions
            .iter()
            .map(|&f| Need::Function(f))
            .chain(fields.iter().map(|&n| Need::Field(n)))
            .collect()
    }

    /// What `need` needs, the edges [`Mentions::schedule`] follows, each
    /// with where `need`'s initialiser or body first mentions it: functions,
    /// then field nodes, each in index order, or, for an interior node, its
    /// children, which it has rather than mentions (`None`).
    #[must_use]
    pub fn needs(&self, need: Need) -> Vec<(Need, Option<Span>)> {
        let position = match need {
            Need::Function(function) => self.functions.get(function as usize).map(|f| f.position),
            Need::Field(node) => match self.tree.nodes.get(node as usize) {
                Some(FieldNode {
                    leaf: Some(leaf), ..
                }) if !self.functions.iter().any(|f| f.item == leaf.item) => Some(leaf.position),
                Some(FieldNode {
                    leaf: None,
                    children,
                    ..
                }) => {
                    return children.iter().map(|&n| (Need::Field(n), None)).collect();
                }
                Some(_) => None,
                None => {
                    let children = self.view(node).map_or(&[][..], |view| &view.children);
                    return children.iter().map(|&n| (Need::Field(n), None)).collect();
                }
            },
        };
        let Some(position) = position else {
            return Vec::new();
        };
        let at = |list: &Vec<Vec<u32>>, spans: &Vec<Vec<Span>>| {
            let list = list.get(position).map_or(&[][..], Vec::as_slice);
            let spans = spans.get(position).map_or(&[][..], Vec::as_slice);
            list.iter()
                .zip(spans)
                .map(|(&index, &span)| (index, Some(span)))
                .collect::<Vec<_>>()
        };
        at(&self.items, &self.spans)
            .into_iter()
            .map(|(f, span)| (Need::Function(f), span))
            .chain(
                at(&self.fields, &self.field_spans)
                    .into_iter()
                    .map(|(n, span)| (Need::Field(n), span)),
            )
            .collect()
    }
}

/// The state of [`Mentions::schedule`].
struct Scheduler<'a> {
    mentions: &'a Mentions,
    /// For each item of the outermost block, whether it is a top-level
    /// function.
    is_function: Vec<bool>,
    /// Each field node's point, `usize::MAX` until it is scheduled.
    point: Vec<usize>,
    /// Whether each graph node has been reached: the field nodes, then the
    /// functions.
    visited: Vec<bool>,
    /// The field nodes scheduled so far, in order.
    order: Vec<u32>,
}

impl Scheduler<'_> {
    /// What `need` needs (see [`Mentions::schedule`]).
    fn needs_of(&self, need: Need) -> Vec<Need> {
        let mentions = self.mentions;
        match need {
            Need::Function(function) => mentions
                .functions
                .get(function as usize)
                .map_or_else(Vec::new, |f| mentions.needs_of_item(f.position)),
            Need::Field(node) => match mentions.tree.nodes.get(node as usize) {
                Some(FieldNode {
                    leaf: Some(leaf), ..
                }) => {
                    if self.is_function.get(leaf.position) == Some(&true) {
                        Vec::new()
                    } else {
                        mentions.needs_of_item(leaf.position)
                    }
                }
                Some(entry) => entry.children.iter().map(|&n| Need::Field(n)).collect(),
                None => mentions.view(node).map_or_else(Vec::new, |view| {
                    view.children.iter().map(|&n| Need::Field(n)).collect()
                }),
            },
        }
    }

    /// The index of `need` in `visited`.
    fn index(&self, need: Need) -> usize {
        match need {
            Need::Field(node) => node as usize,
            Need::Function(function) => self.mentions.node_count() + function as usize,
        }
    }

    /// Schedules at `position` every field node `start` reaches that is not
    /// scheduled yet, dependencies first, on an explicit stack.
    fn visit(&mut self, start: Need, position: usize) {
        if self.visited.get(self.index(start)).is_none_or(|&seen| seen) {
            return;
        }
        let mut stack: Vec<(Need, Vec<Need>, usize)> = Vec::new();
        let index = self.index(start);
        self.visited[index] = true;
        stack.push((start, self.needs_of(start), 0));
        while let Some((need, needs, next)) = stack.last_mut() {
            if let Some(&target) = needs.get(*next) {
                *next += 1;
                let index = self.index(target);
                if self.visited.get(index) == Some(&false) {
                    self.visited[index] = true;
                    let needs = self.needs_of(target);
                    stack.push((target, needs, 0));
                }
                continue;
            }
            if let Need::Field(node) = *need
                && let Some(point) = self.point.get_mut(node as usize)
            {
                *point = position;
                self.order.push(node);
            }
            stack.pop();
        }
    }
}

/// Each target of `found` once, in index order, with where it is first
/// mentioned in source order.
fn first_of_each(mut found: Vec<(u32, Span)>) -> (Vec<u32>, Vec<Span>) {
    found.sort_unstable_by_key(|&(target, span)| (target, span.start));
    found.dedup_by_key(|&mut (target, _)| target);
    found.into_iter().unzip()
}

/// The name, function expression and function of the item `id`, if it is
/// a named function item or a `def`: the name `None` for a `def` named by a
/// dotted path (see [Dotted `def`s](#dotted-defs)).
#[must_use]
pub fn top_level_function(ast: &Ast, id: ItemId) -> Option<(Option<Name>, ExprId, FnId)> {
    let (path, value, def) = match ast.item(id).kind {
        // A definition made with a def-type whose namespace is known: a
        // leaf under the namespace's key (see [Def-types](#def-types)).
        ItemKind::DefTypeDef { def, value } => {
            ast.def_type(def).key?;
            let ExprKind::Fn(function) = ast.get(value).kind else {
                return None;
            };
            return Some((None, value, function));
        }
        // A definition's meta whose namespace is known: a leaf under the
        // meta key.
        ItemKind::DefMeta { def, value } => {
            ast.def_type(def).meta?;
            let ExprKind::Fn(function) = ast.get(value).kind else {
                return None;
            };
            return Some((None, value, function));
        }
        ItemKind::Let {
            path,
            value,
            function: true,
            ..
        } => (path, value, false),
        ItemKind::Field {
            path,
            value,
            function: true,
            ..
        } => (path, value, true),
        _ => return None,
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        return None;
    };
    match ast.path(path) {
        [single] => Some((Some(single.name()?), value, function)),
        segments @ [_, _, ..] if def && segments.iter().all(|s| s.name().is_some()) => {
            Some((None, value, function))
        }
        _ => None,
    }
}

/// The strongly connected components of the graph whose node `n` has an
/// edge to each node of `edges[n]`, callee first, with each node's
/// component.
pub(crate) fn components(edges: &[&[u32]]) -> (Vec<Vec<u32>>, Vec<u32>) {
    /// Tarjan's state for one node: its visit index and low link, while it
    /// is on the stack.
    #[derive(Clone, Copy)]
    struct Visit {
        index: u32,
        low: u32,
        on_stack: bool,
    }
    let count = edges.len();
    let mut visits: Vec<Option<Visit>> = vec![None; count];
    let mut stack: Vec<u32> = Vec::new();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut group_of = vec![0; count];
    let mut next = 0u32;
    // An explicit work list of (node, next edge to follow), so deep chains
    // of functions do not overflow the native stack.
    let mut work: Vec<(u32, usize)> = Vec::new();
    for start in 0..count {
        if visits[start].is_some() {
            continue;
        }
        work.push((index_u32(start), 0));
        while let Some(&mut (node, ref mut edge)) = work.last_mut() {
            let n = node as usize;
            if *edge == 0 && visits[n].is_none() {
                visits[n] = Some(Visit {
                    index: next,
                    low: next,
                    on_stack: true,
                });
                next += 1;
                stack.push(node);
            }
            let targets = edges.get(n).copied().unwrap_or_default();
            if let Some(&target) = targets.get(*edge) {
                *edge += 1;
                match visits.get(target as usize).copied().flatten() {
                    None if (target as usize) < count => work.push((target, 0)),
                    Some(visit) if visit.on_stack => {
                        if let Some(Some(own)) = visits.get_mut(n) {
                            own.low = own.low.min(visit.index);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            work.pop();
            let Some(own) = visits[n] else { continue };
            if let Some(&(parent, _)) = work.last()
                && let Some(Some(up)) = visits.get_mut(parent as usize)
            {
                up.low = up.low.min(own.low);
            }
            if own.low == own.index {
                let mut group = Vec::new();
                while let Some(member) = stack.pop() {
                    if let Some(Some(visit)) = visits.get_mut(member as usize) {
                        visit.on_stack = false;
                    }
                    group_of[member as usize] = index_u32(groups.len());
                    group.push(member);
                    if member == node {
                        break;
                    }
                }
                group.sort_unstable();
                groups.push(group);
            }
        }
    }
    (groups, group_of)
}

/// `index` as a `u32`, saturating: no input has that many functions.
fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Walks a tree for the mentions of the top-level functions.
struct Walker<'a> {
    ast: &'a Ast,
    /// What the names of the declarations mean.
    scope: &'a dyn Resolve,
    /// The names declared where the walk is, innermost last.
    shadows: Vec<Name>,
    /// The functions mentioned so far, each with where, possibly repeated.
    found: Vec<(u32, Span)>,
    /// The field nodes mentioned so far, each with where, possibly
    /// repeated.
    found_fields: Vec<(u32, Span)>,
}

impl Walker<'_> {
    /// Records `name`, read as a variable at `span`, if it is a top-level
    /// function's or a top-level field's root and nothing shadows it: a
    /// root read whole mentions the functions named by dotted paths under
    /// it too.
    fn name(&mut self, name: Name, span: Span) {
        if self.shadows.contains(&name) {
            return;
        }
        self.function_name(name, span);
        if let Some(root) = self.scope.root(name) {
            self.found_fields.push((root, span));
            self.dotted_defs(root, span);
        }
    }

    /// Records, as mentioned at `span`, every top-level function named by a
    /// dotted path whose leaf is the field node `node` or under it: a
    /// reference to a node reads the functions at or under it (see [Dotted
    /// `def`s](#dotted-defs)).
    fn dotted_defs(&mut self, node: u32, span: Span) {
        let found = &mut self.found;
        self.scope
            .dotted_defs_under(node, &mut |function| found.push((function, span)));
    }

    /// Records `name`, read as a variable at `span`, if it is a top-level
    /// function's and nothing shadows it.
    fn function_name(&mut self, name: Name, span: Span) {
        if self.shadows.contains(&name) {
            return;
        }
        if let Some(index) = self.scope.function(name) {
            self.found.push((index, span));
        }
    }

    /// Walks an item of the outermost block. A dotted field item there
    /// declares its root rather than reading it, so its path's first
    /// segment is a mention of a function of that name only.
    fn top_item(&mut self, id: ItemId) {
        if let ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } =
            self.ast.item(id).kind
        {
            self.def_type_def(def, value);
        } else if let ItemKind::Field { path, value, .. } = self.ast.item(id).kind
            && let [first, _, ..] = self.ast.path(path)
        {
            if let Some(name) = first.name() {
                self.function_name(name, first.span);
            }
            self.expr(value);
        } else {
            self.item(id);
        }
    }

    /// Walks a field access: a reference to a top-level field when it is a
    /// chain of accesses on a variable naming a root that nothing shadows,
    /// recorded as a mention of the node it means. Kept out of
    /// [`Walker::expr`], as [`Walker::function`] is.
    #[inline(never)]
    fn field_access(&mut self, id: ExprId) {
        let ast = self.ast;
        let mut base = id;
        while let ExprKind::Field { expr, .. } = ast.get(base).kind {
            base = expr;
        }
        match ast.get(base).kind {
            ExprKind::Name(name) if !self.shadows.contains(&name) => {
                self.function_name(name, ast.get(base).span);
                if let Some((node, at)) = self.scope.reference(ast, id) {
                    let span = ast.get(at).span;
                    self.found_fields.push((node, span));
                    self.dotted_defs(node, span);
                }
            }
            _ => self.expr(base),
        }
    }

    /// Walks the items of a block in a scope of its own.
    fn block(&mut self, items: ItemRange) {
        let depth = self.shadows.len();
        for &id in self.ast.items(items) {
            self.item(id);
        }
        self.shadows.truncate(depth);
    }

    /// Walks one item, declaring what it declares for the items after it.
    fn item(&mut self, id: ItemId) {
        match self.ast.item(id).kind {
            ItemKind::Let { path, value, .. } => self.binding(path, value, false),
            ItemKind::Field { path, value, .. } => self.binding(path, value, true),
            ItemKind::Rebind { path, value } => {
                let segments = self.ast.path(path);
                if let Some(first) = segments.first()
                    && let Some(name) = first.name()
                {
                    self.name(name, first.span);
                }
                for segment in segments {
                    if let SegmentKind::Index { position, .. } = segment.kind {
                        self.expr(position);
                    }
                }
                self.expr(value);
            }
            ItemKind::Spread(value) | ItemKind::Result(value) | ItemKind::Expr(value) => {
                self.expr(value);
            }
            ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
                if let Some(value) = jump.value {
                    self.expr(value);
                }
            }
            // An import has no expression to walk. The names it binds are
            // the scope's: a workspace's resolves them ([`Resolve::root`]
            // finds an imported name after the declarations of the
            // directory chain, with which it cannot share a name), and a
            // reference through one is a mention of what it names. A `type`
            // has no expression either, and declares no variable: its name
            // is a type's, read only in type positions (see
            // [Types](#types)).
            // What the parser could not parse mentions nothing.
            ItemKind::Import(_) | ItemKind::Type { .. } | ItemKind::Error => {}
            ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } => {
                self.def_type_def(def, value);
            }
        }
    }

    /// Walks a definition made with a def-type, `def DT PATH(PARAMS) BODY`,
    /// or a definition's meta, `defmeta DT PATH { ... }`, of def-type `def`
    /// and function `value`, which declares nothing a variable can name: its
    /// body, its `DT`, a read of the def-type, whose type its function's is
    /// made from, and, for a definition whose metas' namespace is known, its
    /// read of its meta, the meta key with a field access for its path as
    /// one name, which nothing can shadow: so a definition needs its meta, a
    /// function of its own file or of another of its directory, which its
    /// closure reads when the meta is read (see [Def-types](#def-types)).
    fn def_type_def(&mut self, def: DefTypeId, value: ExprId) {
        let def = *self.ast.def_type(def);
        self.expr(def.dt);
        if let (Some(_), Some(read)) = (def.meta, def.meta_read) {
            self.expr(read);
        }
        self.expr(value);
    }

    /// Walks a `let` or a field (`field`) of the path `path` initialised
    /// with `value`, declaring its first segment for the items after it. A
    /// dotted `let` reads the variable its first segment names, which it
    /// updates; a dotted field of an inner block reads nothing, since it
    /// declares or extends a field of its own block. Kept out of
    /// [`Walker::item`], as [`Walker::function`] is out of
    /// [`Walker::expr`].
    #[inline(never)]
    fn binding(&mut self, path: PathRange, value: ExprId, field: bool) {
        let segments = self.ast.path(path);
        let single = match segments {
            [single] => single.name(),
            _ => None,
        };
        for segment in segments {
            if let SegmentKind::Index { position, .. } = segment.kind {
                self.expr(position);
            }
        }
        match (single, self.ast.get(value).kind) {
            // A function sees itself by the name it is bound to.
            (Some(name), ExprKind::Fn(_)) => {
                self.shadows.push(name);
                self.expr(value);
            }
            (Some(name), _) => {
                self.expr(value);
                self.shadows.push(name);
            }
            (None, _) => {
                let first = segments
                    .first()
                    .and_then(|first| Some((first.name()?, first.span)));
                match first {
                    Some((name, span)) if !field => self.name(name, span),
                    _ => {}
                }
                self.expr(value);
                if let Some((name, _)) = first
                    && field
                {
                    self.shadows.push(name);
                }
            }
        }
    }

    /// Walks an expression.
    fn expr(&mut self, id: ExprId) {
        let ast = self.ast;
        match ast.get(id).kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Error => {}
            ExprKind::Interp(parts) => {
                for &part in ast.parts(parts) {
                    if let StrPart::Expr(expr) = part {
                        self.expr(expr);
                    }
                }
            }
            ExprKind::If {
                cond, then, else_, ..
            } => {
                self.expr(cond);
                self.expr(then);
                self.expr(else_);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Field { .. } => self.field_access(id),
            ExprKind::Paren(expr)
            | ExprKind::Ascribe { expr, .. }
            | ExprKind::Is { expr, .. }
            | ExprKind::As { expr, .. }
            | ExprKind::NoField(expr) => self.expr(expr),
            ExprKind::Name(name) => self.name(name, ast.get(id).span),
            ExprKind::Block { items, .. } => self.block(items),
            ExprKind::Fn(function) => self.function(function),
            ExprKind::Lazy(function) => self.expr(ast.fn_decl(function).body),
            ExprKind::Call { callee, args } | ExprKind::Unclosed { callee, args } => {
                self.expr(callee);
                for arg in ast.args(args) {
                    self.expr(arg.value);
                }
            }
            ExprKind::List(elements) => {
                for element in ast.elements(elements) {
                    self.expr(element.value);
                }
            }
            ExprKind::Index { expr, index } => {
                self.expr(expr);
                self.expr(index);
            }
            ExprKind::For(id) => self.for_loop(id),
            // A read of the def-type, and, once its namespace is known, of
            // the definition's cell, by a path under the namespace's key,
            // which nothing can shadow (see [Def-types](#def-types)).
            ExprKind::DefTypeRef(def) => {
                let def = *ast.def_type(def);
                self.expr(def.dt);
                if let (Some(_), Some(read)) = (def.key, def.read) {
                    self.expr(read);
                }
            }
        }
    }

    /// Walks a function expression, whose parameters shadow names in its
    /// body and each in the defaults after it. Kept out of [`Walker::expr`],
    /// so that the frame each level of
    /// nesting takes on the native stack stays small.
    #[inline(never)]
    fn function(&mut self, function: FnId) {
        let decl = *self.ast.fn_decl(function);
        let depth = self.shadows.len();
        // Each default sees the parameters before it, and the body all.
        for param in self.ast.params(decl.params) {
            if let Some(default) = param.default {
                self.expr(default);
            }
            self.shadows.push(param.name);
        }
        self.expr(decl.body);
        self.shadows.truncate(depth);
    }

    /// Walks a `for` loop, whose variables shadow names in it. Kept out of
    /// [`Walker::expr`], as [`Walker::function`] is.
    #[inline(never)]
    fn for_loop(&mut self, id: ForId) {
        let ast = self.ast;
        let decl = *ast.for_decl(id);
        let depth = self.shadows.len();
        match decl.head {
            ForHead::Each {
                name,
                iterable,
                guard,
                ..
            } => {
                self.expr(iterable);
                self.shadows.push(name);
                if let Some(guard) = guard {
                    self.expr(guard);
                }
                self.expr(decl.body);
            }
            ForHead::While {
                before,
                cond,
                after,
            } => {
                for &item in ast.items(before) {
                    self.item(item);
                }
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                self.expr(decl.body);
                for &item in ast.items(after) {
                    self.item(item);
                }
            }
            ForHead::Do { cond } => {
                self.expr(decl.body);
                self.expr(cond);
            }
        }
        self.shadows.truncate(depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::source::SourceFile;

    /// What the items of `text`'s outermost block mention, by the names of
    /// the functions, and the groups, by their members' names.
    fn mentions(text: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, strings, &source).unwrap();
        let found = find(&ast);
        let by_name = |indices: &[u32]| {
            indices
                .iter()
                .map(|&index| {
                    let top = found.functions[index as usize];
                    match (top.name, ast.item(top.item).kind) {
                        (Some(name), _) => ast.names().get(name).to_owned(),
                        (None, ItemKind::Field { path, .. }) => {
                            ast.show_path(ast.path(path)).to_string()
                        }
                        (None, _) => String::new(),
                    }
                })
                .collect::<Vec<_>>()
        };
        let items = found.items.iter().map(|item| by_name(item)).collect();
        let groups = found.groups.iter().map(|group| by_name(group)).collect();
        (items, groups)
    }

    #[test]
    fn items_mention_the_functions_they_name() {
        let (items, groups) =
            mentions("x = f()\nfn f(): Int { g() }\nfn g(): Int { f() }\ndef h(): Int { 1 }");
        assert_eq!(items, [vec!["f"], vec!["g"], vec!["f"], vec![]]);
        // `f` and `g` are one group; `h` is a group of its own; a function
        // naming itself is not an edge of its own.
        assert_eq!(groups, [vec!["f", "g"], vec!["h"]]);
        // Mentioned anywhere: in a function expression or a `lazy` the
        // item only builds, in a loop body, a labelled block.
        let (items, _) = mentions(
            "a = fn(): Int { f() }\nb = lazy { f() }\nc = for i in [1] { f() }\nd = {'l f() }\n\
             fn f(): Int { 1 }",
        );
        assert_eq!(items[..4], [vec!["f"], vec!["f"], vec!["f"], vec!["f"]]);
    }

    #[test]
    fn a_nearer_declaration_shadows_a_function() {
        let (items, _) = mentions(
            "fn g(f: Int): Int { f }\nx = { let f = 1; = f }\ny = { f = 1; g = f }\n\
             z = for f in [1] { f }\nfn f(): Int { 1 }",
        );
        assert!(items[..4].iter().all(Vec::is_empty), "{items:?}");
        // From where the declaration is in scope only.
        let (items, _) = mentions("x = { a = f(); f = 1 }\nfn f(): Int { 1 }");
        assert_eq!(items[0], ["f"]);
        // A local function sees itself by its own name.
        let (items, _) = mentions("x = { fn f(): Int { f() }; = 1 }\nfn f(): Int { 1 }");
        assert_eq!(items[0], Vec::<String>::new());
    }

    #[test]
    fn a_name_a_nested_spread_defines_still_counts_as_a_mention() {
        // The spread defines `f` in the inner block, so the `f` read there
        // is the spread's field; it is not known before checking, so it is
        // counted as a mention of the function: an over-approximation.
        let (items, _) = mentions("x = { ...{f = 1}; g = f }\nfn f(): Int { 1 }");
        assert_eq!(items[0], ["f"]);
    }

    /// The field tree of `text`, as each node's path with ` (leaf)` on the
    /// leaves, and the field nodes each item mentions, by their paths.
    fn field_mentions(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, strings, &source).unwrap();
        let found = find(&ast);
        let path = |node: u32| {
            let names: Vec<&str> = found
                .tree
                .path(node)
                .into_iter()
                .map(|name| ast.names().get(name))
                .collect();
            names.join(".")
        };
        let nodes = (0..found.tree.nodes.len())
            .map(|index| {
                let leaf = found.tree.nodes[index].leaf.is_some();
                let node = u32::try_from(index).unwrap();
                format!("{}{}", path(node), if leaf { " (leaf)" } else { "" })
            })
            .collect();
        let items = found
            .fields
            .iter()
            .map(|item| item.iter().map(|&node| path(node)).collect())
            .collect();
        (nodes, items)
    }

    #[test]
    fn the_top_level_fields_form_a_tree() {
        let (nodes, _) =
            field_mentions("com.x.app = 1\nb = 2\ncom.x.lib.name = 3\ndef f(): Int { 1 }");
        assert_eq!(
            nodes,
            [
                "com",
                "com.x",
                "com.x.app (leaf)",
                "b (leaf)",
                "com.x.lib",
                "com.x.lib.name (leaf)",
                "f (leaf)"
            ]
        );
        // A leaf twice, a leaf under a leaf and a leaf over an interior
        // node are left out of the tree, in either order.
        let (nodes, _) = field_mentions("a = 1\na.b = 2\na = 3\nc.d = 1\nc = 2\nc.d.e = 3");
        assert_eq!(nodes, ["a (leaf)", "c", "c.d (leaf)"]);
    }

    #[test]
    fn a_reference_mentions_its_longest_declared_prefix() {
        let (_, items) = field_mentions(
            "x = [com.x.lib.name, com.x.lib, com.x.lib.other, com, (com).x, f(com.x)]\n\
             com.x.lib.name = 1\nfn f(r: {}): Int { y.z }\ny.z = 2",
        );
        assert_eq!(
            items[0],
            ["com", "com.x", "com.x.lib", "com.x.lib.name"],
            "{items:?}"
        );
        // A dotted item's own path declares its root; a function body's
        // references are mentions too.
        assert!(items[1].is_empty(), "{items:?}");
        assert_eq!(items[2], ["y.z"]);
        // Shadowed references are not mentions.
        let (_, items) =
            field_mentions("x = { let a = 1; = a.b }\nf = fn(a: Int): Int { a }\na.b = 1");
        assert!(items[0].is_empty() && items[1].is_empty(), "{items:?}");
        // A dotted field of an inner block declares its own root and reads
        // nothing; a dotted `let` reads the variable it updates.
        let (_, items) = field_mentions(
            "a.c = { a.b = 1; d = a }\nx = { let a.b = 2; a }\ny = { let a = {}; let a.b = a.c }",
        );
        assert!(items[0].is_empty(), "{items:?}");
        assert_eq!(items[1], ["a"], "{items:?}");
        assert!(items[2].is_empty(), "{items:?}");
    }

    /// The field nodes of `text` in the order they are computed, each as
    /// its path and its point.
    fn schedule(text: &str) -> Vec<String> {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, strings, &source).unwrap();
        let found = find(&ast);
        let schedule = found.schedule();
        schedule
            .order
            .iter()
            .map(|&node| {
                let names: Vec<&str> = found
                    .tree
                    .path(node)
                    .into_iter()
                    .map(|name| ast.names().get(name))
                    .collect();
                format!("{}@{}", names.join("."), schedule.point[node as usize])
            })
            .collect()
    }

    #[test]
    fn fields_are_computed_when_first_needed() {
        assert_eq!(
            schedule(
                "let major = 2\ncom.x.app.deps = [com.x.lib]\ncom.x.app.name = \"app-{version}\"\n\
                 com.x.lib.name = \"lib\"\nversion = describe(major)\n\
                 fn describe(n: Int): String { \"v{n}\" }"
            ),
            [
                "com.x.lib.name@1",
                "com.x.lib@1",
                "com.x.app.deps@1",
                "version@2",
                "com.x.app.name@2",
                "com.x.app@3",
                "com.x@4",
                "com@4"
            ]
        );
        // A whole read pulls every child to the reading item.
        assert_eq!(
            schedule("a.b = 1\nc = a\na.d = 2"),
            ["a.b@0", "a.d@1", "a@1", "c@1"]
        );
        // A function's closure does not pull a field: only a use of the
        // function does, through what its body mentions.
        assert_eq!(schedule("let k = 1\nx = k\nfn f(): Int { x }"), ["x@1"]);
        assert_eq!(
            schedule("let y = f()\nlet k = 1\nx = k\nfn f(): Int { x }"),
            ["x@0"]
        );
        // The rest after the last item; an edge of a cycle is skipped.
        assert_eq!(schedule("a = b\nb = a\nlet z = 1"), ["b@0", "a@0"]);
    }

    #[test]
    fn cycles_pass_through_a_field() {
        let cycles = |text: &str| {
            let source = SourceFile::new("test.gcsl", text).unwrap();
            let (tokens, names, strings) = tokenize(&source).unwrap();
            let ast = parse(&tokens, names, strings, &source).unwrap();
            find(&ast).cycles()
        };
        assert_eq!(
            cycles("a = b\nb = f()\nfn f(): Int { a }"),
            [vec![Need::Field(0), Need::Field(1), Need::Function(0)]]
        );
        // Through an interior node, and a field reading itself.
        assert_eq!(cycles("a.b = a"), [vec![Need::Field(0), Need::Field(1)]]);
        assert_eq!(cycles("x = lazy { x }"), [vec![Need::Field(0)]]);
        // Mutual recursion of functions alone is fine.
        assert!(cycles("fn f(): Int { g() }\nfn g(): Int { f() }\nx = f()").is_empty());
    }

    #[test]
    fn a_chain_of_fields_is_scheduled_without_deep_recursion() {
        let mut text = String::from("x = f2000\nf1 = 1\n");
        for index in 2..=2000 {
            let line = format!("f{index} = f{} + 1\n", index - 1);
            text.push_str(&line);
        }
        let order = schedule(&text);
        assert_eq!(order.len(), 2001);
        assert_eq!(order[0], "f1@0");
        assert_eq!(order[2000], "x@0");
    }

    #[test]
    fn a_dotted_def_is_a_top_level_function_found_by_its_path() {
        // Its path mentions it, wherever it is read, so it groups with the
        // functions it calls and that call it.
        let (items, groups) = mentions(
            "x = dev.peios.count(1)\n\
             def dev.peios.count(n: Int): Int { if n == 0 { 0 } else { dev.peios.count(n - 1) } }\n\
             def even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }\n\
             def dev.odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }",
        );
        assert_eq!(
            items,
            [
                vec!["dev.peios.count"],
                vec!["dev.peios.count"],
                vec!["dev.odd"],
                vec!["even"]
            ]
        );
        assert_eq!(groups, [vec!["dev.peios.count"], vec!["even", "dev.odd"]]);
        // It has no name: `count` alone mentions nothing, and a parameter
        // named like the path's root shadows it.
        let (items, _) =
            mentions("x = count\ny = fn(dev: {}): {} { dev }\ndef dev.count(): Int { 1 }");
        assert!(items[..2].iter().all(Vec::is_empty), "{items:?}");
        // A prefix read whole mentions every one under it; a sibling
        // field's path does not.
        let (items, _) = mentions(
            "all = a\nmid = a.b\ng = a.g\ndef a.b.f(): Int { 1 }\ndef a.c(): Int { 2 }\n\
             a.g = 3",
        );
        assert_eq!(items[..3], [vec!["a.b.f", "a.c"], vec!["a.b.f"], vec![]]);
        // One the tree leaves out, as today's rules reject it, is no
        // top-level function, and the groups of the others are as ever.
        let (items, groups) =
            mentions("a.b = 1\ndef a.b(): Int { 1 }\nfn p(): Int { q() }\nfn q(): Int { p() }");
        assert_eq!(items[2..], [vec!["q"], vec!["p"]]);
        assert_eq!(groups, [vec!["p", "q"]]);
        // Its leaf is the tree's, and, as a plain `def`'s, needs nothing:
        // its function carries its body.
        let (nodes, _) = field_mentions("def a.b.f(): Int { c }\nc = 1");
        assert_eq!(nodes, ["a", "a.b", "a.b.f (leaf)", "c (leaf)"]);
        assert_eq!(
            schedule("def a.b.f(): Int { c }\nc = 1\nlet x = 2"),
            ["a.b.f@0", "a.b@1", "a@1", "c@1"]
        );
    }

    #[test]
    fn groups_come_callee_first() {
        let (_, groups) = mentions(
            "fn a(): Int { b() }\nfn b(): Int { c() }\nfn c(): Int { b() + d() }\n\
             fn d(): Int { 1 }",
        );
        assert_eq!(groups, [vec!["d"], vec!["b", "c"], vec!["a"]]);
        // Three in a ring are one group.
        let (_, groups) = mentions("fn a(): Int { b() }\nfn b(): Int { c() }\nfn c(): Int { a() }");
        assert_eq!(groups, [vec!["a", "b", "c"]]);
    }
}
