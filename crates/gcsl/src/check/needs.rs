//! Checking what contexts functions need: inferred requirements.
//!
//! # Needs
//!
//! A function type carries the contexts a call of it *needs* (see
//! [`crate::types`], "Functions"): those with no default that it may read
//! with no `with` of its own around the read, by their declarations' keys.
//! The checker infers them. A read of a context and a call each need
//! something (`Checker::charge_needs`):
//!
//! - a read `^x` (`Checker::check_context_read`) of a context with no
//!   default needs `x`; one of a context with a default needs what its
//!   default's hidden function needs, each need met by binding `x` too:
//!   the binding's type's, or, for another file's, the type table's record
//!   of it
//!   ([`TypeTable::context_needs`](crate::types::TypeTable::context_needs)),
//!   recorded by the declaring file as its check ends
//!   (`Checker::record_context_needs`);
//! - a call (`Checker::call_effect`) needs its callee's type's, whatever the
//!   callee, a named function, a method, a parameter or any other function
//!   value, and the defaults of the parameters it leaves out need theirs,
//!   [`ParamDefault::Function`](super::ParamDefault::Function)'s, since a
//!   default is evaluated on the caller's side (`Checker::record_bound`):
//!   `f()` of `fn f(^arch: String)` needs `arch`, `f(arch = "x")` nothing.
//!
//! What a read or a call needs is charged to the innermost function around it
//! that is not a `lazy` (a `Function` frame with a result), less the contexts
//! the `with`s inside that function bind around it: those past the frame's
//! `with_depth` in `ContextsChecked::bound`. A call runs in its caller's context, so a
//! `with` around a closure's definition covers nothing in its body; a `lazy`
//! captures the context it is made in, so what its body needs is needed where
//! it is made, and awaiting it needs nothing. The frame collects its needs,
//! each with where it was first needed, and the function's type takes them
//! when its body is checked (`Checker::check_fn`); a default function's are
//! its parameter's. With no function around it, a read or a call is at the
//! top level, where an uncovered need is "context `arch` has no value here",
//! noted with what needs it ([`NeedWhy`]); of the needs one read or call
//! leaves unmet, only those no other implies are reported (`strongest`), so
//! that one cause is one error. A definition's type is its
//! def-type's exactly, so one that needs a context its def-type's function
//! type does not is "context `arch` has no value in this definition".
//!
//! A top-level function's uses inside its own group of mutual recursion see
//! its declared type, which needs nothing, before its body is checked. So
//! each use of a member there is recorded (`Checker::note_member_use`), with
//! the contexts covering it, the function it is charged to and whether it is
//! a call. Once the group is checked (`Checker::resolve_group_needs`), each
//! member's needs are what its body needs joined, until nothing changes, with
//! what each member it calls directly, from its own body or a `lazy` of it,
//! needs less what covers the call; its type takes them. Any other use, a call
//! from a closure or a default inside the group, or the member as a value, was
//! typed before its needs were known: one whose needs are not all covered is
//! "cannot tell that `f` needs context `arch` here", never a need dropped.
//! A definition among the members that comes to need a context is reported
//! as one whose body needs it is.

use crate::ast::{ExprId, ExprKind, FnId, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::names::{Name, key_context};
use crate::source::Span;
use crate::types::{Need, TyId, implies, met};

use super::{BodyState, Checker, Slot};

/// Why a read or a call at the top level needs a context, for the note of
/// its error (see [Needs](#needs)).
#[derive(Clone, Copy, Debug)]
pub(super) enum NeedWhy {
    /// A read of the context, which has no default.
    Read,
    /// A read of the context named, whose default needs the context.
    Default(Name),
    /// A call of the callee named, if it has a name.
    Call(Option<Name>),
    /// The defaults of the parameters a call leaves out.
    Defaults,
}

/// Of `items`, each with a need that `need_of` gives, those whose need no
/// other's implies, in order: so that one cause is one error, `arch`'s, not
/// that and `arch | target`'s too.
fn strongest<T>(items: &[T], need_of: impl Fn(&T) -> &Need) -> Vec<&T> {
    items
        .iter()
        .filter(|item| {
            let need = need_of(item);
            !items.iter().any(|other| {
                let other = need_of(other);
                other != need && implies(other, need)
            })
        })
        .collect()
}

/// What a [`MemberUse`] uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UseOf {
    /// A member of the group being checked, by its index in
    /// `Checker::top_fns`.
    Top(usize),
    /// A function that is no top-level one, through its own name for itself,
    /// its callee binding, named so.
    Local(FnId, Name),
}

/// A use, inside its own body or its group, of a function whose needs are
/// not known yet: a member of the group of top-level functions being
/// checked, or a function through its callee binding (see [Needs](#needs)).
#[derive(Clone, Debug)]
pub(super) struct MemberUse {
    /// The use.
    expr: ExprId,
    /// What it uses.
    of: UseOf,
    /// The function its needs are charged to, if it is in one.
    target: Option<FnId>,
    /// The contexts the `with`s inside that function bind around it.
    covered: Vec<Name>,
    /// Whether it is the callee of a call.
    call: bool,
}

/// What the checker keeps of the contexts bound and of what functions need
/// (see [Needs](#needs)).
#[derive(Debug, Default)]
pub(super) struct ContextsChecked {
    /// The contexts the `with`s around the expression being checked bind,
    /// by their declarations' keys, innermost last (see
    /// [Context](crate::check::context#context)).
    pub(super) bound: Vec<Name>,
    /// The uses of functions whose needs are not known yet, inside their own
    /// bodies or groups.
    uses: Vec<MemberUse>,
    /// The contexts the root of the input's run binds, which the host sets,
    /// by their keys: met at the top level as a `with` there meets them.
    pub(super) root: Vec<Name>,
    /// The contexts an input of no workspace's file sees, by their names, a
    /// host's built call among them: those its `with`s bind.
    pub(super) known: Vec<(Name, crate::workspace::SeenContext)>,
}

impl Checker<'_> {
    /// The index in `functions` of the frame a need is charged to: the
    /// innermost that is no `lazy`; `None` at the top level.
    fn need_target(&self) -> Option<usize> {
        self.functions
            .iter()
            .rposition(|function| function.result.is_some())
    }

    /// The contexts bound around the expression being checked inside the
    /// frame `target`, or anywhere at the top level.
    fn covered_in(&self, target: Option<usize>) -> &[Name] {
        let depth = target
            .and_then(|at| self.functions.get(at))
            .map_or(0, |function| function.with_depth);
        self.contexts.bound.get(depth..).unwrap_or_default()
    }

    /// Records in the type table what the default of each context the input
    /// declares needs, its function's type's, now that every body is
    /// checked, for other files' reads (see [Needs](#needs)).
    pub(super) fn record_context_needs(&mut self) {
        let Some(workspace) = self.workspace else {
            return;
        };
        let mut found = Vec::new();
        for decl in self.ast.contexts() {
            let (Some(binding), Some(seen)) = (decl.binding, workspace.context(decl.name)) else {
                continue;
            };
            if let Some((_, binding)) = self.lookup(binding) {
                found.push((seen.key, self.types.needs(binding.ty).to_vec()));
            }
        }
        for (key, needs) in found {
            self.types.set_context_needs(key, &needs);
        }
    }

    /// Whether a `with` binds the context of the key `key` around the
    /// expression being checked, inside the function its needs are charged
    /// to, or anywhere at the top level.
    pub(super) fn is_covered(&self, key: Name) -> bool {
        let target = self.need_target();
        self.covered_in(target).contains(&key)
            || (target.is_none() && self.contexts.root.contains(&key))
    }

    /// Charges `needs`, what a read or a call at `span` needs, for the
    /// reason `why`, to the function it is in, less what covers it there; at
    /// the top level, reports each that nothing covers (see [Needs](#needs)).
    pub(super) fn charge_needs(&mut self, needs: &[Need], span: Span, why: NeedWhy) {
        if needs.is_empty() {
            return;
        }
        let target = self.need_target();
        let covered = self.covered_in(target);
        // At the top level, what the host binds at the root meets a need too.
        let root: &[Name] = if target.is_none() {
            &self.contexts.root
        } else {
            &[]
        };
        let unmet: Vec<Need> = needs
            .iter()
            .filter(|need| !met(need, covered) && !met(need, root))
            .cloned()
            .collect();
        if let Some(frame) = target.and_then(|at| self.functions.get_mut(at)) {
            for need in unmet {
                if !frame.needs.iter().any(|(had, _)| *had == need) {
                    frame.needs.push((need, span));
                }
            }
            return;
        }
        for need in strongest(&unmet, |need| need) {
            let error = self.no_value_here(need, span, why);
            self.errors.push(error);
        }
    }

    /// The name of the context that `need` needs, its first.
    fn context_text(&self, need: &[Name]) -> &str {
        let Some(&key) = need.first() else {
            return "";
        };
        let text = self.ast.names().get(key);
        key_context(text).unwrap_or(text)
    }

    /// "context `arch` has no value here", at `span`, noted with `why`.
    fn no_value_here(&self, need: &[Name], span: Span, why: NeedWhy) -> Diagnostic {
        let names = self.ast.names();
        let note = match why {
            NeedWhy::Read => "no `with` sets it, and it declares no default".to_owned(),
            NeedWhy::Default(read) => format!(
                "the default of `^{}` reads it; no `with` sets it, and it declares no default",
                names.get(read)
            ),
            NeedWhy::Call(Some(callee)) => format!(
                "`{}` needs it, and no `with` around this call sets it",
                names.get(callee)
            ),
            NeedWhy::Call(None) => {
                "the function called needs it, and no `with` around this call sets it".to_owned()
            }
            NeedWhy::Defaults => "the default of a parameter this call leaves out needs it, and \
                                  no `with` around the call sets it"
                .to_owned(),
        };
        let text = self.context_text(need);
        Diagnostic::error(format!("context `{text}` has no value here"), span).with_note(note)
    }

    /// The name a call's callee `callee` is written as, through parentheses:
    /// a variable's, or the last of a field path's or a method's.
    pub(super) fn callee_written_name(&self, callee: ExprId) -> Option<Name> {
        let mut node = callee;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        match self.ast.get(node).kind {
            ExprKind::Name(name) | ExprKind::Field { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Reports each of `needs` of a definition beyond `allowed`, its
    /// def-type's function type's: "context `arch` has no value in this
    /// definition" (see [Needs](#needs)).
    pub(super) fn definition_needs(&mut self, needs: &[(Need, Span)], allowed: &[Need]) {
        let unmet: Vec<(Need, Span)> = needs
            .iter()
            .filter(|(need, _)| !allowed.iter().any(|allowed| implies(allowed, need)))
            .cloned()
            .collect();
        for (need, span) in strongest(&unmet, |(need, _)| need) {
            let (text, span) = (self.context_text(need), *span);
            let error = Diagnostic::error(
                format!("context `{text}` has no value in this definition"),
                span,
            )
            .with_note(
                "its def-type calls a definition with no context bound: bind it with a `with` \
                 inside the definition, or give the context a default",
            );
            self.errors.push(error);
        }
    }

    /// The note on a mismatch of a function of type `found` with the function
    /// type `expected` when `found` needs a context `expected` does not list:
    /// "a function that needs `^arch` fits only a function type that needs it
    /// too", saying to bind it inside the function. `None` otherwise.
    pub(super) fn needs_note(&self, found: TyId, expected: TyId) -> Option<String> {
        let (found, expected) = (self.types.fn_sig(found)?, self.types.fn_sig(expected)?);
        let allowed = self.types.need_set(expected.needs);
        let need = self
            .types
            .need_set(found.needs)
            .iter()
            .find(|need| !allowed.iter().any(|allowed| implies(allowed, need)))?;
        let text = self.context_text(need);
        Some(format!(
            "a function that needs `^{text}` fits only a function type that needs it too; bind \
             it with a `with` inside the function to need nothing"
        ))
    }

    /// Notes the use `id` of what the top level's `slot` holds, if that is
    /// a member of a group whose bodies are being checked (see
    /// [Needs](#needs)).
    pub(super) fn note_member_use(&mut self, id: ExprId, slot: Slot) {
        if self.checking_top.is_none() {
            return;
        }
        let Some(member) = self
            .top_fns
            .iter()
            .position(|top| top.state == BodyState::Checking && self.top_slot(top) == Some(slot))
        else {
            return;
        };
        self.push_use(id, UseOf::Top(member));
    }

    /// Notes the use `id` of the callee binding `name`, at `index` in
    /// `bindings`: of the function whose frame declares it, a member of its
    /// group if it is a top-level function (see [Needs](#needs)).
    pub(super) fn note_callee_use(&mut self, id: ExprId, (index, name): (usize, Name)) {
        let Some(owner) = self
            .functions
            .iter()
            .rev()
            .find(|function| function.bindings_start <= index)
            .map(|function| function.fn_id)
        else {
            return;
        };
        let of = match self.top_fns.iter().position(|top| top.function == owner) {
            Some(member) => UseOf::Top(member),
            None => UseOf::Local(owner, name),
        };
        self.push_use(id, of);
    }

    /// Records the use `id` of `of`, with the function it is charged to and
    /// what covers it there.
    fn push_use(&mut self, id: ExprId, of: UseOf) {
        let target = self.need_target();
        let covered = self.covered_in(target).to_vec();
        let target = target
            .and_then(|at| self.functions.get(at))
            .map(|function| function.fn_id);
        self.contexts.uses.push(MemberUse {
            expr: id,
            of,
            target,
            covered,
            call: false,
        });
    }

    /// Marks the use that `callee` is, through parentheses, if it is one
    /// [`Checker::note_member_use`] noted, as a call.
    pub(super) fn note_member_call(&mut self, callee: ExprId) {
        let mut node = callee;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        if let Some(found) = self
            .contexts
            .uses
            .iter_mut()
            .rev()
            .find(|used| used.expr == node)
        {
            found.call = true;
        }
    }

    /// Gives the function expression `id`, of the function `function` and the
    /// type `ty`, what its body was found to need, `needs`, each with where it
    /// was first needed, and checks its uses through its own name inside it
    /// (see [Needs](#needs)). Returns its type.
    pub(super) fn give_fn_needs(
        &mut self,
        (id, function): (ExprId, FnId),
        ty: TyId,
        needs: Vec<(Need, Span)>,
    ) -> TyId {
        let needs: Vec<Need> = needs.into_iter().map(|(need, _)| need).collect();
        let mut ty = ty;
        if !needs.is_empty() && ty != TyId::ERROR {
            ty = self.types.with_needs(ty, &needs);
            self.expr_types[id.index()] = ty;
        }
        self.resolve_local_needs(function, &needs);
        ty
    }

    /// Gives the members of a checked group, `members`, their needs, and
    /// checks the uses of them inside the group whose types were fixed before
    /// those were known (see [Needs](#needs)).
    pub(super) fn resolve_group_needs(&mut self, members: &[u32]) {
        let tops: Vec<usize> = members.iter().map(|&member| member as usize).collect();
        let (uses, others): (Vec<MemberUse>, Vec<MemberUse>) =
            std::mem::take(&mut self.contexts.uses)
                .into_iter()
                .partition(|used| matches!(used.of, UseOf::Top(top) if tops.contains(&top)));
        self.contexts.uses = others;
        let place = |top: usize| tops.iter().position(|&at| at == top);
        let function_of = |checker: &Self, target: FnId| {
            tops.iter().position(|&top| {
                checker
                    .top_fns
                    .get(top)
                    .is_some_and(|found| found.function == target)
            })
        };
        let mut needs: Vec<Vec<(Need, Option<ExprId>)>> = tops
            .iter()
            .map(|&top| {
                let ty = self
                    .top_fns
                    .get(top)
                    .map_or(TyId::ERROR, |top| self.expr_types[top.expr.index()]);
                self.types
                    .needs(ty)
                    .iter()
                    .map(|need| (need.clone(), None))
                    .collect()
            })
            .collect();
        // The calls from a member's own body, the fixpoint's edges: caller,
        // callee, the use and what covers it.
        let mut edges = Vec::new();
        let mut rest = Vec::new();
        for used in uses {
            let caller = used.target.and_then(|target| function_of(self, target));
            let UseOf::Top(member) = used.of else {
                continue;
            };
            match (used.call, caller, place(member)) {
                (true, Some(from), Some(to)) => edges.push((from, to, used)),
                _ => rest.push(used),
            }
        }
        loop {
            let mut grew = false;
            for (caller, callee, used) in &edges {
                let wanted: Vec<Need> = needs[*callee]
                    .iter()
                    .map(|(need, _)| need)
                    .filter(|need| !met(need, &used.covered))
                    .cloned()
                    .collect();
                for need in wanted {
                    if !needs[*caller].iter().any(|(had, _)| *had == need) {
                        needs[*caller].push((need, Some(used.expr)));
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        for (at, &top) in tops.iter().enumerate() {
            self.give_member_needs(top, &needs[at]);
        }
        for used in rest {
            let UseOf::Top(member) = used.of else {
                continue;
            };
            let Some(callee) = place(member) else {
                continue;
            };
            let found: Vec<Need> = needs[callee].iter().map(|(need, _)| need.clone()).collect();
            self.check_early_use(&used, &found);
        }
    }

    /// Checks the uses of the function `function`, not a top-level one, made
    /// through its callee binding in its own body, now that its needs, `needs`,
    /// are known: a direct call from its body, or a `lazy` of it, needs
    /// nothing more, and any other use is checked (see [Needs](#needs)).
    pub(super) fn resolve_local_needs(&mut self, function: FnId, needs: &[Need]) {
        let (uses, others): (Vec<MemberUse>, Vec<MemberUse>) =
            std::mem::take(&mut self.contexts.uses)
                .into_iter()
                .partition(|used| matches!(used.of, UseOf::Local(owner, _) if owner == function));
        self.contexts.uses = others;
        for used in uses {
            if !(used.call && used.target == Some(function)) {
                self.check_early_use(&used, needs);
            }
        }
    }

    /// Reports each of `needs`, what the function `used` uses needs, that
    /// `used`, a use typed before they were known, does not cover.
    fn check_early_use(&mut self, used: &MemberUse, needs: &[Need]) {
        let unmet: Vec<Need> = needs
            .iter()
            .filter(|need| !(used.call && met(need, &used.covered)))
            .cloned()
            .collect();
        for need in strongest(&unmet, |need| need) {
            let error = self.undetermined_need(used, need);
            self.errors.push(error);
        }
    }

    /// Gives the member `top` of a checked group its needs, `needs`, each with
    /// the call through which it came to need it, if not by its own body: its
    /// type takes them; a definition's, which is its def-type's exactly,
    /// cannot, and one it comes to need through a call is reported.
    fn give_member_needs(&mut self, top: usize, needs: &[(Need, Option<ExprId>)]) {
        let Some(found) = self.top_fns.get(top).copied() else {
            return;
        };
        let ty = self.expr_types[found.expr.index()];
        if ty == TyId::ERROR {
            return;
        }
        if matches!(self.ast.item(found.item).kind, ItemKind::DefTypeDef { .. }) {
            let allowed = self.types.needs(ty).to_vec();
            let through: Vec<(Need, Span)> = needs
                .iter()
                .filter_map(|(need, via)| Some((need.clone(), self.ast.get((*via)?).span)))
                .collect();
            self.definition_needs(&through, &allowed);
            return;
        }
        let found_needs: Vec<Need> = needs.iter().map(|(need, _)| need.clone()).collect();
        self.expr_types[found.expr.index()] = self.types.with_needs(ty, &found_needs);
    }

    /// "cannot tell that `f` needs context `arch` here", for `used`, a use
    /// of a function inside its own recursion typed before its needs were
    /// known.
    fn undetermined_need(&self, used: &MemberUse, need: &[Name]) -> Diagnostic {
        let named = match used.of {
            UseOf::Top(member) => self.top_fns.get(member).and_then(|top| top.name),
            UseOf::Local(_, name) => Some(name),
        };
        let name = named.map_or("the function", |name| self.ast.names().get(name));
        let text = self.context_text(need);
        let how = if used.call {
            "called from a nested function or a default"
        } else {
            "used as a value"
        };
        Diagnostic::error(
            format!("cannot tell that `{name}` needs context `{text}` here"),
            self.ast.get(used.expr).span,
        )
        .with_note(format!(
            "`{name}` is {how} inside its own recursion, where its type is known before what it \
             needs is"
        ))
        .with_note(format!(
            "call it directly from a function of the recursion, or bind `^{text}` with a `with` \
             around the call"
        ))
    }
}
