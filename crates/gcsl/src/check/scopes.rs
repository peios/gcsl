//! Scopes and slots: declaring and binding names, looking them up, and the
//! clashes among the names of a scope.
//!
//! # Scopes
//!
//! Every block, including the file's own body, is a scope, and so is the
//! scope a loop declares its variable, or its header's items, in (see
//! [For](crate::check::loops#for) and [Loops](crate::check::loops#loops)). A `let` declares
//! a variable visible from the next item to the end of its block; a later
//! `let` of the same name shadows it, in the same block or an inner one. A
//! `:=` is a new declaration too, allowed only for a name already declared
//! in the same block, and not for a field; its value must fit the
//! variable's type (a subtype, or an adapting literal), and the new binding
//! keeps that type, so `:=` reads as assignment: a `let` is needed to
//! change a variable's type. A `:=` of a mutable variable, a `let mut`'s, is
//! allowed from any block of the variable's function, and declares
//! nothing: it is an assignment to the variable, which keeps its type (see
//! [Mutable variables](crate::check::mutation#mutable-variables)).
//!
//! The scopes in effect form a stack, kept as one flat vector of bindings
//! with the innermost block's bindings last: entering a block records where
//! its bindings start, and leaving it truncates the vector back. Lookup
//! searches the vector from the end, which visits the innermost block first
//! and, within a block, the latest declaration first, so shadowing needs no
//! extra work. Blocks are short, so a linear search beats a hash map, and
//! the one vector means entering a block allocates nothing.
//!
//! # Slots
//!
//! Every variable and field is stored in a [`Slot`] of its function's flat
//! frame (the top level's, outside functions), assigned stack-fashion, as
//! in Lua and clox: entering a block, or a `for`'s scope, records the next
//! free slot; each declaration (a parameter, a `let`, a field, a `for`'s
//! variable, and `:=` of a variable that is not mutable, which is a new
//! binding) takes the next free slot; leaving the block restores the next
//! free slot to what it was on entry, so the slots of a finished block are
//! reused by whatever comes after it. A `:=` of a mutable variable takes
//! none: it writes the variable's one slot, from whatever block it is in,
//! all of which end before the variable's own block does, so the slot is
//! never reused while the variable is in scope. The highest slot count
//! reached is the
//! frame size. A variable's slot is therefore known statically and never
//! changes, and evaluation needs no environment lookups and no allocation
//! per block. Some items share a slot: each definition of a field after its
//! first, a dotted path extending an open field or an item overriding a
//! soft one, writes the field's one slot. The top-level functions of an
//! input take its first slots, in source order, before the walk starts
//! (see [Top-level functions](crate::check::toplevel#top-level-functions)); a `def` among them is
//! a field whose slot is the function's. The input's top-level field roots
//! take the slots after them, and every other node of the tree of its
//! top-level fields a slot after those, its cell, which no name finds (see
//! [Top-level fields](crate::check::toplevel#top-level-fields)); so a scope's next free slot is
//! not always its bindings' count, and [`Scope`](super::Scope) records it. A spread's
//! operand is held in the next free slot,
//! which counts towards the frame size but stays free (see [Spreads and
//! soft assignment](crate::check::records#spreads-and-soft-assignment)). An `if` that narrows
//! takes the next free slot for the value its test writes, which both
//! branches' narrowed bindings share, and leaves it when it ends (see
//! [Narrowing](crate::check::narrowing#narrowing)).

use crate::ast::{Ast, ExprId, FnId, ItemId, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;

use super::{
    Binding, BindingKind, Capture, Checker, Record, RecordedDecl, Slot, VarRef, position,
    root_declares,
};

impl Checker<'_> {
    /// Checks `value`, the value of a field item that the rules of dotted
    /// fields reject, on its own, unless it is a top-level function's,
    /// whose body its group checks, exactly once.
    pub(super) fn check_rejected(&mut self, value: ExprId) {
        if !self.top_fns.iter().any(|top| top.expr == value) {
            self.check_alone(value);
        }
    }

    /// Where the item `id`'s name is written: the first segment of its
    /// path, or the item, if it has none.
    pub(super) fn name_span(&self, id: ItemId) -> Span {
        let item = self.ast.item(id);
        match item.kind {
            ItemKind::Let { path, .. }
            | ItemKind::Field { path, .. }
            | ItemKind::Rebind { path, .. } => self
                .ast
                .path(path)
                .first()
                .map_or(item.span, |segment| segment.span),
            _ => item.span,
        }
    }

    /// When the walk reaches the top-level function `id`, named `name`:
    /// "function `f` is declared twice" if a top-level function above it
    /// has its name, unless both are `def`s, which are fields defined
    /// twice.
    pub(super) fn check_declared_twice(&mut self, id: ItemId, name: Name) {
        let is_def =
            |ast: &Ast, item: ItemId| matches!(ast.item(item).kind, ItemKind::Field { .. });
        let Some(first) = self
            .top_fns
            .iter()
            .take_while(|top| top.item != id)
            .find(|top| top.name == Some(name))
            .map(|top| top.item)
        else {
            return;
        };
        if is_def(self.ast, first) && is_def(self.ast, id) {
            return;
        }
        let text = self.ast.names().get(name);
        self.errors.push(
            Diagnostic::error(
                format!("function `{text}` is declared twice"),
                self.name_span(id),
            )
            .with_context(self.name_span(first), "first declared here"),
        );
    }

    /// Before the walk checks the top-level item `id`, which is not a
    /// top-level function: "`f` is a function of this file" if it is a
    /// `let`, `let mut`, field or `:=` whose name is a top-level function's,
    /// unless it is a field and the function a `def`, which is a field
    /// defined twice.
    pub(super) fn check_clash(&mut self, id: ItemId) {
        let Some(name) = root_declares(self.ast, id) else {
            return;
        };
        self.check_name_clash(id, name);
    }

    /// [`Checker::check_clash`] for the item `id`, declaring `name`: a
    /// top-level item that is no top-level function, or a `def` named by a
    /// dotted path, whose path's first segment `name` is, as a dotted
    /// field's is (a function named by a dotted path has no name to clash).
    pub(super) fn check_name_clash(&mut self, id: ItemId, name: Name) {
        let Some(function) = self
            .top_fns
            .iter()
            .find(|top| top.name == Some(name))
            .map(|top| top.item)
        else {
            self.check_field_clash(id, name);
            return;
        };
        let field = |item: ItemId| matches!(self.ast.item(item).kind, ItemKind::Field { .. });
        if field(id) && field(function) {
            return;
        }
        // A `fn` shadows a shadowable field silently, in the whole input
        // (see [Shadowed declarations](crate::check::items#shadowed-declarations)).
        if field(id) && self.shadowable_root(name) {
            return;
        }
        let text = self.ast.names().get(name);
        self.errors.push(
            Diagnostic::error(
                format!("`{text}` is a function of this file"),
                self.name_span(id),
            )
            .with_context(self.name_span(function), "declared here")
            .with_note(
                "a function declared at the top level is visible in the whole file, so its \
                 name cannot also be a variable or a field",
            ),
        );
    }

    /// The names of the slots a later input of a session must not take, the
    /// [`Scope`](super::Scope)'s: every binding's slot and every slot below
    /// `top_slots_end`, each named by the last binding in it, or as the
    /// checker named it. A slot that no binding owns must have been named in
    /// `slot_names` for the scope to cover it.
    pub(super) fn scope_slot_names(&self) -> Vec<Name> {
        let top_end = self
            .bindings
            .iter()
            .map(|binding| binding.slot.index().saturating_add(1))
            .chain(std::iter::once(self.top_slots_end as usize))
            .max()
            .unwrap_or_default();
        let mut names = self.slot_names.clone();
        names.truncate(top_end);
        for binding in &self.bindings {
            if let Some(entry) = names.get_mut(binding.slot.index()) {
                *entry = binding.name;
            }
        }
        names
    }

    /// Before the walk checks the top-level item `id`, a `let`, `let mut`,
    /// field or `:=` declaring or rebinding `name`, which no top-level
    /// function has: "`a` is a field of this file" if it is not a field item
    /// and `name` is the root of a top-level field of the input (see
    /// [Top-level fields](crate::check::toplevel#top-level-fields)), labelled on the field's first
    /// item, wherever it is. Field items of one root are the dotted-field
    /// rules' to judge.
    pub(super) fn check_field_clash(&mut self, id: ItemId, name: Name) {
        if matches!(self.ast.item(id).kind, ItemKind::Field { .. }) {
            return;
        }
        // A `let` shadows a shadowable field silently, from its item on
        // (see [Shadowed declarations](crate::check::items#shadowed-declarations)).
        if matches!(self.ast.item(id).kind, ItemKind::Let { .. }) && self.shadowable_root(name) {
            return;
        }
        let tree = &self.mentions.tree;
        let Some(first) = tree
            .root(name)
            .and_then(|root| tree.nodes.get(root as usize))
            .and_then(|root| self.ast.items(self.ast.root()).get(root.first))
            .copied()
        else {
            self.check_directory_clash(id, name);
            return;
        };
        let text = self.ast.names().get(name);
        self.field_clash = Some(id);
        self.errors.push(
            Diagnostic::error(
                format!("`{text}` is a field of this file"),
                self.name_span(id),
            )
            .with_context(self.name_span(first), "declared here")
            .with_note(
                "a field declared at the top level is visible in the whole file, so its name \
                 cannot also be a variable",
            ),
        );
    }

    /// Declares a variable or field, as `kind` says, for the item `id`, in
    /// the innermost block, in the next free slot, which the item writes.
    pub(super) fn declare(&mut self, name: Name, ty: TyId, id: ItemId, kind: BindingKind) {
        self.declare_initialised(name, ty, id, (kind, None));
    }

    /// Declares a variable or field, as `kind` says, for the item `id`, as
    /// [`Checker::declare`] does, initialised directly with the function
    /// expression `function`, if it is `Some`.
    pub(super) fn declare_initialised(
        &mut self,
        name: Name,
        ty: TyId,
        id: ItemId,
        (kind, function): (BindingKind, Option<FnId>),
    ) {
        let span = self.ast.item(id).span;
        let slot = self.declare_binding(name, ty, Some(span), kind);
        if let Some(binding) = self.bindings.last_mut() {
            binding.function = function;
        }
        self.item_slots[id.index()] = slot;
    }

    /// Declares a binding of `kind` in the innermost block, in the next free
    /// slot of the current frame, and returns that slot. `declared` is where
    /// its declaration is written. It is initialised with no known function
    /// expression.
    pub(super) fn declare_binding(
        &mut self,
        name: Name,
        ty: TyId,
        declared: Option<Span>,
        kind: BindingKind,
    ) -> Slot {
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        self.bind(Binding {
            name,
            slot,
            ty,
            declared,
            kind,
            function: None,
            callback: None,
        });
        slot
    }

    /// Adds `binding` to the innermost block, in its slot, which is taken
    /// already: the next free slot [`Checker::declare_binding`] has just
    /// taken, or the one slot of the field it defines again.
    pub(super) fn bind(&mut self, binding: Binding) {
        match self.slot_names.get_mut(binding.slot.index()) {
            Some(entry) => *entry = binding.name,
            // Slots are taken in order, so a slot beyond the table is the
            // next one.
            None => self.slot_names.push(binding.name),
        }
        self.record_decl(binding);
        self.bindings.push(binding);
    }

    /// As the check ends, the input's value being of type `ty`: what it
    /// recorded, if it records (see [Recording](crate::check#recording)), with the
    /// tables as they are.
    pub(super) fn finish_record(&mut self, ty: TyId) -> Option<Box<Record>> {
        let mut record = self.record.take()?;
        record.types.clone_from(&self.expr_types);
        record.declared_types.clone_from(&self.declared_types);
        record.cells = self.cell_types();
        record.value = Some(ty);
        Some(record)
    }

    /// Records that the `:=` item `id` rebinds the binding declared at
    /// `replaced`, if the check records (see [Recording](crate::check#recording)) and
    /// that was declared in this input.
    pub(super) fn record_rebind(&mut self, id: ItemId, replaced: Option<Span>) {
        if let Some(record) = &mut self.record
            && let Some(replaced) = replaced
        {
            record.rebinds.push((self.ast.item(id).span, replaced));
        }
    }

    /// Records `binding` as declared, if the check records (see
    /// [Recording](crate::check#recording)) and it was declared in this input.
    pub(super) fn record_decl(&mut self, binding: Binding) {
        if let Some(record) = &mut self.record
            && let Some(declared) = binding.declared
        {
            record.decls.push(RecordedDecl {
                declared,
                name: binding.name,
                ty: binding.ty,
                kind: binding.kind,
            });
        }
    }

    /// The innermost, latest binding of `name`, with its index in
    /// `bindings`.
    pub(super) fn lookup(&self, name: Name) -> Option<(usize, Binding)> {
        let (index, binding) = self
            .bindings
            .iter()
            .enumerate()
            .rev()
            .find(|(_, binding)| binding.name == name)
            .map(|(index, &binding)| (index, binding))?;
        // A top-level function or field shadows a soft field of its name
        // that a spread of the top level defines, wherever it is read.
        let top_end = self.top_end;
        let root_end = self
            .functions
            .first()
            .map(|function| function.bindings_start)
            .into_iter()
            .chain(self.root_end)
            .min()
            .unwrap_or(self.bindings.len());
        if binding.kind == BindingKind::Field
            && (top_end..root_end).contains(&index)
            && let Some(top) = (self.top_start..top_end)
                .rev()
                .find(|&top| self.bindings.get(top).is_some_and(|b| b.name == name))
        {
            return self.bindings.get(top).map(|&binding| (top, binding));
        }
        Some((index, binding))
    }

    /// Where a use of `binding`, at `index` in `bindings`, reads its value
    /// from in the current function: its slot, or the callee, if it is the
    /// current function's or the top level's own; and otherwise a capture,
    /// which this adds to every function between the binding's home and
    /// the current one that lacks it (see the module documentation).
    pub(super) fn resolve(&mut self, index: usize, binding: Binding) -> VarRef {
        // A top-level function or field of this input is read through the
        // top level's frame from any function, never captured.
        // So is a declaration of another file of a workspace, from its
        // directory chain: the scope a file is checked from holds nothing
        // else (see [Directory scope](crate::check::imports#directory-scope)). A session's input
        // is checked beside the standard library as a file is, but its
        // scope is the session's, read as a session's input reads it.
        let committed = self.workspace.is_some_and(|w| !w.session()) && index < self.top_start;
        if committed || (self.top_start..self.top_end).contains(&index) {
            return if self.functions.is_empty() {
                VarRef::Local(binding.slot)
            } else {
                VarRef::Fn(binding.slot)
            };
        }
        let home = match binding.kind {
            BindingKind::Callee => VarRef::Callee,
            BindingKind::Variable
            | BindingKind::Mutable
            | BindingKind::Header
            | BindingKind::Field
            | BindingKind::Narrowed => VarRef::Local(binding.slot),
        };
        // The outermost function the binding is not in: every function from
        // there inwards must capture it.
        let Some(first) = self
            .functions
            .iter()
            .position(|function| function.bindings_start > index)
        else {
            return home;
        };
        let mut source = home;
        for function in &mut self.functions[first..] {
            let existing = function
                .captures
                .iter()
                .position(|capture| capture.binding == index);
            let capture = existing.unwrap_or_else(|| {
                function.captures.push(Capture {
                    binding: index,
                    source,
                    ty: binding.ty,
                });
                function.captures.len() - 1
            });
            source = VarRef::Capture(position(capture));
        }
        source
    }
}
