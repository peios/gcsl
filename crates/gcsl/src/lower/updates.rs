//! Lowering updates of a variable's fields and elements.

use crate::ast::{ExprId, ItemId, PathSegment, SegmentKind, path_span};
use crate::check::PathResolution;
use crate::diagnostic::InternalError;
use crate::ir::{IrId, IrKind, LazyShapeId, StepAt, StepMode, UpdateStep};
use crate::source::Span;
use crate::value::ValueKind;

use super::{Lowerer, read, updated_kind};

impl Lowerer<'_> {
    /// Lowers the dotted field `id`, whose path is `segments`, to the chain
    /// of `UpdateField`s setting the path to `value`, from the leaf
    /// outwards, each on the record at the path before its segment: the
    /// field's slot or a `Field` of the record before it where they exist,
    /// and `Empty` where the path creates a record. A dotted field extends a
    /// record its block is building, which is never awaited.
    pub(super) fn lower_field_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        let table = self.resolution;
        let resolution = table.item_path(id);
        let existing = resolution.existing as usize;
        if !table
            .path_awaits(resolution)
            .is_some_and(<[bool]>::is_empty)
        {
            return Err(InternalError {
                message: "a dotted field's record is awaited",
                span: path_span(segments),
            });
        }
        let base = self.item_stack.len();
        // The record at each proper prefix of the path, `segments[..=end]`,
        // on the item stack: the record that setting `segments[end + 1]`
        // builds on.
        for end in 0..segments.len().saturating_sub(1) {
            let span = path_span(&segments[..=end]);
            let kind = if end >= existing {
                IrKind::Empty
            } else if end == 0 {
                read(resolution.source, ValueKind::Record).ok_or(InternalError {
                    message: "an update of an existing record has no source",
                    span,
                })?
            } else {
                let (Some(&record), Some(name)) =
                    (self.item_stack.get(base + end - 1), segments[end].name())
                else {
                    return Err(InternalError {
                        message: "an update's record is missing",
                        span,
                    });
                };
                IrKind::Field {
                    record,
                    name,
                    kind: ValueKind::Record,
                }
            };
            let record = self.builder.alloc(kind, span);
            self.item_stack.push(record);
        }
        let mut update = self.lower_expr(value)?;
        for (level, segment) in segments.iter().enumerate().skip(1).rev() {
            let (Some(&record), Some(name)) =
                (self.item_stack.get(base + level - 1), segment.name())
            else {
                return Err(InternalError {
                    message: "an update's record is missing",
                    span: path_span(segments),
                });
            };
            // The evaluator shows a field it adds by this text.
            self.builder.add_name_text(name, self.ast.names().get(name));
            update = self.builder.alloc(
                IrKind::UpdateField {
                    record,
                    name,
                    value: update,
                },
                path_span(&segments[..=level]),
            );
        }
        self.item_stack.truncate(base);
        Ok(update)
    }

    /// Lowers the dotted `let` or `:=` `id`, whose path is `segments`, to
    /// an `Update` setting the path to `value`, spanning the path: its
    /// parts are `value`, then the record or list updated, the variable's
    /// slot or capture read, wrapped in an `Await` if the checker marked it
    /// to be awaited, or `Empty` for a `let` of a name not in scope, and
    /// then the position of each index segment, in order; and its steps,
    /// one for each segment after the first, select a field or an element
    /// and read it, read and await it where the checker marked it, create
    /// it where the static type lacks it, or, for the last, set it.
    pub(super) fn lower_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        let table = self.resolution;
        let resolution = table.item_path(id);
        let existing = resolution.existing as usize;
        let span = path_span(segments);
        let awaits = table.path_awaits(resolution).ok_or(InternalError {
            message: "an update's awaited records are missing",
            span,
        })?;
        let base = self.item_stack.len();
        // The value is evaluated first.
        let value = self.lower_expr(value)?;
        self.item_stack.push(value);
        let root = self.update_root(segments, resolution, awaits)?;
        self.item_stack.push(root);
        let steps = self.steps.len();
        for (level, segment) in segments.iter().enumerate().skip(1) {
            let at = match segment.kind {
                SegmentKind::Name(name) => {
                    // The evaluator shows a field it adds by this text.
                    self.builder.add_name_text(name, self.ast.names().get(name));
                    StepAt::Field(name)
                }
                SegmentKind::Index { position, .. } => {
                    let position = self.lower_operand(position, ValueKind::Int)?;
                    self.item_stack.push(position);
                    StepAt::Element
                }
            };
            // The record or list the step selects is the one at
            // `segments[..=level]`.
            let mode = if level + 1 == segments.len() {
                StepMode::Set
            } else if level >= existing {
                StepMode::Create
            } else if awaits.get(level).copied().unwrap_or(false) {
                StepMode::Await
            } else {
                StepMode::Read
            };
            self.steps.push(UpdateStep {
                at,
                mode,
                span: path_span(&segments[..=level]),
            });
        }
        let path = self
            .builder
            .push_update_path(self.steps.get(steps..).unwrap_or_default());
        self.steps.truncate(steps);
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let kind = updated_kind(segments);
        Ok(self.builder.alloc(
            IrKind::Update {
                parts,
                path,
                kind,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }

    /// The node reading the record or list that the dotted `let` or `:=`
    /// whose path is `segments`, and which resolves as `resolution` says,
    /// updates, spanning its first segment: a read of its variable's slot
    /// or capture, as a use of the variable would read it, wrapped in an
    /// `Await` if `awaits`, the checker's marks for the records and lists
    /// along the path, says so for it, or `Empty` if no variable is
    /// updated.
    pub(super) fn update_root(
        &mut self,
        segments: &[PathSegment],
        resolution: PathResolution,
        awaits: &[bool],
    ) -> Result<IrId, InternalError> {
        let span = segments.first().map_or(Span::new(0, 0), |first| first.span);
        if resolution.existing == 0 {
            return Ok(self.builder.alloc(IrKind::Empty, span));
        }
        let kind = updated_kind(segments);
        // A record or list that may be a future is read as it is and
        // awaited, as the record of a field access is.
        let awaited = awaits.first().copied().unwrap_or(false);
        let read_kind = if awaited { ValueKind::Deferred } else { kind };
        let read = read(resolution.source, read_kind).ok_or(InternalError {
            message: "an update of an existing record has no source",
            span,
        })?;
        let read = self.builder.alloc(read, span);
        if !awaited {
            return Ok(read);
        }
        Ok(self.builder.alloc(
            IrKind::Await {
                value: read,
                kind,
                shape: LazyShapeId::NONE,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }
}
