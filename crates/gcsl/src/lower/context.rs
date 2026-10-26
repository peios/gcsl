//! Lowering context: reads of a context, `with`, and context updates.

use crate::ast::{ExprId, PathSegment, SegmentKind, WithRange, path_span};
use crate::diagnostic::InternalError;
use crate::ir::{IrId, IrKind, StepAt, StepMode, UpdateStep};
use crate::names::Name;
use crate::source::Span;
use crate::value::ValueKind;

use super::{Lowerer, read};

impl Lowerer<'_> {
    /// Lowers `id`, a read of a context, of kind `kind` (see the checker's
    /// "Context"): the context's declaration's key, and its default's
    /// function, the read's variable, if it has one.
    pub(super) fn lower_context_read(
        &mut self,
        id: ExprId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        let key = self.resolution.context_read(id).ok_or(InternalError {
            message: "a read of a context the check did not resolve",
            span,
        })?;
        // The runtime error of a context with no value names it by its key.
        self.builder.add_name_text(key, self.ast.names().get(key));
        // With no default, an `Empty`, which the evaluator reads as none.
        let default = read(self.resolution.expr_var(id), ValueKind::Fn).unwrap_or(IrKind::Empty);
        let default = self.builder.alloc(default, span);
        Ok(IrKind::ContextRead {
            key,
            default,
            kind,
            // Set by `IrBuilder::measure` once every node is built.
            depth: 0,
        })
    }

    /// Lowers `id`, `BODY with { ... }`, of the bindings `bindings`, the body
    /// `body` and kind `kind` (see the checker's "Context"): a binding of
    /// each context, in the order of its first binding, of its plain
    /// binding's value, each dotted key of it after that updating the
    /// value before it, or, for one with none before it, the context's
    /// value it reads.
    pub(super) fn lower_with(
        &mut self,
        id: ExprId,
        (bindings, body): (WithRange, ExprId),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        let keys = self.resolution.with_keys(id).ok_or(InternalError {
            message: "a `with` the check did not resolve",
            span,
        })?;
        let bindings = self.ast.with_bindings(bindings);
        if keys.len() != bindings.len() {
            return Err(InternalError {
                message: "a `with` whose bindings the check did not all resolve",
                span,
            });
        }
        let mut binds: Vec<(Name, IrId)> = Vec::new();
        for (binding, &key) in bindings.iter().zip(keys) {
            let value_kind = self.kind(binding.value)?;
            let value = self.lower_operand(binding.value, value_kind)?;
            let Some(current) = binding.current else {
                binds.push((key, value));
                continue;
            };
            let earlier = binds.iter().position(|&(bound, _)| bound == key);
            let updated = match earlier.and_then(|at| binds.get(at)) {
                Some(&(_, before)) => before,
                None => self.lower_operand(current, ValueKind::Record)?,
            };
            let segments = self.ast.path(binding.path);
            let node = self.context_update((updated, value), segments, binding.name_span)?;
            match earlier.and_then(|at| binds.get_mut(at)) {
                Some(entry) => entry.1 = node,
                None => binds.push((key, node)),
            }
        }
        let base = self.item_stack.len();
        for (key, value) in binds {
            let bind = self.builder.alloc(IrKind::ContextBind { key, value }, span);
            self.item_stack.push(bind);
        }
        let body = self.lower_operand(body, kind)?;
        self.item_stack.push(body);
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::With { parts, kind })
    }

    /// The node setting the field the path `segments` names, the fields
    /// after a context's name of a dotted key of a `with`, at `name_span`,
    /// in the record of `updated`, to the value of `value`: an update of
    /// present records, each step reading, the last setting (see the
    /// checker's "Context").
    pub(super) fn context_update(
        &mut self,
        (updated, value): (IrId, IrId),
        segments: &[PathSegment],
        name_span: Span,
    ) -> Result<IrId, InternalError> {
        let steps = self.steps.len();
        for (level, segment) in segments.iter().enumerate() {
            let SegmentKind::Name(name) = segment.kind else {
                return Err(InternalError {
                    message: "a dotted key of a `with` with an index",
                    span: segment.span,
                });
            };
            // The evaluator shows a field it adds by this text.
            self.builder.add_name_text(name, self.ast.names().get(name));
            let mode = if level + 1 == segments.len() {
                StepMode::Set
            } else {
                StepMode::Read
            };
            self.steps.push(UpdateStep {
                at: StepAt::Field(name),
                mode,
                span: Span::join(name_span, segment.span),
            });
        }
        let path = self
            .builder
            .push_update_path(self.steps.get(steps..).unwrap_or_default());
        self.steps.truncate(steps);
        let parts = self.builder.push_item_list(&[value, updated]);
        let span = Span::join(name_span, path_span(segments));
        Ok(self.builder.alloc(
            IrKind::Update {
                parts,
                path,
                kind: ValueKind::Record,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }
}
