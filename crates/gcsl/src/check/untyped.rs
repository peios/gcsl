//! Checking parameters written with no type: the type each takes from its
//! default, and when a top-level function's is found.
//!
//! # Untyped parameters
//!
//! A parameter written `NAME = DEFAULT` (see the parser's [Functions and
//! calls](crate::parser::functions#functions-and-calls)) has the type of its
//! default, as `let NAME = DEFAULT` would give it: its type expression says
//! nothing ([`TypeExprKind::Inferred`]), and resolves to what the checker
//! found for it (`Checker::type_untyped`), kept by the expression's id. The
//! default is checked once, with nothing expected of it, an untyped integer
//! settled to `Int`, so `n = 3` is an `Int`, `xs = [1, 2]` a `List<Int>`,
//! `m = machine()` what `machine` returns, and `xs = []`, which has no type
//! of its own, is "cannot infer the element type of `[]`", as `let xs = []`
//! is. A constant default is checked where the function expression is, in
//! slots that are free again after it; any other is its parameter's default
//! function, checked as an annotated parameter's is (see "Defaults" under
//! [Functions](crate::check::functions#functions)), so it sees the
//! parameters before its own, `fn f(a: Int, b = a * 2)`, and neither its own
//! nor a later one. A `return` in such a default is "a default its
//! parameter's type is inferred from cannot `return`", since its target
//! would be the default itself.
//!
//! **The type.** The parameter is what it would be had its type been
//! written: the type its default has once awaited, written bare
//! (`Checker::untyped_type`). So, `D` being the default's type and `T` what
//! `D` is with every future layer at its top removed (see
//! [`TypeTable::strip`](crate::types::TypeTable::strip)):
//!
//! - in a pure function it is `T`, present if the body measures it and the
//!   either shape if it never does, as any bare parameter of a pure function
//!   is (see [Inferred
//!   parameters](crate::check::inference#inferred-parameters));
//! - in a memoised function it is `D` awaited deeply (see
//!   [`TypeTable::present`](crate::types::TypeTable::present)), since a
//!   memoised function's arguments are the key its results are kept by;
//! - in an impure function it is the either shape of `T`, and so is every
//!   present position inside `T`, through record and list types and the
//!   members of a union, as writing `List<Int>` there means `List<Int |
//!   Future<Int>> | Future<...>` (`Checker::widened`; see
//!   [Futures](crate::check::futures#futures), "Bare types"). A position
//!   that says what it is, a `Future<...>` or an either shape, stays what
//!   it is.
//!
//! A function type and a type parameter are never made the either shape or
//! provisional, as neither is when written. A provisional type of an
//! enclosing function inside `T` is made present, as an annotation there
//! would have it. The default is then fitted to that type as an annotated
//! parameter's default is to its annotation: so a default that is a future
//! is awaited in a memoised function's default, and in a pure function is
//! the usual purity error, labelled at the function's `:`. From there on
//! the parameter is an annotated one in every rule: calls that give it are
//! checked against its type, calls that leave it out are given its default,
//! and the function's type shows it.
//!
//! **A definition's own.** A parameter of a definition's own written with no
//! type, `def test t(m = machine()) { ... }` (see
//! [Def-types](crate::check::deftypes#def-types)), is a parameter of its
//! body's function, whose default is the argument the definition calls that
//! function with: the argument is checked once, when the body's function is
//! typed, in the definition's frame, where it is evaluated, and the call
//! then only fits it to the parameter (as an argument checked to infer a
//! call's type parameters is fitted).
//!
//! **Top-level functions.** A top-level function's signature is declared
//! before any body is checked, so that functions may call each other in any
//! order (see [Top-level
//! functions](crate::check::toplevel#top-level-functions)), and an untyped
//! parameter's type is not known then: until it is, the function's signature
//! is *pending*, and its binding erroneous. When a group's bodies are about
//! to be checked, after the groups it mentions, the signature of each of its
//! members that is pending is found first (`Checker::type_signatures`): its
//! untyped parameters' defaults are checked, in the view of the top level
//! its body is checked in, and its binding, or its cell, is given the type
//! its signature then declares, which the group's bodies see. Members are
//! taken so that one whose parameters mention another comes after it, as far
//! as that can be. A default cannot need the signature it is part of: a read
//! of a top-level function whose signature is pending, the default's own
//! function or one on a cycle with it, is "cannot infer the type of `x`: its
//! default needs the type of `f`, which needs it", at the read, labelled
//! "`x` is written with no type" at the parameter and noted "write the
//! parameter's type, as in `x: T = ...`"; a call of a method that is one is
//! the same error, and so is a read of a prefix of the path of a `def` that
//! is one, the record that holds it (`Checker::pending_reference`). The
//! default is then erroneous, and so is its function, for its users. Nothing is checked twice and no check nests in another, so
//! a cycle is an error and never a loop. What was found for a signature is
//! kept until the function's own check, which takes it as its own
//! (`EarlyDefaults`).
//!
//! A function expression that is no top-level function has its untyped
//! parameters typed where it is checked. Its own name is not in scope in
//! its defaults, as in any default.

use std::collections::HashMap;

use crate::ast::{Effect, ExprId, ExprKind, FnDecl, FnId, ItemKind, Param, TypeExprId, TypeExprKind};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::mentions::{Need, Resolve};
use crate::names::Name;
use crate::source::Span;
use crate::types::{Laziness, TyId, TyKind};

use super::{Bareness, Checker, Constant, DefaultFn, Slot, bareness_of};

/// The label on what an untyped parameter's default must fit.
pub(super) const UNTYPED_WHY: &str = "this parameter's type is its default's, once awaited";

/// What a parameter's default is checked against (see
/// [`Checker::check_default_fn`]).
#[derive(Clone, Copy, Debug)]
pub(super) enum DefaultType {
    /// Its parameter's type, as resolved before any inference.
    Written(TyId),
    /// Nothing: its parameter is written with no type, and takes its type
    /// from it, in a function of this effect (see [Untyped
    /// parameters](crate::check::untyped#untyped-parameters)).
    Untyped(Effect),
}

/// What the checker keeps of the parameters written with no type (see
/// [Untyped parameters](crate::check::untyped#untyped-parameters)).
#[derive(Debug, Default)]
pub(super) struct UntypedChecked {
    /// The type found for each, by its type expression: `Error` for one
    /// whose default is erroneous.
    pub(super) types: HashMap<TypeExprId, TyId>,
    /// What was found of the defaults of each top-level function whose
    /// signature was found before its body was checked, until its own check
    /// takes it.
    pub(super) early: HashMap<FnId, EarlyDefaults>,
    /// The parameter whose default is being checked for its type, by name
    /// and where it is written, if one is.
    pub(super) typing: Option<(Name, Span)>,
    /// How many top-level functions' signatures are pending.
    pub(super) pending: usize,
}

/// What the check of a function's untyped parameters' defaults found, which
/// the function's own check then has of them as it has of its other
/// defaults (see [`Checker::check_fn`]).
#[derive(Debug, Default)]
pub(super) struct EarlyDefaults {
    /// Each one's default function, as its check found it.
    pub(super) default_fns: Vec<DefaultFn>,
    /// For each parameter, where one of those defaults first measures it.
    pub(super) measured: Vec<Option<Span>>,
    /// Each one's constant default, by its parameter's number: `None` for
    /// one in error.
    pub(super) constants: Vec<(usize, Option<Constant>)>,
    /// The copies of a top-level function's default functions checked
    /// before its body, which are its own (see
    /// [`Checker::add_default_copies`]).
    pub(super) copies: Vec<usize>,
}

impl Checker<'_> {
    /// Whether `param` is written with no type.
    pub(super) fn is_untyped(&self, param: &Param) -> bool {
        self.ast.type_expr(param.ty).kind == TypeExprKind::Inferred
    }

    /// Whether `function` has a parameter written with no type.
    pub(super) fn has_untyped(&self, function: FnId) -> bool {
        let params = self.ast.params(self.ast.fn_decl(function).params);
        params.iter().any(|param| self.is_untyped(param))
    }

    /// The type of a parameter written with no type, whose type expression
    /// is `id`, written at `span`: what was found for it; `Error` with an
    /// internal error if nothing was, as every one is typed before its type
    /// is asked for.
    pub(super) fn untyped_param(&mut self, id: TypeExprId, span: Span) -> TyId {
        if let Some(&ty) = self.calls.untyped.types.get(&id) {
            return ty;
        }
        self.errors.push(
            InternalError {
                message: "a parameter written with no type has none",
                span,
            }
            .into(),
        );
        TyId::ERROR
    }

    /// Whether the parameter written with no type whose type expression is
    /// `id` is bare, as [`Checker::is_bare`] says of a written type: unless
    /// its type is a function type or a type parameter, or erroneous.
    pub(super) fn untyped_is_bare(&self, id: TypeExprId) -> bool {
        let Some(&ty) = self.calls.untyped.types.get(&id) else {
            return false;
        };
        self.widens(self.types.strip(ty))
    }

    /// Whether the present type `ty`, written bare, would be the either
    /// shape in an impure function's parameter: not a function type or a
    /// type parameter, which are what they are, nor a type nothing writes.
    fn widens(&self, ty: TyId) -> bool {
        !matches!(
            self.types.kind(ty),
            TyKind::Fn(_)
                | TyKind::Param { .. }
                | TyKind::Builtin(_)
                | TyKind::Error
                | TyKind::Never
                | TyKind::UntypedInt
        )
    }

    /// The type of a parameter written with no type whose default has the
    /// type `found`, in a function of `effect` (see [Untyped
    /// parameters](#untyped-parameters)): as if it were written as the type
    /// `found` is once awaited. `Error` for `Error`.
    pub(super) fn untyped_type(&mut self, found: TyId, effect: Effect) -> TyId {
        if found == TyId::ERROR {
            return TyId::ERROR;
        }
        if effect == Effect::Memo {
            return self.types.present(found);
        }
        let mut ty = self.types.strip(found);
        // An enclosing function's parameter, which an annotation would
        // have present.
        if self.types.contains_provisional(ty, None) {
            ty = self.types.present(ty);
        }
        match bareness_of(effect).0 {
            Bareness::Present => ty,
            Bareness::Either => self.widened(ty),
        }
    }

    /// The present type `ty` as writing it bare in an impure function's
    /// parameter would mean it (see [Untyped
    /// parameters](#untyped-parameters)): its either shape, with every
    /// present position inside it, through record and list types and a
    /// union's members, widened in turn. A function type and a type
    /// parameter are left as they are, and so is a position that is a
    /// future, an either shape or provisional.
    ///
    /// Recurses once per level of record, list or union nesting.
    pub(super) fn widened(&mut self, ty: TyId) -> TyId {
        if self.types.laziness(ty) != Laziness::Present || !self.widens(ty) {
            return ty;
        }
        let inside = self.widened_inside(ty);
        self.types.deferrable(inside)
    }

    /// The present type `ty` with the positions inside it widened (see
    /// [`Checker::widened`]): a record's fields and rest-field, a list's
    /// element, and those of each member of a union.
    fn widened_inside(&mut self, ty: TyId) -> TyId {
        if let Some(members) = self.types.union_members(ty) {
            // A copy (ids only), since widening a member may intern types.
            let members = members.to_vec();
            let widened: Vec<TyId> = members
                .iter()
                .map(|&member| self.widened_inside(member))
                .collect();
            return if widened == members {
                ty
            } else {
                self.types.union(widened)
            };
        }
        if let Some(element) = self.types.list_element(ty) {
            let widened = self.widened(element);
            return if widened == element {
                ty
            } else {
                self.types.list(widened)
            };
        }
        let Some(fields) = self.types.record_fields(ty) else {
            return ty;
        };
        // A copy (ids only), since widening a field may intern types.
        let fields = fields.to_vec();
        let mut changed = false;
        let mut widened = Vec::with_capacity(fields.len());
        for (name, field) in fields {
            let wide = self.widened(field);
            changed |= wide != field;
            widened.push((name, wide));
        }
        let rest = self.types.record_rest(ty).map(|rest| {
            let wide = self.widened(rest);
            changed |= wide != rest;
            wide
        });
        if changed {
            self.types.record_like(ty, widened, rest)
        } else {
            ty
        }
    }

    /// Checks the default `default` of a parameter written with no type,
    /// whose type expression is `written`, in a function of `effect`, with
    /// nothing expected of it, and records the type the parameter takes
    /// from it (see [Untyped parameters](#untyped-parameters)), which the
    /// default is then fitted to as an annotated one is to its annotation.
    /// Returns the default's type, `Error` if the parameter's is.
    pub(super) fn check_untyped_default(
        &mut self,
        default: ExprId,
        written: TypeExprId,
        effect: Effect,
    ) -> TyId {
        let found = self.check_alone(default);
        let ty = self.untyped_type(found, effect);
        self.calls.untyped.types.insert(written, ty);
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        let context = self.ast.type_expr(written).span;
        self.expect(default, found, ty, context, UNTYPED_WHY);
        found
    }

    /// [`Checker::check_untyped_default`] for the default that is the body
    /// of the default function `decl`, in its frame: a `return` there, whose
    /// target the default would be, is "a default its parameter's type is
    /// inferred from cannot `return`", and makes the parameter erroneous.
    pub(super) fn check_untyped_default_fn(
        &mut self,
        decl: &FnDecl,
        written: TypeExprId,
        effect: Effect,
    ) -> TyId {
        let found = self.check_untyped_default(decl.body, written, effect);
        let returns = self
            .functions
            .last_mut()
            .map(|function| std::mem::take(&mut function.returns))
            .unwrap_or_default();
        let Some(first) = returns.first() else {
            return found;
        };
        self.errors.push(
            Diagnostic::error(
                "a default its parameter's type is inferred from cannot `return`",
                first.at,
            )
            .with_note("write the parameter's type, or give the default's block a label"),
        );
        self.calls.untyped.types.insert(written, TyId::ERROR);
        TyId::ERROR
    }

    /// Finds the type of parameter number `index` of `params`, the
    /// parameters of `function`, of `effect`, which is written with no
    /// type, from its default, and records it (see [Untyped
    /// parameters](#untyped-parameters)); `resolved` has the types of the
    /// parameters before it, before any inference. What its default's
    /// check found joins `early`. A parameter of a definition's body's
    /// function has as its default the argument its definition gives it.
    pub(super) fn type_untyped(
        &mut self,
        (function, effect): (FnId, Effect),
        (params, resolved): (&[Param], &[TyId]),
        index: usize,
        early: &mut EarlyDefaults,
    ) {
        let Some(param) = params.get(index) else {
            return;
        };
        let outer = self.calls.untyped.typing.replace((param.name, param.span));
        if let Some(default_fn) = param.default_fn {
            // A default in a default sees neither's.
            let unseen = self.defaults_checked.unseen.len();
            let later = params.get(index..).unwrap_or_default();
            let later = later.iter().map(|param| (param.name, param.span));
            self.defaults_checked.unseen.extend(later);
            let earlier = resolved.get(..index).unwrap_or_default();
            let expected = (DefaultType::Untyped(effect), param.ty);
            let (checked, demanded, needs) = self.check_default_fn(default_fn, (earlier, expected));
            self.defaults_checked.unseen.truncate(unseen);
            early.default_fns.push(DefaultFn {
                param: index,
                checked,
                needs,
            });
            early.measured.resize(params.len(), None);
            for (first, found) in early.measured.iter_mut().zip(demanded) {
                *first = first.or(found);
            }
        } else if let Some(default) = param.default {
            // A constant, which reads nothing: in slots of its own, free
            // again after it.
            let slots = (self.next_slot, self.frame_size);
            let names = std::mem::take(&mut self.slot_names);
            (self.next_slot, self.frame_size) = (0, 0);
            let errors = self.errors.len();
            let found = self.check_untyped_default(default, param.ty, effect);
            (self.next_slot, self.frame_size) = slots;
            self.slot_names = names;
            let constant = (found != TyId::ERROR && self.errors.len() == errors)
                .then(|| self.constant(default))
                .flatten();
            early.constants.push((index, constant));
        } else if let Some(argument) = self.own_argument(function, index) {
            let found = self.check_alone(argument);
            let ty = self.untyped_type(found, effect);
            self.calls.untyped.types.insert(param.ty, ty);
            // The call fits it, checked already.
            self.calls.prechecked.insert(argument, found);
        } else {
            // The parser gives every one a default.
            self.calls.untyped.types.insert(param.ty, TyId::ERROR);
            self.errors.push(
                InternalError {
                    message: "a parameter written with no type has no default",
                    span: param.span,
                }
                .into(),
            );
        }
        self.calls.untyped.typing = outer;
    }

    /// The argument the definition whose body's function is `function`
    /// gives its parameter number `index`, one of the definition's own (see
    /// [Def-types](crate::check::deftypes#def-types)); `None` for any other
    /// function.
    fn own_argument(&self, function: FnId, index: usize) -> Option<ExprId> {
        let &call = self.def_types.own_calls.get(&function)?;
        let ExprKind::Call { args, .. } = self.ast.get(call).kind else {
            return None;
        };
        self.ast.args(args).get(index).map(|arg| arg.value)
    }

    /// Before the bodies of the top-level functions `members`, a group, are
    /// checked: finds the signature of each whose signature is pending, one
    /// whose parameters mention another after it (see [Untyped
    /// parameters](#untyped-parameters)).
    pub(super) fn type_signatures(&mut self, members: &[u32]) {
        if self.calls.untyped.pending == 0 {
            return;
        }
        let pending: Vec<u32> = members
            .iter()
            .copied()
            .filter(|&member| self.is_pending(member))
            .collect();
        // Depth first, what a member's parameters mention before it, on a
        // stack of its own: each with the mentions still to visit.
        let mut seen = vec![false; pending.len()];
        let mut order = Vec::with_capacity(pending.len());
        for start in 0..pending.len() {
            if std::mem::replace(&mut seen[start], true) {
                continue;
            }
            let mut work = vec![(start, self.signature_mentions(start, &pending), 0)];
            while let Some((at, mentioned, next)) = work.last_mut() {
                if let Some(&callee) = mentioned.get(*next) {
                    *next += 1;
                    if !std::mem::replace(&mut seen[callee], true) {
                        let mentioned = self.signature_mentions(callee, &pending);
                        work.push((callee, mentioned, 0));
                    }
                    continue;
                }
                order.push(pending[*at]);
                work.pop();
            }
        }
        for member in order {
            self.type_top_signature(member as usize);
        }
    }

    /// Whether the signature of the top-level function `member` is pending.
    fn is_pending(&self, member: u32) -> bool {
        self.top_fns
            .get(member as usize)
            .is_some_and(|top| top.pending)
    }

    /// The members of `pending`, by their indices there, that the
    /// parameters of the member at `at` mention: those it first mentions
    /// before the end of its last untyped parameter's default. More than
    /// its untyped parameters' defaults read, which only orders them.
    fn signature_mentions(&self, at: usize, pending: &[u32]) -> Vec<usize> {
        let Some(&member) = pending.get(at) else {
            return Vec::new();
        };
        let Some(top) = self.top_fns.get(member as usize) else {
            return Vec::new();
        };
        let params = self.ast.params(self.ast.fn_decl(top.function).params);
        let end = params
            .iter()
            .filter(|param| self.is_untyped(param))
            .filter_map(|param| param.default)
            .map(|default| self.ast.get(default).span.end)
            .max()
            .unwrap_or(0);
        self.mentions
            .needs(Need::Function(member))
            .into_iter()
            .filter_map(|(need, span)| match (need, span) {
                (Need::Function(other), Some(span)) if span.start < end && other != member => {
                    pending.iter().position(|&pending| pending == other)
                }
                _ => None,
            })
            .collect()
    }

    /// Finds the signature of the top-level function `index`, which is
    /// pending: checks its untyped parameters' defaults in the view of the
    /// top level its body is checked in (see [`Checker::check_top_body`]),
    /// keeps what that found for its own check, and gives its binding, or
    /// its leaf's cell, the type its signature then declares.
    fn type_top_signature(&mut self, index: usize) {
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
        self.callee_name = None;
        let decl = *self.ast.fn_decl(top.function);
        let params = self.ast.params(decl.params);
        let mode = bareness_of(decl.effect).0;
        let mut early = EarlyDefaults::default();
        let mut resolved = Vec::with_capacity(params.len());
        for (at, param) in params.iter().enumerate() {
            if self.is_untyped(param) {
                self.type_untyped(
                    (top.function, decl.effect),
                    (params, &resolved),
                    at,
                    &mut early,
                );
            }
            // What is wrong with a written type the function's own check
            // reports.
            let (errors, warnings) = (self.errors.len(), self.warnings.len());
            resolved.push(self.resolve_type(param.ty, mode));
            self.errors.truncate(errors);
            self.warnings.truncate(warnings);
        }
        early.copies = std::mem::take(&mut self.defaults_checked.copies);
        self.calls.untyped.early.insert(top.function, early);
        self.bindings.extend(below);
        (self.early, self.checking_top) = (None, None);
        if let Some(top) = self.top_fns.get_mut(index) {
            top.pending = false;
        }
        self.calls.untyped.pending = self.calls.untyped.pending.saturating_sub(1);
        let ty = self.declared_type(top.function);
        if let Some(binding) = self.bindings.get_mut(top.binding) {
            binding.ty = ty;
        }
        if let Some(leaf) = top.leaf
            && let Some(cell) = self.field_cells.get_mut(leaf as usize)
            && cell.ty != Some(ty)
        {
            cell.ty = Some(ty);
            self.retype_above(leaf);
        }
        // A method's declaration, which its receiver's type was needed for.
        if matches!(self.ast.item(top.item).kind, ItemKind::Let { .. })
            && let Some(name) = top.name
        {
            self.check_method_named(name);
        }
    }

    /// Reports the use `id` of what the top level's `slot` holds, if that is
    /// a top-level function whose signature is pending (see [Untyped
    /// parameters](#untyped-parameters)), or the record of a root of such a
    /// function's path, and returns whether it is one.
    pub(super) fn pending_use(&mut self, id: ExprId, slot: Slot) -> bool {
        if self.calls.untyped.pending == 0 {
            return false;
        }
        // The function, or the record a root of its path is.
        let held = || {
            let node = self
                .field_cells
                .iter()
                .position(|cell| cell.slot == slot && slot != Slot::NONE)?;
            self.pending_under(u32::try_from(node).ok()?)
        };
        let Some(index) = self
            .top_fns
            .iter()
            .position(|top| top.pending && self.top_slot(top) == Some(slot))
            .or_else(held)
        else {
            return false;
        };
        let function = self.need_text(Need::Function(u32::try_from(index).unwrap_or(u32::MAX)));
        self.pending_error(self.ast.get(id).span, &function);
        true
    }

    /// The top-level function whose signature is pending that the node
    /// `node` of the tree of top-level fields, or of the file's view, is or
    /// has under it, if there is one: a `def` named by a dotted path.
    fn pending_under(&self, node: u32) -> Option<usize> {
        let tree = self.mentions.tree.nodes.len();
        self.top_fns.iter().position(|top| {
            let Some(leaf) = top.leaf.filter(|_| top.pending) else {
                return false;
            };
            // The leaf, the nodes above it, and the nodes of the file's
            // view that have one of those, children before parents.
            let mut above = vec![leaf];
            let mut at = leaf;
            while let Some(parent) = self
                .mentions
                .tree
                .nodes
                .get(at as usize)
                .and_then(|entry| entry.parent)
            {
                above.push(parent);
                at = parent;
            }
            for (number, view) in self.mentions.views.iter().enumerate().rev() {
                if view.children.iter().any(|child| above.contains(child)) {
                    above.push(u32::try_from(tree + number).unwrap_or(u32::MAX));
                }
            }
            above.contains(&node)
        })
    }

    /// Reports the reference `id` to a top-level field of the input (see
    /// [`Checker::field_reference`]), if the node it means is, or has
    /// under it, a function whose signature is pending, and returns whether
    /// it does: a `def` named by a dotted path, read by its path or as part
    /// of the record a prefix of its path is.
    pub(super) fn pending_reference(&mut self, id: ExprId) -> bool {
        if self.calls.untyped.pending == 0 {
            return false;
        }
        let found = match self.workspace {
            Some(workspace) => workspace.scope.reference(self.ast, id),
            None => self.mentions.tree.reference(self.ast, id),
        };
        let Some(index) = found.and_then(|(node, _)| self.pending_under(node)) else {
            return false;
        };
        let function = self.need_text(Need::Function(u32::try_from(index).unwrap_or(u32::MAX)));
        self.pending_error(self.ast.get(id).span, &function);
        true
    }

    /// Reports a use, at `span`, of the top-level function shown as
    /// `function`, whose signature is pending: "cannot infer the type of
    /// `x`: its default needs the type of `f`, which needs it", `x` being
    /// the parameter whose default is being checked.
    pub(super) fn pending_error(&mut self, span: Span, function: &str) {
        let Some((name, written)) = self.calls.untyped.typing else {
            // Only a default checked for its parameter's type is checked
            // while a signature is pending.
            self.errors.push(
                InternalError {
                    message: "a function is used before its parameters are typed",
                    span,
                }
                .into(),
            );
            return;
        };
        let param = self.ast.names().get(name);
        self.errors.push(
            Diagnostic::error(
                format!(
                    "cannot infer the type of `{param}`: its default needs the type of \
                     `{function}`, which needs it"
                ),
                span,
            )
            .with_context(written, format!("`{param}` is written with no type"))
            .with_note(format!(
                "write the parameter's type, as in `{param}: T = ...`"
            )),
        );
    }

    /// Whether the signature of a method of the input named `name` is
    /// pending (see [Untyped parameters](#untyped-parameters)).
    pub(super) fn pending_method(&self, name: Name) -> bool {
        self.calls.untyped.pending > 0
            && self
                .ast
                .methods()
                .iter()
                .filter(|method| method.name == name)
                .any(|method| {
                    self.top_fns
                        .iter()
                        .any(|top| top.pending && top.name == Some(method.binding))
                })
    }
}
