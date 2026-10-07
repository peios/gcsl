//! Which top-level functions each top-level item of an input mentions, and
//! the groups of mutually recursive top-level functions.
//!
//! A *top-level function* of an input is a named function item (`fn
//! NAME`) or a `def` that is directly an item of the input's outermost
//! block. Its name is in scope in the whole input, so the checker must
//! know, before it checks anything, which functions each top-level item
//! and each function body may use: that is what decides the order the
//! bodies are checked in (see the checker's module documentation, "Top-level
//! functions").
//!
//! An item or a body *mentions* a top-level function when a name in it,
//! read as a variable, is the function's and no declaration the tree shows
//! shadows it there: a parameter, a local `let` or `fn` item, a field of an
//! inner block, a `for` variable or a header variable, each from where it
//! is in scope, and a function's name for itself in its own body. The
//! first segment of a dotted path, and the variable a `:=` rebinds, count
//! as mentions too.
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
//! # Top-level fields
//!
//! The field items directly at the top level of an input, plain (`a = e`),
//! quoted (`"a b" = e`), dotted (`a.b.c = e`) and `def`, form a tree by
//! their paths ([`FieldTree`]): each item is a *leaf*, and every proper
//! prefix of a leaf's path is an *interior* node, a record assembled from
//! the nodes below it, whose children are in the order of their first item
//! in the source. An item that today's rules reject (a second leaf with one
//! path, a leaf under a leaf, a leaf over an interior node) is left out of
//! the tree, whatever the order of the two items: the checker reports it.
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

use std::collections::HashMap;

use crate::ast::{
    Ast, ExprId, ExprKind, FnId, ForHead, ForId, ItemId, ItemKind, ItemRange, PathRange,
    PathSegment, SegmentKind, StrPart,
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
}

/// The item of a leaf of a [`FieldTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLeaf {
    /// The field item, `PATH = EXPR` or `def NAME ...`.
    pub item: ItemId,
    /// The item's position among the outermost block's items.
    pub position: usize,
    /// Its initialiser.
    pub value: ExprId,
}

impl FieldTree {
    /// The tree of the field items of `root`, the items of an outermost
    /// block.
    fn build(ast: &Ast, root: &[ItemId]) -> Self {
        let mut tree = Self {
            item_leaf: vec![None; root.len()],
            ..Self::default()
        };
        for (position, &id) in root.iter().enumerate() {
            let ItemKind::Field { path, value, .. } = ast.item(id).kind else {
                continue;
            };
            let names: Option<Vec<Name>> = ast.path(path).iter().map(PathSegment::name).collect();
            let Some(names) = names else { continue };
            if tree.accepts(&names) {
                let node = tree.insert(&names, position);
                if let Some(entry) = tree.nodes.get_mut(node as usize) {
                    entry.leaf = Some(FieldLeaf {
                        item: id,
                        position,
                        value,
                    });
                }
                if let Some(entry) = tree.item_leaf.get_mut(position) {
                    *entry = Some(node);
                }
            }
        }
        tree
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

/// One top-level function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopLevel {
    /// Its item, `fn NAME ...` or `def NAME ...`.
    pub item: ItemId,
    /// The item's position among the outermost block's items.
    pub position: usize,
    /// Its name.
    pub name: Name,
    /// Its function expression.
    pub expr: ExprId,
    /// Its function.
    pub function: FnId,
}

/// Finds the top-level functions of `ast`'s outermost block, what each of
/// its items mentions, and the groups (see the module documentation).
#[must_use]
pub fn find(ast: &Ast) -> Mentions {
    let root = ast.items(ast.root());
    let mut functions = Vec::new();
    for (position, &id) in root.iter().enumerate() {
        if let Some((name, expr, function)) = top_level_function(ast, id) {
            functions.push(TopLevel {
                item: id,
                position,
                name,
                expr,
                function,
            });
        }
    }
    let tree = FieldTree::build(ast, root);
    let mut walker = Walker {
        ast,
        functions: &functions,
        tree: &tree,
        shadows: Vec::new(),
        found: Vec::new(),
        found_fields: Vec::new(),
    };
    let mut items = Vec::with_capacity(root.len());
    let mut spans = Vec::with_capacity(root.len());
    let mut fields = Vec::with_capacity(root.len());
    let mut field_spans = Vec::with_capacity(root.len());
    for &id in root {
        // A top-level item's own declaration shadows nothing for the items
        // after it: the names of the top level that clash with a function's
        // or a field's are errors of their own.
        walker.found.clear();
        walker.found_fields.clear();
        walker.shadows.clear();
        walker.top_item(id);
        let (found, at) = first_of_each(std::mem::take(&mut walker.found));
        items.push(found);
        spans.push(at);
        let (found, at) = first_of_each(std::mem::take(&mut walker.found_fields));
        fields.push(found);
        field_spans.push(at);
    }
    let edges: Vec<&[u32]> = root
        .iter()
        .zip(&items)
        .filter(|&(&id, _)| top_level_function(ast, id).is_some())
        .map(|(_, found)| found.as_slice())
        .collect();
    let (groups, group_of) = components(&edges);
    Mentions {
        functions,
        items,
        spans,
        groups,
        group_of,
        tree,
        fields,
        field_spans,
    }
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
        let nodes = &self.tree.nodes;
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
            point: vec![usize::MAX; nodes.len()],
            visited: vec![false; nodes.len() + self.functions.len()],
            order: Vec::with_capacity(nodes.len()),
        };
        // The interior nodes by the position after their last item.
        let mut interior_at: Vec<Vec<u32>> = vec![Vec::new(); count + 1];
        for (index, node) in nodes.iter().enumerate() {
            if node.leaf.is_none()
                && let Some(at) = interior_at.get_mut(node.last.saturating_add(1).min(count))
            {
                at.push(index_u32(index));
            }
        }
        for position in 0..count {
            for &node in interior_at.get(position).map_or(&[][..], Vec::as_slice) {
                state.visit(Need::Field(node), position);
            }
            match self.tree.item_leaf.get(position).copied().flatten() {
                Some(leaf) => state.visit(Need::Field(leaf), position),
                None if state.is_function.get(position) == Some(&false) => {
                    for need in self.needs_of_item(position) {
                        state.visit(need, position);
                    }
                }
                None => {}
            }
        }
        for node in 0..nodes.len() {
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
        let fields = self.tree.nodes.len();
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
                _ => None,
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
                None => Vec::new(),
            },
        }
    }

    /// The index of `need` in `visited`.
    fn index(&self, need: Need) -> usize {
        match need {
            Need::Field(node) => node as usize,
            Need::Function(function) => self.mentions.tree.nodes.len() + function as usize,
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
/// a named function item or a `def`.
#[must_use]
pub fn top_level_function(ast: &Ast, id: ItemId) -> Option<(Name, ExprId, FnId)> {
    match ast.item(id).kind {
        ItemKind::Let {
            path,
            value,
            function: true,
            ..
        }
        | ItemKind::Field {
            path,
            value,
            function: true,
        } => match (ast.path(path), ast.get(value).kind) {
            ([single], ExprKind::Fn(function)) => Some((single.name()?, value, function)),
            _ => None,
        },
        _ => None,
    }
}

/// The strongly connected components of the graph whose node `n` has an
/// edge to each node of `edges[n]`, callee first, with each node's
/// component.
fn components(edges: &[&[u32]]) -> (Vec<Vec<u32>>, Vec<u32>) {
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
    functions: &'a [TopLevel],
    /// The input's top-level fields.
    tree: &'a FieldTree,
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
    /// function's or a top-level field's root and nothing shadows it.
    fn name(&mut self, name: Name, span: Span) {
        if self.shadows.contains(&name) {
            return;
        }
        self.function_name(name, span);
        if let Some(root) = self.tree.root(name) {
            self.found_fields.push((root, span));
        }
    }

    /// Records `name`, read as a variable at `span`, if it is a top-level
    /// function's and nothing shadows it.
    fn function_name(&mut self, name: Name, span: Span) {
        if self.shadows.contains(&name) {
            return;
        }
        if let Some(index) = self.functions.iter().position(|f| f.name == name) {
            self.found.push((index_u32(index), span));
        }
    }

    /// Walks an item of the outermost block. A dotted field item there
    /// declares its root rather than reading it, so its path's first
    /// segment is a mention of a function of that name only.
    fn top_item(&mut self, id: ItemId) {
        if let ItemKind::Field { path, value, .. } = self.ast.item(id).kind
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
                if let Some((node, at)) = self.tree.reference(ast, id) {
                    self.found_fields.push((node, ast.get(at).span));
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
        }
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
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) => {}
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
            | ExprKind::As { expr, .. } => self.expr(expr),
            ExprKind::Name(name) => self.name(name, ast.get(id).span),
            ExprKind::Block { items, .. } => self.block(items),
            ExprKind::Fn(function) => self.function(function),
            ExprKind::Lazy(function) => self.expr(ast.fn_decl(function).body),
            ExprKind::Call { callee, args } => {
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
        }
    }

    /// Walks a function expression, whose parameters shadow names in its
    /// body. Kept out of [`Walker::expr`], so that the frame each level of
    /// nesting takes on the native stack stays small.
    #[inline(never)]
    fn function(&mut self, function: FnId) {
        let decl = *self.ast.fn_decl(function);
        let depth = self.shadows.len();
        self.shadows
            .extend(self.ast.params(decl.params).iter().map(|param| param.name));
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
                    let name = found.functions[index as usize].name;
                    ast.names().get(name).to_owned()
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
