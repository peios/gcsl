//! Lowering records and lists: record blocks, spreads, field reads, metas, file
//! roots and list literals.

use crate::ast::{ElementRange, ExprId, ExprKind, ItemId, ItemKind};
use crate::check::{AwaitMark, Fileroot, Slot, VarRef};
use crate::diagnostic::InternalError;
use crate::ir::{IrId, IrKind, LazyShapeId};
use crate::mentions;
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;
use crate::value::ValueKind;

use super::{Lowerer, RootField, read};

impl Lowerer<'_> {
    /// Lowers the spread item `id`, whose operand is `value`, spanning `span`,
    /// onto the item stack: a `Store` of the operand, awaited if the checker
    /// marked it to be, into the item's own slot, so that it is evaluated once,
    /// and then, for each field of the operand's static type once awaited, in
    /// the type's order, a `Store` into the field's slot (see
    /// [`Resolution::spread_slots`](super::Resolution::spread_slots)) of a
    /// `Field` of a `Load` of the item's slot, which reads the field by name. A
    /// field the runtime value holds but the static type hides is not read.
    pub(super) fn lower_spread(
        &mut self,
        id: ItemId,
        value: ExprId,
        span: Span,
    ) -> Result<(), InternalError> {
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
        // An operand of type `Never` never gives a record to read.
        if self.kind(value)? == ValueKind::Never {
            return Ok(());
        }
        let operand_span = self.ast.get(value).span;
        let fields = table
            .record_fields(table.strip(self.used_type(value)))
            .ok_or(InternalError {
                message: "a spread of a value that is not a record survived checking",
                span: operand_span,
            })?;
        let slots = resolution.spread_slots(id);
        if slots.len() != fields.len() {
            return Err(InternalError {
                message: "a spread's fields have no slots",
                span,
            });
        }
        // A field whose name is a top-level field of the input has no slot:
        // the spread writes nothing for it, whichever is written first.
        for (&(name, ty), &slot) in fields.iter().zip(slots) {
            if slot == Slot::NONE {
                continue;
            }
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
    ///
    /// In the root block (`is_root`), a top-level field is loaded from the
    /// cell of its root (see [Cells](crate::ir#cells)), at the first item
    /// under that root, and a spread's field that a top-level declaration
    /// of its name overrides, below it or above, takes the declaration's
    /// load.
    ///
    /// The fields the checker gave the block, `block` (`None` for the root,
    /// which is given none), their defaults (see the checker's "Field
    /// defaults", [`crate::check::Resolution::record_fills`]) follow those
    /// it defines, in the order of their names' text, each its constant's
    /// nodes (`Lowerer::lower_constant`) at the block's closing brace; a
    /// `{}` given fields has only those.
    pub(super) fn make_record(
        &mut self,
        (block, items): (Option<ExprId>, &[ItemId]),
        is_root: bool,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let (table, resolution) = (self.table, self.resolution);
        self.layout_names.clear();
        self.record_loads.clear();
        let mut spread = false;
        for (position, &id) in items.iter().enumerate() {
            if is_root && self.is_shadowed(position) {
                continue;
            }
            let (path, value) = match self.ast.item(id).kind {
                ItemKind::Field { path, value, .. } => (path, value),
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
                        // A top-level declaration of the name stands
                        // wherever it is: the spread wrote no slot for it.
                        if slot == Slot::NONE && self.layout_names.contains(&name) {
                            continue;
                        }
                        self.define_field(name, (slot, kind, operand_span), spread);
                    }
                    continue;
                }
                _ => continue,
            };
            // A `def` of one name is its own root, whose slot is its
            // function's; one named by a dotted path is a leaf under its
            // root, as any dotted field is.
            if is_root
                && mentions::top_level_function(self.ast, id)
                    .is_none_or(|(name, _, _)| name.is_none())
            {
                match self.root_field(id)? {
                    RootField::Other => {}
                    RootField::Extends => continue,
                    RootField::First(name, load) => {
                        self.define_field(name, load, spread);
                        continue;
                    }
                }
            }
            let (kind, field) = match self.ast.path(path) {
                [field] => (self.kind(value)?, field),
                // A later definition of an open field extends it.
                [field, ..] if resolution.item_path(id).source == VarRef::None => {
                    (ValueKind::Record, field)
                }
                _ => continue,
            };
            // A field's path's segments are all names.
            let Some(name) = field.name() else {
                continue;
            };
            let slot = resolution.item_slot(id);
            self.define_field(name, (slot, kind, field.span), spread);
        }
        let base = self.item_stack.len();
        for &(slot, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(slot, kind), field_span);
            self.item_stack.push(load);
        }
        let close = Span::new(span.end.saturating_sub(1), span.end);
        let fills = block.map_or(&[][..], |block| resolution.record_fills(block));
        for (name, constant) in fills {
            let value = self.lower_constant(constant, close);
            self.item_stack.push(value);
            self.layout_names.push(*name);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span))
    }

    /// The field of the record of the input that the root item `id` defines,
    /// as a [`RootField`].
    pub(super) fn root_field(&self, id: ItemId) -> Result<RootField, InternalError> {
        let resolution = self.resolution;
        let tree = &resolution.field_schedule().tree;
        let root = self.ast.items(self.ast.root());
        let Some(position) = root.iter().position(|&item| item == id) else {
            return Ok(RootField::Other);
        };
        let Some(mut at) = tree.item_leaf.get(position).copied().flatten() else {
            return Ok(RootField::Other);
        };
        let leaf = at;
        while let Some(parent) = tree.nodes.get(at as usize).and_then(|node| node.parent) {
            at = parent;
        }
        let (Some(node), Some(&slot)) = (
            tree.nodes.get(at as usize),
            resolution.field_cells().get(at as usize),
        ) else {
            return Ok(RootField::Other);
        };
        if node.first != position {
            return Ok(RootField::Extends);
        }
        // A root shared with other files of a workspace: the file's own
        // record of it.
        let slot = match resolution.own_records().get(at as usize) {
            Some(&own) if own != Slot::NONE => own,
            _ => slot,
        };
        let kind = match (leaf == at, node.leaf) {
            (true, Some(leaf)) => self.kind(leaf.value)?,
            _ => ValueKind::Record,
        };
        // Shown at the root's name in its first item, as a field's load is.
        let span = root
            .get(node.first)
            .and_then(|&first| match self.ast.item(first).kind {
                ItemKind::Field { path, .. } => self.ast.path(path).first().map(|s| s.span),
                _ => None,
            })
            .unwrap_or_else(|| self.node_span(node, root));
        Ok(RootField::First(node.name, (slot, kind, span)))
    }

    /// Adds the field `name` to the record being built by
    /// [`Lowerer::make_record`], to be loaded as `load` says: its slot, the
    /// kind of its value and the span of its definition. If `redefinable`,
    /// the block has had a spread, and a field of the name already there is
    /// overridden instead, keeping its place.
    pub(super) fn define_field(
        &mut self,
        name: Name,
        load: (Slot, ValueKind, Span),
        redefinable: bool,
    ) {
        if redefinable && let Some(at) = self.layout_names.iter().position(|&known| known == name) {
            if let Some(entry) = self.record_loads.get_mut(at) {
                *entry = load;
            }
            return;
        }
        self.layout_names.push(name);
        self.record_loads.push(load);
    }

    /// Lowers the field access `id`, `record.name`, of kind `kind`: a read
    /// of a cell if the checker resolved it to a node of the tree of the
    /// input's top-level fields (see [Cells](crate::ir#cells)), and an
    /// ordinary `Field` of the lowered record otherwise.
    pub(super) fn lower_field_access(
        &mut self,
        id: ExprId,
        (record, name): (ExprId, Name),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        if let VarRef::Fileroot(member) = self.resolution.expr_var(id) {
            return self.lower_fileroot(member, kind, self.ast.get(id).span);
        }
        if let Some(meta) = self.resolution.meta_read(id) {
            return self.lower_meta_read(id, (record, name), meta, kind);
        }
        if self.resolution.file_path(id) {
            return Ok(IrKind::FilePath(
                self.lower_operand(record, ValueKind::Mixed)?,
            ));
        }
        Ok(match read(self.resolution.expr_var(id), kind) {
            Some(cell) => cell,
            None => IrKind::Field {
                record: self.lower_operand(record, ValueKind::Record)?,
                name,
                kind,
            },
        })
    }

    /// Lowers the field access `id`, `func.name`, of kind `kind`, which the
    /// checker found reads the meta of `func`, of type `meta_ty`, awaited as
    /// `mark` says (see the checker's "Metas"): to a `Field` of the call of
    /// no arguments of the `Meta` of `func`, which yields its meta's closure,
    /// awaited as marked, so that reading a meta is calling its closure, the
    /// caller, the memoisation of a `META->>` meta and the nesting budget
    /// all as for any call. Apart from `lower_plain`, which is on the
    /// recursion path of every expression.
    pub(super) fn lower_meta_read(
        &mut self,
        id: ExprId,
        (func, name): (ExprId, Name),
        (meta_ty, mark): (TyId, AwaitMark),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        let mistyped = InternalError {
            message: "a meta typed as something no value has",
            span,
        };
        let function = self.lower_operand(func, ValueKind::Fn)?;
        let meta = self.builder.alloc(IrKind::Meta(function), span);
        let args = self.builder.push_item_list(&[]);
        let call = self.builder.alloc(
            IrKind::Call {
                callee: meta,
                args,
                kind: self.table.value_kind(meta_ty).ok_or(mistyped)?,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        );
        let record = match mark {
            AwaitMark::None => call,
            AwaitMark::Top | AwaitMark::Deep | AwaitMark::Toward(_) => {
                let present = self.table.strip(meta_ty);
                self.builder.alloc(
                    IrKind::Await {
                        value: call,
                        kind: self.table.value_kind(present).ok_or(mistyped)?,
                        shape: LazyShapeId::NONE,
                        // Set by `IrBuilder::measure` once every node is built.
                        depth: 0,
                    },
                    span,
                )
            }
        };
        Ok(IrKind::Field { record, name, kind })
    }

    /// Lowers the read of `member` of the `fileroot` module spanning `span`,
    /// of kind `kind`, a `String`'s (see the checker's "Imports"):
    /// `fileroot.this` and `fileroot.workspace` to a `StrConst` of the
    /// file's place, which the checker resolved, constants of the file and
    /// of the run; `fileroot.caller` to a `Caller`, read where it runs. Any
    /// other is an internal error: `helper` is only ever called, and lowered
    /// as its call ([`Lowerer::lower_use_or_function`]).
    pub(super) fn lower_fileroot(
        &mut self,
        member: Fileroot,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let place = self.resolution.place();
        match (member, kind) {
            (Fileroot::This, ValueKind::String) => {
                Ok(IrKind::StrConst(self.builder.push_string(&place.this)))
            }
            (Fileroot::Workspace, ValueKind::String) => {
                Ok(IrKind::StrConst(self.builder.push_string(&place.workspace)))
            }
            (Fileroot::Caller, ValueKind::String) => {
                self.read_caller();
                Ok(IrKind::Caller)
            }
            (Fileroot::Helper, _) => Err(InternalError {
                message: "`fileroot.helper` read as a value",
                span,
            }),
            _ => Err(InternalError {
                message: "a member of `fileroot` typed as something other than a String",
                span,
            }),
        }
    }

    /// Lowers the list literal or `for`, as `expr` is, of kind `kind`,
    /// spanning `span`: an internal error unless `kind` is a list's, but
    /// for a loop that cannot end, or that a `return` leaves, which may be
    /// of any kind (see [`Lowerer::lower_for`]). Apart from `lower_plain`,
    /// which is on the recursion path of every expression.
    pub(super) fn lower_listing(
        &mut self,
        expr: ExprKind,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        match (expr, kind) {
            (ExprKind::List(elements), ValueKind::List) => self.lower_list(elements),
            (ExprKind::For(for_id), ValueKind::List) => self.lower_for(for_id, (kind, true), span),
            (ExprKind::For(for_id), _)
                if self.resolution.loop_of(for_id).endless
                    || self.resolution.loop_of(for_id).returned =>
            {
                self.lower_for(for_id, (kind, true), span)
            }
            _ => Err(InternalError {
                message: "a list literal or a `for` typed as something other than a list",
                span,
            }),
        }
    }

    /// Lowers the elements `elements` of a list literal to the items of a
    /// `MakeList`, in source order: each element's expression, as it is
    /// used, and a `Spread` of each spread's list.
    pub(super) fn lower_list(&mut self, elements: ElementRange) -> Result<IrKind, InternalError> {
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
}
