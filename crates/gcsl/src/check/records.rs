//! Checking record blocks: fields, dotted fields, spreads and soft assignment,
//! record types written in a block, and the dotted `let` and `:=`.
//!
//! # Records
//!
//! A block with at least one field or spread item is a *record block*. Its
//! value is the record of its fields, so it has no result: a `= EXPR` in it
//! is an error, and a final expression statement is a statement like any
//! other. A field is declared like a variable: it takes a slot, is visible
//! from the next item to the end of its block, shadows and is shadowed like
//! one, and is a binding marked as a field so that `:=` on it can be
//! rejected. A field item defines a *hard* field, which is defined once; a
//! spread defines *soft* ones, which later items may define again (see
//! [Spreads and soft assignment](#spreads-and-soft-assignment)). Each
//! block also records its fields in order in one flat vector of field
//! declarations, as it does its bindings, each where it was first defined,
//! and at the end of a record block the block's type is the record of those
//! fields.
//!
//! Where a type is *expected* (an annotation or an ascription), a subtype is
//! accepted (see
//! [`TypeTable::is_subtype`](crate::types::TypeTable::is_subtype)). A record
//! block written where a record type is expected, through any parentheses, is
//! checked against it directly by `Checker::check_expr_expecting`: each field's
//! initialiser is checked against the expected field's type, so literals adapt
//! and nested record blocks are checked the same way; a missing field is an
//! error, unless the record type is expected itself (not as a union's member)
//! and has a default for it, which the block is then given (see "Field
//! defaults" under [Functions](crate::check::functions#functions); `{}` is
//! checked so too where such a type is expected), and a field item the type
//! lacks is a *warning*, since nothing with the expected type can see it.
//! Warnings are collected apart from errors and returned with a successful
//! check.
//!
//! An expected record type with a rest-field, `*: T` (see
//! [`crate::types`], "Rest-fields"), lacks no field: each field item it
//! does not name is checked against `T` as against a field of the type,
//! and so is each soft field a spread defines and each dotted path's value,
//! and an annotated field's annotation must be a subtype of `T`. The
//! block's record then has the rest-field, which no record built otherwise
//! has: so a literal can be a value of such a type, and a value whose type
//! hides fields cannot (`Checker::check_rest_fields`). A field that dotted
//! paths build is a record without one, which does not fit a `T` that has
//! one: "mismatched types" at its first definition, noted to define it
//! whole. A record type with a rest-field reaches `{}` and the record
//! literals a block or an `if` results in as a record type with defaults
//! does, to give them the rest-field. A spread of a value whose type has a
//! rest-field is an error, since a spread defines the fields its operand's
//! type names; so is reading by name a field only the rest-field covers.
//!
//! A field item may be annotated, `PATH: TYPE = EXPR`, in any block, the
//! top level included, exactly as `let PATH: TYPE = EXPR` is for a `let`
//! (`Checker::annotated_value`): the annotation is resolved with every bare
//! type present, the value is checked against it as an annotated `let`'s
//! is (a literal adapting, a record block field by field, a value awaited
//! to fit where it may be, subject to the purity rule, a pure function's
//! parameter measured), and the field's type IS the annotation's, not the
//! value's narrower type; on a dotted path the annotation applies to the
//! last segment, and the segments above it are built from it as from any
//! leaf; a quoted name is annotated as a plain one. A top-level field's
//! annotated type is its leaf's, which every reader sees, an item above it
//! and another file alike (see [Top-level fields](crate::check::toplevel#top-level-fields)). A
//! top-level field that is annotated is never an alias of a built-in
//! function (see [Built-in functions](crate::check::calls#built-in-functions)), as an
//! annotated `let` is never one. In a record block checked against an
//! expected record type, an annotated field has its annotation's type,
//! which must be a subtype of the expected field's, "mismatched types" at
//! the annotation otherwise, and one the expected type lacks is warned
//! about as any field it lacks is. A `def` is not annotated: it has its
//! signature.
//!
//! # Dotted fields
//!
//! A field item whose path has several segments, `a.b.c = e`, declares the
//! field `a` as a record containing `b`, a record containing `c`. Dotted
//! definitions with a common prefix merge, as in TOML:
//!
//! ```text
//! server.host = "localhost"
//! server.port = 8080
//! server.tls.enabled = true
//! # one field, server = {host = ...; port = ...; tls = {enabled = true}}
//! ```
//!
//! A field whose first definition is dotted is *open*: it is one field
//! declaration and one binding, in one slot, and each further dotted path
//! under it extends it. The binding's type grows with each extension, so
//! after `server.host = "..."` the name `server` has type `{host: String}`
//! from the next item on, and after the next line `{host: String, port:
//! Int}`; its slot holds the record built so far. The record's fields are in
//! the order they were first mentioned, at every level, and the field takes
//! its place in its block where it was first mentioned.
//!
//! The rules, checked in the block declaring the field, each reported at the
//! offending path with a label on the earlier definition it conflicts with:
//!
//! 1. A field defined whole (`a = e`) cannot be extended: a later dotted
//!    path under it is "field `a` is already defined". Nor can a segment
//!    whose value a dotted path defined whole as a record (`a.b = { x = 1 }`,
//!    then `a.b.y = 2`: "field `a.b` is already defined"). A whole
//!    definition after a dotted one is "field `a` is defined twice", as for
//!    any field defined twice.
//! 2. A segment along the path that exists as some other value cannot hold
//!    fields: "cannot define `a.b.c`: `a.b` is Int, not a record".
//! 3. The path's last segment must not exist yet: "field `a.b` is defined
//!    twice".
//!
//! A rejected path declares nothing, but its value is still checked, and
//! its block's record is erroneous, so that the mistake does not cascade.
//!
//! To check these, the checker keeps a small tree of the segments of each
//! open field (`Node`), in one arena for the whole checker that each block
//! truncates back when it ends, as it does its field declarations. After an
//! extension, the types of the segments along its path are interned again,
//! from the leaf outwards.
//!
//! All of this, the binding that grows and the slot that holds the record
//! built so far, is how a dotted field of an INNER block works. At the top
//! level of an input the rules above are kept, in either order of the two
//! items, but the paths are order-free: each node of the tree is a cell of
//! its own, a reference means the longest declared prefix of its path, and
//! a root is assembled once from its children's cells, not item by item
//! (see [Top-level fields](crate::check::toplevel#top-level-fields)). So `a.b = 1; c = a; a.d =
//! 2` gives `c` the whole `a`, `{b = 1; d = 2}`. The types of the open
//! nodes above a top-level leaf are interned once, children first, when the
//! input's last item is checked (`Checker::settle_top_nodes`), so that a
//! root of N dotted items costs N steps, not N^2.
//!
//! In a record block checked against an expected record type, a dotted
//! field's value is checked against the type the expected type has at the
//! value's full path, as a field's value is against its field's type, if it
//! has one there. If it lacks a segment of the path, the value is checked
//! on its own and the missing segment is warned about as an extra field, at
//! its first definition only. When the block ends, each open field is
//! checked against the expected type at every level of nesting: a missing
//! field is an error, "missing field `a.x` in record", and a segment that
//! the expected type has as something other than a record is a mismatch.
//!
//! # Spreads and soft assignment
//!
//! A spread item, `...EXPR`, is a sequence of *soft assignments*, one for
//! each field of the record it spreads: `...{ x = 5; y = 2 }` is `x = 5; y =
//! 2`, except that what it defines may be defined again.
//!
//! ```text
//! let defaults = { host = "localhost"; port = 80 }
//! let conf = {
//!   ...defaults
//!   port = 8080
//!   url = "{host}:{port}"
//! }
//! # {host = "localhost"; port = 8080; url = "localhost:8080"}
//! ```
//!
//! The operand is checked on its own, then measured at the top, as the
//! record of a field access is (`Checker::measure`): awaited if it may be a
//! future, subject to the purity rule, and a pure function's bare parameter
//! measured; the await is marked on the operand, an expression, in
//! [`Resolution::await_of`](super::Resolution::await_of), as for a list's spread. It must then be a
//! record: "cannot spread a value of type T: it is not a record". The fields
//! it defines are exactly those of its type once measured, whatever else the
//! runtime value holds, since the static type is the truth: a field hidden
//! by width subtyping is not copied. So an ascription selects fields,
//! `...conf : {b: Int}` defining only `b`, the ascription belonging to the
//! operand, which is parsed as a field's value is; a bare type there is
//! present, so it awaits a field that may be a future, and `{b: Int |
//! Future<Int>}` keeps it lazy. `{}` spreads nothing. A type has no
//! definition order, so the fields are taken in the order the type table
//! holds them, by [`Name`], which is the order of the soft assignments and
//! of the places the fields take in the record.
//!
//! Each field is then softly assigned: it becomes a field of the block's
//! record and a name in scope for the block's later items, exactly as a
//! field item makes it, with the type the operand's type gives it, which
//! may be the either shape or a future, since nothing inside the operand is
//! awaited. A field so defined is *soft* (its `FieldDecl` records the
//! operand of the spread whose definition stands); one a field item defines
//! is *hard*, as a dotted field is.
//!
//! - A later field item of a soft field's name overrides it: the field is
//!   hard from then on, with the new value and type, which may differ.
//! - A later spread with a field of a soft field's name overrides it too,
//!   and the field stays soft.
//! - A spread with a field of a hard field's name is "field `x` is defined
//!   twice", at the operand, labelled "`x` is a field of this, of type T",
//!   with "first defined here" on the hard field's name and the note "a
//!   spread cannot replace a field defined before it; put the spread
//!   first": once for the spread, naming the first such field and counting
//!   the others in the note. The hard field stands; the spread's other
//!   fields are still defined, so that their uses do not cascade, and the
//!   block's record is erroneous.
//! - A field item after a field item is "defined twice", as ever.
//!
//! At the top level of an input it is different, since a top-level field or
//! `def` is declared in the whole input (see [Top-level
//! fields](crate::check::toplevel#top-level-fields)): it overrides a spread's field of its name
//! whether the spread is above it or below it, so a spread below one is no
//! longer "defined twice". The spread defines nothing and writes no slot
//! for a name that is a top-level declaration of the input, so the
//! declaration's cell is never overwritten, whichever is computed first;
//! where the spread comes first, the field keeps the place the spread gave
//! it in the record. A dotted root of a spread's name is "field `a` is
//! already defined" in either order: below the spread at the path, as
//! before, and above it at the spread's operand.
//!
//! An overridden field keeps its place in the record, where it was first
//! defined, and its one slot, which every definition of it writes, as every
//! definition of an open field does; the name, as a variable, is bound anew by
//! each definition, as a `let` shadows, in that slot, so the items between a
//! spread and an override see the spread's value and those after it the
//! override's. Only a block that has had a spread looks for a soft field when a
//! field item defines a name (`BlockSpreads`), so a block without one does no
//! more work than before. A spread holds its operand's value in a slot of its
//! own while its fields are read, the next free slot once they are declared,
//! which it leaves free and no binding owns; where each field is stored is
//! recorded in [`Resolution::spread_slots`](super::Resolution::spread_slots). A
//! dotted path whose first segment is a soft field is "field `a` is already
//! defined", as for a field defined whole; a dotted `let` of the name is an
//! ordinary update of the variable. `:=` of a soft field's name is the error
//! for any field.
//!
//! In a record block checked against an expected record type, each soft
//! field that stands when the block ends, one no later item overrode, is
//! checked against the expected field's type, if the type has the field,
//! as a field's value is, except that it cannot be awaited to fit, there
//! being no expression to await (`Checker::check_soft_fields`): a
//! provisional type in it where the expected type is present is measured,
//! at the operand; one that still does not fit is "mismatched types" at the
//! operand, "expected E, found F for field `a`", with the note "a spread's
//! field cannot be awaited; define the field with its value, as in `a =
//! conf.a`" when awaiting would have made it fit (the example written with
//! the operand when it is a name or a dotted path, and left out otherwise);
//! and one that fits takes the expected type, as a field item's does. A
//! soft field the expected type lacks is not warned about, since spreading
//! a wider record is normal. Missing fields are reported as ever.
//!
//! # Dotted `let` and `:=`
//!
//! `let a.b.c = e` is a functional update. If no `a` is in scope, it
//! declares `a` as `{b = {c = e}}`. If `a` is in scope with a record type,
//! it declares a new binding `a`, shadowing the old one as any `let` does,
//! whose value is the old value with the path set: each segment between the
//! first and the last that the old value's type has as a record is updated
//! inside, each it lacks is created as a record, and the last is replaced
//! if the type has it (its type may change) or added. An annotation applies
//! to the last segment: `let a.b: Int = 5`. A segment along the way that is
//! not a record, or `a` itself not being one, is an error: "cannot update
//! `a.b.c`: `a.b` is Int, not a record". The new type is computed by walking
//! the old one along the path and interning the updated record types from
//! the leaf outwards.
//!
//! `a.b.c := e` is an update too, but `:=` is assignment-compatible: `a`
//! must be something `:=` may rebind, every segment of the path must
//! already exist (`:=` never adds a field: "cannot rebind `a.b.c`: `a.b`
//! has no field `c`"), and `e` must fit the leaf's type, a subtype of it or
//! an adapting literal, which the new binding keeps. Changing a variable's
//! type, or adding a field, takes a `let`.
//!
//! The old value is never changed. An update builds a new record, sharing
//! the old one's unchanged fields, so another binding holding the old value
//! still sees the old record:
//!
//! ```text
//! let a = { x = 1 }
//! let old = a
//! let a.x = 2      # a is {x = 2}; old is still {x = 1}
//! ```
//!
//! **Order.** Every dotted `let` and dotted `:=`, of a mutable variable or
//! not, is evaluated in one order, Python's: first its value, once; then
//! the variable is read, and the path walked from its first segment, each
//! record (or list, see [Lists](crate::check::lists#lists)) along it read once and awaited
//! where it is marked to be, and each index segment's position evaluated
//! once, where the walk reaches it, left to right; and then the new value
//! is built from the leaf outwards and stored. So the update applies to the
//! variable as it is when the assignment happens: `r.a := { r := { a = 100;
//! b = 7 }; r.b + 1 }` makes `r` `{a = 8; b = 7}`, and a record along the
//! path that is a future is awaited after the value's effects. The order
//! changes nothing here: the checker walks the path before it checks the
//! value, since the value is checked against the leaf's type, and its
//! types, awaits, purity errors and their spans are the same whatever order
//! the program runs in (see [`crate::lower`] and [`crate::eval`] for how
//! the order is kept).
//!
//! The static type is the truth. An update that creates an intermediate
//! record where the static type has no such field replaces whatever the
//! runtime value holds there: a field hidden by subtyping, as in
//! `let p: {x: Int} = { x = 1; y = 2 }` followed by `let p.y.z = 3`, whose
//! `y` becomes `{z = 3}`.
//!
//! Updating a record reads it. Every record along the path that exists
//! (the variable's, and each following segment's, short of the last, that
//! the record before it has) is measured at the top, as the record of a
//! field access is (see [Futures](crate::check::futures#futures)): a pure function's bare
//! parameter is measured, which makes it present, and a record that may be
//! a future is awaited, subject to the purity rule, whose error is reported
//! at the path up to that segment. The rules above then apply to its type
//! once awaited, stripped of its future layers, so that a record that is
//! not one once awaited is "not a record", shown as the type it had before,
//! and then alone: the update cannot work whether or not it is awaited, so
//! the purity error is reported only for a value that is a record once
//! awaited. The new binding is built from the stripped types: the updated
//! variable is present, as is every record along the path, and the fields
//! off the path keep their types. After `let r = lazy { a = 1 }`, `let r.a
//! = 2` makes `r` a `{a: Int}`. The last segment is never measured: it is
//! replaced, not read, so for `:=` it keeps the field's type, which may be
//! the either shape. Dotted fields are not affected: they only extend
//! records their own block is building.
//!
//! The path of a `:=`, and only of a `:=`, may index lists too, `xs[i] :=
//! v` or `conf.hosts[0].up := true`, which replaces an element of a list:
//! see "Updating an element" under [Lists](crate::check::lists#lists).
//!
//! How a dotted item resolves (the slot of the record or list it updates, how
//! many of its path's segments name records or lists that already exist, and
//! which of those are awaited) is recorded for lowering as a
//! [`PathResolution`]. A path segment is not an expression, which
//! [`Resolution::await_of`](super::Resolution::await_of) could mark, so the
//! awaited records are listed in a table of their own,
//! [`Resolution::path_awaits`](super::Resolution::path_awaits), in which only
//! an item that awaits one has entries.

use crate::ast::{ExprId, ExprKind, FnId, ItemId, PathRange, PathSegment, TypeExprId, path_span};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::{Laziness, TyId};

use super::{
    Bareness, Binding, BindingKind, BlockSpreads, Checker, Constant, DOTTED_REST_NOTE,
    EXTRA_FIELD_NOTE, Expecting, FieldDecl, NO_NODE, Node, NodeKind, OpenField, PathResolution,
    SPREAD_REST_NOTE, Slot, VarRef, WHOLE_FIELD_NOTE, position, spread_first_note,
};

impl Checker<'_> {
    /// Checks `let PATH (: TYPE)? = VALUE` and declares the variable: with
    /// the value's type, or for a dotted path the type of the update. A
    /// `let mut` (`mutable`), whose path is a single name, is checked in
    /// the same way and declares a mutable variable, which remembers no
    /// function expression, since it may come to hold another function (see
    /// the module documentation).
    pub(super) fn check_let(
        &mut self,
        id: ItemId,
        path: PathRange,
        annotation: Option<TypeExprId>,
        (value, mutable): (ExprId, bool),
    ) {
        let expected = annotation.map(|annotation| {
            (
                self.resolve_type(annotation, Bareness::Present),
                self.ast.type_expr(annotation).span,
            )
        });
        let segments = self.ast.path(path);
        self.name_callee(segments, value);
        let ty = match expected {
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
            Some((expected, context)) => {
                let found = self.check_expr_expecting(
                    value,
                    expected,
                    context,
                    "expected because of this annotation",
                );
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
        };
        self.callee_name = None;
        match segments {
            [variable] => {
                // A `let` path's segments are all names.
                let Some(name) = variable.name() else {
                    return;
                };
                if mutable {
                    self.declare(name, ty, id, BindingKind::Mutable);
                } else {
                    let function = self.direct_function(segments, value);
                    self.declare_initialised(name, ty, id, (BindingKind::Variable, function));
                }
            }
            segments => self.declare_update(id, segments, ty),
        }
    }

    /// The function expression that the initialiser `value` of a `let` or
    /// field whose path is `segments` directly is, if the path is a single
    /// name: the same condition as [`Checker::name_callee`]'s.
    pub(super) fn direct_function(&self, segments: &[PathSegment], value: ExprId) -> Option<FnId> {
        match (segments, self.ast.get(value).kind) {
            ([_], ExprKind::Fn(function)) => Some(function),
            _ => None,
        }
    }

    /// Before the initialiser `value` of a `let` or field whose path is
    /// `segments` is checked: if the path is a single name and `value` is
    /// directly a function expression, lets that function see itself by
    /// the name (see the module documentation). The caller clears the name
    /// afterwards, in case the function did not take it.
    pub(super) fn name_callee(&mut self, segments: &[PathSegment], value: ExprId) {
        self.callee_name = match (segments, self.ast.get(value).kind) {
            ([single], ExprKind::Fn(_)) => single.name(),
            _ => None,
        };
    }

    /// Checks the field item `id`, `PATH = VALUE`, and declares or extends
    /// the field. With `expecting`, its block is checked against an
    /// expected record type: a field the type has is checked against that
    /// field's type and then has it, and one the type lacks is a warning.
    /// A single name that is a soft field of the block overrides it (see
    /// the module documentation): the field keeps its place and its slot,
    /// is hard from then on, and the name is bound anew. An annotated field,
    /// `PATH: TYPE = VALUE`, is checked against its annotation, as an
    /// annotated `let` is, and has its type (see [Records](#records)).
    pub(super) fn check_field(
        &mut self,
        id: ItemId,
        path: PathRange,
        (annotation, value): (Option<TypeExprId>, ExprId),
        expecting: Option<Expecting<'_>>,
    ) {
        let segments = self.ast.path(path);
        let [field] = segments else {
            self.check_dotted_field(id, segments, (annotation, value), expecting);
            return;
        };
        // A field's path's segments are all names.
        let Some(name) = field.name() else {
            self.check_alone(value);
            return;
        };
        let name_span = field.span;
        // The soft field of the name that this overrides, if any; only a
        // block with a spread can have one.
        let soft = match self.spreads {
            BlockSpreads::None => None,
            BlockSpreads::Some | BlockSpreads::Rejected => self
                .block_field(name)
                .filter(|&index| self.fields[index].spread.is_some()),
        };
        // A top-level field of the input: its leaf was computed at its
        // point, and its root declared as the input started.
        if let Some(ty) = self.computed_value(value) {
            self.reach_top_field(id, (name, name_span), ty, soft);
            return;
        }
        let expected =
            expecting.map(|expecting| (expecting, self.types.field_or_rest(expecting.ty, name)));
        self.name_callee(segments, value);
        let ty = match expected {
            _ if annotation.is_some() => {
                let ty = self.annotated_value(annotation, value);
                if let Some((expecting, expected)) = expected {
                    let named = (name, name_span);
                    self.annotated_against(annotation, ty, (expecting, expected), named);
                }
                ty
            }
            Some((expecting, Some(expected))) => {
                let found =
                    self.check_expr_expecting(value, expected, expecting.context, expecting.why);
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
            Some((expecting, None)) => {
                let found = self.check_expr(value);
                let ty = self.settle(value, found);
                let text = self.ast.names().get(name);
                let record = self.in_message(expecting.ty);
                self.warnings.push(
                    Diagnostic::warning(
                        format!("field `{}` is not in {record}", FieldName(text)),
                        name_span,
                    )
                    .with_context(expecting.context, expecting.why)
                    .with_note(EXTRA_FIELD_NOTE),
                );
                ty
            }
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
        };
        self.callee_name = None;
        let function = self.direct_function(segments, value);
        let Some(index) = soft else {
            self.declare_initialised(name, ty, id, (BindingKind::Field, function));
            self.fields.push(FieldDecl {
                name,
                name_span,
                ty,
                slot: self.item_slots[id.index()],
                open: None,
                spread: None,
                rejected: false,
            });
            return;
        };
        // The field keeps its place and its slot, and is hard from now on;
        // the name is bound anew, as a `let` shadows.
        let slot = self.fields[index].slot;
        self.bind(Binding {
            name,
            slot,
            ty,
            declared: Some(self.ast.item(id).span),
            kind: BindingKind::Field,
            function,
            callback: None,
        });
        self.item_slots[id.index()] = slot;
        let field = &mut self.fields[index];
        field.ty = ty;
        field.name_span = name_span;
        field.spread = None;
    }

    /// The type of the leaf of a dotted field item of an inner block, whose
    /// value is `value`, annotated `annotation` if it is, and whose last
    /// segment is `last`, its block checked against an expected record type
    /// that has the type `expected` at the item's path if it does: the
    /// annotation's, which applies to the last segment, as a dotted `let`'s
    /// does, the value checked against it ([`Checker::annotated_value`]);
    /// else the expected type, the value checked against it; else the
    /// value's.
    pub(super) fn dotted_leaf(
        &mut self,
        (annotation, value): (Option<TypeExprId>, ExprId),
        expected: Option<(Expecting<'_>, TyId)>,
        last: Option<&PathSegment>,
    ) -> TyId {
        if annotation.is_some() {
            let ty = self.annotated_value(annotation, value);
            if let (Some((expecting, expected)), Some(last)) = (expected, last)
                && let Some(name) = last.name()
            {
                let named = (name, last.span);
                self.annotated_against(annotation, ty, (expecting, Some(expected)), named);
            }
            return ty;
        }
        let Some((expecting, expected)) = expected else {
            return self.check_alone(value);
        };
        let found = self.check_expr_expecting(value, expected, expecting.context, expecting.why);
        if found == TyId::ERROR {
            TyId::ERROR
        } else {
            expected
        }
    }

    /// Checks `value`, the value of a field annotated `annotation`, against
    /// the annotation exactly as an annotated `let`'s value is checked
    /// against its own ([`Checker::check_let`]): the annotation resolved
    /// with every bare type present, the value checked against it, a
    /// literal adapting, a record block field by field, and a value that
    /// fits once awaited awaited, subject to the purity rule, a parameter
    /// of a pure function being measured. Returns the field's type: the
    /// annotation's, not the value's, or `Error`. Without an annotation, the
    /// value checked on its own, and its type.
    pub(super) fn annotated_value(
        &mut self,
        annotation: Option<TypeExprId>,
        value: ExprId,
    ) -> TyId {
        let Some(annotation) = annotation else {
            return self.check_alone(value);
        };
        let expected = self.resolve_type(annotation, Bareness::Present);
        let context = self.ast.type_expr(annotation).span;
        let found = self.check_expr_expecting(
            value,
            expected,
            context,
            "expected because of this annotation",
        );
        if found == TyId::ERROR {
            TyId::ERROR
        } else {
            expected
        }
    }

    /// Where a field annotated `annotation`, and so of type `ty`, is an item
    /// of a record block checked against an expected record type,
    /// `expecting`, whose type for the field is `expected`, if it has one:
    /// a field whose type is not a subtype of that is "mismatched types",
    /// at the annotation, and one the expected type lacks, `name` written
    /// at `name_span`, is the warning that any field it lacks is.
    pub(super) fn annotated_against(
        &mut self,
        annotation: Option<TypeExprId>,
        ty: TyId,
        (expecting, expected): (Expecting<'_>, Option<TyId>),
        (name, name_span): (Name, Span),
    ) {
        let Some(annotation) = annotation else {
            return;
        };
        let Some(expected) = expected else {
            let text = FieldName(self.ast.names().get(name)).to_string();
            let record = self.in_message(expecting.ty);
            self.warnings.push(
                Diagnostic::warning(format!("field `{text}` is not in {record}"), name_span)
                    .with_context(expecting.context, expecting.why)
                    .with_note(EXTRA_FIELD_NOTE),
            );
            return;
        };
        if ty == TyId::ERROR || expected == TyId::ERROR || self.types.is_subtype(ty, expected) {
            return;
        }
        let label = format!(
            "expected {}, found {}",
            self.in_label(expected),
            self.in_label(ty)
        );
        self.errors.push(
            Diagnostic::error("mismatched types", self.ast.type_expr(annotation).span)
                .with_label(label)
                .with_context(expecting.context, expecting.why),
        );
    }

    /// When the walk reaches the item `id` of a top-level field whose path
    /// is the single name `name`, written at `name_span`, and whose value
    /// has the type `ty`, computed at its point: gives the item its root's
    /// cell and adds the field to the record of the input, or makes the
    /// soft field `soft` of its name hard, keeping its place. Nothing is
    /// declared: the root's binding was, as the input started.
    pub(super) fn reach_top_field(
        &mut self,
        id: ItemId,
        (name, name_span): (Name, Span),
        ty: TyId,
        soft: Option<usize>,
    ) {
        let slot = self
            .mentions
            .tree
            .root(name)
            .and_then(|root| self.field_cells.get(root as usize))
            .map_or(Slot::NONE, |cell| cell.slot);
        self.item_slots[id.index()] = slot;
        match soft {
            Some(index) => {
                let field = &mut self.fields[index];
                (field.ty, field.slot, field.name_span) = (ty, slot, name_span);
                field.spread = None;
            }
            None => self.fields.push(FieldDecl {
                name,
                name_span,
                ty,
                slot,
                open: None,
                spread: None,
                rejected: false,
            }),
        }
    }

    /// The index in `fields` of the innermost block's field named `name`,
    /// by its first declaration, if it has one.
    pub(super) fn block_field(&self, name: Name) -> Option<usize> {
        self.fields[self.fields_start..]
            .iter()
            .position(|field| field.name == name)
            .map(|index| index + self.fields_start)
    }

    /// Whether `ty`, the type of a spread item's operand, written at
    /// `operand`, has a rest-field, which is reported at the item, at
    /// `item_span`: a spread defines the fields its type names, and those a
    /// rest-field covers are known only when it runs.
    pub(super) fn rest_spread(&mut self, ty: TyId, item_span: Span, operand: Span) -> bool {
        if self.types.record_rest(ty).is_none() {
            return false;
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        self.errors.push(
            Diagnostic::error(
                format!("cannot spread a value of type {shown}: it has a rest-field"),
                item_span,
            )
            .with_context(operand, label)
            .with_note(SPREAD_REST_NOTE),
        );
        true
    }

    /// Checks the spread item `id`, `...VALUE` (see the module
    /// documentation). Its operand is checked on its own and measured at
    /// the top (see [`Checker::measure`]), and must then be a record:
    /// "cannot spread a value of type T: it is not a record" otherwise, and
    /// one whose type has no rest-field, "cannot spread a value of type T:
    /// it has a rest-field" otherwise, since a spread defines the fields
    /// its type names, and the fields a rest-field covers are known only at
    /// runtime.
    /// Each field of the record's type, in the order the type table holds
    /// them, by name, is then softly assigned: declared as a soft field of
    /// the block, with the type the record's type gives it, and bound anew
    /// as a name in scope, in the field's one slot if it is a soft field
    /// already, which this overrides, keeping its place, and in the next
    /// free slot otherwise. A hard field of one of the names is not
    /// replaced, but reported: "field `x` is defined twice", at the
    /// operand, once for the spread, naming the first such field and
    /// counting the others. A spread rejected either way makes its block's
    /// record erroneous. The item's own slot, which holds the operand's
    /// value while its fields are read, is the next free slot once its
    /// fields are declared, and is left free.
    pub(super) fn check_spread_item(&mut self, id: ItemId, value: ExprId, top: bool) {
        let found = self.check_expr(value);
        let found = self.settle(value, found);
        let ty = self.measure(value, found);
        if self.spreads == BlockSpreads::None {
            self.spreads = BlockSpreads::Some;
        }
        let item_span = self.ast.item(id).span;
        let operand = self.ast.get(value).span;
        // An erroneous operand has been reported already.
        if ty == TyId::ERROR || self.rest_spread(ty, item_span, operand) {
            self.spreads = BlockSpreads::Rejected;
            return;
        }
        // An operand of type `Never` never gives a record: it spreads
        // nothing, and nothing after it runs.
        let fields = if ty == self.never {
            Some(&[][..])
        } else {
            self.types.record_fields(ty)
        };
        let Some(fields) = fields else {
            let shown = self.in_message(ty);
            let label = format!("this is {}", self.in_label(ty));
            self.errors.push(
                Diagnostic::error(
                    format!("cannot spread a value of type {shown}: it is not a record"),
                    item_span,
                )
                .with_context(operand, label),
            );
            self.spreads = BlockSpreads::Rejected;
            return;
        };
        let base = self.sorted_fields.len();
        self.sorted_fields
            .extend(fields.iter().map(|&(name, ty)| (name, 0, ty)));
        let end = self.sorted_fields.len();
        let start = self.spread_slots.len();
        // The first field defined before the spread, hard, with its type in
        // the operand and its declaration, and the number of others.
        let mut conflict: Option<(Name, TyId, FieldDecl)> = None;
        let mut others = 0_usize;
        for at in base..end {
            let Some(&(name, _, ty)) = self.sorted_fields.get(at) else {
                continue;
            };
            // A top-level declaration of the input overrides the spread's
            // field whichever is written first: the spread defines nothing
            // and writes no slot for its name. A spread above it holds the
            // name's place in the record for it, as a soft field would.
            if top && self.spread_over_top_field((name, ty), (value, operand)) {
                continue;
            }
            let slot = match self.block_field(name) {
                None => {
                    let slot = self.declare_binding(name, ty, Some(item_span), BindingKind::Field);
                    self.fields.push(FieldDecl {
                        name,
                        name_span: operand,
                        ty,
                        slot,
                        open: None,
                        spread: Some(value),
                        rejected: false,
                    });
                    slot
                }
                Some(index) if self.fields[index].spread.is_some() => {
                    let slot = self.fields[index].slot;
                    self.bind(Binding {
                        name,
                        slot,
                        ty,
                        declared: Some(item_span),
                        kind: BindingKind::Field,
                        function: None,
                        callback: None,
                    });
                    let field = &mut self.fields[index];
                    field.ty = ty;
                    field.name_span = operand;
                    field.spread = Some(value);
                    slot
                }
                Some(index) => {
                    match conflict {
                        None => conflict = Some((name, ty, self.fields[index])),
                        Some(_) => others += 1,
                    }
                    Slot::NONE
                }
            };
            self.spread_slots.push(slot);
        }
        self.sorted_fields.truncate(base);
        self.item_spreads[id.index()] = (
            position(start),
            position(self.spread_slots.len().saturating_sub(start)),
        );
        self.item_slots[id.index()] = Slot(self.next_slot);
        self.frame_size = self.frame_size.max(self.next_slot.saturating_add(1));
        if let Some(conflict) = conflict {
            self.report_spread_conflict(operand, conflict, others);
            self.spreads = BlockSpreads::Rejected;
        }
    }

    /// Reports that the spread whose operand is at `operand` has a field
    /// named `name`, of type `ty`, that the hard field `hard` was defined as
    /// before it, and `others` more such fields: "field `x` is defined
    /// twice", once for the spread (see the module documentation).
    pub(super) fn report_spread_conflict(
        &mut self,
        operand: Span,
        (name, ty, hard): (Name, TyId, FieldDecl),
        others: usize,
    ) {
        let text = FieldName(self.ast.names().get(name)).to_string();
        let note = spread_first_note(others);
        let label = format!("`{text}` is a field of this, of type {}", self.in_label(ty));
        self.errors.push(
            Diagnostic::error(format!("field `{text}` is defined twice"), operand)
                .with_label(label)
                .with_context(hard.name_span, "first defined here")
                .with_note(note),
        );
    }

    /// Checks the dotted field item `id`, `a.b.c = VALUE`, whose path is
    /// `segments`, and declares the open field `a` or extends it (see the
    /// module documentation). A path that breaks the rules is reported, and
    /// its value checked on its own, without declaring anything.
    pub(super) fn check_dotted_field(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (annotation, value): (Option<TypeExprId>, ExprId),
        expecting: Option<Expecting<'_>>,
    ) {
        // A field's path's segments are all names.
        let Some((first, first_name)) = segments
            .first()
            .and_then(|first| Some((first, first.name()?)))
        else {
            self.check_alone(value);
            return;
        };
        // Where the path attaches to what the block already has: the field's
        // declaration and the deepest existing node along the path, with the
        // number of segments that exist.
        let declared = self.block_field(first_name);
        let attach = match declared {
            None => None,
            Some(index) => {
                let field = self.fields[index];
                let Some(open) = field.open else {
                    let shown = self.ast.show_path(&segments[..1]);
                    self.errors.push(
                        Diagnostic::error(
                            format!("field `{shown}` is already defined"),
                            path_span(segments),
                        )
                        .with_context(field.name_span, "defined whole here")
                        .with_note(WHOLE_FIELD_NOTE),
                    );
                    self.fields[index].rejected = true;
                    self.check_rejected(value);
                    return;
                };
                let Some((node, existing)) = self.attach_point(open.node, segments) else {
                    self.fields[index].rejected = true;
                    self.check_rejected(value);
                    return;
                };
                Some((index, open, node, existing))
            }
        };
        let existing = attach.map_or(0, |(_, _, _, existing)| existing);
        let expected = expecting.and_then(|expecting| {
            self.expected_at(expecting, segments, existing)
                .map(|ty| (expecting, ty))
        });
        // A top-level field's leaf was computed at its point.
        let computed = self.computed_value(value);
        let leaf_ty = match computed {
            Some(ty) => ty,
            None => self.dotted_leaf((annotation, value), expected, segments.last()),
        };
        // The new nodes, from the first segment that does not exist yet.
        let (parent, root) = if let Some((_, open, node, _)) = attach {
            (node, open.node)
        } else {
            let root = self.add_node(NO_NODE, first_name, first.span, NodeKind::Open);
            (root, root)
        };
        let parent = self.add_path_nodes(parent, segments, existing.max(1));
        // `parent` is now the leaf. Its type is the value's, and the type of
        // each node above it changes: at once in an inner block, whose later
        // items see the record built so far, and once the block ends for a
        // top-level field, whose readers see its cells instead
        // (`Checker::settle_top_nodes`), so that a root of N dotted items
        // costs N steps, not N^2.
        self.nodes[parent as usize].ty = leaf_ty;
        let top_root = computed.and_then(|_| self.top_root_cell(first_name));
        if top_root.is_none() {
            let mut node = self.nodes[parent as usize].parent;
            while node != NO_NODE {
                let ty = self.node_type(node);
                let entry = &mut self.nodes[node as usize];
                entry.ty = ty;
                node = entry.parent;
            }
        }
        let ty = self.nodes[root as usize].ty;
        match attach {
            None => {
                let binding = self.open_binding(top_root, (first_name, ty), id);
                self.fields.push(FieldDecl {
                    name: first_name,
                    name_span: first.span,
                    ty,
                    slot: self.item_slots[id.index()],
                    open: Some(OpenField {
                        node: root,
                        binding,
                    }),
                    spread: None,
                    rejected: false,
                });
            }
            Some((index, open, _, existing)) => {
                self.fields[index].ty = ty;
                let binding = &mut self.bindings[open.binding];
                // A top-level root's binding has the whole root's type.
                if top_root.is_none() {
                    binding.ty = ty;
                }
                self.item_slots[id.index()] = binding.slot;
                // The field is in this block, so in the current frame. The
                // record a dotted field extends is one its block is building,
                // never a future.
                let source = VarRef::Local(binding.slot);
                self.item_paths[id.index()] = PathResolution::unawaited(source, position(existing));
            }
        }
    }

    /// Adds a node for each segment of the dotted field path `segments` from
    /// the one at `from` on, each a child of the one before it and the first
    /// of `parent`, the last a leaf and the others open, spanning the path up
    /// to it. Returns the last, the leaf.
    pub(super) fn add_path_nodes(
        &mut self,
        parent: u32,
        segments: &[PathSegment],
        from: usize,
    ) -> u32 {
        let last = segments.len() - 1;
        let mut parent = parent;
        for (index, segment) in segments.iter().enumerate().skip(from) {
            // A field's path's segments are all names.
            let Some(name) = segment.name() else {
                continue;
            };
            let span = path_span(&segments[..=index]);
            let kind = if index == last {
                NodeKind::Leaf
            } else {
                NodeKind::Open
            };
            parent = self.add_node(parent, name, span, kind);
        }
        parent
    }

    /// Finds where the path `segments`, whose first segment names the open
    /// field whose root node is `root`, attaches to that field: the deepest
    /// node along the path that exists, and the number of the path's
    /// segments that exist, at least 1 and short of all of them. Reports the
    /// path and returns `None` if it breaks a rule: a segment that exists as
    /// a leaf, or a last segment that exists at all.
    pub(super) fn attach_point(
        &mut self,
        root: u32,
        segments: &[PathSegment],
    ) -> Option<(u32, usize)> {
        let mut node = root;
        let last = segments.len() - 1;
        for (index, segment) in segments.iter().enumerate().skip(1) {
            let Some(child) = segment.name().and_then(|name| self.child(node, name)) else {
                return Some((node, index));
            };
            let found = self.nodes[child as usize];
            let prefix = self.ast.show_path(&segments[..=index]);
            let path = path_span(segments);
            if index == last {
                self.errors.push(
                    Diagnostic::error(format!("field `{prefix}` is defined twice"), path)
                        .with_context(found.span, "first defined here"),
                );
                return None;
            }
            match found.kind {
                NodeKind::Open => node = child,
                // Reported already.
                NodeKind::Leaf if found.ty == TyId::ERROR => return None,
                // A record, or one that may be or is a future, as one
                // defined whole where an either shape is expected is.
                NodeKind::Leaf if self.types.is_record(self.types.strip(found.ty)) => {
                    self.errors.push(
                        Diagnostic::error(format!("field `{prefix}` is already defined"), path)
                            .with_context(found.span, "defined whole here")
                            .with_note(WHOLE_FIELD_NOTE),
                    );
                    return None;
                }
                NodeKind::Leaf => {
                    let shown = self.ast.show_path(segments);
                    let ty = self.in_message(found.ty);
                    self.errors.push(
                        Diagnostic::error(
                            format!("cannot define `{shown}`: `{prefix}` is {ty}, not a record"),
                            path,
                        )
                        .with_context(found.span, format!("`{prefix}` defined here")),
                    );
                    return None;
                }
            }
        }
        // The last segment either exists or does not, and both return
        // above.
        None
    }

    /// The type `expecting` expects at the end of the path `segments`, of
    /// which the first `existing` segments are defined already, or `None`
    /// if it has none there. Walking the expected record type along the
    /// path stops at a segment it does not have, which is warned about as
    /// an extra field if this path defines it first, or at a type that is
    /// not a record, which is reported as a mismatch when the block ends.
    pub(super) fn expected_at(
        &mut self,
        expecting: Expecting<'_>,
        segments: &[PathSegment],
        existing: usize,
    ) -> Option<TyId> {
        let mut expected = expecting.ty;
        for (index, segment) in segments.iter().enumerate() {
            // The record the path builds is present, which the either shape
            // of a record type accepts.
            expected = self.present_member(expected);
            // Not a record: reported when the block ends.
            self.types.record_fields(expected)?;
            let name = segment.name()?;
            let Some(ty) = self.types.field_or_rest(expected, name) else {
                if index >= existing {
                    let text = self.ast.names().get(name);
                    let record = self.in_message(expected);
                    self.warnings.push(
                        Diagnostic::warning(
                            format!("field `{}` is not in {record}", FieldName(text)),
                            segment.span,
                        )
                        .with_context(expecting.context, expecting.why)
                        .with_note(EXTRA_FIELD_NOTE),
                    );
                }
                return None;
            };
            expected = ty;
        }
        Some(expected)
    }

    /// The type a present value is checked against where `expected` is
    /// expected: the present member of an either shape, which a present
    /// value satisfies by satisfying that, and `expected` itself otherwise,
    /// a `Future` included, which no present value satisfies.
    pub(super) fn present_member(&self, expected: TyId) -> TyId {
        if self.types.laziness(expected) == Laziness::Either {
            self.types.strip(expected)
        } else {
            expected
        }
    }

    /// Adds a node named `name`, at `span`, as the newest child of `parent`
    /// (or as a root, for [`NO_NODE`]), and returns its index. Its type is
    /// set by the caller.
    pub(super) fn add_node(&mut self, parent: u32, name: Name, span: Span, kind: NodeKind) -> u32 {
        let index = position(self.nodes.len());
        let next_sibling = match self.nodes.get_mut(parent as usize) {
            Some(parent) => std::mem::replace(&mut parent.first_child, index),
            None => NO_NODE,
        };
        self.nodes.push(Node {
            name,
            span,
            ty: TyId::EMPTY,
            kind,
            parent,
            first_child: NO_NODE,
            next_sibling,
        });
        index
    }

    /// The child of `node` named `name`, if it has one.
    pub(super) fn child(&self, node: u32, name: Name) -> Option<u32> {
        let mut child = self.nodes.get(node as usize)?.first_child;
        while let Some(entry) = self.nodes.get(child as usize) {
            if entry.name == name {
                return Some(child);
            }
            child = entry.next_sibling;
        }
        None
    }

    /// The type of the open node `node`: the record of its children's
    /// types, or `Error` if any of them is.
    pub(super) fn node_type(&mut self, node: u32) -> TyId {
        let base = self.sorted_fields.len();
        let mut erroneous = false;
        let mut child = self
            .nodes
            .get(node as usize)
            .map_or(NO_NODE, |n| n.first_child);
        while let Some(entry) = self.nodes.get(child as usize) {
            erroneous |= entry.ty == TyId::ERROR;
            self.sorted_fields.push((entry.name, 0, entry.ty));
            child = entry.next_sibling;
        }
        let ty = if erroneous {
            TyId::ERROR
        } else {
            let fields = self.sorted_fields.get(base..).unwrap_or_default();
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        self.sorted_fields.truncate(base);
        ty
    }

    /// Once the input's outermost block has had its last item: gives every
    /// open node the record of its children's types, children first, and
    /// every open field its root's type. A top-level dotted item leaves the
    /// nodes above its leaf as they were (see `Checker::check_dotted_field`),
    /// and this settles them in one pass over the arena, whose nodes come
    /// after their parents.
    pub(super) fn settle_top_nodes(&mut self) {
        for node in (0..self.nodes.len()).rev() {
            if self.nodes[node].kind == NodeKind::Open {
                let ty = self.node_type(u32::try_from(node).unwrap_or(NO_NODE));
                self.nodes[node].ty = ty;
            }
        }
        for index in 0..self.fields.len() {
            if let Some(open) = self.fields[index].open
                && let Some(node) = self.nodes.get(open.node as usize)
            {
                self.fields[index].ty = node.ty;
            }
        }
    }

    /// Checks `value` with no expected type, settling an untyped integer to
    /// `Int`, and returns its type.
    pub(super) fn check_alone(&mut self, value: ExprId) -> TyId {
        let found = self.check_expr(value);
        self.settle(value, found)
    }

    /// The type of the record block whose fields are the innermost block's,
    /// once they are all declared: the record of the fields, or `Error` if
    /// a field is defined twice, has an erroneous type or had a dotted path
    /// under it rejected, or a spread was rejected. With `expecting`,
    /// also checks each soft field that stands against the expected type
    /// (see [`Checker::check_soft_fields`]), and reports each field of the
    /// expected type that the block lacks,
    /// at any level of an open field, and each open field or segment of one
    /// that the expected type has as something other than a record, which
    /// make the type `Error` too.
    pub(super) fn record_type(&mut self, expecting: Option<Expecting<'_>>) -> TyId {
        let mut erroneous = self.spreads == BlockSpreads::Rejected;
        if self.spreads != BlockSpreads::None
            && let Some(expecting) = expecting
        {
            erroneous |= self.check_soft_fields(expecting);
        }
        let fields = self.fields.get(self.fields_start..).unwrap_or_default();
        let base = self.sorted_fields.len();
        self.sorted_fields.extend(
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| (field.name, position(index), field.ty)),
        );
        erroneous |= fields
            .iter()
            .any(|field| field.ty == TyId::ERROR || field.rejected);
        let sorted = self.sorted_fields.get_mut(base..).unwrap_or_default();
        sorted.sort_unstable_by_key(|&(name, index, _)| (name, index));
        // A name's first definition sorts first among its definitions, and
        // each later one is reported against it.
        let mut first: Option<(Name, &FieldDecl)> = None;
        for &(name, index, _) in &*sorted {
            let Some(field) = fields.get(index as usize) else {
                continue;
            };
            match first {
                Some((first_name, first)) if first_name == name => {
                    let text = self.ast.names().get(name);
                    self.errors.push(
                        Diagnostic::error(
                            format!("field `{}` is defined twice", FieldName(text)),
                            field.name_span,
                        )
                        .with_context(first.name_span, "first defined here"),
                    );
                    erroneous = true;
                }
                _ => first = Some((name, field)),
            }
        }
        if let Some(expecting) = expecting {
            erroneous |= self.report_missing_fields(base, expecting);
            erroneous |= self.check_open_fields(expecting);
        }
        let rest = expecting.and_then(|expecting| {
            let rest = self.types.record_rest(expecting.ty)?;
            erroneous |= self.check_rest_fields(expecting, rest);
            Some(rest)
        });
        let ty = if erroneous {
            TyId::ERROR
        } else {
            let sorted = self.sorted_fields.get(base..).unwrap_or_default();
            self.types.record_with_rest(
                sorted.iter().map(|&(name, _, ty)| (name, ty)),
                rest,
                Vec::new(),
            )
        };
        self.sorted_fields.truncate(base);
        ty
    }

    /// Checks that each field of the innermost block that `expecting`'s
    /// record type does not name fits its rest-field, `rest`, so that the
    /// block's record can have it (see [Records](#records)), and returns
    /// whether one does not. A field item, a spread's field and an
    /// annotated field were checked against `rest` as they were defined,
    /// and one that did not fit reported; an open field, whose record its
    /// dotted paths build has no rest-field of its own, may not fit a
    /// `rest` that has one, which is "mismatched types" at its first
    /// definition.
    pub(super) fn check_rest_fields(&mut self, expecting: Expecting<'_>, rest: TyId) -> bool {
        let mut erroneous = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            if field.ty == TyId::ERROR
                || rest == TyId::ERROR
                || self.types.field(expecting.ty, field.name).is_some()
                || self.types.is_subtype(field.ty, rest)
            {
                continue;
            }
            erroneous = true;
            if field.open.is_none() {
                continue;
            }
            let label = format!(
                "expected {}, found {}",
                self.in_label(rest),
                self.in_label(field.ty)
            );
            self.errors.push(
                Diagnostic::error("mismatched types", field.name_span)
                    .with_label(label)
                    .with_context(expecting.context, expecting.why)
                    .with_note(DOTTED_REST_NOTE),
            );
        }
        erroneous
    }

    /// Checks each soft field of the innermost block that stands, one that
    /// no later item overrode, and that `expecting`'s record type has,
    /// against the type it has there, as a field's value is (see the module
    /// documentation), except that it cannot be awaited to fit, since there
    /// is no expression to await: a provisional type in it where the
    /// expected type is present is measured, at the spread's operand, and a
    /// field that still does not fit is "mismatched types" at the operand,
    /// "expected E, found F for field `a`", with a note if awaiting it would
    /// have made it fit, and one if it lacks a defaulted field
    /// ([`Checker::default_note`]). A field that fits takes the expected type, as a
    /// field item's does. A soft field the type lacks is not warned about:
    /// spreading a wider record is normal. Returns whether anything was
    /// reported.
    pub(super) fn check_soft_fields(&mut self, expecting: Expecting<'_>) -> bool {
        let mut reported = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            let Some(operand) = field.spread else {
                continue;
            };
            let Some(expected) = self.types.field_or_rest(expecting.ty, field.name) else {
                continue;
            };
            // An unknown type has been reported.
            if expected == TyId::ERROR || field.ty == TyId::ERROR {
                continue;
            }
            let span = self.ast.get(operand).span;
            let found = if self.types.is_subtype(field.ty, expected) {
                None
            } else {
                match self.claim_against_at(span, field.ty, expected) {
                    Some(claimed) if self.types.is_subtype(claimed, expected) => None,
                    Some(claimed) => Some(claimed),
                    None => Some(field.ty),
                }
            };
            let Some(found) = found else {
                self.fields[index].ty = expected;
                continue;
            };
            let text = FieldName(self.ast.names().get(field.name)).to_string();
            let label = format!(
                "expected {}, found {} for field `{text}`",
                self.in_label(expected),
                self.in_label(found)
            );
            let mut error = Diagnostic::error("mismatched types", span)
                .with_label(label)
                .with_context(expecting.context, expecting.why);
            if self.types.needs_await(found, expected).is_some() {
                let example = self
                    .path_text(operand)
                    .map(|path| format!(", as in `{text} = {path}.{text}`"))
                    .unwrap_or_default();
                error = error.with_note(format!(
                    "a spread's field cannot be awaited; define the field with its value{example}"
                ));
            }
            if let Some(note) = self.default_note(found, expected) {
                error = error.with_note(note);
            }
            self.errors.push(error);
            reported = true;
        }
        reported
    }

    /// Reports each field of `expecting`'s record type that is not among the
    /// block's fields, `sorted_fields[base..]`, in alphabetical order, unless
    /// the block fills it (see [`Expecting::fill`]) and the type has a default
    /// for it: such a field is added to the block's fields, at the end of
    /// `sorted_fields`, with the type's type of it, and recorded, with its
    /// default, as given it
    /// ([`Resolution::record_fills`](super::Resolution::record_fills); see
    /// "Field defaults" under [Functions](crate::check::functions#functions)).
    /// Returns whether any was missing.
    pub(super) fn report_missing_fields(&mut self, base: usize, expecting: Expecting<'_>) -> bool {
        let names = self.ast.names();
        let present = self.sorted_fields.get(base..).unwrap_or_default();
        let mut missing: Vec<&str> = Vec::new();
        let mut filled: Vec<(Name, TyId, Constant)> = Vec::new();
        for &(name, ty) in self.types.record_fields(expecting.ty).unwrap_or_default() {
            if present
                .binary_search_by_key(&name, |&(present, _, _)| present)
                .is_ok()
            {
                continue;
            }
            let default = self
                .types
                .field_default(expecting.ty, name)
                .filter(|_| expecting.fill.is_some());
            match default {
                Some(default) => filled.push((name, ty, default.clone())),
                None => missing.push(names.get(name)),
            }
        }
        if let Some(block) = expecting.fill
            && !filled.is_empty()
        {
            filled.sort_unstable_by(|(a, _, _), (b, _, _)| names.get(*a).cmp(names.get(*b)));
            self.sorted_fields
                .extend(filled.iter().map(|&(name, ty, _)| (name, u32::MAX, ty)));
            let fills = filled
                .into_iter()
                .map(|(name, _, default)| (name, default))
                .collect();
            self.lowering.record_fills.insert(block, fills);
        }
        if missing.is_empty() {
            return false;
        }
        missing.sort_unstable();
        let record = self.in_message(expecting.ty);
        for text in missing {
            self.errors.push(
                Diagnostic::error(
                    format!("missing field `{}` in record", FieldName(text)),
                    expecting.close,
                )
                .with_context(expecting.context, expecting.why)
                .with_note(format!("the type {record} needs it")),
            );
        }
        true
    }

    /// Checks each open field of the innermost block that `expecting`'s
    /// record type has against the type it has there, at every level of
    /// nesting, through a work list rather than recursion. A segment is
    /// checked against a record type by its fields: each one it lacks is
    /// reported at the block's closing brace as "missing field `a.x` in
    /// record", and each open segment is checked in turn (a leaf was checked
    /// against its type when it was defined). A segment expected to be
    /// something other than a record is a mismatch, reported at its first
    /// definition. Returns whether anything was reported.
    pub(super) fn check_open_fields(&mut self, expecting: Expecting<'_>) -> bool {
        let mut reported = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            let Some(open) = field.open else {
                continue;
            };
            // An erroneous field has been reported already.
            if field.ty == TyId::ERROR {
                continue;
            }
            let Some(expected) = self.types.field_or_rest(expecting.ty, field.name) else {
                continue;
            };
            self.pending.push((open.node, expected));
            while let Some((node, expected)) = self.pending.pop() {
                reported |= self.check_open_node(node, expected, expecting);
            }
        }
        reported
    }

    /// Checks the open node `node` against the type `expected`, reporting a
    /// mismatch or each missing field, and queues its open children that
    /// the type has on `pending`. Returns whether anything was reported.
    pub(super) fn check_open_node(
        &mut self,
        node: u32,
        expected: TyId,
        expecting: Expecting<'_>,
    ) -> bool {
        let entry = self.nodes[node as usize];
        // An unknown type has been reported, and a record type never has a
        // field of one, but nothing is to be checked against it either way.
        if expected == TyId::ERROR || entry.ty == TyId::ERROR {
            return false;
        }
        // An open node is a present record, which the either shape of a
        // record type accepts.
        let record = self.present_member(expected);
        let Some(fields) = self.types.record_fields(record) else {
            let label = format!(
                "expected {}, found {}",
                self.in_label(expected),
                self.in_label(entry.ty)
            );
            self.errors.push(
                Diagnostic::error("mismatched types", entry.span)
                    .with_label(label)
                    .with_context(expecting.context, expecting.why),
            );
            return true;
        };
        let names = self.ast.names();
        let mut missing: Vec<&str> = Vec::new();
        for &(name, ty) in fields {
            match self.child(node, name) {
                None => missing.push(names.get(name)),
                Some(child) if self.nodes[child as usize].kind == NodeKind::Open => {
                    self.pending.push((child, ty));
                }
                Some(_) => {}
            }
        }
        // An open child the type does not name is checked against its
        // rest-field, if it has one.
        if let Some(rest) = self.types.record_rest(record) {
            let mut child = entry.first_child;
            while let Some(&found) = self.nodes.get(child as usize) {
                if found.kind == NodeKind::Open && self.types.field(record, found.name).is_none() {
                    self.pending.push((child, rest));
                }
                child = found.next_sibling;
            }
        }
        if missing.is_empty() {
            return false;
        }
        missing.sort_unstable();
        let path = self.node_path(node);
        let record = self.in_message(record);
        for text in missing {
            self.errors.push(
                Diagnostic::error(
                    format!("missing field `{path}.{}` in record", FieldName(text)),
                    expecting.close,
                )
                .with_context(expecting.context, expecting.why)
                .with_note(format!("the type {record} needs it")),
            );
        }
        true
    }

    /// The path from its field's name to the node `node`, as it is
    /// written, for diagnostics.
    pub(super) fn node_path(&self, node: u32) -> String {
        let names = self.ast.names();
        let mut texts = Vec::new();
        let mut current = node;
        while let Some(entry) = self.nodes.get(current as usize) {
            texts.push(FieldName(names.get(entry.name)).to_string());
            current = entry.parent;
        }
        texts.reverse();
        texts.join(".")
    }
}
