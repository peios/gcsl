//! A file's scope as the scan resolves it: the names it sees, by its imports,
//! its directory and the standard library.

use std::collections::HashMap;

use crate::mentions::{self, FieldTree, Resolve};
use crate::names::Name;

use super::{File, Merged, ROOT, Scan, Target, View, ViewPart, count};

/// The scope a file's references are resolved in: its view, the merged trees of
/// its directory chain and what it imports (see the module documentation, "The
/// view of a file"), and the `def`s of its chain, numbered as
/// [`crate::mentions::Resolve`] asks. The file's own field nodes come first, by
/// their own numbers, then the nodes of its view with a leaf of the file under
/// them ([`Mentions::views`](crate::mentions::Mentions::views)), then every
/// merged node at its index in [`Scan::nodes`] after those, then every target
/// an import or the view reaches (a [`Target::View`] among them); its own
/// functions come first, then every other one at its index in
/// [`Scan::functions`] after them.
#[derive(Debug)]
pub struct FileScope<'a> {
    /// The workspace.
    pub(super) scan: &'a Scan,
    /// The file's directory.
    pub(super) dir: u32,
    /// The file's own functions.
    pub(super) own: &'a [mentions::TopLevel],
    /// The file's own tree of top-level fields.
    pub(super) tree: &'a FieldTree,
    /// For each of the file's own nodes, the merged node of its path.
    pub(super) global: &'a [u32],
    /// The file's own node of each merged node it has one of.
    pub(super) local: HashMap<u32, u32>,
    /// What each name the file imports names; of a name several sources
    /// have, the view's node says.
    pub(super) aliases: HashMap<Name, Target>,
    /// The names an import binds that is shadowable, the implicit `import *
    /// from prelude`'s among them ([`FileScope::yields`]).
    pub(super) shadowable: Vec<Name>,
    /// The names of the imports a session's input inherits from earlier
    /// inputs ([`FileScope::inherited`]).
    pub(super) inherited: Vec<Name>,
    /// The file's view.
    pub(super) view: &'a View,
}

impl<'a> FileScope<'a> {
    /// The scope of `file` of `scan`.
    #[must_use]
    pub fn new(scan: &'a Scan, file: &'a File) -> Self {
        let local = (0u32..)
            .zip(&file.global)
            .map(|(own, &global)| (global, own))
            .collect();
        let yielding =
            |a: &super::Alias| file.marked(a) || (!a.whole && scan.target_shadowable(a.target));
        // A shadowable import's name never displaces another import's, an
        // earlier session input's among them, whatever their order.
        let mut aliases: HashMap<Name, Target> = file
            .aliases
            .iter()
            .filter(|a| !yielding(a))
            .map(|a| (a.name, a.target))
            .collect();
        let mut shadowable = Vec::new();
        for alias in file.aliases.iter().filter(|a| yielding(a)) {
            if let std::collections::hash_map::Entry::Vacant(entry) = aliases.entry(alias.name) {
                entry.insert(alias.target);
                shadowable.push(alias.name);
            }
        }
        Self {
            scan,
            dir: file.dir,
            own: &file.mentions.functions,
            tree: &file.mentions.tree,
            global: &file.global,
            local,
            aliases,
            shadowable,
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
    /// and clashes with nothing (see [`inherit_imports`](super::imports::inherit_imports)).
    #[must_use]
    pub fn inherited(&self, name: Name) -> bool {
        self.inherited.contains(&name)
    }

    /// Whether what the root `name` means in the file's view is shadowable
    /// (see the module documentation, "Shadowable declarations"): the
    /// prelude's names that nothing else has among them. A binding the file
    /// declares of its own of the name, a top-level `let` or `fn`, shadows
    /// it, as an earlier session input's variable does, and the clash family
    /// ignores it.
    #[must_use]
    pub fn yields(&self, name: Name) -> bool {
        if let Some(&merged) = self.view.merged.get(&(ROOT, name)) {
            return match merged {
                Merged::One(part) => self.scan.part_shadowable(part),
                Merged::View(view) => self.scan.target_shadowable(Target::View(view)),
            };
        }
        if let Some(&node) = self
            .scan
            .chain(self.dir)
            .find_map(|dir| self.scan.by_name.get(&(dir, ROOT, name)))
        {
            return self.scan.target_shadowable(Target::Node(node));
        }
        self.shadowable.contains(&name)
    }

    /// Where the numbers of the merged nodes start: after the file's own
    /// nodes and its view's own nodes.
    pub(super) fn merged_start(&self) -> u32 {
        count(self.global.len()).saturating_add(self.view.own)
    }

    /// Where the numbers of targets reached through imports or the view
    /// start: after the merged nodes.
    pub(super) fn targets_start(&self) -> u32 {
        self.merged_start()
            .saturating_add(count(self.scan.nodes.len()))
    }

    /// What the scope's number `node` is when it is reached through an import
    /// or a node of the view without a leaf of the file under it, `None` for
    /// the file's own nodes, its view's own nodes and the merged nodes of its
    /// directory chain ([`FileScope::merged`]). A reference through an import
    /// resolves to one of these, and so does a mention of it in
    /// [`Mentions::foreign`](crate::mentions::Mentions::foreign); a
    /// [`Target::Node`] is a cell like any merged node, read by its slot.
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
    pub(super) fn target_number(&self, target: Target) -> u32 {
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
    pub(super) fn number(&self, node: u32) -> u32 {
        self.local
            .get(&node)
            .copied()
            .unwrap_or_else(|| self.merged_start().saturating_add(node))
    }

    /// The scope's number of what the view makes of a name.
    pub(super) fn merged_number(&self, merged: Merged) -> u32 {
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

    /// Whether the scope's number `node` is a leaf of the view: the file's
    /// own leaf, one of a directory of its chain, or one an import reaches.
    #[must_use]
    pub fn is_leaf(&self, node: u32) -> bool {
        if self.own_view(node).is_some() {
            return false;
        }
        let merged = self.merged(node).or(match self.target(node) {
            Some(Target::Node(merged)) => Some(merged),
            _ => None,
        });
        merged
            .and_then(|merged| self.scan.nodes.get(merged as usize))
            .is_some_and(|entry| entry.leaf.is_some())
    }

    /// The children of the scope's number `node` in the view, each with its
    /// name and its scope's number, in the view's order: none for a leaf.
    #[must_use]
    pub fn children(&self, node: u32) -> Vec<(Name, u32)> {
        if self.own_view(node).is_some() || matches!(self.target(node), Some(Target::View(_))) {
            return self.view_children(node);
        }
        let names = match (self.merged(node), self.target(node)) {
            (Some(merged), _) => self.scan.child_names(Target::Node(merged)),
            (None, Some(target)) => self.scan.child_names(target),
            (None, None) => Vec::new(),
        };
        names
            .into_iter()
            .filter_map(|name| Some((name, self.child(node, name)?)))
            .collect()
    }

    /// The file whose leaf the scope's number `node` is, with the file's own
    /// node of it, if it is a leaf of the view.
    pub(super) fn leaf_owner(&self, node: u32) -> Option<(u32, u32)> {
        let merged = self.merged(node).or(match self.target(node) {
            Some(Target::Node(merged)) => Some(merged),
            _ => None,
        })?;
        let owner = self.scan.nodes.get(merged as usize)?.leaf?;
        let own = self
            .scan
            .files
            .get(owner as usize)?
            .global
            .iter()
            .position(|&g| g == merged)?;
        Some((owner, count(own)))
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
        if let Some(own) = self.own.iter().position(|f| f.name == Some(name)) {
            return Some(count(own));
        }
        let def = self
            .scan
            .chain(self.dir)
            .find_map(|dir| self.scan.defs.get(&(dir, name)))?;
        Some(count(self.own.len()).saturating_add(*def))
    }

    fn dotted_defs_under(&self, node: u32, each: &mut dyn FnMut(u32)) {
        // The file's own node, or the file's own node of its view's node:
        // another file's functions are no mentions of this file's.
        let own = match self.own_view(node) {
            Some(view) => self.scan.views.get(view as usize).and_then(|v| v.own),
            None => (node < count(self.global.len())).then_some(node),
        };
        if let Some(own) = own {
            self.tree.dotted_defs_under(own, each);
        }
    }

    fn own(&self) -> (u32, u32) {
        (self.merged_start(), count(self.own.len()))
    }
}
