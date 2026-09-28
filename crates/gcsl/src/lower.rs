//! Lowering: a checked [`Ast`] to [`Ir`].
//!
//! Lowering picks the type-specialised IR node for every operator, drops
//! `Paren` and `Ascribe` nodes (the checker has already coerced whatever an
//! ascription affects), turns integer literals typed `Float` into
//! `FloatConst` directly, and inserts `IntToFloat` for concrete `Int`
//! operands of `/`. Variables become `Store` nodes on the slots the checker
//! assigned, and each use becomes what its [`VarRef`] says: a `Load` of a
//! slot, a `LoadCapture` of a captured value, or a `LoadCallee`. Every
//! block, including the file's body, becomes a `Block` whose items are its
//! declarations and non-final expression statements, and whose result is
//! its `= EXPR` or final expression statement, or an explicit `Empty` node
//! if it has neither.
//!
//! A function expression's body is lowered into the IR's function table at
//! its [`crate::ast::FnId`], as code of [`FnKind::Memo`] if it is declared
//! with `->>` and of [`FnKind::Function`] otherwise (the IR does not tell
//! a pure function from an impure one, since only the checker cares), with
//! its frame and capture counts from the
//! checker's [`crate::check::FnResolution`] and its type as text, computed
//! once here, for its closures to display as. The expression itself becomes
//! a `MakeClosure` whose captures are the reads, in the frame creating the
//! closure, of each value the function captures. Function bodies nest, and
//! so does their lowering. A call lowers to a `Call` whose `Arg` nodes are
//! its arguments in source order, each with the parameter the checker bound
//! it to.
//!
//! A record block's fields become `Store`s to their slots, as `let`s do, so
//! that later items can read them, and its result is a `MakeRecord` whose
//! values are `Load`s of those slots in definition order; every expression
//! statement in it, the last included, is a statement. Each record block's
//! field names become a [`Layout`], built here once and shared by every
//! record the block builds; blocks with the same names in the same order
//! share one layout, found through a map from the names to the layout.
//!
//! A spread item is what its soft assignments say: a `Store` of its
//! operand, awaited first if the checker marked it to be, into the item's
//! own slot, so that it is evaluated once, and then, for each field of the
//! operand's static type once awaited, in the type's order, a `Store` into
//! the field's slot (see [`crate::check::Resolution::spread_slots`]) of a
//! `Field` of a `Load` of the item's slot, which reads the field by name, of
//! the kind of the field's type, as it is. So `...r`, with `r: {a: Int, b:
//! String}`, lowers to `Store(t, Load(r))`, `Store(a, Field(Load(t), a))`
//! and `Store(b, Field(Load(t), b))`. A field that overrides a soft one
//! stores into its slot, and the record loads each field once, from its one
//! slot, in the place where it was first defined, as the kind of the
//! definition that stands. The layout is static, as for any record block.
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
//! `Store(a', UpdateField(Load(a), b, UpdateField(Empty, c, e)))`. A record
//! along the path that the checker marked to be awaited (see
//! [`crate::check::Resolution::path_awaits`]) is read as it would be
//! otherwise, but of kind [`ValueKind::Deferred`], and wrapped in an `Await`
//! of kind `Record`, spanning the path up to its segment, as the record of a
//! field access is; that `Await` is the record node the update and the
//! access into it share. So with `a: Future<{b: {c: Int}}>`, the first
//! record is `Await(Load(a))`, and the second `Field(Await(Load(a)), b)`.
//! So the record walker that evaluates an update never sees a value that
//! may be a future. A dotted field stores back into its field's one slot.
//! Each record block's field is loaded for its record once, from the slot
//! its first definition declared. The text of each name an update sets is
//! recorded in the IR, for the evaluator to show a field it adds.
//!
//! Comparisons are specialised by their operands' kind, and an `if` keeps
//! the empty else block the parser gave it if the source had none. `===`
//! and `!==` lower exactly as `==` and `!=` on every kind but records and
//! lists. `==` and `!=` on records and lists compare over the [`Shape`] (a
//! [`ListShape`] for a list) of the operands' static type, read from the
//! type table: one shape per distinct record or list type met in such a
//! comparison, nested record and list types included, memoised by type and
//! built through a work list rather than recursion.
//!
//! A list literal lowers to a `MakeList` of its elements' expressions, in
//! source order, a spread's operand wrapped in a `Spread`, and an index to
//! an `Index` of the element's kind. A call of a built-in function, whose
//! callee the checker resolved to [`VarRef::Builtin`], lowers to the
//! built-in's own node, of its arguments as the checker bound them: `len`
//! to a `ListLen` of its list. Each
//! string literal, and each literal piece of an interpolated string,
//! becomes a `StrConst` whose text is copied from the AST's string table
//! into the IR's own, so the AST's table is not needed after lowering. A
//! literal piece has no span of its own in the AST, so its node has its
//! whole interpolated string's span.
//!
//! A `lazy` lowers like a function expression: its body, any expression,
//! into the function table, as code of [`FnKind::Future`], and the
//! expression to a `MakeFuture` that captures as a `MakeClosure` does.
//!
//! An expression the checker marked to be awaited (see
//! [`crate::check::Resolution::await_of`]) is lowered as it would be
//! otherwise, of the kind of its type before the await, which is
//! [`ValueKind::Deferred`] for a future or an either, and wrapped in an
//! `Await` whose kind is that of its type once awaited: its type stripped of
//! every future layer at its top, whose kind is the same as its type's once
//! awaited deeply. A deep await carries a [`LazyShape`] of that type, listing
//! the record fields to await, or for a list what to do with each element,
//! made once per type, as record shapes are,
//! and built from the leaves up through a work list. An explicit `await e`
//! is that `Await` itself, spanning the `await` expression, rather than an
//! `Await` of `e`; one of a present value is just its operand. Everywhere a
//! node of kind `Deferred` is expected, as a branch of an `if` or the result
//! of a block of such a type, a node of any kind is accepted, since a
//! present value may be where a future may.
//!
//! Once every node is built, one post-pass computes what the evaluator's
//! nesting budget needs (see [`crate::MAX_DEPTH`]): each `Call`'s and each
//! `Await`'s depth in the body of its frame, the body node being at depth 0,
//! and each function's body height, the number of nodes on its longest path
//! to a leaf. It walks the root and then each function's body, depth first
//! and iteratively, with an explicit stack, since a body can be `MAX_DEPTH`
//! deep: a call or an await gets its depth on the way down, and every node
//! its height, in one vector indexed by node, on the way up. A
//! `MakeClosure`'s or a `MakeFuture`'s captures are its children, loads in
//! the creating frame; the function it makes is not, since that body is
//! another frame, walked on its own. The nodes of a frame form a tree,
//! except the record nodes an update chain shares between the update that
//! builds on a record and the access into it: such a node's depth is
//! counted from the first path the walk reaches it by. The shared nodes are
//! loads, captures, the callee, field accesses, `Empty` and the `Await`s of
//! records that may be futures, never calls. An `Await`'s depth matters
//! only when it runs a future, and a shared one runs it, if at all, where it
//! is first evaluated: as the record of the update that builds on it, which
//! the evaluator evaluates before the update's value, where the access into
//! it is, and which is also the first parent the walk reaches it by, since
//! it visits a node's children in order. Every later evaluation, through
//! the access, finds the future already run, and only copies its value.
//!
//! Lowering cannot fail on tables produced by [`crate::check::check`]. Any
//! combination the checker should have ruled out is reported as an
//! [`InternalError`] rather than a panic.

use std::collections::HashMap;

use crate::ast::{
    ArgRange, Ast, BinaryOp, Effect, ElementRange, ExprId, ExprKind, FnId, ItemId, ItemKind,
    ItemRange, PartRange, PathRange, PathSegment, StrPart, UnaryOp, path_span,
};
use crate::check::{AwaitMark, Builtin, Checked, Resolution, Slot, Types, VarRef};
use crate::diagnostic::InternalError;
use crate::ir::{
    Cmp, FnCode, FnKind, Ir, IrBuilder, IrId, IrKind, IrRange, LayoutId, LazyField, LazyShape,
    LazyShapeId, ListShape, Operands, Shape, ShapeId, ShapedOperands,
};
use crate::names::Name;
use crate::source::Span;
use crate::types::{Laziness, TyId, TypeTable};
use crate::value::{Layout, ValueKind, int_to_float};

/// Lowers `ast`, whose types and resolution are `checked`, with its types
/// interned into `types`, to IR, recording that it is session input number
/// `input` (0 for a file; see [`Ir::input`]).
///
/// # Errors
///
/// Returns an [`InternalError`] if `checked` was not produced by checking
/// `ast` against `types`, or is otherwise inconsistent with it: a bug in
/// the caller or the checker, never an error in the user's program.
pub fn lower(
    ast: &Ast,
    checked: &Checked,
    types: &TypeTable,
    input: u32,
) -> Result<Ir, InternalError> {
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
    if resolution.expr_len() != ast.len()
        || resolution.item_len() != ast.item_count()
        || resolution.function_len() != ast.fn_count()
        || resolution.arg_len() != ast.arg_count()
    {
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
        record_loads: Vec::new(),
        shapes: HashMap::new(),
        shape_pending: Vec::new(),
        lazy_shapes: HashMap::new(),
        lazy_pending: Vec::new(),
        functions: vec![None; ast.fn_count()],
    };
    let root = ast.root();
    let kind = lowerer.body_kind(root)?;
    let kind = lowerer.lower_body(root, root_span, kind)?;
    let root = lowerer.builder.alloc(kind, root_span);
    // Every function expression is reachable from the root, so each has
    // been lowered.
    let mut functions = lowerer
        .functions
        .into_iter()
        .collect::<Option<Vec<FnCode>>>()
        .ok_or(InternalError {
            message: "a function expression was never lowered",
            span: root_span,
        })?;
    lowerer.builder.measure(root, &mut functions);
    Ok(lowerer.builder.finish_program(
        root,
        (resolution.frame_size(), resolution.slot_names().to_vec()),
        functions,
        input,
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
    /// Reused for collecting how each field of each record block is
    /// loaded, beside its name in `layout_names`: its slot, the kind of its
    /// value and the span of its definition.
    record_loads: Vec<(Slot, ValueKind, Span)>,
    /// The shape already made for each record and list type.
    shapes: HashMap<TyId, ShapeId>,
    /// Reused as the work list of record and list types whose shapes have
    /// ids but no fields, or element's shape, yet.
    shape_pending: Vec<TyId>,
    /// The lazy shape already made for each record and list type, or
    /// [`LazyShapeId::NONE`] if it has no lazy values at any depth.
    lazy_shapes: HashMap<TyId, LazyShapeId>,
    /// Reused as the work list of record and list types whose lazy shapes
    /// are being made, each with whether the types inside it have been
    /// queued.
    lazy_pending: Vec<(TyId, bool)>,
    /// Each function lowered so far, indexed by [`FnId`].
    functions: Vec<Option<FnCode>>,
}

impl Lowerer<'_> {
    /// The type of expression `id` where it is used: its type once awaited
    /// at the top, if it is awaited, and its type otherwise.
    fn used_type(&self, id: ExprId) -> TyId {
        let ty = self.expr_types.get(id);
        match self.resolution.await_of(id) {
            AwaitMark::None => ty,
            AwaitMark::Top | AwaitMark::Deep => self.table.strip(ty),
        }
    }

    /// The kind of the value of expression `id` where it is used: once
    /// awaited, if it is awaited. A deep await changes a record's fields,
    /// not its kind, so the type stripped at the top gives it.
    fn kind(&self, id: ExprId) -> Result<ValueKind, InternalError> {
        self.kind_of_type(id, self.used_type(id))
    }

    /// The kind of the value of expression `id` before any await: what the
    /// node it lowers to yields.
    fn plain_kind(&self, id: ExprId) -> Result<ValueKind, InternalError> {
        self.kind_of_type(id, self.expr_types.get(id))
    }

    /// The kind of the values of `ty`, the type of expression `id`, or an
    /// internal error if it has none.
    fn kind_of_type(&self, id: ExprId, ty: TyId) -> Result<ValueKind, InternalError> {
        let span = self.ast.get(id).span;
        match ty {
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
            Some(
                ItemKind::Let { .. }
                | ItemKind::Rebind { .. }
                | ItemKind::Field { .. }
                | ItemKind::Spread(_),
            )
            | None => Ok(ValueKind::Record),
        }
    }

    /// Whether `value` is directly an `await` expression.
    fn is_await(&self, value: ExprId) -> bool {
        matches!(
            self.ast.get(value).kind,
            ExprKind::Unary {
                op: UnaryOp::Await,
                ..
            }
        )
    }

    /// Whether `body` has a field or spread item, which makes it a record
    /// block.
    fn is_record_body(&self, body: ItemRange) -> bool {
        self.ast.items(body).iter().any(|&id| {
            matches!(
                self.ast.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        })
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
                ItemKind::Spread(value) => self.lower_spread(id, value, item.span)?,
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
                    // Evaluated for effect only, so it must be `{}`, unless
                    // it is a bare `await`, whose value is discarded
                    // whatever it is.
                    let statement = if self.is_await(value) {
                        self.lower_expr(value)?
                    } else {
                        self.lower_operand(value, ValueKind::Record)?
                    };
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
    /// each wrapped in an `Await` if the checker marked it to be awaited,
    /// and `Empty` where the path creates a record.
    fn lower_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        let table = self.resolution;
        let resolution = table.item_path(id);
        let existing = resolution.existing as usize;
        let awaits = table.path_awaits(resolution).ok_or(InternalError {
            message: "an update's awaited records are missing",
            span: path_span(segments),
        })?;
        let base = self.item_stack.len();
        // The record at each proper prefix of the path, `segments[..=end]`,
        // on the item stack: the record that setting `segments[end + 1]`
        // builds on.
        for end in 0..segments.len().saturating_sub(1) {
            let span = path_span(&segments[..=end]);
            // A record that may be a future is read as it is and awaited, as
            // the record of a field access is.
            let awaited = awaits.get(end).copied().unwrap_or(false);
            let read_kind = if awaited {
                ValueKind::Deferred
            } else {
                ValueKind::Record
            };
            let kind = if end >= existing {
                IrKind::Empty
            } else if end == 0 {
                read(resolution.source, read_kind).ok_or(InternalError {
                    message: "an update of an existing record has no source",
                    span,
                })?
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
                    kind: read_kind,
                }
            };
            let mut record = self.builder.alloc(kind, span);
            if awaited {
                record = self.builder.alloc(
                    IrKind::Await {
                        value: record,
                        kind: ValueKind::Record,
                        shape: LazyShapeId::NONE,
                        // Set by `IrBuilder::measure` once every node is
                        // built.
                        depth: 0,
                    },
                    span,
                );
            }
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

    /// Lowers the spread item `id`, whose operand is `value`, spanning
    /// `span`, onto the item stack: a `Store` of the operand, awaited if the
    /// checker marked it to be, into the item's own slot, so that it is
    /// evaluated once, and then, for each field of the operand's static
    /// type once awaited, in the type's order, a `Store` into the field's
    /// slot (see [`Resolution::spread_slots`]) of a `Field` of a `Load` of
    /// the item's slot, which reads the field by name. A field the runtime
    /// value holds but the static type hides is not read.
    fn lower_spread(&mut self, id: ItemId, value: ExprId, span: Span) -> Result<(), InternalError> {
        let (table, resolution) = (self.table, self.resolution);
        let held = resolution.item_slot(id);
        if held == Slot::NONE {
            return Err(InternalError {
                message: "a spread has no slot",
                span,
            });
        }
        let operand = self.lower_expr(value)?;
        let store = self.builder.alloc(IrKind::Store(held, operand), span);
        self.item_stack.push(store);
        let operand_span = self.ast.get(value).span;
        let fields = table
            .record_fields(table.strip(self.used_type(value)))
            .ok_or(InternalError {
                message: "a spread of a value that is not a record survived checking",
                span: operand_span,
            })?;
        let slots = resolution.spread_slots(id);
        if slots.len() != fields.len() || slots.contains(&Slot::NONE) {
            return Err(InternalError {
                message: "a spread's fields have no slots",
                span,
            });
        }
        for (&(name, ty), &slot) in fields.iter().zip(slots) {
            let kind = table.value_kind(ty).ok_or(InternalError {
                message: "a type with no values survived type checking",
                span: operand_span,
            })?;
            let record = self
                .builder
                .alloc(IrKind::Load(held, ValueKind::Record), operand_span);
            let field = self
                .builder
                .alloc(IrKind::Field { record, name, kind }, operand_span);
            let store = self.builder.alloc(IrKind::Store(slot, field), span);
            self.item_stack.push(store);
        }
        Ok(())
    }

    /// Builds the `MakeRecord` of the record block of `items`, spanning
    /// `span`, whose fields have been stored: a `Load` of each field's slot,
    /// at its name, in the order the fields were first defined, of the kind
    /// of its definition that stands. An open field is loaded once, from
    /// the slot its first definition declared, as a record. A soft field,
    /// one a spread defines, is loaded as the type its spread's operand
    /// gives it, unless a later item overrides it; since only a spread
    /// makes a field that may be defined again, the names defined so far
    /// are searched for the one being defined only from the block's first
    /// spread on.
    fn make_record(&mut self, items: &[ItemId], span: Span) -> Result<IrId, InternalError> {
        let (table, resolution) = (self.table, self.resolution);
        self.layout_names.clear();
        self.record_loads.clear();
        let mut spread = false;
        for &id in items {
            let (path, value) = match self.ast.item(id).kind {
                ItemKind::Field { path, value } => (path, value),
                ItemKind::Spread(operand) => {
                    spread = true;
                    let operand_span = self.ast.get(operand).span;
                    let fields = table
                        .record_fields(table.strip(self.used_type(operand)))
                        .unwrap_or_default();
                    // A type holds its fields in the order their names
                    // were interned, which is no order a reader knows: the
                    // fields take their places in the order of their names'
                    // text.
                    let names = self.ast.names();
                    let mut defined: Vec<_> = fields
                        .iter()
                        .zip(resolution.spread_slots(id))
                        .map(|(&(name, ty), &slot)| (name, ty, slot))
                        .collect();
                    defined.sort_unstable_by_key(|&(name, _, _)| names.get(name));
                    for (name, ty, slot) in defined {
                        let kind = table.value_kind(ty).ok_or(InternalError {
                            message: "a type with no values survived type checking",
                            span: operand_span,
                        })?;
                        self.define_field(name, (slot, kind, operand_span), spread);
                    }
                    continue;
                }
                _ => continue,
            };
            let (kind, field) = match self.ast.path(path) {
                [field] => (self.kind(value)?, field),
                // A later definition of an open field extends it.
                [field, ..] if resolution.item_path(id).source == VarRef::None => {
                    (ValueKind::Record, field)
                }
                _ => continue,
            };
            let slot = resolution.item_slot(id);
            self.define_field(field.name, (slot, kind, field.span), spread);
        }
        let base = self.item_stack.len();
        for &(slot, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(slot, kind), field_span);
            self.item_stack.push(load);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span))
    }

    /// Adds the field `name` to the record being built by
    /// [`Lowerer::make_record`], to be loaded as `load` says: its slot, the
    /// kind of its value and the span of its definition. If `redefinable`,
    /// the block has had a spread, and a field of the name already there is
    /// overridden instead, keeping its place.
    fn define_field(&mut self, name: Name, load: (Slot, ValueKind, Span), redefinable: bool) {
        if redefinable && let Some(at) = self.layout_names.iter().position(|&known| known == name) {
            if let Some(entry) = self.record_loads.get_mut(at) {
                *entry = load;
            }
            return;
        }
        self.layout_names.push(name);
        self.record_loads.push(load);
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

    /// Lowers expression `id`, wrapped in an `Await` if the checker marked
    /// it to be awaited where it is used.
    fn lower_expr(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        match self.resolution.await_of(id) {
            AwaitMark::None => {
                let kind = self.kind(id)?;
                self.lower_plain(id, kind)
            }
            mark => self.lower_awaited(id, mark, self.ast.get(id).span),
        }
    }

    /// Lowers expression `id`, which the checker marked to be awaited as
    /// `mark` says, to an `Await` spanning `span` of the node it lowers to
    /// otherwise: at the top, or deeply through the lazy shape of its type.
    fn lower_awaited(
        &mut self,
        id: ExprId,
        mark: AwaitMark,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let plain = self.plain_kind(id)?;
        let value = self.lower_plain(id, plain)?;
        let kind = self.kind(id)?;
        let shape = match mark {
            AwaitMark::Deep => self.lazy_shape(self.used_type(id)),
            AwaitMark::Top | AwaitMark::None => LazyShapeId::NONE,
        };
        Ok(self.builder.alloc(
            IrKind::Await {
                value,
                kind,
                shape,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }

    /// Lowers expression `id`, of kind `kind`, as it is, without any await
    /// the checker marked it for.
    fn lower_plain(&mut self, id: ExprId, kind: ValueKind) -> Result<IrId, InternalError> {
        let expr = *self.ast.get(id);
        let span = expr.span;
        let lowered = match (expr.kind, kind) {
            (ExprKind::Paren(inner) | ExprKind::Ascribe { expr: inner, .. }, _) => {
                return self.lower_operand(inner, kind);
            }
            (
                ExprKind::Unary {
                    op: UnaryOp::Await,
                    operand,
                },
                _,
            ) => {
                // An await of a present value does nothing; any other is
                // the operand's own await, spanning the whole expression.
                return match self.resolution.await_of(operand) {
                    AwaitMark::None => self.lower_operand(operand, kind),
                    mark => self.lower_awaited(operand, mark, span),
                };
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
            (
                ExprKind::Name(_) | ExprKind::Fn(_) | ExprKind::Call { .. } | ExprKind::Lazy(_),
                _,
            ) => self.lower_use_or_function(id, kind)?,
            (ExprKind::Block(body), _) => self.lower_body(body, span, kind)?,
            (ExprKind::Field { expr: record, name }, _) => IrKind::Field {
                record: self.lower_operand(record, ValueKind::Record)?,
                name,
                kind,
            },
            (ExprKind::Index { expr: list, index }, _) => IrKind::Index {
                list: self.lower_operand(list, ValueKind::List)?,
                index: self.lower_operand(index, ValueKind::Int)?,
                kind,
            },
            (ExprKind::List(elements), ValueKind::List) => self.lower_list(elements)?,
            (ExprKind::List(_), _) => {
                return Err(InternalError {
                    message: "a list literal typed as something other than a list",
                    span,
                });
            }
            (ExprKind::Float(_), ValueKind::Int) => {
                return Err(InternalError {
                    message: "float literal typed as Int",
                    span,
                });
            }
            (
                ExprKind::Int(_) | ExprKind::Float(_),
                ValueKind::Bool
                | ValueKind::String
                | ValueKind::Record
                | ValueKind::List
                | ValueKind::Fn
                | ValueKind::Deferred,
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

    /// Lowers `id`, of kind `kind`, which is a variable use, a function
    /// expression, a call or a `lazy`. Apart from `lower_plain`, which is on
    /// the recursion path of every expression, so that its frame does not
    /// hold the temporaries of these cases.
    fn lower_use_or_function(
        &mut self,
        id: ExprId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let expr = *self.ast.get(id);
        match (expr.kind, kind) {
            (ExprKind::Name(_), _) => {
                read(self.resolution.expr_var(id), kind).ok_or(InternalError {
                    message: "a variable use has no slot",
                    span: expr.span,
                })
            }
            (ExprKind::Fn(function), ValueKind::Fn) => self.lower_fn(id, function),
            (ExprKind::Lazy(function), ValueKind::Deferred) => self.lower_lazy(id, function),
            (ExprKind::Lazy(_), _) => Err(InternalError {
                message: "a `lazy` typed as something other than a future",
                span: expr.span,
            }),
            (ExprKind::Call { callee, args }, _) => match self.resolution.expr_var(callee) {
                VarRef::Builtin(builtin) => self.lower_builtin(builtin, args, kind, expr.span),
                _ => Ok(IrKind::Call {
                    callee: self.lower_operand(callee, ValueKind::Fn)?,
                    args: self.lower_args(args, expr.span)?,
                    kind,
                    // Set by `IrBuilder::measure` once every node is built.
                    depth: 0,
                }),
            },
            (ExprKind::Fn(_), _) => Err(InternalError {
                message: "a function expression typed as a non-function type",
                span: expr.span,
            }),
            _ => Err(InternalError {
                message: "an expression lowered as a variable, function or call is none",
                span: expr.span,
            }),
        }
    }

    /// Lowers the function expression `id`, whose details are `function`:
    /// its body into the function table, as code of [`FnKind::Memo`] if it
    /// is declared with `->>` and of [`FnKind::Function`] otherwise, and the
    /// expression to the `MakeClosure` that captures, in the current frame,
    /// what the checker found the function captures.
    fn lower_fn(&mut self, id: ExprId, function: FnId) -> Result<IrKind, InternalError> {
        let kind = match self.ast.fn_decl(function).effect {
            Effect::Memo => FnKind::Memo,
            Effect::Pure | Effect::Impure => FnKind::Function,
        };
        let captures = self.lower_function(id, function, kind)?;
        Ok(IrKind::MakeClosure { function, captures })
    }

    /// Lowers the `lazy` `id`, whose body is function `function`: the body
    /// into the function table, and the expression to the `MakeFuture` that
    /// captures, in the current frame, what the checker found it captures.
    fn lower_lazy(&mut self, id: ExprId, function: FnId) -> Result<IrKind, InternalError> {
        let captures = self.lower_function(id, function, FnKind::Future)?;
        Ok(IrKind::MakeFuture { function, captures })
    }

    /// Lowers the body of function `function`, of kind `kind`, the function
    /// expression or `lazy` `id`, into the function table, and returns the
    /// list of the loads, in the current frame, of what it captures. On the
    /// recursion path of nested functions and lazies, so the work after the
    /// body is in a helper whose temporaries this frame does not hold.
    fn lower_function(
        &mut self,
        id: ExprId,
        function: FnId,
        kind: FnKind,
    ) -> Result<IrRange, InternalError> {
        let body = self.lower_expr(self.ast.fn_decl(function).body)?;
        self.enter_function(id, function, (kind, body))
    }

    /// Enters function `function`, of kind `kind`, the function expression
    /// or `lazy` `id`, whose body lowered to `body`, into the function
    /// table, and returns the list of the loads, in the current frame, of
    /// what it captures.
    fn enter_function(
        &mut self,
        id: ExprId,
        function: FnId,
        (kind, body): (FnKind, IrId),
    ) -> Result<IrRange, InternalError> {
        let span = self.ast.get(id).span;
        let resolution = self.resolution.function(function);
        let display = self
            .table
            .display(self.expr_types.get(id), self.ast.names())
            .to_string()
            .into_boxed_str();
        let code = FnCode {
            body,
            kind,
            frame_size: resolution.frame_size,
            param_count: resolution.param_count,
            capture_count: u32::try_from(resolution.captures.len()).unwrap_or(u32::MAX),
            // Set by `IrBuilder::measure` once every node is built.
            height: 0,
            name: resolution.callee,
            display,
            slot_names: resolution.slot_names.clone().into_boxed_slice(),
        };
        let Some(entry) = self.functions.get_mut(function.index()) else {
            return Err(InternalError {
                message: "a function expression has no entry in the function table",
                span,
            });
        };
        *entry = Some(code);
        let base = self.item_stack.len();
        for &(var, ty) in &resolution.captures {
            let kind = self.table.value_kind(ty).ok_or(InternalError {
                message: "a captured value has a type with no values",
                span,
            })?;
            let read = read(var, kind).ok_or(InternalError {
                message: "a captured value is not a variable",
                span,
            })?;
            let load = self.builder.alloc(read, span);
            self.item_stack.push(load);
        }
        let captures = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(captures)
    }

    /// Lowers the call spanning `span` of the built-in function `builtin`
    /// with the arguments `args`, of kind `kind`, to the built-in's own
    /// node: `len` to a `ListLen` of its list.
    fn lower_builtin(
        &mut self,
        builtin: Builtin,
        args: ArgRange,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        match builtin {
            Builtin::Len if kind == ValueKind::Int => {
                let list = self.builtin_argument(args, 0, span)?;
                Ok(IrKind::ListLen(self.lower_operand(list, ValueKind::List)?))
            }
            Builtin::Len => Err(InternalError {
                message: "a call of `len` typed as something other than an Int",
                span,
            }),
        }
    }

    /// The argument, among `args`, of the call spanning `span` of a
    /// built-in function, that the checker bound to its parameter number
    /// `param`.
    fn builtin_argument(
        &self,
        args: ArgRange,
        param: u32,
        span: Span,
    ) -> Result<ExprId, InternalError> {
        self.ast
            .args(args)
            .iter()
            .zip(self.resolution.arg_params(args))
            .find(|&(_, &bound)| bound == param)
            .map(|(arg, _)| arg.value)
            .ok_or(InternalError {
                message: "a parameter of a built-in function is bound to no argument",
                span,
            })
    }

    /// Lowers the elements `elements` of a list literal to the items of a
    /// `MakeList`, in source order: each element's expression, as it is
    /// used, and a `Spread` of each spread's list.
    fn lower_list(&mut self, elements: ElementRange) -> Result<IrKind, InternalError> {
        let ast = self.ast;
        let base = self.item_stack.len();
        let mut spreads = false;
        for element in ast.elements(elements) {
            let item = if element.spread.is_some() {
                spreads = true;
                let list = self.lower_operand(element.value, ValueKind::List)?;
                self.builder.alloc(IrKind::Spread(list), element.span(ast))
            } else {
                self.lower_expr(element.value)?
            };
            self.item_stack.push(item);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::MakeList { items, spreads })
    }

    /// Lowers the arguments `args` of the call spanning `span` to `Arg`
    /// nodes in source order, each with the parameter the checker bound it
    /// to, and returns their list.
    fn lower_args(&mut self, args: ArgRange, span: Span) -> Result<IrRange, InternalError> {
        let (ast, resolution) = (self.ast, self.resolution);
        let params = resolution.arg_params(args);
        let base = self.item_stack.len();
        for (arg, &param) in ast.args(args).iter().zip(params) {
            if param == u32::MAX {
                return Err(InternalError {
                    message: "an argument of a call is bound to no parameter",
                    span,
                });
            }
            let value = self.lower_expr(arg.value)?;
            let node = self
                .builder
                .alloc(IrKind::Arg { param, value }, arg.span(ast));
            self.item_stack.push(node);
        }
        let list = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(list)
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
        // Typed equality on records and lists compares over the operands'
        // type, once awaited.
        let shape = match (operand_kind, op) {
            (ValueKind::Record | ValueKind::List, BinaryOp::Eq | BinaryOp::Ne) => {
                Some(self.shape(self.used_type(lhs), span)?)
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
            (ValueKind::List, BinaryOp::Eq, Some(shape)) => {
                IrKind::ListEq(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::List, BinaryOp::Ne, Some(shape)) => {
                IrKind::ListNe(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::List, BinaryOp::Same, _) => IrKind::ListSame(operands),
            (ValueKind::List, BinaryOp::NotSame, _) => IrKind::ListNotSame(operands),
            (
                ValueKind::Bool
                | ValueKind::Record
                | ValueKind::List
                | ValueKind::Fn
                | ValueKind::Deferred,
                ..,
            ) => {
                return Err(InternalError {
                    message: "comparison of a type it cannot compare",
                    span,
                });
            }
        })
    }

    /// The shape of the record or list type `ty`, made, with the shapes of
    /// the record and list types nested in it, if this is the first
    /// comparison of it. Each shape is given its id when it is first met
    /// and its fields, or its element's shape, when it is taken off the work
    /// list, so that nesting does not recurse. A field or an element that
    /// may be a future is compared as its value once awaited, which the
    /// deep await of the operands has done, so the shape follows its type
    /// stripped.
    fn shape(&mut self, ty: TyId, span: Span) -> Result<ShapeId, InternalError> {
        if let Some(&id) = self.shapes.get(&ty) {
            return Ok(id);
        }
        let table = self.table;
        let id = self.new_shape(ty);
        while let Some(ty) = self.shape_pending.pop() {
            if let Some(element) = table.list_element(ty) {
                let element = self.nested_shape(table.strip(element));
                if let Some(&shape) = self.shapes.get(&ty) {
                    self.builder.set_list_shape(shape, ListShape { element });
                }
                continue;
            }
            let Some(fields) = table.record_fields(ty) else {
                self.shape_pending.clear();
                return Err(InternalError {
                    message: "a record or list comparison of a type that is neither",
                    span,
                });
            };
            let mut shaped = Vec::with_capacity(fields.len());
            for &(name, field_ty) in fields {
                let nested = self.nested_shape(table.strip(field_ty));
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

    /// The shape a record's field or a list's element of type `ty`, stripped,
    /// is compared through: its own if it is a record or a list type, made
    /// and queued if it has none yet, and none otherwise.
    fn nested_shape(&mut self, ty: TyId) -> Option<ShapeId> {
        if !self.is_container(ty) {
            return None;
        }
        Some(match self.shapes.get(&ty) {
            Some(&nested) => nested,
            None => self.new_shape(ty),
        })
    }

    /// Whether `ty` is a record or a list type, which can hold values of
    /// other types.
    fn is_container(&self, ty: TyId) -> bool {
        self.table.is_record(ty) || self.table.list_element(ty).is_some()
    }

    /// Gives the record or list type `ty` a shape with no fields yet, and
    /// queues it to be given its fields or its element's shape.
    fn new_shape(&mut self, ty: TyId) -> ShapeId {
        let id = self.builder.push_shape(Shape {
            fields: Box::default(),
        });
        self.shapes.insert(ty, id);
        self.shape_pending.push(ty);
        id
    }

    /// The lazy shape of `ty`, a type present at its top: for a record type,
    /// each field whose type is a future or an either, with the lazy shape
    /// of its type stripped, and each present record or list field with
    /// lazy values somewhere inside, with its own; for a list type, what is
    /// done with each element, in the same way, if its type is lazy or has
    /// lazy values inside; [`LazyShapeId::NONE`] if it has no such field or
    /// element, or is neither. Made once per type, and the shapes of the
    /// record and list types inside it first, through a work list rather
    /// than recursion, since record and list types can nest as deeply as a
    /// program is long.
    fn lazy_shape(&mut self, ty: TyId) -> LazyShapeId {
        let table = self.table;
        if let Some(&id) = self.lazy_shapes.get(&ty) {
            return id;
        }
        self.lazy_pending.push((ty, false));
        while let Some((container, queued)) = self.lazy_pending.pop() {
            if self.lazy_shapes.contains_key(&container) {
                continue;
            }
            let element = table.list_element(container);
            let fields = table.record_fields(container).unwrap_or_default();
            if !queued {
                // The record and list types inside it first, then it again.
                self.lazy_pending.push((container, true));
                let inside = fields.iter().map(|&(_, field_ty)| field_ty).chain(element);
                for inner in inside {
                    let inner = table.strip(inner);
                    if self.is_container(inner) && !self.lazy_shapes.contains_key(&inner) {
                        self.lazy_pending.push((inner, false));
                    }
                }
                continue;
            }
            // What a deep await does with a value of type `ty` inside it.
            let action = |lazy_shapes: &HashMap<TyId, LazyShapeId>, ty: TyId| {
                let inner = lazy_shapes
                    .get(&table.strip(ty))
                    .copied()
                    .unwrap_or(LazyShapeId::NONE);
                if table.laziness(ty) != Laziness::Present {
                    Some(LazyField::Await(inner))
                } else if inner != LazyShapeId::NONE {
                    Some(LazyField::Within(inner))
                } else {
                    None
                }
            };
            let id = if let Some(element) = element {
                match action(&self.lazy_shapes, element) {
                    Some(each) => self.builder.push_lazy_list(each),
                    None => LazyShapeId::NONE,
                }
            } else {
                let lazy: Vec<(Name, LazyField)> = fields
                    .iter()
                    .filter_map(|&(name, field_ty)| {
                        action(&self.lazy_shapes, field_ty).map(|field| (name, field))
                    })
                    .collect();
                if lazy.is_empty() {
                    LazyShapeId::NONE
                } else {
                    self.builder.push_lazy_shape(LazyShape {
                        fields: lazy.into_boxed_slice(),
                    })
                }
            };
            self.lazy_shapes.insert(container, id);
        }
        self.lazy_shapes
            .get(&ty)
            .copied()
            .unwrap_or(LazyShapeId::NONE)
    }

    /// Lowers an operand that must already be of kind `expected`, where it
    /// is used. Where a value of kind `Deferred` is expected, a value of
    /// any kind is accepted, since a present value may be where a future
    /// may.
    fn lower_operand(&mut self, id: ExprId, expected: ValueKind) -> Result<IrId, InternalError> {
        if expected == ValueKind::Deferred || self.kind(id)? == expected {
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
        match self.used_type(id) {
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

/// The node reading a value of kind `kind` from `var`, or `None` if it is
/// not a variable, a built-in function being none.
const fn read(var: VarRef, kind: ValueKind) -> Option<IrKind> {
    match var {
        VarRef::Local(slot) if slot.get() != Slot::NONE.get() => Some(IrKind::Load(slot, kind)),
        VarRef::Capture(index) => Some(IrKind::LoadCapture(index, kind)),
        VarRef::Callee => Some(IrKind::LoadCallee),
        VarRef::Local(_) | VarRef::Builtin(_) | VarRef::None => None,
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
        lower(&ast, &checked, &types, 0).unwrap()
    }

    fn dump(text: &str) -> String {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 0).unwrap();
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
        let ir = lower(&ast, &checked, &TypeTable::new(), 0).unwrap();
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
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "an untyped integer survived type checking");
        assert_eq!(error.span, Span::new(0, 1));
    }

    #[test]
    fn error_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::ERROR, TyId::FLOAT]);
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "a type error survived type checking");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn a_type_from_another_table_is_an_internal_error() {
        let ast = parse_text("{ x = 1 }");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(
            error.message,
            "a type with no values survived type checking"
        );
    }

    #[test]
    fn impossible_result_type_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::INT]);
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
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
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
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
        let error = lower(&ast, &unresolved, &types, 0).unwrap_err();
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
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "type table does not belong to this AST");
        let checked = Checked {
            types: Types::new(vec![TyId::INT, TyId::INT, TyId::FLOAT]),
            resolution: Resolution::without_variables(3, 2),
        };
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
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
        let ir = lower(&ast, &checked, &types, 0).unwrap();
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

    #[test]
    fn a_closure_captures_in_the_creating_frame() {
        assert_eq!(
            dump("let x = 1\nlet f = fn(y: Int): Int { x + y }"),
            "(block @0..43\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store f#1 @10..43\n\
             \x20   (make-closure #0 @18..43\n\
             \x20     (load x#0 @18..43)))\n\
             \x20 (empty @0..43))\n\
             fn #0 f (frame 1, params 1, captures 1, height 3)\n\
             \x20 (block @34..43\n\
             \x20   (int-add @36..41\n\
             \x20     (load-capture 0 @36..37)\n\
             \x20     (load y#0 @40..41)))"
        );
    }

    #[test]
    fn a_call_passes_its_arguments_in_source_order() {
        let text = "fn f(a: Int, b: String): Int { a }\nf(b = \"s\", a = 1)";
        assert_eq!(
            dump(text),
            "(block @0..52\n\
             \x20 (store f#0 @0..34\n\
             \x20   (make-closure #0 @0..34))\n\
             \x20 (call @35..52\n\
             \x20   (load f#0 @35..36)\n\
             \x20   (arg 1 @37..44\n\
             \x20     (str \"s\" @41..44))\n\
             \x20   (arg 0 @46..51\n\
             \x20     (int 1 @50..51))))\n\
             fn #0 f (frame 2, params 2, captures 0, height 2)\n\
             \x20 (block @29..34\n\
             \x20   (load a#0 @31..32))"
        );
        // The root's result, after the function's declaration.
        let call = |ir: &Ir| match ir.get(ir.root()).kind {
            IrKind::Block { result, .. } => result,
            other => panic!("the root is not a block: {other:?}"),
        };
        let ir = lower_text(text);
        assert_eq!(ir.kind_of(call(&ir)), ValueKind::Int);
        let IrKind::Call { args, kind, .. } = ir.get(call(&ir)).kind else {
            panic!("expected a call");
        };
        assert_eq!(kind, ValueKind::Int);
        let arg = ir.items(args)[0];
        assert_eq!(ir.kind_of(arg), ValueKind::String);
        // A trailing block is the last argument.
        let ir = lower_text("fn f(a: Int, r: {x: Int}): Int { a }\nf(1) { x = 2 }");
        let IrKind::Call { args, .. } = ir.get(call(&ir)).kind else {
            panic!("expected a call");
        };
        let params: Vec<u32> = ir
            .items(args)
            .iter()
            .map(|&arg| match ir.get(arg).kind {
                IrKind::Arg { param, .. } => param,
                other => panic!("expected an argument, got {other:?}"),
            })
            .collect();
        assert_eq!(params, vec![0, 1]);
    }

    #[test]
    fn a_recursive_function_loads_its_callee() {
        assert_eq!(
            dump("fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }"),
            "(block @0..65\n\
             \x20 (store fact#0 @0..65\n\
             \x20   (make-closure #0 @0..65))\n\
             \x20 (empty @0..65))\n\
             fn #0 fact (frame 1, params 1, captures 0, height 8)\n\
             \x20 (block @21..65\n\
             \x20   (if @23..63\n\
             \x20     (int-cmp <= @26..32\n\
             \x20       (load n#0 @26..27)\n\
             \x20       (int 1 @31..32))\n\
             \x20     (block @33..38\n\
             \x20       (int 1 @35..36))\n\
             \x20     (block @44..63\n\
             \x20       (int-mul @46..61\n\
             \x20         (load n#0 @46..47)\n\
             \x20         (call @50..61\n\
             \x20           (load-callee @50..54)\n\
             \x20           (arg 0 @55..60\n\
             \x20             (int-sub @55..60\n\
             \x20               (load n#0 @55..56)\n\
             \x20               (int 1 @59..60))))))))"
        );
    }

    #[test]
    fn nested_functions_chain_their_captures() {
        // The innermost, pure, function measures `a` as its result, which
        // makes `a` present for `outer`.
        let text = "fn outer(a: Int): fn(): fn(): Int { fn(): fn(): Int { fn(): Int { a } } }";
        assert_eq!(
            dump(text),
            "(block @0..73\n\
             \x20 (store outer#0 @0..73\n\
             \x20   (make-closure #2 @0..73))\n\
             \x20 (empty @0..73))\n\
             fn #0 (frame 0, params 0, captures 1, height 2)\n\
             \x20 (block @64..69\n\
             \x20   (load-capture 0 @66..67))\n\
             fn #1 (frame 0, params 0, captures 1, height 3)\n\
             \x20 (block @52..71\n\
             \x20   (make-closure #0 @54..69\n\
             \x20     (load-capture 0 @54..69)))\n\
             fn #2 outer (frame 1, params 1, captures 0, height 3)\n\
             \x20 (block @34..73\n\
             \x20   (make-closure #1 @36..71\n\
             \x20     (load a#0 @36..71)))"
        );
        // A captured callee is loaded as the callee in the creating frame.
        let ir = lower_text("fn f(): fn(): Int { fn(): Int { f()() } }");
        let outer = ir.function(FnId::new(1));
        let IrKind::Block { result, .. } = ir.get(outer.body).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeClosure { function, captures } = ir.get(result).kind else {
            panic!("expected a closure");
        };
        assert_eq!(function, FnId::new(0));
        let captures: Vec<IrKind> = ir
            .items(captures)
            .iter()
            .map(|&id| ir.get(id).kind)
            .collect();
        assert_eq!(captures, vec![IrKind::LoadCallee]);
    }

    #[test]
    fn a_memoised_function_is_code_of_its_own_kind() {
        let ir = lower_text(
            "fn f(x: Int) ->> Int { x }\n\
             fn g(x: Int) -> Int { x }\n\
             fn h(x: Int): Int { x }\n\
             let l = lazy f(1)",
        );
        let kinds: Vec<FnKind> = (0..4).map(|id| ir.function(FnId::new(id)).kind).collect();
        assert_eq!(
            kinds,
            vec![
                FnKind::Memo,
                FnKind::Function,
                FnKind::Function,
                FnKind::Future
            ]
        );
        assert_eq!(&*ir.function(FnId::new(0)).display, "fn(x: Int) ->> Int");
        // The IR's dump shows it.
        assert_eq!(
            dump("fn f(x: Int) ->> Int { x }"),
            "(block @0..26\n\
             \x20 (store f#0 @0..26\n\
             \x20   (make-closure #0 @0..26))\n\
             \x20 (empty @0..26))\n\
             memo fn #0 f (frame 1, params 1, captures 0, height 2)\n\
             \x20 (block @21..26\n\
             \x20   (load x#0 @23..24))"
        );
    }

    #[test]
    fn the_function_table_describes_each_function() {
        let ast = parse_text(
            "let k = 2\n\
             fn scale(x: Int, by: {f: Float}) -> Float { let y = x * k\ny / by.f }\n\
             let anon = fn(): {} {}",
        );
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 7).unwrap();
        assert_eq!(ir.input(), 7);
        assert_eq!(ir.function_count(), 2);
        let scale = ir.function(FnId::new(0));
        assert_eq!(
            &*scale.display,
            "fn(x: Int | Future<Int>, by: {f: Float | Future<Float>} | Future<{f: Float | \
             Future<Float>}>) -> Float"
        );
        assert_eq!(scale.kind, FnKind::Function);
        assert_eq!(scale.frame_size, 3);
        assert_eq!(scale.param_count, 2);
        assert_eq!(scale.capture_count, 1);
        assert_eq!(scale.name, ast.names().lookup("scale"));
        let slot_names: Vec<&str> = scale
            .slot_names
            .iter()
            .map(|&name| ast.names().get(name))
            .collect();
        assert_eq!(slot_names, vec!["x", "by", "y"]);
        assert_eq!(ir.kind_of(scale.body), ValueKind::Float);
        let anon = ir.find_function(FnId::new(1)).unwrap();
        assert_eq!(&*anon.display, "fn(): {}");
        assert_eq!(anon.name, ast.names().lookup("anon"));
        assert_eq!(
            (anon.frame_size, anon.param_count, anon.capture_count),
            (0, 0, 0)
        );
        assert!(ir.find_function(FnId::new(2)).is_none());
        assert_eq!(lower_text("1").input(), 0);
        // A closure is a function.
        let ir = lower_text("fn f(): fn(): Int { fn(): Int { 1 } }");
        assert_eq!(ir.kind_of(ir.function(FnId::new(1)).body), ValueKind::Fn);
    }

    /// The depth of each call in the frame whose body is `body`, in the
    /// order a depth-first walk meets them.
    fn depths(ir: &Ir, body: IrId) -> Vec<u16> {
        ir.calls_in(body)
            .into_iter()
            .map(|(depth, _)| depth)
            .collect()
    }

    #[test]
    fn a_call_is_as_deep_as_its_path_from_its_frames_body() {
        let g = "fn g(n: Int): Int { n }\n";
        // In a function's body: the block (0), the `+` (1), the `*` (2),
        // the call (3).
        let ir = lower_text(&format!("{g}fn f(n: Int): Int {{ 1 + g(n) * 2 }}"));
        assert_eq!(depths(&ir, ir.function(FnId::new(1)).body), vec![3]);
        // In the file's body: the block (0), the `let`'s store (1), the
        // outer call (2), its argument (3), the inner call (4).
        let ir = lower_text(&format!("{g}let x = g(g(1))\nx"));
        assert_eq!(depths(&ir, ir.root()), vec![2, 4]);
        // A callee is a child of its call: the block (0), the call of the
        // closure (1), the call that makes it (2).
        let ir = lower_text("fn make(): fn(): Int { fn(): Int { 1 } }\nmake()()");
        assert_eq!(depths(&ir, ir.root()), vec![1, 2]);
    }

    #[test]
    fn the_shared_nodes_of_an_update_chain_do_not_disturb_depths() {
        let g = "fn g(n: Int): Int { n }\n";
        // `let a.b.c = g(a.b.c)`: the block (0), the store (1), the update
        // of `b` (2), the update of `c` (3), the call (4). The update of `b`
        // shares its load of `a` with the access `a.b` that the update of
        // `c` builds on, so the walk reaches that load twice; the call's
        // depth is its own path's, and so is the later call's (1).
        let ir = lower_text(&format!(
            "{g}let a = {{ b = {{ c = 1 }} }}\nlet a.b.c = g(a.b.c)\n= g(a.b.c)"
        ));
        assert_eq!(depths(&ir, ir.root()), vec![4, 1]);
        // In a function's body, where the chain's longest path is through
        // the call: the block (1), the store (2), the update of `b` (3), the
        // update of `c` (4), the call (5), its argument (6), the literal (7).
        let ir = lower_text(&format!(
            "{g}fn u(r: {{b: {{c: Int}}}}): {{b: {{c: Int}}}} {{ let r.b.c = g(1)\nr }}"
        ));
        let u = ir.function(FnId::new(1));
        assert_eq!(depths(&ir, u.body), vec![4]);
        assert_eq!(u.height, 7);
    }

    #[test]
    fn a_closures_body_is_a_frame_of_its_own() {
        let text = "fn g(n: Int): Int { n }\nlet k = { { fn(): Int { g(1) } } }\ng(k())";
        let ir = lower_text(text);
        // In the frame creating the closure: the block (0), the call of `g`
        // (1), its argument (2), the call of `k` (3). The call in the
        // closure's body is not in this frame.
        assert_eq!(depths(&ir, ir.root()), vec![1, 3]);
        // In the closure's own, however deep it was made: the block (0), the
        // call (1). Its body's height: the block, the call, its argument, the
        // literal.
        let closure = ir.function(FnId::new(1));
        assert_eq!(depths(&ir, closure.body), vec![1]);
        assert_eq!(closure.height, 4);
        // A body that makes a closure is as tall as the loads of what it
        // captures, not the closure's body: the block, the closure, its
        // capture of `g`.
        let ir = lower_text(
            "fn g(n: Int): Int { n }\nfn outer(): fn(): Int { fn(): Int { g(g(g(1))) } }",
        );
        assert_eq!(ir.function(FnId::new(2)).height, 3);
        // The block, three calls and their arguments, and the literal.
        assert_eq!(ir.function(FnId::new(1)).height, 8);
    }

    #[test]
    fn a_function_body_is_as_tall_as_its_longest_path() {
        // The block (1), the `if` (2), the then-block (3), the negations (4
        // and 5), the load (6); the condition and the else branch are
        // shorter.
        let ir = lower_text("fn h(n: Int): Int { if n > 0 { -(-n) } else { 0 } }");
        assert_eq!(ir.function(FnId::new(0)).height, 6);
        // An empty body is a block with an empty result.
        let ir = lower_text("fn e(): {} {}");
        assert_eq!(ir.function(FnId::new(0)).height, 2);
    }

    #[test]
    fn an_argument_bound_to_no_parameter_is_an_internal_error() {
        let ast = parse_text("fn f(a: Int): Int { a }\nf(1)");
        let mut types = TypeTable::new();
        let (mut checked, _) = check(&ast, &mut types).unwrap();
        checked.resolution.unbind_arguments();
        let error = lower(&ast, &checked, &types, 0).unwrap_err();
        assert_eq!(
            error.message,
            "an argument of a call is bound to no parameter"
        );
        // A resolution without the tree's functions does not belong to it.
        let unresolved = Checked {
            types: checked.types.clone(),
            resolution: Resolution::without_variables(ast.len(), ast.item_count()),
        };
        let error = lower(&ast, &unresolved, &types, 0).unwrap_err();
        assert_eq!(error.message, "resolution does not belong to this AST");
    }

    // ----- Futures -----

    #[test]
    fn a_lazy_makes_a_future_capturing_in_the_creating_frame() {
        let text = "let x = 1\nlet f = lazy x + 1";
        assert_eq!(
            dump(text),
            "(block @0..28\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store f#1 @10..28\n\
             \x20   (make-future #0 @18..28\n\
             \x20     (load x#0 @18..28)))\n\
             \x20 (empty @0..28))\n\
             future #0 (frame 0, params 0, captures 1, height 2)\n\
             \x20 (int-add @23..28\n\
             \x20   (load-capture 0 @23..24)\n\
             \x20   (int 1 @27..28))"
        );
        let ir = lower_text(text);
        let code = ir.function(FnId::new(0));
        assert_eq!(code.kind, FnKind::Future);
        assert_eq!(&*code.display, "Future<Int>");
        assert_eq!(
            (code.param_count, code.capture_count, code.name),
            (0, 1, None)
        );
        // A future's body is any node, of the kind of what it yields.
        assert_eq!(ir.kind_of(code.body), ValueKind::Int);
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        let IrKind::Store(_, future) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert_eq!(ir.kind_of(future), ValueKind::Deferred);
        // A future made in a function body is made in that frame. `n` is
        // only captured by the `lazy`, which does not measure it for `f`,
        // so it is the either shape and its load `Deferred`.
        let ir = lower_text("fn f(n: Int): Future<Int> { lazy n * 2 }");
        assert_eq!(ir.function(FnId::new(0)).kind, FnKind::Future);
        assert_eq!(ir.function(FnId::new(1)).kind, FnKind::Function);
        let IrKind::Block { result, .. } = ir.get(ir.function(FnId::new(1)).body).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeFuture { function, captures } = ir.get(result).kind else {
            panic!("expected a future");
        };
        assert_eq!(function, FnId::new(0));
        assert_eq!(
            ir.items(captures)
                .iter()
                .map(|&id| ir.get(id).kind)
                .collect::<Vec<_>>(),
            vec![IrKind::Load(Slot::new(0), ValueKind::Deferred)]
        );
    }

    #[test]
    fn an_await_wraps_what_is_awaited() {
        // At the top, of the kind of what is awaited once present.
        let text = "let x = lazy 1\nx + 1";
        assert_eq!(
            dump(text),
            "(block @0..20\n\
             \x20 (store x#0 @0..14\n\
             \x20   (make-future #0 @8..14))\n\
             \x20 (int-add @15..20\n\
             \x20   (await @15..16\n\
             \x20     (load x#0 @15..16))\n\
             \x20   (int 1 @19..20)))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 1 @13..14)"
        );
        let ir = lower_text(text);
        let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an addition");
        };
        let IrKind::Await {
            value, kind, shape, ..
        } = ir.get(operands.lhs).kind
        else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::Int);
        assert_eq!(shape, LazyShapeId::NONE);
        assert_eq!(ir.kind_of(operands.lhs), ValueKind::Int);
        assert_eq!(ir.kind_of(value), ValueKind::Deferred);
        // An explicit await is one node, spanning the `await`.
        assert_eq!(
            dump("let x = lazy 1\nawait x").lines().nth(3),
            Some("  (await @15..22")
        );
        let ir = lower_text("let x = lazy 1\nawait x");
        let awaited = result_after_items(&ir);
        let IrKind::Await { value, .. } = ir.get(awaited).kind else {
            panic!("expected an await");
        };
        assert!(matches!(ir.get(value).kind, IrKind::Load(..)));
        // Of a present value, it is its operand.
        assert_eq!(
            dump("let x = 1\nawait x"),
            "(block @0..17\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (load x#0 @16..17))"
        );
        // A parenthesis is dropped, and the await spans it.
        assert_eq!(
            dump("let x = lazy 1\n(x) + 1").lines().nth(4),
            Some("    (await @15..18")
        );
    }

    #[test]
    fn a_call_of_an_inferred_present_result_is_not_awaited() {
        // An impure function whose body returns a present record of present
        // fields returns one: its call is of a concrete kind, and neither it
        // nor its field is awaited.
        let body = "{ a = 1; b = \"x\" }\n";
        let text = format!("fn get() -> {{a: Int, b: String}} {body}get().a + 1");
        let ir = lower_text(&text);
        let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an addition");
        };
        let IrKind::Field { record, kind, .. } = ir.get(operands.lhs).kind else {
            panic!("expected a field access");
        };
        assert_eq!(kind, ValueKind::Int);
        let IrKind::Call { kind, .. } = ir.get(record).kind else {
            panic!("expected a call");
        };
        assert_eq!(kind, ValueKind::Record);
        assert!(!dump(&text).contains("(await"));
        // Written as the either shape, the call may be a future, and so may
        // its field: both are awaited.
        let either = "{a: Int | Future<Int>, b: String | Future<String>}";
        let text = format!("fn get() -> {either} | Future<{either}> {body}get().a + 1");
        let ir = lower_text(&text);
        let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an addition");
        };
        let IrKind::Await { value, .. } = ir.get(operands.lhs).kind else {
            panic!("expected an await of the field");
        };
        let IrKind::Field { record, .. } = ir.get(value).kind else {
            panic!("expected a field access");
        };
        let IrKind::Await { value, .. } = ir.get(record).kind else {
            panic!("expected an await of the call");
        };
        let IrKind::Call { kind, .. } = ir.get(value).kind else {
            panic!("expected a call");
        };
        assert_eq!(kind, ValueKind::Deferred);
        assert_eq!(dump(&text).matches("(await").count(), 2);
    }

    #[test]
    fn a_deep_await_carries_a_lazy_shape() {
        let text = "let r = { a = lazy 1; b = { c = lazy 2; d = 3 }; e = 4 }\n\"{r}\"";
        let shown = dump(text);
        assert!(
            shown.contains(
                "\n  (interp @57..62\n\
                 \x20   (str \"\" @57..62)\n\
                 \x20   (await [a! b[c!]] @59..60\n\
                 \x20     (load r#0 @59..60))\n"
            ),
            "{shown}"
        );
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 0).unwrap();
        assert_eq!(ir.lazy_shape_count(), 2);
        let IrKind::Interp(items) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an interpolation");
        };
        let IrKind::Await { shape, kind, .. } = ir.get(ir.items(items)[1]).kind else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::Record);
        let name = |text: &str| ast.names().lookup(text).unwrap();
        let [
            (a, LazyField::Await(LazyShapeId::NONE)),
            (b, LazyField::Within(inner)),
        ] = *ir.lazy_shape(shape).fields
        else {
            panic!("unexpected shape {:?}", ir.lazy_shape(shape));
        };
        assert_eq!((a, b), (name("a"), name("b")));
        assert_eq!(
            &*ir.lazy_shape(inner).fields,
            &[(name("c"), LazyField::Await(LazyShapeId::NONE))]
        );
        assert_eq!(ir.find_lazy_shape(LazyShapeId::NONE), None);
        // A field that is lazy and descended into, and a present record
        // with lazy fields.
        let shown = dump(
            "let r = { a = lazy { x = lazy 1; y = 2 }; b = { c = { d = lazy 3 } } }\n= \"{r}\"",
        );
        assert!(shown.contains("(await [a![x!] b[c[d!]]] @"), "{shown}");
        // Only the lazy parts are listed; a record with none needs no
        // shape, and is not awaited.
        let shown = dump("let r = { a = 1; b = { c = 2 } }\n\"{r}\"");
        assert!(!shown.contains("await"), "{shown}");
        // Shapes are shared by the deep awaits of one type.
        let ir = lower_text("let r = { a = lazy 1 }\nlet s = \"{r}\"\nlet t = \"{r} {r}\"");
        assert_eq!(ir.lazy_shape_count(), 1);
        // An equality awaits deeply and compares over the present fields;
        // the nested record type, in an impure signature, may be a future
        // itself.
        let shown = dump("fn f(r: {a: {b: Int}}) -> Bool { r == r }");
        assert!(shown.contains("\n    (record-eq == [a[b]] @"), "{shown}");
        assert!(shown.contains("\n      (await [a![b!]] @"), "{shown}");
    }

    #[test]
    fn values_that_may_be_futures_are_of_kind_deferred() {
        // An `if` of such a type, with a present branch.
        let text = "let x = lazy 1\nlet y = if true { x } else { 2 }";
        assert_eq!(
            dump(text),
            "(block @0..47\n\
             \x20 (store x#0 @0..14\n\
             \x20   (make-future #0 @8..14))\n\
             \x20 (store y#1 @15..47\n\
             \x20   (if @23..47\n\
             \x20     (bool true @26..30)\n\
             \x20     (block @31..36\n\
             \x20       (load x#0 @33..34))\n\
             \x20     (block @42..47\n\
             \x20       (int 2 @44..45))))\n\
             \x20 (empty @0..47))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 1 @13..14)"
        );
        let ir = lower_text(text);
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        let IrKind::Store(_, choice) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert_eq!(ir.kind_of(choice), ValueKind::Deferred);
        // A present value stored in a slot of such a type.
        assert_eq!(
            dump("let x: Int | Future<Int> = 1"),
            "(block @0..28\n\
             \x20 (store x#0 @0..28\n\
             \x20   (int 1 @27..28))\n\
             \x20 (empty @0..28))"
        );
        // A pure function passing an either parameter along: a load of
        // kind `Deferred`, stored in a record, never awaited.
        let text = "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }";
        assert_eq!(
            dump(text),
            "(block @0..63\n\
             \x20 (store keep#0 @0..63\n\
             \x20   (make-closure #0 @0..63))\n\
             \x20 (empty @0..63))\n\
             fn #0 keep (frame 2, params 1, captures 0, height 3)\n\
             \x20 (block @54..63\n\
             \x20   (store v#1 @56..61\n\
             \x20     (load x#0 @60..61))\n\
             \x20   (make-record [v] @54..63\n\
             \x20     (load v#1 @56..57)))"
        );
        let ir = lower_text(text);
        let IrKind::Block { items, .. } = ir.get(ir.function(FnId::new(0)).body).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, load) = ir.get(ir.items(items)[0]).kind else {
            panic!("expected a store");
        };
        assert_eq!(
            ir.get(load).kind,
            IrKind::Load(Slot::new(0), ValueKind::Deferred)
        );
        // A bare `await` item of a value that is not `{}`.
        let ir = lower_text("let x = lazy 1\nawait x\n2");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        assert!(matches!(
            ir.get(ir.items(items)[1]).kind,
            IrKind::Await {
                kind: ValueKind::Int,
                ..
            }
        ));
    }

    #[test]
    fn awaits_are_as_deep_as_their_path_from_their_frames_body() {
        // In the file's body: the block (0), the `let`'s store (1), the `+`
        // (2), the await (3). And the explicit await of the result (1).
        let ir = lower_text("let x = lazy 1\nlet y = x + 1\nawait x");
        assert_eq!(
            ir.awaits_in(ir.root())
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![3, 1]
        );
        // In a function's body, which counts the await in its height: the
        // block (1), the `*` (2), the await (3), the load (4).
        let ir = lower_text("fn f(x: Int) -> Int { x * 2 }");
        let code = ir.function(FnId::new(0));
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(code.height, 4);
        // In a future's body: the await (0), the capture (1).
        let ir = lower_text("let x = lazy 1\nlet y = lazy x + 1");
        let code = ir.function(FnId::new(1));
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(code.height, 3);
    }

    #[test]
    fn an_update_awaits_each_record_along_its_path_that_may_be_a_future() {
        // An impure function's parameter, the either shape at every level:
        // each record is awaited, spanning the path up to it, and the await
        // is the record node the update and the access into it share.
        let text = "fn f(r: {a: {b: Int}}) -> {} { let r.a.b = 2 }";
        assert_eq!(
            dump(text),
            "(block @0..46\n\
             \x20 (store f#0 @0..46\n\
             \x20   (make-closure #0 @0..46))\n\
             \x20 (empty @0..46))\n\
             fn #0 f (frame 2, params 1, captures 0, height 8)\n\
             \x20 (block @29..46\n\
             \x20   (store r#1 @31..44\n\
             \x20     (update-field a @35..38\n\
             \x20       (await @35..36\n\
             \x20         (load r#0 @35..36))\n\
             \x20       (update-field b @35..40\n\
             \x20         (await @35..38\n\
             \x20           (field a @35..38\n\
             \x20             (await @35..36\n\
             \x20               (load r#0 @35..36))))\n\
             \x20         (int 2 @43..44))))\n\
             \x20   (empty @29..46))"
        );
        let ir = lower_text(text);
        let code = ir.function(FnId::new(0));
        let IrKind::Block { items, .. } = ir.get(code.body).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, outer) = ir.get(ir.items(items)[0]).kind else {
            panic!("expected a store");
        };
        let IrKind::UpdateField {
            record: first,
            value: inner,
            ..
        } = ir.get(outer).kind
        else {
            panic!("expected an update");
        };
        let IrKind::UpdateField { record: second, .. } = ir.get(inner).kind else {
            panic!("expected an update");
        };
        // Each read is of kind `Deferred`, and each await of a record.
        let IrKind::Await {
            value: load,
            kind: ValueKind::Record,
            shape: LazyShapeId::NONE,
            ..
        } = ir.get(first).kind
        else {
            panic!("expected an await of a record");
        };
        assert_eq!(
            ir.get(load).kind,
            IrKind::Load(Slot::new(0), ValueKind::Deferred)
        );
        let IrKind::Await {
            value: field,
            kind: ValueKind::Record,
            ..
        } = ir.get(second).kind
        else {
            panic!("expected an await of a record");
        };
        let IrKind::Field { record, kind, .. } = ir.get(field).kind else {
            panic!("expected a field access");
        };
        assert_eq!(kind, ValueKind::Deferred);
        assert_eq!(record, first);
        // The block (0), the store (1), the update of `a` (2), its record's
        // await (3); the update of `b` (3), its record's await (4). The
        // shared await, met again under the access, keeps the depth of the
        // path the walk first reaches it by, where it is first evaluated.
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![3, 4, 3]
        );
        // A record that is present is read as before, beside one that is
        // not.
        let text = "let r = { a = lazy { b = 1 } }\nlet r.a.b = 2";
        let shown = dump(text);
        assert!(
            shown.contains(
                "  (store r#1 @31..44\n\
                 \x20   (update-field a @35..38\n\
                 \x20     (load r#0 @35..36)\n\
                 \x20     (update-field b @35..40\n\
                 \x20       (await @35..38\n\
                 \x20         (field a @35..38\n\
                 \x20           (load r#0 @35..36)))\n\
                 \x20       (int 2 @43..44))))"
            ),
            "{shown}"
        );
    }

    /// The result of the root block of `ir`, whatever items come before it.
    fn result_after_items(ir: &Ir) -> IrId {
        match ir.get(ir.root()).kind {
            IrKind::Block { result, .. } => result,
            other => panic!("the root is not a block: {other:?}"),
        }
    }

    // ----- Lists -----

    #[test]
    fn a_list_literal_makes_a_list_of_its_elements() {
        assert_eq!(
            dump("[1, lazy 2, ...[3]]"),
            "(block @0..19\n\
             \x20 (make-list @0..19\n\
             \x20   (int 1 @1..2)\n\
             \x20   (make-future #0 @4..10)\n\
             \x20   (spread @12..18\n\
             \x20     (make-list @15..18\n\
             \x20       (int 3 @16..17)))))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 2 @9..10)"
        );
        let ir = lower_text("[1, 2]");
        assert!(matches!(
            ir.get(result_of(&ir)).kind,
            IrKind::MakeList { spreads: false, .. }
        ));
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::List);
        let ir = lower_text("[...[1]]");
        assert!(matches!(
            ir.get(result_of(&ir)).kind,
            IrKind::MakeList { spreads: true, .. }
        ));
        // An element awaited to fit its expected type is wrapped in its
        // await, and a spread's list that may be a future too.
        let shown = dump("let xs: List<Present<Int>> = [lazy 1]\nlet ys = lazy [2]\n[...ys]");
        assert!(
            shown.contains(
                "(make-list @29..37\n\
                 \x20     (await @30..36\n\
                 \x20       (make-future #0 @30..36)))"
            ),
            "{shown}"
        );
        assert!(
            shown.contains(
                "(spread @57..62\n\
                 \x20     (await @60..62\n\
                 \x20       (load ys#1 @60..62)))"
            ),
            "{shown}"
        );
    }

    #[test]
    fn an_index_reads_an_element_of_its_kind() {
        assert_eq!(
            dump("let xs = [1.5]\nxs[0] + 1.0"),
            "(block @0..26\n\
             \x20 (store xs#0 @0..14\n\
             \x20   (make-list @9..14\n\
             \x20     (float 1.5 @10..13)))\n\
             \x20 (float-add @15..26\n\
             \x20   (index @15..20\n\
             \x20     (load xs#0 @15..17)\n\
             \x20     (int 0 @18..19))\n\
             \x20   (float 1.0 @23..26)))"
        );
        // An element that may be a future is read as one and awaited where
        // it is used.
        let ir = lower_text("let xs = [lazy 1]\nxs[0]");
        let index = result_after_items(&ir);
        assert!(matches!(
            ir.get(index).kind,
            IrKind::Index {
                kind: ValueKind::Deferred,
                ..
            }
        ));
        let shown = dump("let xs = [1, lazy 2]\nxs[1] * 2");
        assert!(
            shown.contains(
                "(int-mul @21..30\n\
                 \x20   (await @21..26\n\
                 \x20     (index @21..26\n"
            ),
            "{shown}"
        );
    }

    #[test]
    fn len_lowers_to_the_length_of_its_list() {
        assert_eq!(
            dump("len(list = [1])"),
            "(block @0..15\n\
             \x20 (list-len @0..15\n\
             \x20   (make-list @11..14\n\
             \x20     (int 1 @12..13))))"
        );
        let ir = lower_text("len([1])");
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Int);
        // Its list is awaited if it may be a future.
        let shown = dump("let xs = lazy [1]\nlen(xs)");
        assert!(
            shown.contains("(list-len @18..25\n    (await @22..24\n"),
            "{shown}"
        );
    }

    #[test]
    fn list_equality_compares_over_a_list_shape() {
        assert_eq!(
            dump("[1] == [2]"),
            "(block @0..10\n\
             \x20 (list-eq == * @0..10\n\
             \x20   (make-list @0..3\n\
             \x20     (int 1 @1..2))\n\
             \x20   (make-list @7..10\n\
             \x20     (int 2 @8..9))))"
        );
        assert!(dump("[1] !== [2]").contains("(list-same !== @0..11\n"));
        assert!(dump("[1] === [2]").contains("(list-same === @0..11\n"));
        assert!(dump("[1] != [2]").contains("(list-eq != * @0..10\n"));
        // Through records' shapes, and records' fields through lists'.
        let text = "let q: {x: Int} = { x = 1; y = 2 }\n[q] == [q]";
        assert!(dump(text).contains("(list-eq == *[x] @"), "{}", dump(text));
        let text = "let q = { xs = [[{ a = 1 }]]; n = 1 }\nq == q";
        assert!(
            dump(text).contains("(record-eq == [xs**[a] n] @"),
            "{}",
            dump(text)
        );
        // One shape per type, nested ones included.
        let ir = lower_text("let q = [[{ a = 1 }]]\nlet a = q == q\n= q[0] == q[0]");
        assert_eq!(ir.shape_count(), 3);
        let ir = lower_text("[{ a = 1 }] == [{ a = 1 }]");
        let IrKind::ListEq(ShapedOperands { shape, .. }) = ir.get(result_of(&ir)).kind else {
            panic!("expected a list comparison");
        };
        let Some(ListShape {
            element: Some(element),
        }) = ir.list_shape(shape)
        else {
            panic!("expected a list's shape");
        };
        assert_eq!(ir.list_shape(element), None);
        assert_eq!(ir.shape(element).fields.len(), 1);
        assert!(ir.shape(shape).fields.is_empty());
    }

    #[test]
    fn a_deep_await_of_a_list_awaits_each_element() {
        let text = "let xs = [1, lazy 2]\n\"{xs}\"";
        let shown = dump(text);
        assert!(
            shown.contains("(await *! @23..25\n      (load xs#0 @23..25))"),
            "{shown}"
        );
        let ir = lower_text(text);
        let IrKind::Interp(items) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an interpolation");
        };
        let IrKind::Await { shape, kind, .. } = ir.get(ir.items(items)[1]).kind else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::List);
        assert_eq!(
            ir.lazy_list(shape),
            Some(LazyField::Await(LazyShapeId::NONE))
        );
        assert!(ir.lazy_shape(shape).fields.is_empty());
        assert_eq!(ir.lazy_list(LazyShapeId::NONE), None);
        // Each element's lazy values, and a record's lazy list.
        for (text, shape) in [
            ("let xs = [{ a = lazy 1 }]\n\"{xs}\"", "*[a!]"),
            ("let xs = [[1, lazy 2]]\n\"{xs}\"", "**!"),
            ("let xs = [lazy [1, lazy 2]]\n\"{xs}\"", "*!*!"),
            ("let r = { xs = [lazy 1]; n = 2 }\n\"{r}\"", "[xs*!]"),
            (
                "let xs = [{ a = lazy [{ b = lazy 1 }] }]\n\"{xs}\"",
                "*[a!*[b!]]",
            ),
        ] {
            let shown = dump(text);
            assert!(
                shown.contains(&format!("(await {shape} @")),
                "{text}: {shown}"
            );
        }
        // A list with nothing lazy inside is not awaited.
        assert!(!dump("let xs = [[1]]\n\"{xs}\"").contains("await"));
    }

    // ----- Spreads -----

    #[test]
    fn a_spread_stores_its_operand_once_and_each_field_by_name() {
        // The operand is held in the item's own slot, which no variable
        // owns, and each field of its static type, in the type's order, is
        // read from it into the field's slot. An override stores into the
        // same slot, and the field keeps its place in the record, loaded as
        // the definition that stands.
        let text = "let r = { a = 1; b = 2 }\n...r\na = \"s\"";
        assert_eq!(
            dump(text),
            "(block @0..37\n\
             \x20 (store r#0 @0..24\n\
             \x20   (block @8..24\n\
             \x20     (store r#0 @10..15\n\
             \x20       (int 1 @14..15))\n\
             \x20     (store a#1 @17..22\n\
             \x20       (int 2 @21..22))\n\
             \x20     (make-record [a b] @8..24\n\
             \x20       (load r#0 @10..11)\n\
             \x20       (load a#1 @17..18))))\n\
             \x20 (store ?#3 @25..29\n\
             \x20   (load r#0 @28..29))\n\
             \x20 (store a#1 @25..29\n\
             \x20   (field a @28..29\n\
             \x20     (load ?#3 @28..29)))\n\
             \x20 (store b#2 @25..29\n\
             \x20   (field b @28..29\n\
             \x20     (load ?#3 @28..29)))\n\
             \x20 (store a#1 @30..37\n\
             \x20   (str \"s\" @34..37))\n\
             \x20 (make-record [a b] @0..37\n\
             \x20   (load a#1 @30..31)\n\
             \x20   (load b#2 @28..29)))"
        );
        let ir = lower_text(text);
        let IrKind::MakeRecord { values, .. } = ir.get(result_after_items(&ir)).kind else {
            panic!("expected a record");
        };
        let loads: Vec<IrKind> = ir.items(values).iter().map(|&id| ir.get(id).kind).collect();
        assert_eq!(
            loads,
            vec![
                IrKind::Load(Slot::new(1), ValueKind::String),
                IrKind::Load(Slot::new(2), ValueKind::Int)
            ]
        );
        // A later spread overrides a soft field in its slot too.
        let ir = lower_text("{ ...{ a = 1; b = 2 }; ...{ b = \"s\"; c = 3 } }");
        let IrKind::Block { result, .. } = ir.get(result_after_items(&ir)).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeRecord { layout, values } = ir.get(result).kind else {
            panic!("expected a record");
        };
        let names: Vec<&str> = ir.layout(layout).texts.iter().map(AsRef::as_ref).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        let loads: Vec<IrKind> = ir.items(values).iter().map(|&id| ir.get(id).kind).collect();
        assert_eq!(
            loads,
            vec![
                IrKind::Load(Slot::new(0), ValueKind::Int),
                IrKind::Load(Slot::new(1), ValueKind::String),
                IrKind::Load(Slot::new(2), ValueKind::Int)
            ]
        );
    }

    #[test]
    fn a_spreads_operand_that_may_be_a_future_is_awaited_once() {
        assert_eq!(
            dump("let f = lazy { a = 1 }\n...f")
                .lines()
                .collect::<Vec<_>>()[..11],
            [
                "(block @0..27",
                "  (store f#0 @0..22",
                "    (make-future #0 @8..22))",
                "  (store ?#2 @23..27",
                "    (await @26..27",
                "      (load f#0 @26..27)))",
                "  (store a#1 @23..27",
                "    (field a @26..27",
                "      (load ?#2 @26..27)))",
                "  (make-record [a] @0..27",
                "    (load a#1 @26..27)))",
            ]
        );
        // A field that may be a future is read as it is, not awaited.
        let ir = lower_text("...{ a = lazy 1 }");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        let IrKind::Store(_, field) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert!(matches!(
            ir.get(field).kind,
            IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
        ));
    }
}
