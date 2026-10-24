//! Checking mutable variables: rebinding, with `:=`, and updates of a
//! variable's fields and elements.
//!
//! # Mutable variables
//!
//! `let mut x = e` declares a *mutable variable*, checked exactly as `let x
//! = e` is (and `let mut x: T = e` as `let x: T = e`), whose type is fixed
//! there for its whole life. Its binding is of kind `BindingKind::Mutable`,
//! and shadows and is shadowed as any variable's is: a later `let x`, `let
//! mut x` or dotted `let x.a = ...` is a new variable in a new slot, which
//! is mutable only if it is a `let mut`. A `let mut`'s path is a single
//! name (the parser rejects a dotted one).
//!
//! ```text
//! let mut total = 0
//! for x in xs { total := total + x }
//! let mut conf = { port = 80; host = "a" }
//! if tls { conf.port := 443 }
//! ```
//!
//! Values never change: a mutable variable is one that `:=` may give a new
//! value from *any* block of the function that declares it (the top level,
//! for one of the top level), an `if`'s branch, a `for`'s body or guard, a
//! nested block or a record block, and not only from its declaring block.
//! `x := e`, where the innermost binding of `x` (`lookup`) is a mutable
//! variable, is checked by `Checker::check_mutable_rebind`:
//!
//! 1. If the variable is of the function being checked, the rebind is allowed
//!    from any block, and `e` is checked against the variable's type exactly as
//!    a `:=` in the declaring block of a variable that is not mutable is: "the
//!    variable was declared here", literals adapting, a value awaited to fit
//!    where an await is allowed. Nothing is declared: the item writes the
//!    variable's own slot (see
//!    [`Resolution::item_slot`](super::Resolution::item_slot)), and the
//!    variable keeps its type, its declaration and its place in `bindings`. So
//!    in the declaring block too, a mutable variable has one slot for its whole
//!    life, and a frame does not grow with its rebinds.
//! 2. If it is of an enclosing function or of the top level, and the `:=` is in
//!    a function expression or a `lazy` nested in that, it is "cannot rebind
//!    `x` here: it is declared outside this function" (or "this `lazy`", for
//!    the innermost being one), labelled at the declaration, with a note that
//!    the function holds its own copy of the variable. The value is checked on
//!    its own and nothing is declared, so nothing cascades.
//!
//! A rebind of a mutable variable that is erroneous already, whose error
//! has been reported, checks its value on its own and declares nothing too.
//!
//! A dotted rebind, `x.a.b := e` or `xs[i].a := e`, of a mutable variable
//! of the function being checked, from any block, walks its path exactly
//! as any dotted `:=` does (see "Dotted `let` and `:=`" above): each
//! existing record or list measured, awaited where it may be a future,
//! subject to the purity rule; every segment existing; each position
//! checked; `e` fitting the leaf's type. The updated record or list is
//! written to the variable's own slot, and the variable keeps its declared
//! type, which an assignment in an `if` must not change for the code after
//! it, where a variable that is not mutable takes the updated type, the
//! records and lists along the path present. So the updated type must be a
//! subtype of the declared one (`Checker::kept_type`), which it is whenever
//! the variable and the records along the path are present or the either
//! shape; otherwise, as for a variable declared exactly `Future<{a: Int}>`,
//! it is "cannot rebind `r.a`: `r` is declared `Future<{a: Int}>`, and the
//! update makes it `{a: Int}`", at the path, the value still being checked.
//! It is evaluated as every dotted update is: its value first, then the
//! variable read, so `r.a := { r := { a = 100; b = 7 }; r.b + 1 }` updates
//! the record the value stored, and `r` is `{a = 8; b = 7}`.
//!
//! A variable that is not mutable is rebound as before: a `:=` in its
//! declaring block declares a new binding, and one from an inner block is
//! "declared in an enclosing block", whose note says that only a `let mut`
//! variable can be rebound there (a parameter and a `for`'s variable, which
//! cannot be declared `mut`, included), unless the `:=` is in a function or
//! a `lazy` nested in the variable's own, where `let mut` would not help
//! either. Fields and the callee are rejected as ever.
//!
//! Nothing else is new. Rebinding a mutable variable is not an effect
//! anything outside its function can see, so pure, memoised and impure
//! functions alike may. A function expression or a `lazy` that reads a
//! mutable variable of an enclosing function captures it, by value, when
//! it is created, as any variable (see [Functions](crate::check::functions#functions)), so later
//! rebinds do not reach it, and no value is ever shared and changed. A `for`
//! and an `if` are not functions, so their guards and bodies may rebind the
//! enclosing function's mutable variables. There is no flow typing: a
//! mutable variable has its declared type at every use, so one that may be
//! a future is measured, and awaited, at each.
//!
//! Since the variable may come to hold another value, nothing is inferred
//! from its initialiser beyond its type: a mutable variable initialised
//! directly with a function expression does not remember the function
//! (`Binding::function` is `None`, and so is its entry of a session's
//! [`Scope`](super::Scope)), so a call through it is typed by its type alone (see "The
//! call-site error" under [Inferred parameters](crate::check::inference#inferred-parameters)).
//! Inside `let mut f = fn ...`, the name `f` is still the function being
//! called, whatever `f` holds later. Its type is its initialiser's, which
//! for a function is the type inferred for it (see [Inferred
//! results](crate::check::inference#inferred-results)); an annotation gives it a wider one.

use crate::ast::{ExprId, FnId, ItemId, PathRange, PathSegment, SegmentKind, path_span};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::{Laziness, TyId};

use super::{
    Binding, BindingKind, Checker, PathResolution, leaf_label, narrowed_rebind_error, position,
};

impl Checker<'_> {
    /// Checks `PATH := VALUE` and declares the new binding: with the value's
    /// type, or for a dotted path the type of the update. The binding is
    /// declared even when the rebind is rejected, so that later uses of the
    /// name do not cascade into "undefined variable" errors. A rebind of a
    /// mutable variable declares nothing: see
    /// [`Checker::check_mutable_rebind`]. The positions of the path's index
    /// segments are checked once each, before the value: as the path is
    /// walked, or on their own if it is not.
    pub(super) fn check_rebind(&mut self, id: ItemId, path: PathRange, value: ExprId) {
        let segments = self.ast.path(path);
        // A path's first segment is a name.
        let Some((name, name_span)) = segments
            .first()
            .and_then(|first| Some((first.name()?, first.span)))
        else {
            return;
        };
        let text = self.ast.names().get(name);
        // The binding being rebound, with its index in `bindings`, once the
        // `:=` rules pass.
        let target = match self.lookup(name) {
            // A top-level field of this input, already reported at the item
            // as a field of the file.
            Some((_, binding))
                if binding.kind == BindingKind::Field && self.field_clash == Some(id) =>
            {
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Field => {
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}`: it is a field, not a variable"),
                    name_span,
                );
                // A field from an earlier session input has no item here to
                // point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(declared, "declared here");
                }
                self.errors.push(error.with_note(format!(
                    "rebind with `{text} = ...` is not allowed either: a field is defined once"
                )));
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Callee => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: it is the function being defined"),
                        name_span,
                    )
                    .with_note("shadow it with `let` instead"),
                );
                None
            }
            Some((index, binding)) if binding.kind == BindingKind::Mutable => {
                self.record_rebind(id, binding.declared);
                self.check_mutable_rebind(id, segments, (index, binding), value);
                return;
            }
            Some((index, binding)) if binding.kind == BindingKind::Header => {
                self.record_rebind(id, binding.declared);
                if self.header_rebind(id, segments, (index, binding), value) {
                    return;
                }
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Narrowed => {
                let error = narrowed_rebind_error(text, name_span, binding);
                self.errors.push(error);
                // Nothing is declared: the copy stands, so nothing cascades.
                self.check_positions_alone(segments);
                self.check_alone(value);
                return;
            }
            Some((index, binding)) if index >= self.block_start => Some((index, binding)),
            Some((index, binding)) => {
                let error = self.enclosing_rebind_error(name_span, (index, binding));
                self.errors.push(error);
                None
            }
            None => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: no variable named `{text}` is declared"),
                        name_span,
                    )
                    .with_note(format!("declare it with `let {text} = ...`")),
                );
                None
            }
        };
        let Some((index, target)) =
            target.filter(|(_, target)| target.ty != TyId::ERROR && target.ty != self.never)
        else {
            self.rejected_rebind(id, (segments, name), value, target.map(|(_, t)| t.ty));
            return;
        };
        self.record_rebind(id, target.declared);
        // `:=` is assignment-compatible: the value must fit the variable's
        // type (or the path's leaf type), and the binding keeps that type.
        let context = target.declared.unwrap_or(name_span);
        if segments.len() == 1 {
            self.check_expr_expecting(value, target.ty, context, "the variable was declared here");
            self.declare(name, target.ty, id, BindingKind::Variable);
            return;
        }
        // The path is walked, measuring its records and lists and checking
        // its positions, before the value is checked, since the value is
        // checked against its leaf's type; its records and lists stay on
        // `path_types` until the variable is declared.
        let base = self.path_types.len();
        if let Some(leaf) = self.rebind_leaf(segments, (target.ty, index)) {
            self.check_expr_expecting(value, leaf, path_span(segments), leaf_label(segments));
            self.finish_update(id, segments, (Some((index, target)), base), leaf);
        } else {
            self.check_alone(value);
            // A path through a value of type `Never` is never walked.
            let ty = if std::mem::take(&mut self.unreached_path) {
                self.never
            } else {
                TyId::ERROR
            };
            self.declare(name, ty, id, BindingKind::Variable);
        }
    }

    /// Checks `PATH := VALUE`, the item `id` whose path is `segments`, of
    /// `target`, at `index` in `bindings`, a variable a plain `let` of a
    /// `for` header declared (see [For](crate::check::loops#for)): from the header, its own
    /// block, and of its own function, it is a rebind of the variable in
    /// its one slot, keeping its type, as a mutable variable's is (see
    /// [`Checker::check_mutable_rebind`]), and this returns `true`;
    /// otherwise it is reported (see [`Checker::header_rebind_error`]), and
    /// this returns `false`, the caller checking the rest as for any
    /// rejected `:=`.
    pub(super) fn header_rebind(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (index, target): (usize, Binding),
        value: ExprId,
    ) -> bool {
        let own_function = self
            .functions
            .last()
            .is_none_or(|function| index >= function.bindings_start);
        if index >= self.block_start && own_function {
            self.check_mutable_rebind(id, segments, (index, target), value);
            return true;
        }
        let name_span = segments.first().map_or(Span::new(0, 0), |first| first.span);
        let error = self.header_rebind_error(name_span, (index, target));
        self.errors.push(error);
        false
    }

    /// Checks the `:=` item `id`, whose path is `segments`, of the variable
    /// `name`, with the value `value`, which was rejected, or whose
    /// variable, of type `ty`, is erroneous or `Never`: the positions and
    /// the value are checked on their own, and the name declared with what
    /// it has, so later uses do not cascade. The path of a variable that may
    /// be a future is not walked: awaiting it would be reported of a
    /// variable that cannot be rebound anyway. Nor is one with an index
    /// segment, whose walk would check the positions again. A variable of
    /// type `Never` never holds a value, so the `:=` is never run, and the
    /// name is declared `Never` again, so that nothing after it is an
    /// error.
    pub(super) fn rejected_rebind(
        &mut self,
        id: ItemId,
        (segments, name): (&[PathSegment], Name),
        value: ExprId,
        ty: Option<TyId>,
    ) {
        self.check_positions_alone(segments);
        let value_ty = self.check_alone(value);
        if ty == Some(self.never) {
            self.declare(name, self.never, id, BindingKind::Variable);
            return;
        }
        let lazy = self.lookup(name).is_some_and(|(_, binding)| {
            binding.ty != TyId::ERROR && self.types.laziness(binding.ty) != Laziness::Present
        });
        let indexed = segments.iter().any(|segment| segment.position().is_some());
        if segments.len() == 1 {
            self.declare(name, value_ty, id, BindingKind::Variable);
        } else if lazy || indexed {
            self.declare(name, TyId::ERROR, id, BindingKind::Variable);
        } else {
            self.declare_update(id, segments, value_ty);
        }
    }

    /// The error for a `:=`, whose name is at `name_span`, of the variable
    /// `binding`, at `index` in `bindings`, a plain `let` of a `for` header,
    /// from outside the header: "cannot rebind `i` here: it is declared in
    /// the header of a `for`", labelled at the declaration, with a note
    /// that a `let mut` in the header can be rebound from the body; or, from
    /// a function or a `lazy` nested in the variable's own, the error for
    /// any variable declared in an enclosing block.
    pub(super) fn header_rebind_error(
        &self,
        name_span: Span,
        (index, binding): (usize, Binding),
    ) -> Diagnostic {
        let outside = self
            .functions
            .last()
            .is_some_and(|function| index < function.bindings_start);
        if outside {
            return self.enclosing_rebind_error(name_span, (index, binding));
        }
        let text = self.ast.names().get(binding.name);
        let mut error = Diagnostic::error(
            format!("cannot rebind `{text}` here: it is declared in the header of a `for`"),
            name_span,
        );
        if let Some(declared) = binding.declared {
            error = error.with_context(declared, "declared here");
        }
        error.with_note(format!(
            "a variable of a `for` header is rebound in the header, after the condition; declare \
             it with `let mut {text} = ...` in the header to rebind it from the body too"
        ))
    }

    /// Checks `PATH := VALUE`, the item `id` whose path is `segments`, of
    /// the mutable variable `target`, at `index` in `bindings`, which is the
    /// innermost binding of the path's first segment (see the module
    /// documentation). From the variable's own function, or from the top
    /// level for a variable of the top level, the rebind is allowed in any
    /// block, and writes the variable's own slot: the value must fit the
    /// variable's type as for a `:=` of a variable that is not mutable, in
    /// its declaring block, and a dotted path is walked and its leaf
    /// checked as for one too, but nothing is declared, and the variable
    /// keeps its type, its declaration and its place in `bindings`. A
    /// dotted rebind's record must therefore be of the variable's type (see
    /// [`Checker::kept_type`]). From a function expression or a `lazy`
    /// nested in that function, the rebind is rejected: the function holds
    /// its own copy of the variable. A rejected rebind, and one of a
    /// variable that is erroneous already, check the positions of the path's
    /// index segments and the value on their own and declare nothing, so
    /// nothing cascades.
    pub(super) fn check_mutable_rebind(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (index, target): (usize, Binding),
        value: ExprId,
    ) {
        let Some(first) = segments.first() else {
            return;
        };
        if let Some(function) = self.functions.last()
            && index < function.bindings_start
        {
            let error =
                self.outside_rebind_error((target.name, first.span), target, function.fn_id);
            self.errors.push(error);
            self.check_positions_alone(segments);
            self.check_alone(value);
            return;
        }
        if target.ty == TyId::ERROR || target.ty == self.never {
            self.check_positions_alone(segments);
            self.check_alone(value);
            // A variable of type `Never` never holds a value, so the `:=`
            // is never run, but is lowered as any other is.
            if target.ty == self.never {
                self.item_slots[id.index()] = target.slot;
            }
            return;
        }
        if segments.len() == 1 {
            let context = target.declared.unwrap_or(first.span);
            self.check_expr_expecting(value, target.ty, context, "the variable was declared here");
            self.item_slots[id.index()] = target.slot;
            return;
        }
        // As for any dotted `:=`, the path is walked before the value is
        // checked against its leaf's type.
        let base = self.path_types.len();
        let Some(leaf) = self.rebind_leaf(segments, (target.ty, index)) else {
            self.check_alone(value);
            if std::mem::take(&mut self.unreached_path) {
                self.item_slots[id.index()] = target.slot;
            }
            return;
        };
        self.check_expr_expecting(value, leaf, path_span(segments), leaf_label(segments));
        let updated = self.record_update(id, segments, (Some((index, target)), base), leaf);
        self.kept_type(segments, target, updated);
        self.item_slots[id.index()] = target.slot;
    }

    /// The error for a `:=`, whose name is at `name_span`, of the variable
    /// `binding`, at `index` in `bindings`, which is not mutable and is
    /// declared in a block enclosing the `:=`'s own: "cannot rebind `x`
    /// here: it is declared in an enclosing block", labelled at the
    /// declaration when there is one. Its note says to shadow the variable,
    /// and that a `let mut` variable can be rebound from an inner block,
    /// unless the `:=` is in a function or a `lazy` nested in the
    /// variable's own, where `let mut` would not help either.
    pub(super) fn enclosing_rebind_error(
        &self,
        name_span: Span,
        (index, binding): (usize, Binding),
    ) -> Diagnostic {
        let text = self.ast.names().get(binding.name);
        let mut error = Diagnostic::error(
            format!("cannot rebind `{text}` here: it is declared in an enclosing block"),
            name_span,
        );
        // A binding from an earlier session input has no item here to
        // point at.
        if let Some(declared) = binding.declared {
            error = error.with_context(declared, "declared here");
        }
        let outside = self
            .functions
            .last()
            .is_some_and(|function| index < function.bindings_start);
        error.with_note(if outside {
            format!("shadow it with `let {text} = ...` instead")
        } else {
            format!(
                "shadow it with `let {text} = ...` instead; only a variable declared with `let \
                 mut` can be rebound from an inner block"
            )
        })
    }

    /// The error for a `:=` of the mutable variable `target`, the first
    /// segment of whose path is `name`, written at `span`, from inside the
    /// function expression or `lazy` `function`, nested in the variable's
    /// own function or in the top level: "cannot rebind `x` here: it is
    /// declared outside this function" (or "this `lazy`"), labelled at the
    /// declaration when there is one.
    pub(super) fn outside_rebind_error(
        &self,
        (name, span): (Name, Span),
        target: Binding,
        function: FnId,
    ) -> Diagnostic {
        let text = self.ast.names().get(name);
        // A `lazy` is the one function that declares no return type.
        let kind = if self.ast.fn_decl(function).ret.is_none() {
            "`lazy`"
        } else {
            "function"
        };
        let mut error = Diagnostic::error(
            format!("cannot rebind `{text}` here: it is declared outside this {kind}"),
            span,
        );
        // A variable from an earlier session input has no item here to
        // point at.
        if let Some(declared) = target.declared {
            error = error.with_context(declared, "declared here");
        }
        error.with_note(format!(
            "a {kind} holds its own copy of each variable it uses, made when the {kind} is \
             created; rebinding the copy would not change `{text}`"
        ))
    }

    /// Checks that `updated`, the record a dotted `:=` of the path
    /// `segments` makes of the mutable variable `target`, is of the
    /// variable's type, which a mutable variable keeps: it is whenever the
    /// variable and the records along the path are present or the either
    /// shape, since the update makes them present. Otherwise, as for a
    /// variable declared exactly `Future<{a: Int}>`, reports "cannot rebind
    /// `r.a`: `r` is declared `Future<{a: Int}>`, and the update makes it
    /// `{a: Int}`" at the path.
    pub(super) fn kept_type(&mut self, segments: &[PathSegment], target: Binding, updated: TyId) {
        if updated == TyId::ERROR || self.types.is_subtype(updated, target.ty) {
            return;
        }
        let path = self.ast.show_path(segments).to_string();
        let text = self.ast.names().get(target.name);
        let declared = self.in_message(target.ty);
        let made = self.in_message(updated);
        let mut error = Diagnostic::error(
            format!(
                "cannot rebind `{path}`: `{text}` is declared {declared}, and the update makes \
                 it {made}"
            ),
            path_span(segments),
        );
        if let Some(span) = target.declared {
            error = error.with_context(span, "declared here");
        }
        self.errors.push(error.with_note(
            "a mutable variable keeps its type; declare it with a type that allows both, or \
             await it where it is declared",
        ));
    }

    /// The type of the leaf of the dotted `:=` path `segments`, whose first
    /// segment's variable, at `binding` in `bindings`, has type `root`, once
    /// its path is walked (see [`Checker::walk_path`]), which leaves the
    /// records and lists along it on `path_types` for
    /// [`Checker::finish_update`]: a field's type, or a list's element type
    /// as it is for an index. Every segment must already exist, since `:=`
    /// never adds a field: a missing one, or a segment along the way that is
    /// not a record or a list as its segment needs, is reported and `None`
    /// returned, with `path_types` as it was. An index segment always
    /// exists: its position is a runtime matter.
    pub(super) fn rebind_leaf(
        &mut self,
        segments: &[PathSegment],
        (root, binding): (TyId, usize),
    ) -> Option<TyId> {
        let base = self.path_types.len();
        let root = self.walk_path(segments, (root, binding))?;
        let records = self.path_types.get(base..)?;
        let (walked, &(record, _), last) = (records.len(), records.last()?, segments.last()?);
        // The first segment that the record before it lacks: one the walk
        // stopped at, or the leaf.
        let missing = if walked + 1 < segments.len() {
            walked
        } else {
            let leaf = match last.kind {
                SegmentKind::Name(name) => self.types.field(record, name),
                SegmentKind::Index { .. } => self.types.list_element(record),
            };
            match leaf {
                Some(leaf) => return Some(leaf),
                None => walked,
            }
        };
        self.path_types.truncate(base);
        // Only a name can be missing.
        let name = segments[missing].name()?;
        let path = self.ast.show_path(segments);
        let prefix = self.ast.show_path(&segments[..missing]);
        let field = FieldName(self.ast.names().get(name));
        let label = format!("this is {}", self.in_label(root));
        let error = Diagnostic::error(
            format!("cannot rebind `{path}`: `{prefix}` has no field `{field}`"),
            path_span(segments),
        )
        .with_context(segments[0].span, label);
        // A `let` path cannot index a list, so a path with an index segment
        // has no `let` to suggest.
        let indexed = segments.iter().any(|segment| segment.position().is_some());
        self.errors.push(if indexed {
            error
        } else {
            error.with_note(format!(
                "`:=` cannot add a field; add it with `let {path} = ...`"
            ))
        });
        None
    }

    /// Walks the path `segments` of a dotted `let` or `:=` whose first
    /// segment's variable, at `binding` in `bindings`, has type `old`, which
    /// is not erroneous. Updating a record or a list reads it, so each one
    /// along the path that exists is measured at the top (see
    /// [`Checker::measure_path_record`]): the variable's, and each following
    /// segment's, short of the last, that the record before it has, or that
    /// is an element of the list before it. Each is pushed onto
    /// `path_types`, stripped, with whether it is awaited; the walk stops at
    /// the first segment the record before it lacks, and before the last,
    /// which is replaced, not read. The position of an index segment, which
    /// only a `:=` path has, is checked where the walk reaches it, after its
    /// list is measured (see [`Checker::check_position`]).
    ///
    /// Returns `old` as measured, for labelling a later error. A value that
    /// is not a record, before a name, or a list, before an index, once
    /// stripped is reported instead, "cannot update `a.b.c`: `a.b` is Int,
    /// not a record" or "cannot update `xs[0]`: `xs` is Int, not a list",
    /// showing its type before stripping, and `None` returned, with
    /// `path_types` as it was; so is an index whose position is not an
    /// `Int`. Every position the walk does not reach is checked on its own,
    /// so that each is checked exactly once whatever happens.
    pub(super) fn walk_path(
        &mut self,
        segments: &[PathSegment],
        (old, binding): (TyId, usize),
    ) -> Option<TyId> {
        let base = self.path_types.len();
        let mut root = old;
        // The type of `segments[..index]`, which exists.
        let mut current = old;
        for (index, next) in segments.iter().enumerate().skip(1) {
            let prefix = &segments[..index];
            let variable = (index == 1).then_some(binding);
            let position = next.position();
            let (measured, awaited) =
                self.measure_path_record(prefix, current, (variable, position.is_some()));
            if index == 1 {
                root = measured;
            }
            let record = self.types.strip(measured);
            if record == self.never {
                // No value has it, so the update is never run: the path
                // stops quietly, and the caller declares `Never`.
                self.path_types.truncate(base);
                self.check_positions_alone(&segments[index..]);
                self.unreached_path = true;
                return None;
            }
            let inner = match position {
                None if self.types.is_record(record) => {
                    next.name().map(|name| self.types.field(record, name))
                }
                Some(_) => self.types.list_element(record).map(Some),
                None => None,
            };
            let Some(inner) = inner else {
                self.path_types.truncate(base);
                self.not_updatable(segments, prefix, (measured, root));
                self.check_positions_alone(&segments[index..]);
                return None;
            };
            self.path_types.push((record, awaited));
            if let Some(position) = position
                && !self.check_position(position)
            {
                self.path_types.truncate(base);
                self.check_positions_alone(&segments[index + 1..]);
                return None;
            }
            // After the last record or list comes the leaf, which is never
            // measured.
            match inner {
                Some(ty) if index + 1 < segments.len() => current = ty,
                _ => {
                    self.check_positions_alone(&segments[index + 1..]);
                    break;
                }
            }
        }
        Some(root)
    }

    /// Reports that the update whose path is `segments` cannot select its
    /// next segment in the value at `prefix`, of type `measured`, the
    /// variable's being `root` (see [`Checker::walk_path`]): "cannot update
    /// `a.b.c`: `a.b` is Int, not a record" before a name, or "... not a
    /// list" before an index, labelled at the variable with its type.
    pub(super) fn not_updatable(
        &mut self,
        segments: &[PathSegment],
        prefix: &[PathSegment],
        (measured, root): (TyId, TyId),
    ) {
        let needed = match segments.get(prefix.len()).and_then(PathSegment::position) {
            Some(_) => "list",
            None => "record",
        };
        let message = format!(
            "cannot update `{}`: `{}` is {}, not a {needed}",
            self.ast.show_path(segments),
            self.ast.show_path(prefix),
            self.in_message(measured)
        );
        let label = format!("this is {}", self.in_label(root));
        self.errors.push(
            Diagnostic::error(message, path_span(segments)).with_context(segments[0].span, label),
        );
    }

    /// Checks the position of each index segment of `segments` on its own,
    /// for a `:=` that is rejected, or whose walk stops before reaching
    /// them, so that their mistakes are reported and nothing cascades.
    pub(super) fn check_positions_alone(&mut self, segments: &[PathSegment]) {
        for position in segments.iter().filter_map(PathSegment::position) {
            self.check_alone(position);
        }
    }

    /// Where the record or list at `prefix`, a proper prefix of the path of
    /// a dotted `let` or `:=`, of type `ty`, is read by the update, as
    /// [`Checker::measure`] measures the record of a field access or the
    /// list of an index: if it is a parameter of a claimable function (see
    /// [`Checker::claimable`]) whose type is provisional, the parameter is
    /// measured, which makes it present, and its present type returned; if
    /// it may be a future, it is to be awaited at the top, subject to the
    /// purity rule, reported at `prefix` (see
    /// [`Checker::implicit_await_error`]) if it is what the next segment
    /// needs once stripped, a list if `list` and a record otherwise, and
    /// `ty` is returned with `true`; otherwise, or if it is erroneous, `ty`
    /// is returned with `false`. `variable` is where the value is in
    /// `bindings` if it is the variable the path updates, the path's first
    /// segment, whose name the purity error's note suggests awaiting before
    /// the pure function is created if it can be (see
    /// [`Checker::awaitable_outside`]).
    pub(super) fn measure_path_record(
        &mut self,
        prefix: &[PathSegment],
        ty: TyId,
        (variable, list): (Option<usize>, bool),
    ) -> (TyId, bool) {
        if ty == TyId::ERROR || self.types.laziness(ty) == Laziness::Present {
            return (ty, false);
        }
        let span = path_span(prefix);
        let claimed = self.claim_top(span, ty);
        if claimed != ty {
            return (claimed, false);
        }
        // A future, an either, or a provisional type of an enclosing
        // function, which may be a future for all this body knows. A path
        // is not an expression, so the await is recorded with the item's
        // path, by `finish_update`. A value that is not a record, or a list,
        // once awaited cannot be updated, awaited or not, which `walk_path`
        // reports instead.
        let stripped = self.types.strip(ty);
        let updatable = if list {
            self.types.list_element(stripped).is_some()
        } else {
            self.types.is_record(stripped)
        };
        if let Some(marker) = self.pure_marker()
            && updatable
        {
            let capture = match (variable, prefix) {
                (Some(binding), [first]) if self.awaitable_outside(binding) => first.name(),
                _ => None,
            };
            let error = self.implicit_await_error(span, ty, marker, capture);
            self.errors.push(error);
        }
        (ty, true)
    }

    /// Declares the variable of the dotted `let` or `:=` item `id`, whose
    /// path is `segments` and whose value has type `leaf_ty`: a new binding
    /// of the path's first segment, `a`, whose type is that of `a` in scope,
    /// or of `{}` if there is none, with the rest of the path set to
    /// `leaf_ty` (see the module documentation). Walks the path first (see
    /// [`Checker::walk_path`]). If `a` or a segment along the way is not a
    /// record, that is reported and the binding is `Error`.
    pub(super) fn declare_update(&mut self, id: ItemId, segments: &[PathSegment], leaf_ty: TyId) {
        let Some(name) = segments.first().and_then(PathSegment::name) else {
            return;
        };
        let base = self.path_types.len();
        // A dotted `let` of a top-level field's name has been reported as
        // "`a` is a field of this file"; the field is not what it updates.
        let clash = self.field_clash == Some(id);
        let mut found = if clash { None } else { self.lookup(name) };
        let mut failed = leaf_ty == TyId::ERROR || clash;
        if let Some((index, old)) = found {
            // An erroneous variable has been reported already.
            failed |= old.ty == TyId::ERROR || self.walk_path(segments, (old.ty, index)).is_none();
        }
        // A path through a value of type `Never` is never walked: the
        // update builds its records from nothing, as if no variable were in
        // scope, and is never run.
        if std::mem::take(&mut self.unreached_path) {
            (failed, found) = (leaf_ty == TyId::ERROR, None);
        }
        let leaf_ty = if failed { TyId::ERROR } else { leaf_ty };
        self.finish_update(id, segments, (found, base), leaf_ty);
    }

    /// Declares the variable of the dotted `let` or `:=` item `id`, whose
    /// path is `segments`, whose leaf is set to a value of type `leaf_ty`,
    /// `Error` if the update was rejected, and whose path has been walked
    /// (see [`Checker::walk_path`]): the records and lists along it that
    /// exist are the entries of `path_types` from `base` on, which this
    /// truncates back. `found` is the binding of the path's first segment in
    /// scope, with its index in `bindings`, if there is one. The new
    /// binding's type is built from the leaf outwards: each record along
    /// the path, as it was once measured or created empty, with its segment
    /// set, and each list as it was once measured, its element type kept
    /// (see [`Checker::updated_list`]). Records for lowering where the old
    /// record or list is read from, how many records and lists along the
    /// path exist, and which of them are awaited, as a [`PathResolution`].
    pub(super) fn finish_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (found, base): (Option<(usize, Binding)>, usize),
        leaf_ty: TyId,
    ) {
        let Some(name) = segments.first().and_then(PathSegment::name) else {
            return;
        };
        let ty = self.record_update(id, segments, (found, base), leaf_ty);
        self.declare(name, ty, id, BindingKind::Variable);
    }

    /// The type of the record or list that the dotted `let` or `:=` item
    /// `id`, whose path is `segments`, makes, as [`Checker::finish_update`]
    /// describes it, for whatever the item declares or rebinds; records
    /// for lowering how its path resolves, and truncates `path_types` back
    /// to `base`. `Error` if `leaf_ty` is, if an updated element does not
    /// fit its list, or if `segments` is empty.
    pub(super) fn record_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (found, base): (Option<(usize, Binding)>, usize),
        leaf_ty: TyId,
    ) -> TyId {
        let Some((_, rest)) = segments.split_first() else {
            self.path_types.truncate(base);
            return TyId::ERROR;
        };
        let existing = self.path_types.len().saturating_sub(base);
        let mut ty = leaf_ty;
        for (level, segment) in rest.iter().enumerate().rev() {
            if ty == TyId::ERROR {
                break;
            }
            let record = self
                .path_types
                .get(base + level)
                .map_or(TyId::EMPTY, |&(record, _)| record);
            ty = match segment.kind {
                SegmentKind::Name(name) => self
                    .types
                    .with_field(record, name, ty)
                    .unwrap_or(TyId::ERROR),
                // The list's prefix is the path up to it.
                SegmentKind::Index { .. } => {
                    self.updated_list(segments, &segments[..=level], (record, ty))
                }
            };
        }
        if let Some((index, old)) = found
            && existing > 0
        {
            // The old record may belong to an enclosing function, and is
            // then read from the closure's captures.
            let source = self.resolve(index, old);
            let records = self.path_types.get(base..).unwrap_or_default();
            // Most updates await nothing, and add no entries.
            let awaits = if records.iter().any(|&(_, awaited)| awaited) {
                let start = position(self.path_awaits.len());
                self.path_awaits
                    .extend(records.iter().map(|&(_, awaited)| awaited));
                start
            } else {
                PathResolution::NOT_AWAITED
            };
            self.item_paths[id.index()] = PathResolution {
                source,
                existing: position(existing),
                awaits,
            };
        }
        self.path_types.truncate(base);
        ty
    }

    /// The type of the list `list`, at `prefix` of the path `segments` of a
    /// `:=`, once its element is replaced by one of type `element`: `list`
    /// itself, present, since a list's type never changes, if the element
    /// is a subtype of its element type, as it is whenever the element type
    /// is present or the either shape. Otherwise, as for elements declared
    /// exactly `Future<{a: Int}>` of a path continuing through one, reports
    /// "cannot rebind `xs[0].a`: the elements of `xs` are `Future<{a:
    /// Int}>`, and the update makes one `{a: Int}`", at the path, and
    /// returns `Error`.
    pub(super) fn updated_list(
        &mut self,
        segments: &[PathSegment],
        prefix: &[PathSegment],
        (list, element): (TyId, TyId),
    ) -> TyId {
        let Some(declared) = self.types.list_element(list) else {
            return TyId::ERROR;
        };
        if self.types.is_subtype(element, declared) {
            return list;
        }
        let message = format!(
            "cannot rebind `{}`: the elements of `{}` are {}, and the update makes one {}",
            self.ast.show_path(segments),
            self.ast.show_path(prefix),
            self.in_message(declared),
            self.in_message(element)
        );
        self.errors
            .push(Diagnostic::error(message, path_span(segments)).with_note(
                "a list keeps its element type; declare the elements with a type that allows both",
            ));
        TyId::ERROR
    }
}
