//! Scheduling the checking of top-level bodies: when a function may be used,
//! which bodies are ready, and the groups of bodies checked together.
//!
//! # When a function may be used
//!
//! Top-level code (the initialiser of a `let` or a field, a spread, a `:=`,
//! a statement, the result) can only use a function that exists by then,
//! and a function exists once it has copied what it reads. From the
//! captures its checked body resolved (not from the tree), each top-level
//! function is made ([`MadeAt`]):
//!
//! - at the start of the input, if it copies nothing from this input's top
//!   level (what it copies from an earlier input is bound before the input
//!   starts, and does not count);
//! - just after the item that binds the last of what it copies, if it
//!   copies only variables that are not mutable, and fields: the latest
//!   definition of each name above the function;
//! - at its own declaration, if it copies a mutable variable, whose value
//!   it copies there, as any function does.
//!
//! A function nested in a top-level function, or a `lazy` in it, that
//! reads a top-level variable makes the top-level function copy it. A
//! top-level function *needs* every function it mentions, and what those
//! need, and so on. A top-level item that mentions a top-level function
//! anywhere in it, even inside a function expression or a `lazy` it only
//! builds, is "`f` cannot be used here yet" unless every function `f`
//! needs exists before the item starts (`Checker::check_uses`). The label
//! names the reason first in source order and the shortest chain to it:
//! "`f` reads `k`, which is bound here", "`a` uses `b`, and `b` reads `k`,
//! which is bound here", up to three names, and "`a` uses `b`, ..., and
//! `z` reads" beyond; for a mutable variable it is on the function that
//! copies it, "`b` reads the mutable variable `n`, so it exists from here";
//! for a read blocked by a spread, "`g` reads `port`, which this spread may
//! define". There is one error per item and function. Inside function
//! bodies there is no such check: the rule, applied where top-level code
//! mentions the function whose body it is, covers what it calls. The rule
//! asks what is mentioned, not what is called, which is conservative, and
//! deliberately so.
//!
//! The "this is never run" warning neither starts at nor counts a
//! top-level function, since its closure may be made before the item that
//! never completes.

use std::collections::HashSet;

use crate::ast::ItemKind;
use crate::diagnostic::Diagnostic;
use crate::mentions::Need;
use crate::source::Span;
use crate::types::TyId;

use super::{BindingKind, Blocker, BlockerKind, BodyState, Checker, MadeAt, Slot, TopFn, position};

impl Checker<'_> {
    /// Before the walk checks the top-level item at `position` in the
    /// outermost block, once the bodies it needs are checked: "`f` cannot
    /// be used here yet" for each top-level function or field it mentions
    /// that needs a function that does not exist, or a leaf that cannot be
    /// computed, before the item (see [When a function may be
    /// used](#when-a-function-may-be-used) and [Top-level
    /// fields](crate::check::toplevel#top-level-fields)). A field item is held to it only when its
    /// leaf is computed at its own item: one computed earlier is the item's
    /// that needed it.
    pub(super) fn check_uses(&mut self, position: usize) {
        if self.computed_early(position) {
            return;
        }
        let at = |list: &[Vec<u32>], spans: &[Vec<Span>]| {
            let list = list.get(position).map_or(&[][..], Vec::as_slice);
            let spans = spans.get(position).map_or(&[][..], Vec::as_slice);
            list.iter()
                .copied()
                .zip(spans.iter().copied())
                .collect::<Vec<_>>()
        };
        let mut mentioned: Vec<(Need, Span)> = at(&self.mentions.items, &self.mentions.spans)
            .into_iter()
            .map(|(function, span)| (Need::Function(function), span))
            .chain(
                at(&self.mentions.fields, &self.mentions.field_spans)
                    .into_iter()
                    .map(|(node, span)| (Need::Field(node), span)),
            )
            .collect();
        mentioned.sort_by_key(|&(_, span)| span.start);
        let mut errors = Vec::new();
        for (need, span) in mentioned {
            if self.closure_ready(need) > position
                && let Some(error) = self.not_ready(need, position, span)
            {
                errors.push(error);
            }
        }
        self.errors.extend(errors);
    }

    /// The latest item before which something `start` needs (itself, or
    /// what it mentions or has, and so on, not through a node on a cycle)
    /// cannot be had, 0 if nothing it needs is ever blocked; remembered in
    /// `closure_ready`. Everything `start` needs is checked by the time the
    /// walk reaches an item that mentions it, so the answer never changes.
    /// The search reuses what earlier searches found, so each item of a
    /// chain of fields adds one step; only a mention that may be an error
    /// is searched in full, by `Checker::not_ready`.
    pub(super) fn closure_ready(&mut self, start: Need) -> usize {
        let index = self.need_index(start);
        if let Some(&Some(known)) = self.closure_ready.get(index) {
            return known;
        }
        let mut seen = HashSet::from([index]);
        let mut stack = vec![start];
        let mut latest = 0;
        while let Some(need) = stack.pop() {
            let at = self.need_index(need);
            if at != index
                && let Some(&Some(known)) = self.closure_ready.get(at)
            {
                latest = latest.max(known);
                continue;
            }
            let (ready, blocked) = match need {
                Need::Function(function) => self
                    .top_fns
                    .get(function as usize)
                    .map_or((0, false), |top| (top.ready, top.blocker.is_some())),
                Need::Field(node) => self
                    .field_cells
                    .get(node as usize)
                    .map_or((0, false), |cell| (cell.ready, cell.blocker.is_some())),
            };
            if self.on_cycle(need) {
                continue;
            }
            if blocked {
                latest = latest.max(ready);
            }
            for (next, _) in self.mentions.needs(need) {
                if seen.insert(self.need_index(next)) {
                    stack.push(next);
                }
            }
        }
        if let Some(entry) = self.closure_ready.get_mut(index) {
            *entry = Some(latest);
        }
        latest
    }

    /// Whether the item at `position` of the outermost block, of `kind`,
    /// does not run where it is written: a top-level field whose leaf is
    /// computed before its own item ([`Checker::computed_early`]), or a
    /// declaration, a `type`, an `enum` or a context without a default,
    /// which never runs.
    pub(super) fn runs_elsewhere(&self, position: usize, kind: ItemKind) -> bool {
        matches!(
            kind,
            ItemKind::Type { .. }
                | ItemKind::Enum { .. }
                | ItemKind::Variant { .. }
                | ItemKind::Context { .. }
        ) || self.computed_early(position)
    }

    /// Whether the item at `position` of the outermost block is a top-level
    /// field whose leaf is computed before its own item.
    pub(super) fn computed_early(&self, position: usize) -> bool {
        self.mentions
            .tree
            .item_leaf
            .get(position)
            .copied()
            .flatten()
            .and_then(|leaf| self.schedule.point.get(leaf as usize))
            .is_some_and(|&point| point < position)
    }

    /// The index of `need` among the nodes of the graph of what needs what:
    /// the field nodes, then the top-level functions.
    pub(super) fn need_index(&self, need: Need) -> usize {
        match need {
            Need::Field(node) => node as usize,
            Need::Function(function) => self.mentions.node_count() + function as usize,
        }
    }

    /// The name of `need` in a diagnostic: a function's name, or its path
    /// if it is a `def` named by a dotted path, or a field node's full path.
    pub(super) fn need_text(&self, need: Need) -> String {
        let names = self.ast.names();
        match need {
            Need::Function(function) => {
                let Some(top) = self.top_fns.get(function as usize) else {
                    return String::new();
                };
                match (top.name, self.ast.item(top.item).kind) {
                    (Some(name), _) => names.get(name).to_string(),
                    (None, ItemKind::Field { path, .. }) => {
                        self.ast.show_path(self.ast.path(path)).to_string()
                    }
                    // A definition's meta, as `defmeta`, its def-type and its
                    // path.
                    (None, ItemKind::DefMeta { def, .. }) => self
                        .ast
                        .def_type(def)
                        .meta
                        .map(|(key, leaf)| crate::names::show_path(names, &[key, leaf]))
                        .unwrap_or_default(),
                    // A definition made with a def-type, as its def-type and
                    // its path are written.
                    (None, ItemKind::DefTypeDef { def, .. }) => {
                        let (shown, path) = self.ast.show_def_type(def);
                        format!("{shown} {path}")
                    }
                    (None, _) => String::new(),
                }
            }
            Need::Field(node) => {
                let path: Vec<&str> = self
                    .mentions
                    .path(node)
                    .into_iter()
                    .map(|name| names.get(name))
                    .collect();
                path.join(".")
            }
        }
    }

    /// Why `need` cannot be had before the item at `position`, if it
    /// cannot: a function whose closure is not made by then, or a leaf
    /// that cannot be computed by then. A node on a cycle, already
    /// reported, is not reported again.
    pub(super) fn blocker_of(&self, need: Need, position: usize) -> Option<Blocker> {
        let (ready, blocker) = match need {
            Need::Function(function) => {
                let top = self.top_fns.get(function as usize)?;
                (top.ready, top.blocker)
            }
            Need::Field(node) => {
                let cell = self.field_cells.get(node as usize)?;
                (cell.ready, cell.blocker)
            }
        };
        blocker.filter(|_| ready > position && !self.on_cycle(need))
    }

    /// Whether `need` is on a cycle through fields, already reported.
    pub(super) fn on_cycle(&self, need: Need) -> bool {
        match need {
            Need::Field(node) => self
                .field_cells
                .get(node as usize)
                .is_some_and(|cell| cell.cyclic),
            Need::Function(function) => self.cyclic_fns.contains(&function),
        }
    }

    /// The error for a mention at `span`, by the item at `position`, of the
    /// top-level function or field node `start`, if something it needs
    /// (itself, or what it mentions or has, and so on) does not exist
    /// before the item: the reason first in source order, with the chain of
    /// uses that leads to it.
    pub(super) fn not_ready(&self, start: Need, position: usize, span: Span) -> Option<Diagnostic> {
        // Breadth first, so the chain to each node is a shortest one.
        let count = self.mentions.node_count() + self.top_fns.len();
        let mut parent: Vec<Option<(Need, bool)>> = vec![None; count];
        *parent.get_mut(self.need_index(start))? = Some((start, false));
        let (mut queue, mut head) = (vec![start], 0);
        let mut culprit: Option<(Need, Blocker)> = None;
        while let Some(&need) = queue.get(head) {
            head += 1;
            if let Some(blocker) = self.blocker_of(need, position)
                && culprit.is_none_or(|(_, first)| blocker.span.start < first.span.start)
            {
                culprit = Some((need, blocker));
            }
            if self.on_cycle(need) {
                continue;
            }
            for (next, at) in self.mentions.needs(need) {
                if let Some(entry) = parent.get_mut(self.need_index(next))
                    && entry.is_none()
                {
                    *entry = Some((need, at.is_none()));
                    queue.push(next);
                }
            }
        }
        let (culprit, blocker) = culprit?;
        // The chain from `start`, each step with whether it is a "has":
        // a run of those is said once, from its first node to its last.
        let mut chain = vec![(culprit, false)];
        while let Some(&(last, _)) = chain.last()
            && last != start
        {
            let (up, has) = (*parent.get(self.need_index(last))?)?;
            if let Some(entry) = chain.last_mut() {
                entry.1 = has;
            }
            chain.push((up, false));
        }
        chain.reverse();
        // Each step's verb now belongs to the node it leaves.
        let mut steps: Vec<(Need, bool)> = Vec::with_capacity(chain.len());
        for window in chain.windows(2) {
            let ((from, _), (_, has)) = (window[0], window[1]);
            steps.push((from, has));
        }
        let mut collapsed: Vec<(Need, bool)> = Vec::with_capacity(steps.len());
        for (from, has) in steps {
            if has && collapsed.last().is_some_and(|&(_, previous)| previous) {
                continue;
            }
            collapsed.push((from, has));
        }
        let text = |need: Need| self.need_text(need);
        let verb = |has: bool| if has { "has" } else { "uses" };
        let uses = match collapsed.as_slice() {
            [] => format!("`{}`", text(culprit)),
            [(first, has), (second, _), ..] if collapsed.len() > 2 => format!(
                "`{}` {} `{}`, ..., and `{}`",
                text(*first),
                verb(*has),
                text(*second),
                text(culprit)
            ),
            _ => {
                let mut said: Vec<String> = Vec::with_capacity(collapsed.len());
                for (index, &(from, has)) in collapsed.iter().enumerate() {
                    let to = collapsed.get(index + 1).map_or(culprit, |&(to, _)| to);
                    said.push(format!("`{}` {} `{}`", text(from), verb(has), text(to)));
                }
                format!("{}, and `{}`", said.join(", "), text(culprit))
            }
        };
        let read = self.ast.names().get(blocker.name);
        let field = matches!(culprit, Need::Field(_));
        let bound = if field {
            "a field can be computed once every variable it reads is bound"
        } else {
            "a function can be used once every variable it reads is bound"
        };
        let (label, note) = match blocker.kind {
            BlockerKind::Bound => (format!("{uses} reads `{read}`, which is bound here"), bound),
            BlockerKind::Spread => (
                format!("{uses} reads `{read}`, which this spread may define"),
                bound,
            ),
            BlockerKind::Mutable if field => (
                format!("{uses} reads the mutable variable `{read}`, so it is computed here"),
                "a field that reads a mutable variable is computed where it is written",
            ),
            BlockerKind::Mutable => (
                format!("{uses} reads the mutable variable `{read}`, so it exists from here"),
                "a function copies a mutable variable where the function is written",
            ),
        };
        Some(
            Diagnostic::error(format!("`{}` cannot be used here yet", text(start)), span)
                .with_context(blocker.span, label)
                .with_note(note),
        )
    }

    /// Before the walk checks the top-level item at `position` in the
    /// outermost block: checks the body of every top-level function the
    /// item mentions that is not checked yet, with the groups they mention
    /// first.
    pub(super) fn check_mentioned(&mut self, position: usize) {
        let Some(mentioned) = self.mentions.items.get(position).cloned() else {
            return;
        };
        for function in mentioned {
            if let Some(&group) = self.mentions.group_of.get(function as usize) {
                self.check_group(group);
            }
        }
    }

    /// After the last item of the outermost block: checks the bodies not
    /// checked yet, groups callee first, and gives each `def`'s field its
    /// function's type.
    pub(super) fn check_remaining_bodies(&mut self) {
        for group in 0..self.mentions.groups.len() {
            self.check_group(position(group));
        }
        for top in &self.top_fns {
            let Some(binding) = self.bindings.get(top.binding) else {
                continue;
            };
            if let Some(field) = self.fields[self.fields_start..]
                .iter_mut()
                .find(|field| field.slot == binding.slot)
            {
                field.ty = binding.ty;
            }
        }
    }

    /// Checks the bodies of the group `group` of top-level functions, if
    /// they are not checked yet: first those of the groups it mentions,
    /// then its members', in source order, each exactly once. A use of a
    /// member inside the group sees its declared type; once the group is
    /// checked, every member's binding has the type its body gives it and
    /// its function, as a function declared above its users' has.
    ///
    /// The walk keeps its own stack of the groups being entered, each with
    /// the callee groups still to visit, so that a long chain of functions
    /// does not recurse on the native stack.
    pub(super) fn check_group(&mut self, group: u32) {
        let mut work: Vec<(u32, Vec<u32>, usize)> = Vec::new();
        if let Some(callees) = self.enter_group(group) {
            work.push((group, callees, 0));
        }
        while let Some((group, callees, next)) = work.last_mut() {
            if let Some(&callee) = callees.get(*next) {
                *next += 1;
                if let Some(callees) = self.enter_group(callee) {
                    work.push((callee, callees, 0));
                }
                continue;
            }
            let group = *group;
            work.pop();
            self.finish_group(group);
        }
    }

    /// Starts checking the group `group`, if none of its members' bodies is
    /// checked or being checked: marks them as being checked and returns
    /// the other groups its members mention, in the order they mention
    /// them, which must be checked first.
    pub(super) fn enter_group(&mut self, group: u32) -> Option<Vec<u32>> {
        let members = self.mentions.groups.get(group as usize)?.clone();
        if members.iter().any(|&member| {
            self.top_fns
                .get(member as usize)
                .is_none_or(|top| top.state != BodyState::Unchecked)
        }) {
            return None;
        }
        let mut callee_groups = Vec::new();
        for &member in &members {
            if let Some(top) = self.top_fns.get_mut(member as usize) {
                top.state = BodyState::Checking;
            }
            let position = self
                .mentions
                .functions
                .get(member as usize)
                .map_or(usize::MAX, |top| top.position);
            for &callee in self
                .mentions
                .items
                .get(position)
                .map_or(&[][..], Vec::as_slice)
            {
                match self.mentions.group_of.get(callee as usize) {
                    Some(&callee_group) if callee_group != group => {
                        callee_groups.push(callee_group);
                    }
                    _ => {}
                }
            }
        }
        Some(callee_groups)
    }

    /// Checks the bodies of the group `group`, whose callee groups are
    /// checked, members in source order, and gives each member's binding
    /// the type and function its body gives it.
    pub(super) fn finish_group(&mut self, group: u32) {
        let Some(members) = self.mentions.groups.get(group as usize).cloned() else {
            return;
        };
        for &member in &members {
            self.check_top_body(member as usize);
        }
        for &member in &members {
            let Some(top) = self.top_fns.get_mut(member as usize) else {
                continue;
            };
            top.state = BodyState::Done;
            // A function whose body read a name it cannot see yet is
            // erroneous for its users.
            let (ty, function) = match top.ready {
                usize::MAX => (TyId::ERROR, None),
                _ => (self.expr_types[top.expr.index()], Some(top.function)),
            };
            let (leaf, node) = (top.leaf, top.node);
            if let Some(binding) = self.bindings.get_mut(top.binding) {
                binding.ty = ty;
                binding.function = function;
            }
            // A `def` named by a dotted path: its cell, the nodes computed
            // above it and its node of the input's record take the type.
            if let Some(leaf) = leaf
                && let Some(cell) = self.field_cells.get_mut(leaf as usize)
                && cell.ty != Some(ty)
            {
                cell.ty = Some(ty);
                self.retype_above(leaf);
            }
            if let Some(entry) = self.nodes.get_mut(node as usize) {
                entry.ty = ty;
            }
        }
    }

    /// Checks the body of the top-level function `index` in the view of
    /// the top level it sees: the bindings above its own declaration, or,
    /// before the walk has reached it, above the item being checked. The
    /// bindings below are set aside while it is checked, which is cheap,
    /// since they are a suffix of `bindings`.
    pub(super) fn check_top_body(&mut self, index: usize) {
        let Some(top) = self.top_fns.get(index).copied() else {
            return;
        };
        let cut = top
            .cut
            .unwrap_or(self.bindings.len())
            .max(self.top_end)
            .min(self.bindings.len());
        let own = self
            .mentions
            .functions
            .get(index)
            .map_or(usize::MAX, |function| function.position);
        self.early = top.cut.is_none().then_some((self.root_position, own));
        self.checking_top = Some(index);
        let below = self.bindings.split_off(cut);
        // A `def` named by a dotted path sees itself by its path only.
        self.callee_name = top.name;
        let found = self.check_expr(top.expr);
        self.settle(top.expr, found);
        self.callee_name = None;
        self.bindings.extend(below);
        (self.early, self.checking_top) = (None, None);
        self.settle_top(index, own);
    }

    /// Once the body of the top-level function `index`, declared at the
    /// position `own` among the outermost block's items, is checked: works
    /// out from what it copies where its closure is made, and so before
    /// which item it exists (see [When a function may be
    /// used](#when-a-function-may-be-used)). What it copies from an
    /// earlier input does not count.
    pub(super) fn settle_top(&mut self, index: usize, own: usize) {
        let copies = self
            .top_copies
            .get(index)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let bindings = copies
            .iter()
            .filter(|&&binding| binding >= self.top_start)
            .filter_map(|&binding| self.bindings.get(binding).copied());
        let blocked = self
            .top_fns
            .get(index)
            .is_some_and(|top| top.blocker.is_some());
        let (mut ready, mut why, mut made) = (0, None, MadeAt::Start);
        if blocked {
            ready = usize::MAX;
            made = MadeAt::Own;
        } else if let Some(mutable) = bindings
            .clone()
            .find(|binding| binding.kind == BindingKind::Mutable)
        {
            let span = self
                .top_fns
                .get(index)
                .map(|top| self.ast.item(top.item).span);
            ready = own.saturating_add(1);
            made = MadeAt::Own;
            why = span.map(|span| Blocker {
                name: mutable.name,
                span,
                kind: BlockerKind::Mutable,
            });
        } else {
            let root = self.ast.items(self.ast.root());
            for binding in bindings {
                let Some(declared) = binding.declared else {
                    continue;
                };
                let Some((position, &item)) = root.iter().enumerate().find(|&(_, &id)| {
                    let span = self.ast.item(id).span;
                    (span.start..span.end).contains(&declared.start)
                }) else {
                    continue;
                };
                if position + 1 > ready {
                    ready = position + 1;
                    made = MadeAt::After(item);
                    why = Some(Blocker {
                        name: binding.name,
                        span: declared,
                        kind: BlockerKind::Bound,
                    });
                }
            }
        }
        if let Some(top) = self.top_fns.get_mut(index) {
            top.ready = ready;
            top.made = made;
            top.blocker = top.blocker.or(why);
        }
    }

    /// The top-level function whose binding, or for a `def` named by a
    /// dotted path whose leaf's cell, is in `slot`, if one of this input's
    /// is.
    pub(super) fn top_fn_in(&self, slot: Slot) -> Option<&TopFn> {
        self.top_fns.iter().find(|top| {
            let cell = top
                .leaf
                .and_then(|leaf| self.field_cells.get(leaf as usize))
                .map(|cell| cell.slot);
            cell.or_else(|| self.bindings.get(top.binding).map(|binding| binding.slot))
                == Some(slot)
        })
    }
}
