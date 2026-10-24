//! Checking a file's items and statements: imports, type items, the result item
//! and the statements of a block, and the declarations a file shadows.
//!
//! # Shadowed declarations
//!
//! A top-level field or `def` marked `#[shadowable]` may be shadowed by a
//! later one it overlaps, in this input or, in a workspace, in a later
//! file of its directory (see [`crate::workspace`], "Shadowable
//! declarations", and [`crate::mentions`], "Shadowed declarations"). The
//! one that stands is the leaf for the whole input, a read above it
//! included, so it cannot read the one it shadows (`#[shadowable] a = 1`
//! then `a = a + 1` is "`a` depends on itself"). A shadowed declaration is
//! in no tree and no top-level function: the walk passes it by
//! (`Checker::set_aside_shadowed`) and, once the walk is done, checks its
//! value as it is written (`Checker::check_shadowed_items`), declaring
//! nothing, in the view of the top level above its own item, as a body
//! checked late is, a `def`'s function seeing itself by its name (one named
//! by a dotted path has none: its path reads the declaration that stands). So its
//! own errors are reported. It needs nothing, so what it alone would read
//! may not be computed, or checked, before it: a cell of a file this one
//! does not otherwise need has no type yet, and a record of an import only
//! it reads whole is not assembled; each is erroneous for it, which
//! suppresses the errors that would depend on it. It is never lowered (its
//! functions have code that does nothing, since none is made), so its
//! erroneous types break nothing ([`Checker::check`]'s last check exempts
//! its expressions). A top-level `let` or `fn` of the name of a field all
//! of whose leaves are shadowable shadows it in this input, silently: the
//! `let` from its item on, as it does any variable, the `fn` in the whole
//! input, its binding declared after the field roots'
//! (`Checker::declare_shadowing_fns`). The field is still a field of the
//! input's record and, in a workspace, of its directory, which commits
//! only fields ([`Scope`](super::Scope)'s `commit`): a `let` or `fn` is the file's own.

use crate::ast::{Effect, ExprId, ExprKind, ItemId, ItemKind, UnaryOp};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::types::{Laziness, TyId};

use super::{
    BUILTIN_TYPE_NAMES, Bareness, CONSTANT_FIELDS_NOTE, Checker, Expecting, NOT_CONSTANT_RECORD,
    Reach,
};

impl Checker<'_> {
    /// After the walk of the outermost block, whose items are `count`:
    /// computes the top-level fields left, checks the bodies left, settles
    /// the types of the open nodes, and checks the shadowed declarations.
    pub(super) fn finish_top_level(&mut self, count: usize) {
        self.root_position = count;
        self.compute_fields(count);
        self.check_remaining_bodies();
        self.settle_top_nodes();
        self.check_shadowed_items();
    }

    /// When the walk of the outermost block reaches its item `id`, at
    /// `position`: if a later declaration shadows it, sets it aside, with
    /// the bindings declared so far, to be checked once the walk is done
    /// ([`Checker::check_shadowed_items`]), and returns `true`.
    pub(super) fn set_aside_shadowed(&mut self, position: usize, id: ItemId) -> bool {
        let shadowed = self.mentions.shadowed.get(position) == Some(&true);
        if shadowed {
            self.shadowed_items.push((id, self.bindings.len()));
        }
        shadowed
    }

    /// After the walk of the outermost block: checks each shadowed
    /// declaration's value as it is written, in the view of the top level
    /// above its own item, as a body checked late is (see [Shadowed
    /// declarations](#shadowed-declarations)), declaring nothing.
    pub(super) fn check_shadowed_items(&mut self) {
        let ast = self.ast;
        for (id, cut) in std::mem::take(&mut self.shadowed_items) {
            let (path, ty, value) = match ast.item(id).kind {
                ItemKind::Field {
                    path, ty, value, ..
                } => (Some(path), ty, value),
                // A definition made with a def-type has no name to see
                // itself by.
                ItemKind::DefTypeDef { value, .. } => (None, None, value),
                _ => continue,
            };
            let cut = cut.max(self.top_end).min(self.bindings.len());
            let below = self.bindings.split_off(cut);
            let outer = std::mem::replace(&mut self.checking_shadowed, true);
            if let Some(path) = path {
                self.name_callee(ast.path(path), value);
            }
            self.annotated_value(ty, value);
            self.callee_name = None;
            self.checking_shadowed = outer;
            self.bindings.extend(below);
        }
    }

    /// The expressions of the outermost block's shadowed declarations, as
    /// ranges of ids: never lowered, they may hold erroneous types without
    /// an error, where they read a cell no file they need computes.
    pub(super) fn shadowed_exprs(&self) -> Vec<(usize, usize)> {
        let root = self.ast.items(self.ast.root());
        root.iter()
            .enumerate()
            .filter(|&(position, _)| self.mentions.shadowed.get(position) == Some(&true))
            .filter_map(|(_, &id)| match self.ast.item(id).kind {
                ItemKind::Field { value, .. } => {
                    let first = self.ast.subtree(value).next()?;
                    Some((first.index(), value.index()))
                }
                // A definition's `DT` comes just before its function.
                ItemKind::DefTypeDef { def, value } => {
                    let first = self.ast.subtree(self.ast.def_type(def).dt).next()?;
                    Some((first.index(), value.index()))
                }
                _ => None,
            })
            .collect()
    }

    /// Checks the placement of the result item spanning `span`, `= VALUE`,
    /// `value` being of type `ty`, in a block whose first field, if it has
    /// one, is `first_field`, and whose item after it, if any, is `next`:
    /// it cannot be in a record block, and it must be the last item.
    /// Returns whether it is the block's result; otherwise, its value is
    /// settled and the error reported.
    pub(super) fn result_item(
        &mut self,
        span: Span,
        (value, ty): (ExprId, TyId),
        (first_field, next): (Option<ItemId>, Option<ItemId>),
    ) -> bool {
        let eq = Span::new(span.start, span.start + 1);
        if let Some(first_field) = first_field {
            self.settle(value, ty);
            self.errors.push(
                Diagnostic::error("a block with fields cannot have a result", eq)
                    .with_context(self.ast.item(first_field).span, "first field declared here")
                    .with_note(
                        "a block is either a record of its fields or a single value; bind the \
                         value with `let` or make it a field",
                    ),
            );
            return false;
        }
        let Some(next) = next else {
            return true;
        };
        self.settle(value, ty);
        self.errors.push(
            Diagnostic::error("the result must be the last item in its block", eq)
                .with_context(self.ast.item(next).span, "this item comes after it"),
        );
        false
    }

    /// Checks the expression item `value`, or the value of a result item,
    /// `= VALUE`, which is its block's result if `is_result`, and a
    /// statement otherwise, against what is expected of the block's result,
    /// if anything is, and returns its type.
    pub(super) fn expression_item(
        &mut self,
        value: ExprId,
        is_result: bool,
        expected: Option<Expecting<'_>>,
    ) -> TyId {
        match expected {
            Some(expected) => {
                let context = (expected.context, expected.why);
                match expected.reach {
                    Reach::Whole => self.fit(value, expected.ty, context),
                    Reach::Literal => self.fill_result(value, expected.ty, context),
                    Reach::Elements => self.fill_elements(value, expected.ty, context),
                }
            }
            None if !is_result && self.is_empty_list(value) => {
                // A statement `[]` holds nothing, and is the unit-like
                // `List<{}>`.
                let ty = self.types.list(TyId::EMPTY);
                self.expr_types[value.index()] = ty;
                ty
            }
            None => self.check_expr(value),
        }
    }

    /// The type of a block whose last item is `id`, a `break`, a `continue`
    /// or a `return`, checked: `Never`, or erroneous if the item is an
    /// error and so has nothing to leave for, so that nothing more is
    /// reported of the block.
    pub(super) fn left_type(&self, id: ItemId) -> TyId {
        if self.jumps[id.index()].is_some() {
            self.never
        } else {
            TyId::ERROR
        }
    }

    /// Resolves the file's `type` items, each's type with every bare type
    /// present, recording what each stands for (see [Type
    /// declarations](crate::check::resolve#type-declarations)): reporting what is wrong in each,
    /// with "`Int` is a built-in type" for one named like a built-in type
    /// ([`BUILTIN_TYPE_NAMES`]) and "type `Foo` is declared twice" at a
    /// second of one name, labelled "first declared here", unless resolving
    /// quietly, as a workspace's check of a file does, the workspace having
    /// checked them first ([`check_types`](super::check_types)).
    pub(super) fn type_items(&mut self) {
        let ast = self.ast;
        let mut declared: Vec<(Name, Span)> = Vec::new();
        for &id in ast.items(ast.root()) {
            let ItemKind::Type {
                name,
                name_span,
                ty,
            } = ast.item(id).kind
            else {
                continue;
            };
            let text = ast.names().get(name);
            let first = declared
                .iter()
                .find(|&&(earlier, _)| earlier == name)
                .map(|&(_, span)| span);
            let error = if BUILTIN_TYPE_NAMES.contains(&text) {
                Some(Diagnostic::error(
                    format!("`{text}` is a built-in type"),
                    name_span,
                ))
            } else if let Some(first) = first {
                Some(
                    Diagnostic::error(format!("type `{text}` is declared twice"), name_span)
                        .with_context(first, "first declared here"),
                )
            } else {
                declared.push((name, name_span));
                None
            };
            if let Some(error) = error {
                if self.quiet.types {
                    self.quiet.failed = true;
                } else {
                    self.errors.push(error);
                }
            }
            let errors = self.errors.len();
            let resolved = self.resolve_type(ty, Bareness::Present);
            // A generic attribute is the workspace's error.
            if ast.is_attribute(id)
                && ast.type_arity(id) == 0
                && resolved != TyId::ERROR
                && !self.holds_constants(resolved, true)
            {
                self.errors.push(
                    Diagnostic::error(NOT_CONSTANT_RECORD, ast.type_expr(ty).span)
                        .with_note(CONSTANT_FIELDS_NOTE),
                );
            }
            if self.quiet.types && self.errors.len() > errors {
                self.errors.truncate(errors);
                self.quiet.failed = true;
            }
            if resolved != TyId::ERROR {
                self.declared_types.push((id, resolved));
            }
        }
    }

    /// Whether the item `id`, of `kind`, checked, never completes, so that
    /// no item after it in its block runs: a `break`, a `continue` or a
    /// `return`, but one that is an error and so has nothing to leave for,
    /// and an expression item, a result, a `let` or a `:=` whose value is
    /// of type `Never`.
    pub(super) fn never_completes(&self, id: ItemId, kind: ItemKind) -> bool {
        match kind {
            ItemKind::Break(_) | ItemKind::Continue(_) | ItemKind::Return(_) => {
                self.jumps[id.index()].is_some()
            }
            ItemKind::Expr(value)
            | ItemKind::Result(value)
            | ItemKind::Let { value, .. }
            | ItemKind::Rebind { value, .. } => self.expr_types[value.index()] == self.never,
            ItemKind::Field { .. }
            | ItemKind::Spread(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::Enum { .. }
            | ItemKind::Variant { .. }
            | ItemKind::Context { .. }
            | ItemKind::DefTypeDef { .. }
            | ItemKind::DefMeta { .. }
            | ItemKind::Error => false,
        }
    }

    /// Whether `value` is directly an `await` expression.
    pub(super) fn is_await(&self, value: ExprId) -> bool {
        matches!(
            self.ast.get(value).kind,
            ExprKind::Unary {
                op: UnaryOp::Await,
                ..
            }
        )
    }

    /// Whether `value` is directly the empty list literal, `[]`.
    pub(super) fn is_empty_list(&self, value: ExprId) -> bool {
        matches!(
            self.ast.get(value).kind,
            ExprKind::List(elements) if self.ast.elements(elements).is_empty()
        )
    }

    /// Applies the statement rule to `value`, an expression item that is
    /// not its block's result, of type `ty` (see the module documentation).
    /// A bare `await` is run for its effects, and its value discarded,
    /// whatever it is. Otherwise a statement is a position expecting a
    /// unit-like value (see [`TypeTable::is_unit_like`](crate::types::TypeTable::is_unit_like)): one that is
    /// unit-like as it is needs nothing; one that is unit-like once awaited,
    /// at the top as a `Future<{}>` is or deeply as a `List<Future<{}>>`
    /// is, is awaited here, so that its effects are not lost, subject to
    /// the purity rule, a parameter of this function being measured. A call
    /// of a function that is not pure, outside a pure function, is run for
    /// its effects, as a bare `await` is: it is awaited deeply, so that no
    /// effect it defers is lost, and its value discarded, whatever it is.
    /// Any other is an unused value.
    pub(super) fn check_statement(&mut self, value: ExprId, ty: TyId) {
        if self.is_await(value) {
            self.settle(value, ty);
            return;
        }
        if ty == TyId::ERROR || self.types.is_unit_like(ty) {
            return;
        }
        if self.types.strip(ty) == TyId::EMPTY {
            self.measure(value, ty);
            return;
        }
        let present = self.types.present(ty);
        if self.types.is_unit_like(present) {
            self.measure_deep(value, ty);
        } else if self.impure_calls.contains(&value)
            && self
                .functions
                .last()
                .is_none_or(|function| function.effect != Effect::Pure)
        {
            // In a pure function the call is an error already.
            self.measure_deep(value, ty);
            self.discarded.insert(value);
        } else {
            self.unused_value(value, ty);
        }
    }

    /// Reports the statement `value`, of type `ty`, which is neither `{}`
    /// nor erroneous, as an unused value.
    pub(super) fn unused_value(&mut self, value: ExprId, ty: TyId) {
        let ty = self.settle(value, ty);
        let shown = self.in_message(ty);
        let resolved = self.shown(ty);
        let note = if self.types.laziness(resolved) == Laziness::Present {
            "make it the block's result with `= ...`, bind it with `let`, or remove it"
        } else {
            "await it with `await`, bind it with `let`, or remove it"
        };
        self.errors.push(
            Diagnostic::error(
                format!("unused value of type {shown}"),
                self.ast.get(value).span,
            )
            .with_note(note),
        );
    }
}
