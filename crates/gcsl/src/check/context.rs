//! Checking context: declarations of contexts, reads of them, and `with`.
//!
//! # Context
//!
//! A context (see the parser's [Context](crate::parser::context#context)) is
//! declared once in a workspace, and seen by every file of it, its standard
//! files' among them, without an import: the workspace gives each file the
//! one of a name (`Context::context`), its own declaration's first, then,
//! for a session's input, an earlier input's, then the first of the
//! workspace's. A second declaration of a name is "context `arch` is
//! declared twice" (see [`crate::workspace`], "Context"). Its type is
//! resolved in its file before any file is checked
//! (`check_context_types`), present, and recorded by its declaration's key
//! ([`crate::names::context_key`]). A default is the body of a hidden
//! function, pure and of no parameters, returning the type, the `let` of a
//! binding no source can write, checked as any top-level function is; a call
//! of an impure function in it is "the default of a context must be pure",
//! noting that a read of a context is pure (`Checker::is_context_default`).
//!
//! `^NAME` (`Checker::check_context_read`) is of the declared type; a name
//! no context has is "no context `arch` is declared", suggesting a close
//! one (`Checker::context_named`). Its default's function, if it has one,
//! is the read's own variable: the binding of the input's or an earlier
//! input's declaration, read as a name is, so that a top-level read is
//! scheduled after what the default reads, or another file's slot, read as
//! an import's. What the read needs, unless a `with` binds the context
//! where its needs are charged, is charged as
//! [Needs](crate::check::needs#needs) says: the context, if it has no
//! default, or what its default needs, each need met by binding the context
//! too; at the input's top level, a need nothing meets is "context `arch` has
//! no value here". `TypeExprKind::Context`, a `^NAME` parameter's type, is
//! the context's type exactly, neither bare nor inferred.
//!
//! `BODY with { ... }` (`Checker::check_with`) checks each binding against its
//! context: a plain one's value against its type, a dotted key's against the
//! type of the field its path names, through present records, "context
//! `platform` has no field `archx`" otherwise, its read of the value it updates
//! checked as a read is. A second plain binding of a context, or one after a
//! dotted key of it, is "context `x` is bound twice in this `with`". The body
//! is checked with the contexts bound, and its type is the `with`'s; an untyped
//! integer body is coerced through it. Each read and `with` is recorded for
//! lowering by its declarations' keys
//! ([`Resolution::context_read`](super::Resolution::context_read),
//! [`Resolution::with_keys`](super::Resolution::with_keys)).

use crate::ast::{ExprId, ExprKind, FnId, ItemKind, WithRange};
use crate::diagnostic::Diagnostic;
use crate::names::Name;
use crate::source::Span;
use crate::suggest;
use crate::types::{Need, TyId, through};
use crate::workspace::{Context, ContextDefault, SeenContext};

use super::needs::NeedWhy;
use super::{AwaitSite, Checker};

impl Checker<'_> {
    /// Whether `function` is the function of a context's default, of the
    /// input's (see [Context](#context)).
    pub(super) fn is_context_default(&self, function: FnId) -> bool {
        self.ast.contexts().iter().any(|context| {
            matches!(
                self.ast.item(context.item).kind,
                ItemKind::Let { value, .. }
                    if self.ast.get(value).kind == ExprKind::Fn(function)
            )
        })
    }

    /// The context named `name` that the input sees, read or bound at `span`
    /// (see [Context](#context)); `None` (reported) if none is declared:
    /// "no context `arch` is declared", suggesting a close name.
    pub(super) fn context_named(&mut self, name: Name, span: Span) -> Option<SeenContext> {
        if let Some(found) = self.workspace.and_then(|workspace| workspace.context(name)) {
            return Some(found);
        }
        let names = self.ast.names();
        let text = names.get(name);
        let declared = self
            .workspace
            .map(Context::context_names)
            .unwrap_or_default();
        // At the name, which ends the span.
        let at = self.ast.written_name(span.end, text);
        let suggestion = suggest::best(text, declared.iter().map(|&n| (names.get(n), 0)))
            .map(|best| (at, best.to_owned()));
        self.errors.push(
            Diagnostic::error(format!("no context `{text}` is declared"), span)
                .with_suggestion(suggestion),
        );
        None
    }

    /// Types the read of a context `id`, `^NAME` (see [Context](#context)):
    /// its declared type, of the context the input sees by the name; its
    /// default's function, if it has one, read as the read's own variable,
    /// so that lowering calls it where no `with` binds the context: a binding
    /// of the input's or of an earlier input's, read as a name is, or
    /// another file's, from its slot. Recorded for lowering. A read at the
    /// input's top level of a context with no default that no `with` around
    /// it binds is "context `arch` has no value here", noted "no `with` sets
    /// it, and it declares no default".
    pub(super) fn check_context_read(&mut self, id: ExprId, name: Name) -> TyId {
        let span = self.ast.get(id).span;
        let Some(context) = self.context_named(name, span) else {
            return TyId::ERROR;
        };
        // An erroneous declaration is reported where it is.
        let ty = self.types.context_type(context.key).unwrap_or(TyId::ERROR);
        if ty == TyId::ERROR {
            self.quiet.failed = true;
            return TyId::ERROR;
        }
        // What the read needs, unless a `with` binds the context where its
        // needs are charged (see [Needs](crate::check::needs#needs)).
        let bound = self.is_covered(context.key);
        match context.default {
            ContextDefault::Binding(binding) => {
                let default_ty = self.use_name(id, binding, span).0;
                if default_ty == TyId::ERROR {
                    return TyId::ERROR;
                }
                // The read calls the default where nothing binds it.
                self.note_member_call(id);
                if !bound {
                    let needs: Vec<Need> = self
                        .types
                        .needs(default_ty)
                        .iter()
                        .map(|need| through(need, context.key))
                        .collect();
                    self.charge_needs(&needs, span, NeedWhy::Default(name));
                }
            }
            ContextDefault::Foreign(Some(slot)) => {
                self.read_import(id, slot);
                if !bound {
                    let needs: Vec<Need> = self
                        .types
                        .context_needs(context.key)
                        .iter()
                        .map(|need| through(need, context.key))
                        .collect();
                    self.charge_needs(&needs, span, NeedWhy::Default(name));
                }
            }
            // Its file failed its check, which is reported.
            ContextDefault::Foreign(None) => {
                self.quiet.failed = true;
                return TyId::ERROR;
            }
            ContextDefault::None => {
                let need: Need = Box::new([context.key]);
                self.charge_needs(&[need], span, NeedWhy::Read);
            }
        }
        self.lowering.context_reads.insert(id, context.key);
        ty
    }

    /// Types `id`, `BODY with { ... }`, of the bindings `bindings` and the
    /// body `body` (see [Context](#context)): each binding's context, the
    /// one the input sees by its name; a plain binding's value checked
    /// against the context's type; a dotted key's, `platform.arch = v`,
    /// against the type of the field its path names in the context's type,
    /// a record's, "context `platform` has no field `archx`" otherwise, the
    /// value it updates read as `^platform` is. A second plain binding of a
    /// context, or one after a dotted key of it, is "context `platform` is
    /// bound twice in this `with`". The body is checked with each context
    /// bound, and its type is the `with`'s. Recorded for lowering.
    pub(super) fn check_with(&mut self, id: ExprId, bindings: WithRange, body: ExprId) -> TyId {
        let mut keys = Vec::new();
        let mut erroneous = false;
        // The contexts bound plainly so far, and by a dotted key.
        let (mut plain, mut dotted): (Vec<Name>, Vec<Name>) = (Vec::new(), Vec::new());
        for binding in self.ast.with_bindings(bindings).to_vec() {
            let found = self.context_named(binding.name, binding.name_span);
            let ty = found.map_or(TyId::ERROR, |context| {
                self.types.context_type(context.key).unwrap_or(TyId::ERROR)
            });
            let Some(context) = found.filter(|_| ty != TyId::ERROR) else {
                // Its value checked alone, for what is wrong in it.
                self.check_expr(binding.value);
                erroneous = true;
                continue;
            };
            let text = self.ast.names().get(binding.name).to_owned();
            let segments = self.ast.path(binding.path);
            let expected = match binding.current {
                None => {
                    if plain.contains(&context.key) || dotted.contains(&context.key) {
                        self.errors.push(Diagnostic::error(
                            format!("context `{text}` is bound twice in this `with`"),
                            binding.name_span,
                        ));
                        erroneous = true;
                    }
                    plain.push(context.key);
                    ty
                }
                Some(current) => {
                    dotted.push(context.key);
                    if self.check_expr(current) == TyId::ERROR {
                        erroneous = true;
                    }
                    let mut at = ty;
                    for segment in segments {
                        let Some(field) = segment.name() else {
                            break;
                        };
                        if let Some(field_ty) = self.types.field(self.types.strip(at), field) {
                            at = field_ty;
                        } else {
                            let shown = self.ast.names().get(field);
                            self.errors.push(Diagnostic::error(
                                format!("context `{text}` has no field `{shown}`"),
                                segment.span,
                            ));
                            at = TyId::ERROR;
                            erroneous = true;
                            break;
                        }
                    }
                    at
                }
            };
            let why = format!("the type of context `{text}`");
            let site = (AwaitSite::Implicit, None);
            if self.check_expr_expecting_at(
                binding.value,
                expected,
                (binding.name_span, &why),
                site,
            ) == TyId::ERROR
            {
                erroneous = true;
            }
            keys.push(context.key);
        }
        let outer = self.contexts.bound.len();
        self.contexts.bound.extend(keys.iter().copied());
        let ty = self.check_expr(body);
        self.contexts.bound.truncate(outer);
        self.lowering.withs.insert(id, keys.into());
        if erroneous { TyId::ERROR } else { ty }
    }
}
