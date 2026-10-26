//! Checking names: a variable or a callee read by name, what it reads, and the
//! suggestions for a name that names nothing.
//!
//! # Suggestions
//!
//! An error about a name that names nothing carries a suggestion of what
//! it was probably meant to be, if a name usable where it is written is
//! close enough by [`crate::suggest`]'s rule, as the exact span of the name
//! and its replacement ([`crate::diagnostic::Suggestion`]):
//!
//! - an undefined variable: the bindings in scope, the nearest first (the
//!   innermost block's, then the enclosing ones', then the top level's,
//!   then those the input was checked from: committed by its directory
//!   chain or an earlier session input), then the roots of the file's view
//!   (its own, its directories', what it imports, the implicit prelude's),
//!   leaving out the top-level field whose initialiser is being checked,
//!   a name the file reads as `Error`, one that a top-level function
//!   checked early could not read yet, and a name that is no identifier;
//! - a missing field, "no field `x` in ..." and "the meta of `f` has no
//!   field `x`": the record type's fields, a view node's children among
//!   them, quoted as a field's name is if need be;
//! - an unknown type: the types of the file's view, then `Int`, `Float`,
//!   `Bool` and `String`; written with a type argument, `List`, `Future`
//!   and `Present` instead, which alone take one;
//! - a named argument no parameter has: the callee's named parameters that
//!   no other argument fills.
//!
//! The ranks are coarse, as the checker keeps no stack of blocks: names of
//! one rank tie and are taken in alphabetical order. The search runs only
//! as such an error is reported, never on valid input.

use crate::ast::{ExprId, ExprKind, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::suggest;
use crate::types::TyId;
use crate::workspace::Context;

use super::{Binding, BindingKind, Blocker, BlockerKind, Checker, VarRef, root_declares};

impl Checker<'_> {
    /// Resolves a use of the variable `name` at expression `id`, recording
    /// where it reads from, capturing it if it belongs to an enclosing
    /// function, and returns its type. A use of a callback parameter is
    /// recorded as a use of it as a value, which keeps it as written (see
    /// the module documentation); a call through its name is checked by
    /// [`Checker::check_callee`] instead.
    pub(super) fn check_name(&mut self, id: ExprId, name: Name, span: Span) -> TyId {
        let (ty, callback) = self.use_name(id, name, span);
        if let Some(callback) = callback.and_then(|callback| self.callbacks.get_mut(callback)) {
            // The first use in source order.
            if callback.used.is_none_or(|first| span.start < first.start) {
                callback.used = Some(span);
            }
        }
        ty
    }

    /// Checks the callee `callee` of a call, returning its type and, if it
    /// is exactly a name, not in parentheses, whose binding is a callback
    /// parameter, the parameter's index in `callbacks`: a call through the
    /// name is recorded by what it gives each position of the callback's
    /// type (see [`Checker::record_argument`]), not as a use of it as a
    /// value.
    pub(super) fn check_callee(&mut self, callee: ExprId) -> (TyId, Option<usize>) {
        let expr = self.ast.get(callee);
        let ExprKind::Name(name) = expr.kind else {
            return (self.check_expr(callee), None);
        };
        let (ty, callback) = self.use_name(callee, name, expr.span);
        self.expr_types[callee.index()] = ty;
        (ty, callback)
    }

    /// Whether `binding`, at `index` among the top level's bindings, the
    /// latest of `name`, is read rather than what the file's view has of
    /// `name` (see [Imports](crate::check::imports#imports)): only when what the view has is
    /// shadowable, and the binding is one the clash family lets shadow it, a
    /// top-level `fn` of the file or an earlier session input's variable. A
    /// field root of the file, or one its directory chain committed, is a
    /// declaration of the view's own order, which the view has decided.
    pub(super) fn binding_shadows_view(
        &self,
        workspace: &Context<'_>,
        (index, binding): (usize, Binding),
        name: Name,
    ) -> bool {
        if !workspace.scope.yields(name) {
            return false;
        }
        if index >= self.top_start {
            binding.kind != BindingKind::Field
        } else {
            workspace.session()
        }
    }

    /// As [`Checker::check_name`], but recording nothing of a callback
    /// parameter: returns the type, and the index in `callbacks` of the
    /// callback parameter the name's binding is, if it is one.
    pub(super) fn use_name(&mut self, id: ExprId, name: Name, span: Span) -> (TyId, Option<usize>) {
        let found = self.lookup(name);
        if self.early.is_some() && self.reads_between(name, found) {
            return (TyId::ERROR, None);
        }
        if self.reads_poisoned(name, found) {
            return (TyId::ERROR, None);
        }
        // An imported name, or a root several sources of the file's view
        // have, that nothing nearer shadows (see [Imports](crate::check::imports#imports)); a
        // shadowable one, the implicit prelude's among them, only if no
        // binding of the file's own or of an earlier input has it.
        if let Some(workspace) = self.workspace
            && workspace.scope.reads_through(name)
            && found.is_none_or(|(index, binding)| {
                index < self.top_end
                    && !self.binding_shadows_view(workspace, (index, binding), name)
            })
            && let Some(ty) = self.field_reference(id)
        {
            return (self.builtin_use(id, ty, span), None);
        }
        if let Some((index, binding)) = found {
            if binding.kind == BindingKind::Mutable {
                self.leaf_reads_mutable(index, binding.name);
            }
            if let Some(record) = &mut self.record
                && let Some(declared) = binding.declared
            {
                record.uses.push((id, declared));
            }
            let source = self.resolve(index, binding);
            self.expr_vars[id.index()] = source;
            // A use of a function whose needs are not known yet (see
            // [Needs](crate::check::needs#needs)).
            if let VarRef::Fn(slot) = source {
                self.note_member_use(id, slot);
            } else if binding.kind == BindingKind::Callee {
                self.note_callee_use(id, (index, binding.name));
            }
            return (self.builtin_use(id, binding.ty, span), binding.callback);
        }
        let suggestion = self.variable_suggestion(name, span);
        let text = self.ast.names().get(name);
        let mut error = Diagnostic::error(format!("undefined variable `{text}`"), span)
            .with_suggestion(suggestion);
        if text.contains('-') {
            error = error.with_note(
                "names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction",
            );
        }
        // A method's name is no variable (see [Methods](crate::check::methods#methods)).
        if self.sees_method(name) {
            error = error.with_note(format!(
                "`{text}` is a method: call it on a value, as in `x.{text}()`"
            ));
        }
        // In a default, its own parameter or a later one.
        if let Some(&(_, written)) = self
            .defaults_checked
            .unseen
            .iter()
            .rfind(|&&(unseen, _)| unseen == name)
        {
            error = error
                .with_context(written, format!("`{text}` is a parameter from here on"))
                .with_note("a parameter's default sees only the parameters before it");
        }
        // Inside a function, or a top-level field's initialiser, a name the
        // top level declares further down.
        let field = self.checking_leaf.is_some();
        if (field || !self.functions.is_empty())
            && let Some(below) = self.ast.items(self.ast.root()).iter().find(|&&item| {
                self.ast.item(item).span.start > span.start
                    && root_declares(self.ast, item) == Some(name)
            })
        {
            let (what, note) = if field {
                (
                    "field",
                    "a field's initialiser sees the variables declared above it",
                )
            } else {
                (
                    "function",
                    "a function sees the variables declared above it",
                )
            };
            error = error
                .with_context(
                    self.ast.item(*below).span,
                    format!("`{text}` is declared here, below the {what}"),
                )
                .with_note(note);
        }
        self.unresolved(error);
        (TyId::ERROR, None)
    }

    /// While a top-level function's body is checked before the walk has
    /// reached its declaration (see `early`): whether a read of `name`,
    /// which the lookup found as `found`, is of a name the top level
    /// declares between the item being checked and the function, by an
    /// item or possibly by a spread, if it reads the top level or an
    /// earlier input. If so, it records the first such read as why the
    /// function cannot be used yet, and the read is not resolved.
    pub(super) fn reads_between(&mut self, name: Name, found: Option<(usize, Binding)>) -> bool {
        let Some((span, kind)) = self.early_blocker(name, found) else {
            return false;
        };
        let blocker = Some(Blocker { name, span, kind });
        if let Some(top) = self.checking_top.and_then(|top| self.top_fns.get_mut(top)) {
            top.blocker = top.blocker.or(blocker);
        } else if let Some(cell) = self
            .checking_leaf
            .and_then(|node| self.field_cells.get_mut(node as usize))
        {
            cell.blocker = cell.blocker.or(blocker);
        }
        true
    }

    /// What keeps a read of `name`, found as `found`, from being resolved
    /// while a top-level function's body is checked early, as
    /// [`Checker::reads_between`] says: the item between that declares it,
    /// or a spread there, with where it is; `None` if nothing does.
    pub(super) fn early_blocker(
        &self,
        name: Name,
        found: Option<(usize, Binding)>,
    ) -> Option<(Span, BlockerKind)> {
        let (from, to) = self.early?;
        if self.checking_top.is_none() && self.checking_leaf.is_none() {
            return None;
        }
        let inner = self
            .functions
            .first()
            .map_or(usize::MAX, |function| function.bindings_start);
        let top_range = self.top_start..self.top_end;
        match found {
            Some((index, _)) if index >= inner || top_range.contains(&index) => return None,
            _ => {}
        }
        let mutable = found.is_some_and(|(_, binding)| binding.kind == BindingKind::Mutable);
        let root = self.ast.items(self.ast.root());
        let between = root.get(from..to).unwrap_or_default();
        between.iter().find_map(|&item| {
            let declared = self.ast.item(item);
            match declared.kind {
                ItemKind::Spread(_) => Some((declared.span, BlockerKind::Spread)),
                // A `:=` of a mutable variable declares nothing.
                ItemKind::Rebind { .. } if mutable => None,
                _ => (root_declares(self.ast, item) == Some(name))
                    .then_some((declared.span, BlockerKind::Bound)),
            }
        })
    }

    /// The suggestion for the undefined variable `name`, read at `span`
    /// (see [Suggestions](#suggestions)): the name nearest it among the
    /// bindings in scope and the roots of the file's view that a read here
    /// would resolve. None in a file whose view sees one with syntax
    /// errors, where the error is not reported.
    pub(super) fn variable_suggestion(&mut self, name: Name, span: Span) -> Option<(Span, String)> {
        if self.sees_broken() {
            return None;
        }
        // The top-level field whose initialiser is being checked: a read of
        // it there would be of itself.
        let declaring = self
            .checking_leaf
            .and_then(|node| self.field_cells.get(node as usize))
            .and_then(|cell| cell.binding);
        let innermost = self.block_start;
        let function = self
            .functions
            .last()
            .map_or(usize::MAX, |function| function.bindings_start);
        let mut candidates: Vec<(Name, u8)> = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            if Some(index) == declaring {
                continue;
            }
            let rank = if index >= innermost {
                0
            } else if index >= function || index >= self.top_end {
                1
            } else if index >= self.top_start {
                2
            } else {
                3
            };
            candidates.push((binding.name, rank));
        }
        if let Some(workspace) = self.workspace {
            let roots = self
                .view_roots
                .get_or_insert_with(|| workspace.suggestion_roots());
            candidates.extend(roots.iter().copied());
        }
        let names = self.ast.names();
        let ranked = suggest::ranked(
            names.get(name),
            candidates
                .into_iter()
                .map(|(candidate, rank)| (names.get(candidate), rank, candidate)),
        );
        // The closest that a read here would resolve: looked for among those
        // close enough alone, which are few.
        let best = ranked.into_iter().find(|&candidate| {
            if !crate::lexer::is_identifier(names.get(candidate)) {
                return false;
            }
            let found = self.lookup(candidate);
            if declaring.is_some() && found.map(|(index, _)| index) == declaring {
                return false;
            }
            !self
                .workspace
                .is_some_and(|workspace| workspace.poisoned(candidate))
                && self.early_blocker(candidate, found).is_none()
        })?;
        Some((span, names.get(best).to_owned()))
    }

    /// While a top-level leaf's initialiser is checked (see
    /// `checking_leaf`): records a read of the mutable variable `name`, the
    /// binding `index`, if it is one of this input's top level, as what
    /// makes the leaf be computed at its own item and no earlier (see
    /// [Top-level fields](crate::check::toplevel#top-level-fields)).
    pub(super) fn leaf_reads_mutable(&mut self, index: usize, name: Name) {
        let Some(node) = self.checking_leaf else {
            return;
        };
        let inner = self
            .functions
            .first()
            .map_or(usize::MAX, |function| function.bindings_start);
        if !(self.top_end..inner).contains(&index) {
            return;
        }
        let Some(leaf) = self
            .mentions
            .tree
            .nodes
            .get(node as usize)
            .and_then(|entry| entry.leaf)
        else {
            return;
        };
        let span = self.ast.item(leaf.item).span;
        if let Some(cell) = self.field_cells.get_mut(node as usize)
            && cell.blocker.is_none()
        {
            cell.ready = leaf.position;
            cell.blocker = Some(Blocker {
                name,
                span,
                kind: BlockerKind::Mutable,
            });
        }
    }
}
