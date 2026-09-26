//! Lowering: a checked [`Ast`] to [`Ir`].
//!
//! Lowering picks the type-specialised IR node for every operator, drops
//! `Paren` and `Ascribe` nodes (the checker has already coerced whatever an
//! ascription affects), turns integer literals typed `Float` into
//! `FloatConst` directly, and inserts `IntToFloat` for concrete `Int`
//! operands of `/`. Variables become `Load` and `Store` nodes on the slots
//! the checker assigned. Every block, including the file's body, becomes a
//! `Block` whose items are its declarations and non-final expression
//! statements, and whose result is its `= EXPR` or final expression
//! statement, or an explicit `Empty` node if it has neither.
//!
//! A record block's fields become `Store`s to their slots, as `let`s do, so
//! that later items can read them, and its result is a `MakeRecord` whose
//! values are `Load`s of those slots in definition order; every expression
//! statement in it, the last included, is a statement. Each record block's
//! field names become a [`Layout`], built here once and shared by every
//! record the block builds; blocks with the same names in the same order
//! share one layout, found through a map from the names to the layout.
//!
//! Only the kind of each node's value matters to the IR (see
//! [`crate::ir::Ir::kind_of`]), so lowering reads each expression's type
//! from the checker's tables and keeps only its [`ValueKind`]. A value of a
//! record subtype where a supertype is expected needs no conversion: a
//! record is the same value whatever record type it is seen as.
//!
//! A dotted `let`, `:=` or field lowers to a `Store` of a chain of
//! `UpdateField` nodes, one for each segment of its path after the first,
//! from the leaf outwards, each setting its segment in the record at the
//! path before it. That record is the updated variable's or field's slot,
//! loaded, for the first segment, and a `Field` of the record before it for
//! each later one that the static type has (see
//! [`crate::check::PathResolution`]); a record the path creates is `Empty`,
//! as is the first record of a dotted field's first definition. So
//! `let a.b.c = e`, with `a: {b: {c: Int}}` in scope, lowers to
//! `Store(a', UpdateField(Load(a), b, UpdateField(Field(Load(a), b), c,
//! e)))`, where each record node is shared by the update that builds on it
//! and the access into it, and with `a: {x: Int}` to
//! `Store(a', UpdateField(Load(a), b, UpdateField(Empty, c, e)))`. A dotted
//! field stores back into its field's one slot. Each record block's field
//! is loaded for its record once, from the slot its first definition
//! declared. The text of each name an update sets is recorded in the IR,
//! for the evaluator to show a field it adds.
//!
//! Comparisons are specialised by their operands' kind, and an `if` keeps
//! the empty else block the parser gave it if the source had none. `===`
//! and `!==` lower exactly as `==` and `!=` on every kind but records. `==`
//! and `!=` on records compare over the [`Shape`] of the operands' static
//! type, read from the type table: one shape per distinct record type met
//! in such a comparison, nested record types included, memoised by type
//! and built through a work list rather than recursion. Each
//! string literal, and each literal piece of an interpolated string,
//! becomes a `StrConst` whose text is copied from the AST's string table
//! into the IR's own, so the AST's table is not needed after lowering. A
//! literal piece has no span of its own in the AST, so its node has its
//! whole interpolated string's span.
//!
//! Lowering cannot fail on tables produced by [`crate::check::check`]. Any
//! combination the checker should have ruled out is reported as an
//! [`InternalError`] rather than a panic.

use std::collections::HashMap;

use crate::ast::{
    Ast, BinaryOp, ExprId, ExprKind, ItemId, ItemKind, ItemRange, PartRange, PathRange,
    PathSegment, StrPart, UnaryOp, path_span,
};
use crate::check::{Checked, Resolution, Slot, Types};
use crate::diagnostic::InternalError;
use crate::ir::{
    Cmp, Ir, IrBuilder, IrId, IrKind, LayoutId, Operands, Shape, ShapeId, ShapedOperands,
};
use crate::names::Name;
use crate::source::Span;
use crate::types::{TyId, TypeTable};
use crate::value::{Layout, ValueKind, int_to_float};

/// Lowers `ast`, whose types and resolution are `checked`, with its types
/// interned into `types`, to IR.
///
/// # Errors
///
/// Returns an [`InternalError`] if `checked` was not produced by checking
/// `ast` against `types`, or is otherwise inconsistent with it: a bug in
/// the caller or the checker, never an error in the user's program.
pub fn lower(ast: &Ast, checked: &Checked, types: &TypeTable) -> Result<Ir, InternalError> {
    let Checked {
        types: expr_types,
        resolution,
    } = checked;
    let root_span = ast.root_span();
    if expr_types.len() != ast.len() {
        return Err(InternalError {
            message: "type table does not belong to this AST",
            span: root_span,
        });
    }
    if resolution.expr_len() != ast.len() || resolution.item_len() != ast.item_count() {
        return Err(InternalError {
            message: "resolution does not belong to this AST",
            span: root_span,
        });
    }
    let mut lowerer = Lowerer {
        ast,
        expr_types,
        table: types,
        resolution,
        // One IR node per AST node, less parentheses, plus any conversions,
        // stores, empty records and field loads.
        builder: IrBuilder::with_capacity(ast.len()),
        item_stack: Vec::new(),
        layouts: HashMap::new(),
        layout_names: Vec::new(),
        shapes: HashMap::new(),
        shape_pending: Vec::new(),
    };
    let root = ast.root();
    let kind = lowerer.body_kind(root)?;
    let kind = lowerer.lower_body(root, root_span, kind)?;
    let root = lowerer.builder.alloc(kind, root_span);
    Ok(lowerer.builder.finish(
        root,
        resolution.frame_size(),
        resolution.slot_names().to_vec(),
    ))
}

struct Lowerer<'a> {
    ast: &'a Ast,
    expr_types: &'a Types,
    table: &'a TypeTable,
    resolution: &'a Resolution,
    builder: IrBuilder,
    /// The items of the blocks being lowered, innermost last, moved to the
    /// builder's pool as each block ends, as the parser does for the AST.
    /// A record's values are collected here in the same way.
    item_stack: Vec<IrId>,
    /// The layout already made for each sequence of field names.
    layouts: HashMap<Box<[Name]>, LayoutId>,
    /// Reused for collecting the field names of each record block.
    layout_names: Vec<Name>,
    /// The shape already made for each record type.
    shapes: HashMap<TyId, ShapeId>,
    /// Reused as the work list of record types whose shapes have ids but no
    /// fields yet.
    shape_pending: Vec<TyId>,
}

impl Lowerer<'_> {
    /// The kind of the value of expression `id`.
    fn kind(&self, id: ExprId) -> Result<ValueKind, InternalError> {
        let span = self.ast.get(id).span;
        match self.expr_types.get(id) {
            TyId::UNTYPED_INT => Err(InternalError {
                message: "an untyped integer survived type checking",
                span,
            }),
            TyId::ERROR => Err(InternalError {
                message: "a type error survived type checking",
                span,
            }),
            ty => self.table.value_kind(ty).ok_or(InternalError {
                message: "a type with no values survived type checking",
                span,
            }),
        }
    }

    /// The kind of a body's value: a record for a record block, its
    /// result's kind, or a record (`{}`) if it has none.
    fn body_kind(&self, body: ItemRange) -> Result<ValueKind, InternalError> {
        if self.is_record_body(body) {
            return Ok(ValueKind::Record);
        }
        match self
            .ast
            .items(body)
            .last()
            .map(|&id| self.ast.item(id).kind)
        {
            Some(ItemKind::Result(value) | ItemKind::Expr(value)) => self.kind(value),
            Some(ItemKind::Let { .. } | ItemKind::Rebind { .. } | ItemKind::Field { .. })
            | None => Ok(ValueKind::Record),
        }
    }

    /// Whether `body` has a field item, which makes it a record block.
    fn is_record_body(&self, body: ItemRange) -> bool {
        self.ast
            .items(body)
            .iter()
            .any(|&id| matches!(self.ast.item(id).kind, ItemKind::Field { .. }))
    }

    /// Lowers a body whose value is of kind `kind` to a `Block`, which the
    /// caller allocates.
    fn lower_body(
        &mut self,
        body: ItemRange,
        span: Span,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let items = self.ast.items(body);
        let is_record = self.is_record_body(body);
        let base = self.item_stack.len();
        let mut result = None;
        for (index, &id) in items.iter().enumerate() {
            // A record block's final expression is a statement.
            let is_result = !is_record && index + 1 == items.len();
            let item = self.ast.item(id);
            match item.kind {
                ItemKind::Let { path, value, .. }
                | ItemKind::Rebind { path, value }
                | ItemKind::Field { path, value } => {
                    let slot = self.resolution.item_slot(id);
                    if slot == Slot::NONE {
                        return Err(InternalError {
                            message: "a declaration has no slot",
                            span: item.span,
                        });
                    }
                    let value = self.lower_declaration(id, path, value)?;
                    let store = self.builder.alloc(IrKind::Store(slot, value), item.span);
                    self.item_stack.push(store);
                }
                ItemKind::Result(value) | ItemKind::Expr(value) if is_result => {
                    result = Some(self.lower_operand(value, kind)?);
                }
                ItemKind::Result(_) => {
                    return Err(InternalError {
                        message: "a result before the end of its block, or in a record block, \
                                  survived checking",
                        span: item.span,
                    });
                }
                ItemKind::Expr(value) => {
                    // Evaluated for effect only, so it must be `{}`.
                    let statement = self.lower_operand(value, ValueKind::Record)?;
                    self.item_stack.push(statement);
                }
            }
        }
        let result = match result {
            Some(result) => result,
            None if kind != ValueKind::Record => {
                return Err(InternalError {
                    message: "a block without a result is not a record",
                    span,
                });
            }
            None if is_record => self.make_record(items, span)?,
            None => self.builder.alloc(IrKind::Empty, span),
        };
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Block { items, result })
    }

    /// Lowers the value that the declaration `id`, whose path is `path`,
    /// stores: its expression `value` for a plain path, and for a dotted
    /// one the chain of updates that sets the path to it.
    fn lower_declaration(
        &mut self,
        id: ItemId,
        path: PathRange,
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        match self.ast.path(path) {
            [_] => self.lower_expr(value),
            segments => self.lower_update(id, segments, value),
        }
    }

    /// Lowers the dotted declaration `id`, whose path is `segments`, to the
    /// chain of `UpdateField`s setting the path to `value`, from the leaf
    /// outwards, each on the record at the path before its segment: the
    /// updated slot or a `Field` of the record before it where they exist,
    /// and `Empty` where the path creates a record.
    fn lower_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        let resolution = self.resolution.item_path(id);
        let existing = resolution.existing as usize;
        let base = self.item_stack.len();
        // The record at each proper prefix of the path, `segments[..=end]`,
        // on the item stack: the record that setting `segments[end + 1]`
        // builds on.
        for end in 0..segments.len().saturating_sub(1) {
            let span = path_span(&segments[..=end]);
            let kind = if end >= existing {
                IrKind::Empty
            } else if end == 0 {
                if resolution.source == Slot::NONE {
                    return Err(InternalError {
                        message: "an update of an existing record has no source slot",
                        span,
                    });
                }
                IrKind::Load(resolution.source, ValueKind::Record)
            } else {
                let Some(&record) = self.item_stack.get(base + end - 1) else {
                    return Err(InternalError {
                        message: "an update's record is missing",
                        span,
                    });
                };
                IrKind::Field {
                    record,
                    name: segments[end].name,
                    kind: ValueKind::Record,
                }
            };
            let record = self.builder.alloc(kind, span);
            self.item_stack.push(record);
        }
        // The evaluator shows a field it adds by this text.
        for segment in segments.get(1..).unwrap_or_default() {
            let text = self.ast.names().get(segment.name);
            self.builder.add_name_text(segment.name, text);
        }
        let mut update = self.lower_expr(value)?;
        for (level, segment) in segments.iter().enumerate().skip(1).rev() {
            let Some(&record) = self.item_stack.get(base + level - 1) else {
                return Err(InternalError {
                    message: "an update's record is missing",
                    span: path_span(segments),
                });
            };
            update = self.builder.alloc(
                IrKind::UpdateField {
                    record,
                    name: segment.name,
                    value: update,
                },
                path_span(&segments[..=level]),
            );
        }
        self.item_stack.truncate(base);
        Ok(update)
    }

    /// Builds the `MakeRecord` of the record block of `items`, spanning
    /// `span`, whose fields have been stored: a `Load` of each field's slot,
    /// at its name, in the order the fields were declared. An open field is
    /// loaded once, from the slot its first definition declared, as a
    /// record.
    fn make_record(&mut self, items: &[ItemId], span: Span) -> Result<IrId, InternalError> {
        self.layout_names.clear();
        let base = self.item_stack.len();
        for &id in items {
            let ItemKind::Field { path, value } = self.ast.item(id).kind else {
                continue;
            };
            let (kind, field) = match self.ast.path(path) {
                [field] => (self.kind(value)?, field),
                // A later definition of an open field extends it.
                [field, ..] if self.resolution.item_path(id).source == Slot::NONE => {
                    (ValueKind::Record, field)
                }
                _ => continue,
            };
            let slot = self.resolution.item_slot(id);
            let load = self.builder.alloc(IrKind::Load(slot, kind), field.span);
            self.item_stack.push(load);
            self.layout_names.push(field.name);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span))
    }

    /// The layout for the names in `layout_names`, made if this is the
    /// first record block with those names in that order.
    fn layout(&mut self) -> LayoutId {
        if let Some(&id) = self.layouts.get(self.layout_names.as_slice()) {
            return id;
        }
        let names: Box<[Name]> = self.layout_names.as_slice().into();
        let texts = names
            .iter()
            .map(|&name| Box::from(self.ast.names().get(name)))
            .collect();
        let id = self.builder.push_layout(Layout {
            names: names.clone(),
            texts,
        });
        self.layouts.insert(names, id);
        id
    }

    fn lower_expr(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        let expr = *self.ast.get(id);
        let span = expr.span;
        let kind = self.kind(id)?;
        let lowered = match (expr.kind, kind) {
            (ExprKind::Paren(inner) | ExprKind::Ascribe { expr: inner, .. }, _) => {
                return self.lower_operand(inner, kind);
            }
            (ExprKind::Int(value), ValueKind::Int) => IrKind::IntConst(value),
            (ExprKind::Int(value), ValueKind::Float) => IrKind::FloatConst(int_to_float(value)),
            (ExprKind::Float(value), ValueKind::Float) => IrKind::FloatConst(value),
            (ExprKind::Bool(value), ValueKind::Bool) => IrKind::BoolConst(value),
            (ExprKind::Str(text), ValueKind::String) => {
                IrKind::StrConst(self.builder.push_string(self.ast.strings().get(text)))
            }
            (ExprKind::Interp(parts), ValueKind::String) => {
                self.lower_interpolation(parts, span)?
            }
            (
                ExprKind::If {
                    cond, then, else_, ..
                },
                _,
            ) => IrKind::If {
                cond: self.lower_operand(cond, ValueKind::Bool)?,
                then: self.lower_operand(then, kind)?,
                else_: self.lower_operand(else_, kind)?,
            },
            (ExprKind::Unary { op, operand }, _) => self.lower_unary(op, operand, kind, span)?,
            (
                ExprKind::Binary {
                    op,
                    op_start,
                    lhs,
                    rhs,
                },
                _,
            ) => self.lower_binary(op, op_start, (lhs, rhs), kind, id)?,
            (ExprKind::Name(_), _) => {
                let slot = self.resolution.expr_slot(id);
                if slot == Slot::NONE {
                    return Err(InternalError {
                        message: "a variable use has no slot",
                        span,
                    });
                }
                IrKind::Load(slot, kind)
            }
            (ExprKind::Block(body), _) => self.lower_body(body, span, kind)?,
            (ExprKind::Field { expr: record, name }, _) => IrKind::Field {
                record: self.lower_operand(record, ValueKind::Record)?,
                name,
                kind,
            },
            (ExprKind::Float(_), ValueKind::Int) => {
                return Err(InternalError {
                    message: "float literal typed as Int",
                    span,
                });
            }
            (
                ExprKind::Int(_) | ExprKind::Float(_),
                ValueKind::Bool | ValueKind::String | ValueKind::Record,
            ) => {
                return Err(InternalError {
                    message: "numeric literal typed as a non-numeric type",
                    span,
                });
            }
            (ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Interp(_), _) => {
                return Err(InternalError {
                    message: "literal typed as a type it cannot have",
                    span,
                });
            }
        };
        Ok(self.builder.alloc(lowered, span))
    }

    /// Lowers the prefix operator `op`, of kind `kind` and spanning `span`,
    /// applied to `operand`.
    fn lower_unary(
        &mut self,
        op: UnaryOp,
        operand: ExprId,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        Ok(match (op, kind) {
            (UnaryOp::Neg, ValueKind::Int) => {
                IrKind::IntNeg(self.lower_operand(operand, ValueKind::Int)?)
            }
            (UnaryOp::Neg, ValueKind::Float) => {
                IrKind::FloatNeg(self.lower_operand(operand, ValueKind::Float)?)
            }
            (UnaryOp::Not, ValueKind::Bool) => {
                IrKind::Not(self.lower_operand(operand, ValueKind::Bool)?)
            }
            _ => {
                return Err(InternalError {
                    message: "prefix operator has a result type it cannot have",
                    span,
                });
            }
        })
    }

    /// Lowers the parts of an interpolated string spanning `span` to an
    /// `Interp` whose items are a `StrConst` for each literal piece and the
    /// lowered expressions, in order.
    fn lower_interpolation(
        &mut self,
        parts: PartRange,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let base = self.item_stack.len();
        for &part in self.ast.parts(parts) {
            let item = match part {
                StrPart::Lit(text) => {
                    let text = self.builder.push_string(self.ast.strings().get(text));
                    self.builder.alloc(IrKind::StrConst(text), span)
                }
                StrPart::Expr(expr) => self.lower_expr(expr)?,
            };
            self.item_stack.push(item);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Interp(items))
    }

    /// Lowers the binary operator `op`, whose symbol starts at `op_start`,
    /// of kind `kind`, the expression `id`.
    fn lower_binary(
        &mut self,
        op: BinaryOp,
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        if let Some(cmp) = comparison(op) {
            return self.lower_comparison((op, cmp), op_start, (lhs, rhs), kind, id);
        }
        let (make, operand_kind): (fn(Operands) -> IrKind, ValueKind) = match (op, kind) {
            (BinaryOp::Add, ValueKind::String) => (IrKind::StrConcat, ValueKind::String),
            (BinaryOp::And, ValueKind::Bool) => (IrKind::And, ValueKind::Bool),
            (BinaryOp::Or, ValueKind::Bool) => (IrKind::Or, ValueKind::Bool),
            (BinaryOp::Add, ValueKind::Int) => (IrKind::IntAdd, ValueKind::Int),
            (BinaryOp::Sub, ValueKind::Int) => (IrKind::IntSub, ValueKind::Int),
            (BinaryOp::Mul, ValueKind::Int) => (IrKind::IntMul, ValueKind::Int),
            (BinaryOp::FloorDiv, ValueKind::Int) => (IrKind::IntFloorDiv, ValueKind::Int),
            (BinaryOp::Rem, ValueKind::Int) => (IrKind::IntRem, ValueKind::Int),
            (BinaryOp::Add, ValueKind::Float) => (IrKind::FloatAdd, ValueKind::Float),
            (BinaryOp::Sub, ValueKind::Float) => (IrKind::FloatSub, ValueKind::Float),
            (BinaryOp::Mul, ValueKind::Float) => (IrKind::FloatMul, ValueKind::Float),
            (BinaryOp::Div, ValueKind::Float) => {
                let lhs = self.lower_as_float(lhs)?;
                let rhs = self.lower_as_float(rhs)?;
                return Ok(IrKind::FloatDiv(Operands { lhs, rhs, op_start }));
            }
            _ => {
                return Err(InternalError {
                    message: "operator has a result type it cannot have",
                    span: self.ast.get(id).span,
                });
            }
        };
        let lhs = self.lower_operand(lhs, operand_kind)?;
        let rhs = self.lower_operand(rhs, operand_kind)?;
        Ok(make(Operands { lhs, rhs, op_start }))
    }

    /// Lowers the comparison `op`, which tests `cmp` on every kind but
    /// records, of kind `kind`, to the node for its operands' kind, which
    /// the checker has made the same on both sides.
    fn lower_comparison(
        &mut self,
        (op, cmp): (BinaryOp, Cmp),
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        if kind != ValueKind::Bool {
            return Err(InternalError {
                message: "operator has a result type it cannot have",
                span,
            });
        }
        let operand_kind = self.kind(lhs)?;
        // Typed equality on records compares over the operands' type.
        let shape = match (operand_kind, op) {
            (ValueKind::Record, BinaryOp::Eq | BinaryOp::Ne) => {
                Some(self.shape(self.expr_types.get(lhs), span)?)
            }
            _ => None,
        };
        let lhs = self.lower_operand(lhs, operand_kind)?;
        let rhs = self.lower_operand(rhs, operand_kind)?;
        let operands = Operands { lhs, rhs, op_start };
        Ok(match (operand_kind, op, shape) {
            (ValueKind::Int, ..) => IrKind::IntCmp(cmp, operands),
            (ValueKind::Float, ..) => IrKind::FloatCmp(cmp, operands),
            (ValueKind::String, ..) => IrKind::StrCmp(cmp, operands),
            (ValueKind::Bool, BinaryOp::Eq | BinaryOp::Same, _) => IrKind::BoolEq(true, operands),
            (ValueKind::Bool, BinaryOp::Ne | BinaryOp::NotSame, _) => {
                IrKind::BoolEq(false, operands)
            }
            (ValueKind::Record, BinaryOp::Eq, Some(shape)) => {
                IrKind::RecordEq(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::Record, BinaryOp::Ne, Some(shape)) => {
                IrKind::RecordNe(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::Record, BinaryOp::Same, _) => IrKind::RecordSame(operands),
            (ValueKind::Record, BinaryOp::NotSame, _) => IrKind::RecordNotSame(operands),
            (ValueKind::Bool | ValueKind::Record, ..) => {
                return Err(InternalError {
                    message: "comparison of a type it cannot compare",
                    span,
                });
            }
        })
    }

    /// The shape of the record type `ty`, made, with the shapes of the
    /// record types nested in it, if this is the first comparison of it.
    /// Each shape is given its id when it is first met and its fields when
    /// it is taken off the work list, so that nesting does not recurse.
    fn shape(&mut self, ty: TyId, span: Span) -> Result<ShapeId, InternalError> {
        if let Some(&id) = self.shapes.get(&ty) {
            return Ok(id);
        }
        let table = self.table;
        let id = self.new_shape(ty);
        while let Some(ty) = self.shape_pending.pop() {
            let Some(fields) = table.record_fields(ty) else {
                self.shape_pending.clear();
                return Err(InternalError {
                    message: "a record comparison of a type that is not a record",
                    span,
                });
            };
            let mut shaped = Vec::with_capacity(fields.len());
            for &(name, field_ty) in fields {
                let nested = if table.is_record(field_ty) {
                    Some(match self.shapes.get(&field_ty) {
                        Some(&nested) => nested,
                        None => self.new_shape(field_ty),
                    })
                } else {
                    None
                };
                shaped.push((name, nested));
            }
            let Some(&shape) = self.shapes.get(&ty) else {
                continue;
            };
            self.builder.set_shape(
                shape,
                Shape {
                    fields: shaped.into_boxed_slice(),
                },
            );
        }
        Ok(id)
    }

    /// Gives the record type `ty` a shape with no fields yet, and queues it
    /// to be given them.
    fn new_shape(&mut self, ty: TyId) -> ShapeId {
        let id = self.builder.push_shape(Shape {
            fields: Box::default(),
        });
        self.shapes.insert(ty, id);
        self.shape_pending.push(ty);
        id
    }

    /// Lowers an operand that must already be of kind `expected`.
    fn lower_operand(&mut self, id: ExprId, expected: ValueKind) -> Result<IrId, InternalError> {
        if self.kind(id)? == expected {
            self.lower_expr(id)
        } else {
            Err(InternalError {
                message: "operand type does not match its operator",
                span: self.ast.get(id).span,
            })
        }
    }

    /// Lowers an operand of `/`, converting a concrete `Int` to `Float`.
    fn lower_as_float(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        let span = self.ast.get(id).span;
        match self.expr_types.get(id) {
            TyId::FLOAT => self.lower_expr(id),
            TyId::INT => {
                let operand = self.lower_expr(id)?;
                Ok(self.builder.alloc(IrKind::IntToFloat(operand), span))
            }
            TyId::UNTYPED_INT => Err(InternalError {
                message: "an untyped integer survived type checking",
                span,
            }),
            TyId::ERROR => Err(InternalError {
                message: "a type error survived type checking",
                span,
            }),
            _ => Err(InternalError {
                message: "operand type does not match its operator",
                span,
            }),
        }
    }
}

/// The relation a comparison operator tests on values other than records,
/// where `===` and `!==` are `==` and `!=`, or `None` for any other
/// operator.
const fn comparison(op: BinaryOp) -> Option<Cmp> {
    match op {
        BinaryOp::Eq | BinaryOp::Same => Some(Cmp::Eq),
        BinaryOp::Ne | BinaryOp::NotSame => Some(Cmp::Ne),
        BinaryOp::Lt => Some(Cmp::Lt),
        BinaryOp::Le => Some(Cmp::Le),
        BinaryOp::Gt => Some(Cmp::Gt),
        BinaryOp::Ge => Some(Cmp::Ge),
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::FloorDiv
        | BinaryOp::Rem
        | BinaryOp::And
        | BinaryOp::Or => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AstBuilder, Expr, Item};
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::names::Names;
    use crate::parser::parse;
    use crate::source::SourceFile;
    use crate::strings::Strings;

    fn parse_text(text: &str) -> Ast {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        parse(&tokens, names, strings, &source).unwrap()
    }

    fn lower_text(text: &str) -> Ir {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        lower(&ast, &checked, &types).unwrap()
    }

    fn dump(text: &str) -> String {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types).unwrap();
        ir.pretty(ast.names()).to_string()
    }

    /// The node a file of one expression lowers to: its root block's result.
    fn result_of(ir: &Ir) -> IrId {
        let IrKind::Block { items, result } = ir.get(ir.root()).kind else {
            panic!("the root is not a block: {:?}", ir.get(ir.root()));
        };
        assert!(ir.items(items).is_empty());
        result
    }

    /// `7 / 2` built by hand with both literals given the types in `types`,
    /// e.g. both concrete `Int`, the shape variables produce.
    fn concrete_int_division(types: Vec<TyId>) -> (Ast, Checked) {
        let mut builder = AstBuilder::new();
        let lhs = builder.alloc(Expr {
            kind: ExprKind::Int(7),
            span: Span::new(0, 1),
        });
        let rhs = builder.alloc(Expr {
            kind: ExprKind::Int(2),
            span: Span::new(4, 5),
        });
        let div = builder.alloc(Expr {
            kind: ExprKind::Binary {
                op: BinaryOp::Div,
                op_start: 2,
                lhs,
                rhs,
            },
            span: Span::new(0, 5),
        });
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(div),
            span: Span::new(0, 5),
        });
        let root = builder.push_item_list(&[item]);
        let ast = builder.finish(root, Names::new(), Strings::new());
        let checked = Checked {
            types: Types::new(types),
            resolution: Resolution::without_variables(3, 1),
        };
        (ast, checked)
    }

    #[test]
    fn untyped_division_operands_become_float_constants() {
        let ir = lower_text("7 / 2");
        let root = ir.get(result_of(&ir));
        let IrKind::FloatDiv(operands) = root.kind else {
            panic!("expected FloatDiv, got {root:?}");
        };
        assert_eq!(ir.get(operands.lhs).kind, IrKind::FloatConst(7.0));
        assert_eq!(ir.get(operands.rhs).kind, IrKind::FloatConst(2.0));
        assert_eq!(operands.op_start, 2);
        assert_eq!(
            dump("7 / 2"),
            "(block @0..5\n  (float-div @0..5\n    (float 7.0 @0..1)\n    (float 2.0 @4..5)))"
        );
    }

    #[test]
    fn concrete_int_division_operands_are_converted() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::FLOAT]);
        let ir = lower(&ast, &checked, &TypeTable::new()).unwrap();
        assert_eq!(
            ir.pretty(ast.names()).to_string(),
            "(block @0..5\n  (float-div @0..5\n    (int-to-float @0..1\n      (int 7 @0..1))\n    (int-to-float @4..5\n      (int 2 @4..5))))"
        );
    }

    #[test]
    fn concrete_int_from_floor_division_is_converted() {
        assert_eq!(
            dump("(7 // 2) / 2"),
            "(block @0..12\n  (float-div @0..12\n    (int-to-float @0..8\n      (int-floor-div @1..7\n        (int 7 @1..2)\n        (int 2 @6..7)))\n    (float 2.0 @11..12)))"
        );
    }

    #[test]
    fn parentheses_and_ascriptions_are_dropped() {
        // The root block covers the file's items, parentheses included.
        assert_eq!(dump("((1))"), "(block @0..5\n  (int 1 @2..3))");
        assert_eq!(dump("(1 : Float)"), "(block @0..11\n  (float 1.0 @1..2))");
    }

    #[test]
    fn operators_are_specialised_by_type() {
        let ir = lower_text("-(1 + 2 - 3 * 4 // 5 % 6)");
        let result = result_of(&ir);
        assert_eq!(ir.kind_of(result), ValueKind::Int);
        assert_eq!(
            dump("-(1 + 2 - 3 * 4 // 5 % 6)"),
            "(block @0..25\n  (int-neg @0..25\n    (int-sub @2..24\n      (int-add @2..7\n        (int 1 @2..3)\n        (int 2 @6..7))\n      (int-rem @10..24\n        (int-floor-div @10..20\n          (int-mul @10..15\n            (int 3 @10..11)\n            (int 4 @14..15))\n          (int 5 @19..20))\n        (int 6 @23..24)))))"
        );
        assert_eq!(
            dump("-(1.0 + 2 - 3 * 4.0)"),
            "(block @0..20\n  (float-neg @0..20\n    (float-sub @2..19\n      (float-add @2..9\n        (float 1.0 @2..5)\n        (float 2.0 @8..9))\n      (float-mul @12..19\n        (float 3.0 @12..13)\n        (float 4.0 @16..19)))))"
        );
    }

    #[test]
    fn blocks_variables_and_statements() {
        let text = "let x = 1\n\
                    {\n  let y: Float = 2\n  y := y * 2.0\n}\n\
                    x := x + 1\n\
                    = { x }";
        // Slots are named by the last variable assigned to them: slot 1
        // holds `y` inside the block and then the rebound `x` after it, so
        // it shows as `x#1` throughout.
        assert_eq!(
            dump(text),
            "(block @0..66\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (block @10..47\n\
             \x20   (store x#1 @14..30\n\
             \x20     (float 2.0 @29..30))\n\
             \x20   (store y#2 @33..45\n\
             \x20     (float-mul @38..45\n\
             \x20       (load x#1 @38..39)\n\
             \x20       (float 2.0 @42..45)))\n\
             \x20   (empty @10..47))\n\
             \x20 (store x#1 @48..58\n\
             \x20   (int-add @53..58\n\
             \x20     (load x#0 @53..54)\n\
             \x20     (int 1 @57..58)))\n\
             \x20 (block @61..66\n\
             \x20   (load x#1 @63..64)))"
        );
    }

    #[test]
    fn empty_file_is_an_empty_block_with_an_empty_result() {
        assert_eq!(dump(""), "(block @0..0\n  (empty @0..0))");
        assert_eq!(dump("# nothing\n"), "(block @0..0\n  (empty @0..0))");
        let ir = lower_text("");
        assert_eq!(ir.kind_of(ir.root()), ValueKind::Record);
        assert_eq!(ir.frame_size(), 0);
        assert_eq!(ir.layout_count(), 0);
    }

    #[test]
    fn frame_size_comes_from_the_resolution() {
        assert_eq!(
            lower_text("let a = 1\n{ let b = 2; let c = 3 }").frame_size(),
            3
        );
    }

    #[test]
    fn large_untyped_literal_rounds_to_nearest_float() {
        let ir = lower_text("9007199254740993 / 1");
        let IrKind::FloatDiv(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected FloatDiv");
        };
        assert_eq!(
            ir.get(operands.lhs).kind,
            IrKind::FloatConst(9_007_199_254_740_992.0)
        );
    }

    #[test]
    fn untyped_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::UNTYPED_INT, TyId::INT, TyId::FLOAT]);
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(error.message, "an untyped integer survived type checking");
        assert_eq!(error.span, Span::new(0, 1));
    }

    #[test]
    fn error_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::ERROR, TyId::FLOAT]);
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(error.message, "a type error survived type checking");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn a_type_from_another_table_is_an_internal_error() {
        let ast = parse_text("{ x = 1 }");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(
            error.message,
            "a type with no values survived type checking"
        );
    }

    #[test]
    fn impossible_result_type_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::INT]);
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(error.message, "operator has a result type it cannot have");
        assert_eq!(error.span, Span::new(0, 5));
    }

    #[test]
    fn mismatched_operand_type_is_an_internal_error() {
        let ast = parse_text("1 + 2");
        let checked = Checked {
            types: Types::new(vec![TyId::INT, TyId::FLOAT, TyId::INT]),
            resolution: Resolution::without_variables(3, 1),
        };
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(error.message, "operand type does not match its operator");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn unresolved_variable_is_an_internal_error() {
        let ast = parse_text("let x = 1\nx");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let unresolved = Checked {
            types: checked.types.clone(),
            resolution: Resolution::without_variables(2, 2),
        };
        let error = lower(&ast, &unresolved, &types).unwrap_err();
        assert_eq!(error.message, "a declaration has no slot");
        assert_eq!(error.span, Span::new(0, 9));
    }

    #[test]
    fn interpolations_list_pieces_and_expressions() {
        assert_eq!(
            dump("let n = 1\n\"n = {n}, {n + 1}.\""),
            "(block @0..29\n\
             \x20 (store n#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (interp @10..29\n\
             \x20   (str \"n = \" @10..29)\n\
             \x20   (load n#0 @16..17)\n\
             \x20   (str \", \" @10..29)\n\
             \x20   (int-add @21..26\n\
             \x20     (load n#0 @21..22)\n\
             \x20     (int 1 @25..26))\n\
             \x20   (str \".\" @10..29)))"
        );
    }

    #[test]
    fn comparisons_are_specialised_by_operand_type() {
        assert_eq!(
            dump("1 < 2.5"),
            "(block @0..7\n  (float-cmp < @0..7\n    (float 1.0 @0..1)\n    (float 2.5 @4..7)))"
        );
        assert_eq!(
            dump("1 >= 2"),
            "(block @0..6\n  (int-cmp >= @0..6\n    (int 1 @0..1)\n    (int 2 @5..6)))"
        );
        assert_eq!(
            dump("\"a\" != \"b\""),
            "(block @0..10\n  (str-cmp != @0..10\n    (str \"a\" @0..3)\n    (str \"b\" @7..10)))"
        );
        assert_eq!(
            dump("true != false"),
            "(block @0..13\n  (bool-eq != @0..13\n    (bool true @0..4)\n    (bool false @8..13)))"
        );
        assert_eq!(
            dump("{} == {}"),
            "(block @0..8\n  (record-eq == [] @0..8\n    (block @0..2\n      (empty @0..2))\n    (block @6..8\n      (empty @6..8))))"
        );
    }

    #[test]
    fn if_keeps_both_branches() {
        assert_eq!(
            dump("if true { 1 } else { 2 }"),
            "(block @0..24\n\
             \x20 (if @0..24\n\
             \x20   (bool true @3..7)\n\
             \x20   (block @8..13\n\
             \x20     (int 1 @10..11))\n\
             \x20   (block @19..24\n\
             \x20     (int 2 @21..22))))"
        );
        // The implicit else branch is an empty block yielding `{}`.
        assert_eq!(
            dump("if true {}"),
            "(block @0..10\n\
             \x20 (if @0..10\n\
             \x20   (bool true @3..7)\n\
             \x20   (block @8..10\n\
             \x20     (empty @8..10))\n\
             \x20   (block @10..10\n\
             \x20     (empty @10..10))))"
        );
        let ir = lower_text("if true { 1 } else { 2.5 }");
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Float);
    }

    #[test]
    fn logic_and_concatenation() {
        assert_eq!(
            dump("true and false"),
            "(block @0..14\n  (and @0..14\n    (bool true @0..4)\n    (bool false @9..14)))"
        );
        assert_eq!(
            dump("not true or false"),
            "(block @0..17\n  (or @0..17\n    (not @0..8\n      (bool true @4..8))\n    (bool false @12..17)))"
        );
        assert_eq!(
            dump("\"a\" + r\"b\""),
            "(block @0..10\n  (str-concat @0..10\n    (str \"a\" @0..3)\n    (str \"b\" @6..10)))"
        );
    }

    #[test]
    fn string_constants_are_resolved_into_the_irs_table() {
        let ir = lower_text("\"a{1}b\" + \"c\\n\"");
        let IrKind::StrConcat(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected a concatenation");
        };
        let IrKind::Interp(items) = ir.get(operands.lhs).kind else {
            panic!("expected an interpolation");
        };
        let text = |id: IrId| match ir.get(id).kind {
            IrKind::StrConst(constant) => ir.string(constant).to_string(),
            other => format!("{other:?}"),
        };
        let items: Vec<String> = ir.items(items).iter().map(|&id| text(id)).collect();
        assert_eq!(items, vec!["a", "IntConst(1)", "b"]);
        assert_eq!(text(operands.rhs), "c\n");
    }

    #[test]
    fn tables_of_the_wrong_size_are_an_internal_error() {
        let (ast, _) = concrete_int_division(Vec::new());
        let checked = Checked {
            types: Types::new(vec![TyId::INT]),
            resolution: Resolution::without_variables(3, 1),
        };
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(error.message, "type table does not belong to this AST");
        let checked = Checked {
            types: Types::new(vec![TyId::INT, TyId::INT, TyId::FLOAT]),
            resolution: Resolution::without_variables(3, 2),
        };
        let error = lower(&ast, &checked, &TypeTable::new()).unwrap_err();
        assert_eq!(error.message, "resolution does not belong to this AST");
    }

    #[test]
    fn a_record_block_stores_its_fields_and_makes_a_record_of_them() {
        // Private `let`s and statements are items too; the record is the
        // fields' slots, loaded in definition order.
        assert_eq!(
            dump("{ x = 1; let t = x; {}; \"a b\" = t }"),
            "(block @0..35\n\
             \x20 (block @0..35\n\
             \x20   (store x#0 @2..7\n\
             \x20     (int 1 @6..7))\n\
             \x20   (store t#1 @9..18\n\
             \x20     (load x#0 @17..18))\n\
             \x20   (block @20..22\n\
             \x20     (empty @20..22))\n\
             \x20   (store \"a b\"#2 @24..33\n\
             \x20     (load t#1 @32..33))\n\
             \x20   (make-record [x \"a b\"] @0..35\n\
             \x20     (load x#0 @2..3)\n\
             \x20     (load \"a b\"#2 @24..29))))"
        );
        // A file can be a record block too.
        assert_eq!(
            dump("y = 2.5\nx = true"),
            "(block @0..16\n\
             \x20 (store y#0 @0..7\n\
             \x20   (float 2.5 @4..7))\n\
             \x20 (store x#1 @8..16\n\
             \x20   (bool true @12..16))\n\
             \x20 (make-record [y x] @0..16\n\
             \x20   (load y#0 @0..1)\n\
             \x20   (load x#1 @8..9)))"
        );
        let ir = lower_text("{ x = 1 }");
        assert_eq!(ir.kind_of(ir.root()), ValueKind::Record);
    }

    #[test]
    fn field_access_and_nested_records() {
        // The inner block's `n` takes the slot that `a`, declared after its
        // initialiser, then reuses, so it shows as `a#0` too.
        assert_eq!(
            dump("{ a = { n = 1.5 } }.a.n"),
            "(block @0..23\n\
             \x20 (field n @0..23\n\
             \x20   (field a @0..21\n\
             \x20     (block @0..19\n\
             \x20       (store a#0 @2..17\n\
             \x20         (block @6..17\n\
             \x20           (store a#0 @8..15\n\
             \x20             (float 1.5 @12..15))\n\
             \x20           (make-record [n] @6..17\n\
             \x20             (load a#0 @8..9))))\n\
             \x20       (make-record [a] @0..19\n\
             \x20         (load a#0 @2..3))))))"
        );
        let ir = lower_text("{ a = { n = 1.5 } }.a.n");
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Float);
        // Record equality and a field of every kind.
        assert_eq!(
            dump("{ x = 1 } != { x = 2 }"),
            "(block @0..22\n\
             \x20 (record-eq != [x] @0..22\n\
             \x20   (block @0..9\n\
             \x20     (store x#0 @2..7\n\
             \x20       (int 1 @6..7))\n\
             \x20     (make-record [x] @0..9\n\
             \x20       (load x#0 @2..3)))\n\
             \x20   (block @13..22\n\
             \x20     (store x#0 @15..20\n\
             \x20       (int 2 @19..20))\n\
             \x20     (make-record [x] @13..22\n\
             \x20       (load x#0 @15..16)))))"
        );
    }

    #[test]
    fn a_literal_checked_against_a_record_type_is_lowered_at_that_type() {
        assert_eq!(
            dump("let p: {x: Float} = { x = 1 }"),
            "(block @0..29\n\
             \x20 (store p#0 @0..29\n\
             \x20   (block @20..29\n\
             \x20     (store p#0 @22..27\n\
             \x20       (float 1.0 @26..27))\n\
             \x20     (make-record [x] @20..29\n\
             \x20       (load p#0 @22..23))))\n\
             \x20 (empty @0..29))"
        );
    }

    #[test]
    fn layouts_are_shared_by_blocks_with_the_same_names() {
        let ir = lower_text(
            "let a = { x = 1; y = 2 }\nlet b = { x = \"s\"; y = true }\n{ y = 1; x = 2 }",
        );
        // `x y` twice, then `y x`, a different order.
        assert_eq!(ir.layout_count(), 2);
        let ir = lower_text("{ x = 1 } == { x = 2 }");
        assert_eq!(ir.layout_count(), 1);
        let IrKind::RecordEq(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected a record comparison");
        };
        let layout_of = |block: IrId| {
            let IrKind::Block { result, .. } = ir.get(block).kind else {
                panic!("expected a block");
            };
            let IrKind::MakeRecord { layout, .. } = ir.get(result).kind else {
                panic!("expected a record");
            };
            layout
        };
        let (lhs, rhs) = (layout_of(operands.lhs), layout_of(operands.rhs));
        assert_eq!(lhs, rhs);
        assert_eq!(&*ir.layout(lhs).texts, &[Box::from("x")]);
    }

    #[test]
    fn a_merged_field_updates_its_slot() {
        // The first definition builds from nothing; later ones update the
        // slot, reading into the records that exist, and creating the rest.
        // The field is loaded once for the record.
        assert_eq!(
            dump("a.x = 1\na.y.z = 2\na.y.w = 3"),
            "(block @0..27\n\
             \x20 (store a#0 @0..7\n\
             \x20   (update-field x @0..3\n\
             \x20     (empty @0..1)\n\
             \x20     (int 1 @6..7)))\n\
             \x20 (store a#0 @8..17\n\
             \x20   (update-field y @8..11\n\
             \x20     (load a#0 @8..9)\n\
             \x20     (update-field z @8..13\n\
             \x20       (empty @8..11)\n\
             \x20       (int 2 @16..17))))\n\
             \x20 (store a#0 @18..27\n\
             \x20   (update-field y @18..21\n\
             \x20     (load a#0 @18..19)\n\
             \x20     (update-field w @18..23\n\
             \x20       (field y @18..21\n\
             \x20         (load a#0 @18..19))\n\
             \x20       (int 3 @26..27))))\n\
             \x20 (make-record [a] @0..27\n\
             \x20   (load a#0 @0..1)))"
        );
        // The name of every update is recorded for the evaluator.
        let ast = parse_text("a.x = 1\na.\"y z\".w = 2");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types).unwrap();
        let text = |text: &str| ir.name_text(ast.names().lookup(text).unwrap());
        assert_eq!(text("x"), Some("x"));
        assert_eq!(text("y z"), Some("y z"));
        assert_eq!(text("w"), Some("w"));
        // Not the field's own name, which no update adds.
        assert_eq!(text("a"), None);
    }

    #[test]
    fn a_merged_field_takes_its_place_where_first_mentioned() {
        assert_eq!(
            dump("{ a.x = 1; b = 2; a.y = 3 }"),
            "(block @0..27\n\
             \x20 (block @0..27\n\
             \x20   (store a#0 @2..9\n\
             \x20     (update-field x @2..5\n\
             \x20       (empty @2..3)\n\
             \x20       (int 1 @8..9)))\n\
             \x20   (store b#1 @11..16\n\
             \x20     (int 2 @15..16))\n\
             \x20   (store a#0 @18..25\n\
             \x20     (update-field y @18..21\n\
             \x20       (load a#0 @18..19)\n\
             \x20       (int 3 @24..25)))\n\
             \x20   (make-record [a b] @0..27\n\
             \x20     (load a#0 @2..3)\n\
             \x20     (load b#1 @11..12))))"
        );
    }

    #[test]
    fn a_dotted_let_updates_the_old_record_into_a_new_slot() {
        assert_eq!(
            dump("let a = { b = { c = 1 } }\nlet a.b.c = 2"),
            "(block @0..39\n\
             \x20 (store a#0 @0..25\n\
             \x20   (block @8..25\n\
             \x20     (store a#0 @10..23\n\
             \x20       (block @14..23\n\
             \x20         (store a#0 @16..21\n\
             \x20           (int 1 @20..21))\n\
             \x20         (make-record [c] @14..23\n\
             \x20           (load a#0 @16..17))))\n\
             \x20     (make-record [b] @8..25\n\
             \x20       (load a#0 @10..11))))\n\
             \x20 (store a#1 @26..39\n\
             \x20   (update-field b @30..33\n\
             \x20     (load a#0 @30..31)\n\
             \x20     (update-field c @30..35\n\
             \x20       (field b @30..33\n\
             \x20         (load a#0 @30..31))\n\
             \x20       (int 2 @38..39))))\n\
             \x20 (empty @0..39))"
        );
        // Where the static type lacks a segment, its record is created.
        assert_eq!(
            dump("let a = 1\nlet b.c.d = a\nb.c := { d = 2 }"),
            "(block @0..40\n\
             \x20 (store a#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store b#1 @10..23\n\
             \x20   (update-field c @14..17\n\
             \x20     (empty @14..15)\n\
             \x20     (update-field d @14..19\n\
             \x20       (empty @14..17)\n\
             \x20       (load a#0 @22..23))))\n\
             \x20 (store b#2 @24..40\n\
             \x20   (update-field c @24..27\n\
             \x20     (load b#1 @24..25)\n\
             \x20     (block @31..40\n\
             \x20       (store b#2 @33..38\n\
             \x20         (int 2 @37..38))\n\
             \x20       (make-record [d] @31..40\n\
             \x20         (load b#2 @33..34)))))\n\
             \x20 (empty @0..40))"
        );
    }

    #[test]
    fn typed_record_equality_compares_over_a_shape() {
        assert_eq!(
            dump("let p = { x = 1; inner = { n = 2 } }\np == p"),
            "(block @0..43\n\
             \x20 (store p#0 @0..36\n\
             \x20   (block @8..36\n\
             \x20     (store p#0 @10..15\n\
             \x20       (int 1 @14..15))\n\
             \x20     (store inner#1 @17..34\n\
             \x20       (block @25..34\n\
             \x20         (store inner#1 @27..32\n\
             \x20           (int 2 @31..32))\n\
             \x20         (make-record [n] @25..34\n\
             \x20           (load inner#1 @27..28))))\n\
             \x20     (make-record [x inner] @8..36\n\
             \x20       (load p#0 @10..11)\n\
             \x20       (load inner#1 @17..22))))\n\
             \x20 (record-eq == [x inner[n]] @37..43\n\
             \x20   (load p#0 @37..38)\n\
             \x20   (load p#0 @42..43)))"
        );
        // The shape is the static type's, not the value's.
        let text = "let q: {x: Int} = { x = 1; y = 2 }\nq != { x = 1 }";
        assert!(dump(text).contains("(record-eq != [x] @"));
        // One shape per record type, nested ones included, shared by every
        // comparison of it.
        let ir = lower_text(
            "let p = { x = 1; inner = { n = 2 } }\n\
             let a = p == p\n\
             let b = p.inner != p.inner\n\
             let c = { n = 3 } == p.inner\n\
             = { e = {} } == { e = {} }",
        );
        assert_eq!(ir.shape_count(), 4);
        // An empty record's shape has no fields.
        assert_eq!(
            dump("{ e = {} } == { e = {} }").lines().nth(1),
            Some("  (record-eq == [e[]] @0..24")
        );
    }

    #[test]
    fn total_equality_lowers_like_typed_equality_but_on_records() {
        assert_eq!(
            dump("{ x = 1 } !== { x = 1 }"),
            "(block @0..23\n\
             \x20 (record-same !== @0..23\n\
             \x20   (block @0..9\n\
             \x20     (store x#0 @2..7\n\
             \x20       (int 1 @6..7))\n\
             \x20     (make-record [x] @0..9\n\
             \x20       (load x#0 @2..3)))\n\
             \x20   (block @14..23\n\
             \x20     (store x#0 @16..21\n\
             \x20       (int 1 @20..21))\n\
             \x20     (make-record [x] @14..23\n\
             \x20       (load x#0 @16..17)))))"
        );
        assert_eq!(
            dump("{} === {}"),
            "(block @0..9\n  (record-same === @0..9\n    (block @0..2\n      (empty @0..2))\n    (block @7..9\n      (empty @7..9))))"
        );
        // Scalars lower exactly as `==` and `!=` do.
        assert_eq!(
            dump("1 === 2"),
            "(block @0..7\n  (int-cmp == @0..7\n    (int 1 @0..1)\n    (int 2 @6..7)))"
        );
        assert_eq!(
            dump("1.5 !== 2"),
            "(block @0..9\n  (float-cmp != @0..9\n    (float 1.5 @0..3)\n    (float 2.0 @8..9)))"
        );
        assert_eq!(
            dump("\"a\" === \"b\""),
            "(block @0..11\n  (str-cmp == @0..11\n    (str \"a\" @0..3)\n    (str \"b\" @8..11)))"
        );
        assert_eq!(
            dump("true !== false"),
            "(block @0..14\n  (bool-eq != @0..14\n    (bool true @0..4)\n    (bool false @9..14)))"
        );
        assert_eq!(lower_text("{ x = 1 } === { x = 2 }").shape_count(), 0);
    }
}
