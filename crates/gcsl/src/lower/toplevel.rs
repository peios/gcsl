//! Lowering a file's body and its top level: the cells of its fields and
//! functions, the records it stores, and its assembly.

use crate::ast::{ExprId, ExprKind, FnId, ItemId, ItemKind, ItemRange};
use crate::check::{FieldSchedule, MadeAt, Slot};
use crate::diagnostic::InternalError;
use crate::ir::{FnCode, FnKind, IrId, IrKind};
use crate::mentions::{self, FieldNode};
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;
use crate::value::ValueKind;

use super::{Lowerer, unparsed};

impl Lowerer<'_> {
    /// Lowers a body whose value is of kind `kind` to a `Block`, which the
    /// caller allocates: the body of the block `block`, or the root's for
    /// `None`. A `{}` the checker gave fields their defaults (see the
    /// checker's "Field defaults") is a record of them, as a record block
    /// given them is.
    pub(super) fn lower_body(
        &mut self,
        (block, body): (Option<ExprId>, ItemRange),
        span: Span,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let items = self.ast.items(body);
        let is_record = self.is_record_body(body)
            || block.is_some_and(|block| !self.resolution.record_fills(block).is_empty());
        let base = self.item_stack.len();
        let mut result = None;
        let is_root = body == self.ast.root();
        if is_root {
            self.store_imported_records(span);
            self.store_top_closures(None)?;
        }
        for (index, &id) in items.iter().enumerate() {
            // A record block's final expression is a statement.
            let is_result = !is_record && index + 1 == items.len();
            let item = self.ast.item(id);
            if is_root {
                self.store_cells(index)?;
            }
            // A shadowed declaration is never run (see
            // [`crate::mentions`], "Shadowed declarations").
            if is_root && self.is_shadowed(index) {
                continue;
            }
            if is_root && self.is_stored_elsewhere(id) {
                self.store_top_closures(Some(id))?;
                continue;
            }
            if is_root && self.is_cell_item(index) {
                // Its leaf was stored at its point, at this item or before.
                self.store_top_closures(Some(id))?;
                continue;
            }
            match item.kind {
                // A definition made with a def-type, and a definition's
                // meta, is a top-level function, whose closure is stored
                // into its leaf's cell here if it is made here.
                ItemKind::Let { .. }
                | ItemKind::Rebind { .. }
                | ItemKind::Field { .. }
                | ItemKind::DefTypeDef { .. }
                | ItemKind::DefMeta { .. } => {
                    let store = self.lower_store(id)?;
                    self.item_stack.push(store);
                }
                ItemKind::Spread(value) => self.lower_spread(id, value, item.span)?,
                // An import computes nothing at its point, nor a type,
                // which has no existence at runtime.
                ItemKind::Import(_)
                | ItemKind::Type { .. }
                | ItemKind::Enum { .. }
                | ItemKind::Variant { .. }
                | ItemKind::Context { .. } => {}
                ItemKind::Error => return Err(unparsed(item.span)),
                ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
                    let node = self.lower_jump(id, jump)?;
                    if is_result {
                        result = Some(node);
                    } else {
                        self.item_stack.push(node);
                    }
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
                    let statement = self.lower_statement(value)?;
                    self.item_stack.push(statement);
                }
            }
            if is_root {
                self.store_top_closures(Some(id))?;
            }
        }
        if is_root {
            self.store_cells(items.len())?;
            self.store_own_records()?;
        }
        let mut result = match result {
            Some(result) => result,
            None if kind != ValueKind::Record => {
                return Err(InternalError {
                    message: "a block without a result is not a record",
                    span,
                });
            }
            None if is_record => self.make_record((block, items), is_root, span)?,
            None => self.builder.alloc(IrKind::Empty, span),
        };
        // An imported value file's value is a cell: stored as the block
        // ends, the last item of the top level (see [Imports](crate::check::imports#imports)).
        let value_cell = self.resolution.value_cell();
        if is_root && value_cell != Slot::NONE {
            let store = self.builder.alloc(IrKind::Store(value_cell, result), span);
            self.item_stack.push(store);
            result = self.builder.alloc(IrKind::Load(value_cell, kind), span);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Block { items, result })
    }

    /// Gives each function expression and `lazy` of the root block's
    /// shadowed declarations, which lower to nothing and so are never made
    /// (see [`crate::mentions`], "Shadowed declarations"), code that does
    /// nothing, at `span`: the function table has an entry for every one.
    pub(super) fn stub_shadowed_functions(&mut self, span: Span) {
        let ast = self.ast;
        let root = ast.items(ast.root());
        for (position, &id) in root.iter().enumerate() {
            let (ItemKind::Field { value, .. } | ItemKind::DefTypeDef { value, .. }) =
                ast.item(id).kind
            else {
                continue;
            };
            if !self.is_shadowed(position) {
                continue;
            }
            for expr in ast.subtree(value) {
                let (function, kind) = match ast.get(expr).kind {
                    ExprKind::Fn(function) => (function, self.fn_kind(expr, function)),
                    ExprKind::Lazy(function) => (function, FnKind::Future),
                    _ => continue,
                };
                // A function's default functions have no node of their own
                // (see the checker's "Calls").
                let defaults = ast.params(ast.fn_decl(function).params);
                let defaults = defaults.iter().filter_map(|param| param.default_fn);
                let defaults: Vec<(FnId, FnKind)> = defaults
                    .map(|default| (default, FnKind::Function))
                    .collect();
                for (function, kind) in std::iter::once((function, kind)).chain(defaults) {
                    self.stub_function(function, kind, span);
                }
            }
        }
    }

    /// Enters function `function`, of kind `kind`, into the function table
    /// as code that does nothing, at `span`, unless it has an entry.
    pub(super) fn stub_function(&mut self, function: FnId, kind: FnKind, span: Span) {
        let ast = self.ast;
        let params =
            u32::try_from(ast.params(ast.fn_decl(function).params).len()).unwrap_or(u32::MAX);
        let body = self.builder.alloc(IrKind::Empty, span);
        if let Some(entry @ None) = self.functions.get_mut(function.index()) {
            *entry = Some(FnCode {
                body,
                kind,
                frame_size: params,
                param_count: params,
                capture_count: 0,
                height: 1,
                name: None,
                display: Box::from(""),
                slot_names: Box::from([]),
                attributes: Box::default(),
                defers: false,
                retry: 0,
                timeout: None,
                name_text: None,
            });
        }
    }

    /// Whether the item at `position` of the root block is a declaration a
    /// later one shadows, which lowers to nothing and is in no record.
    pub(super) fn is_shadowed(&self, position: usize) -> bool {
        self.resolution
            .field_schedule()
            .tree
            .shadowed
            .get(position)
            .copied()
            .unwrap_or(false)
    }

    /// Whether the root item `id` is a top-level function whose closure is
    /// made somewhere other than at its own declaration (see
    /// [`MadeAt`]): it then lowers to nothing where it is written, the
    /// `Store` of its closure being placed by
    /// [`Lowerer::store_top_closures`]. A `def`'s field is still where it
    /// is written in its record, which reads the slot when it is made.
    pub(super) fn is_stored_elsewhere(&self, id: ItemId) -> bool {
        self.resolution
            .top_functions()
            .iter()
            .any(|&(item, made)| item == id && made != MadeAt::Own)
    }

    /// Pushes, as items of the root block, the `Store`s of the closures of
    /// the top-level functions made at `at`, in source order: those made
    /// as the input starts for `None`, and those made just after the root
    /// item `item` for `Some(item)` (see [`MadeAt`]).
    pub(super) fn store_top_closures(&mut self, at: Option<ItemId>) -> Result<(), InternalError> {
        let resolution = self.resolution;
        for &(item, made) in resolution.top_functions() {
            let here = match (made, at) {
                (MadeAt::Start, None) => true,
                (MadeAt::After(after), Some(at)) => after == at,
                _ => false,
            };
            if here {
                let store = self.lower_store(item)?;
                self.item_stack.push(store);
            }
        }
        Ok(())
    }

    /// Whether the item at `position` of the root block is a top-level
    /// field item whose leaf has a cell of its own: it then lowers to
    /// nothing where it is written, its `Store` being placed at its point by
    /// [`Lowerer::store_cells`]. A `def` is not one: its cell is its
    /// function's slot, stored as a top-level function's is.
    pub(super) fn is_cell_item(&self, position: usize) -> bool {
        let tree = &self.resolution.field_schedule().tree;
        tree.item_leaf
            .get(position)
            .copied()
            .flatten()
            .and_then(|node| tree.nodes.get(node as usize))
            .and_then(|node| node.leaf)
            .is_some_and(|leaf| mentions::top_level_function(self.ast, leaf.item).is_none())
    }

    /// Pushes, as items of the root block, the `Store`s of the cells of the
    /// top-level field nodes whose point is at or before the item at
    /// `point` (the number of items for after the last), in the order the
    /// schedule computes them, each once (see [Cells](crate::ir#cells)): a
    /// leaf's initialiser, or the record of an interior node's children,
    /// loaded from their cells in the order of their first items; for a
    /// node of a workspace file's view, the record of its children in the
    /// view's order, if the file reads it whole.
    pub(super) fn store_cells(&mut self, point: usize) -> Result<(), InternalError> {
        let resolution = self.resolution;
        let FieldSchedule { tree, schedule } = resolution.field_schedule();
        let root = self.ast.items(self.ast.root());
        while let Some(&index) = schedule.order.get(self.next_cell) {
            let at = schedule
                .point
                .get(index as usize)
                .copied()
                .unwrap_or(usize::MAX);
            if at > point {
                break;
            }
            self.next_cell += 1;
            if let Some((slot, first, fields)) = resolution.view_record(index) {
                if *slot != Slot::NONE {
                    let span = root
                        .get(*first)
                        .map_or_else(|| self.ast.root_span(), |&item| self.ast.item(item).span);
                    self.store_record(*slot, fields, span);
                }
                continue;
            }
            let slot = resolution.field_cells().get(index as usize).copied();
            let (Some(node), Some(slot)) = (tree.nodes.get(index as usize), slot) else {
                continue;
            };
            if slot == Slot::NONE {
                continue;
            }
            let value = match node.leaf {
                Some(leaf) if mentions::top_level_function(self.ast, leaf.item).is_some() => {
                    continue;
                }
                Some(leaf) => self.lower_expr(leaf.value)?,
                None => self.assemble(node, index, root, false)?,
            };
            let span = self.node_span(node, root);
            let store = self.builder.alloc(IrKind::Store(slot, value), span);
            self.item_stack.push(store);
        }
        Ok(())
    }

    /// In a file of a workspace, before its first item: stores each record
    /// it reads whole through an import into its private slot, nested
    /// records first, each a `MakeRecord` of `Load`s of the fields' cells,
    /// which the files that own them, run before this one, have written (a
    /// `def`'s cell being its function's slot; see
    /// [Imports](crate::check::imports#imports)).
    pub(super) fn store_imported_records(&mut self, span: Span) {
        let resolution = self.resolution;
        for (slot, fields) in resolution.imported_records() {
            self.store_record(*slot, fields, span);
        }
    }

    /// Pushes, as an item of the root block, the `Store` into `slot` of
    /// the `MakeRecord` of `fields`, a `Load` of each one's cell at its
    /// name, of the kind of its type, shown at `span`: a record a file of a
    /// workspace assembles of other files' cells, and of its own computed
    /// before it (see [Imports](crate::check::imports#imports)).
    pub(super) fn store_record(&mut self, slot: Slot, fields: &[(Name, Slot, TyId)], span: Span) {
        self.layout_names.clear();
        self.record_loads.clear();
        for &(name, cell, ty) in fields {
            let kind = self.table.value_kind(ty).unwrap_or(ValueKind::Record);
            self.define_field(name, (cell, kind, span), false);
        }
        let base = self.item_stack.len();
        for &(cell, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(cell, kind), field_span);
            self.item_stack.push(load);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let record = self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span);
        let store = self.builder.alloc(IrKind::Store(slot, record), span);
        self.item_stack.push(store);
    }

    /// In a file of a workspace, after its last item: stores, children
    /// first, the file's own record of each node it shares with other
    /// files, which its value holds in place of the shared node's cell,
    /// whose value has the other files' parts (see [Directory
    /// scope](crate::check::imports#directory-scope)).
    pub(super) fn store_own_records(&mut self) -> Result<(), InternalError> {
        let resolution = self.resolution;
        let tree = &resolution.field_schedule().tree;
        let root = self.ast.items(self.ast.root());
        for (index, &slot) in resolution.own_records().iter().enumerate().rev() {
            let Some(node) = tree.nodes.get(index).filter(|_| slot != Slot::NONE) else {
                continue;
            };
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            let value = self.assemble(node, index, root, true)?;
            let span = self.node_span(node, root);
            let store = self.builder.alloc(IrKind::Store(slot, value), span);
            self.item_stack.push(store);
        }
        Ok(())
    }

    /// The `MakeRecord` of the interior node `node` of the tree of the
    /// input's top-level fields, whose root block's items are `root`: a
    /// `Load` of each child's cell, at its name, in the order of their
    /// first items, a leaf's of the kind of its initialiser and an interior
    /// node's as a record. The node is `index` of the tree. In a file of a
    /// workspace, a node shared with other files has their children too,
    /// after its own, unless `own`, which builds the file's own record of
    /// it, from its own children's own records (see [Directory
    /// scope](crate::check::imports#directory-scope)).
    pub(super) fn assemble(
        &mut self,
        node: &FieldNode,
        index: u32,
        root: &[ItemId],
        own: bool,
    ) -> Result<IrId, InternalError> {
        let resolution = self.resolution;
        let tree = &resolution.field_schedule().tree;
        self.layout_names.clear();
        self.record_loads.clear();
        let shared = resolution.shared_children(index);
        let merged = !own && !shared.is_empty();
        // A shared node's children of other files, in the merged node's
        // order, its own among them.
        if merged {
            let span = self.node_span(node, root);
            for &(name, slot, ty) in shared {
                let leaf = node
                    .children
                    .iter()
                    .filter_map(|&child| tree.nodes.get(child as usize))
                    .find(|child| child.name == name)
                    .map(|child| child.leaf);
                let kind = match leaf {
                    Some(Some(leaf)) => self.kind(leaf.value)?,
                    Some(None) => ValueKind::Record,
                    None => self.table.value_kind(ty).unwrap_or(ValueKind::Record),
                };
                self.define_field(name, (slot, kind, span), false);
            }
        }
        for &child in node.children.iter().filter(|_| !merged) {
            let (Some(child_node), Some(&slot)) = (
                tree.nodes.get(child as usize),
                resolution.field_cells().get(child as usize),
            ) else {
                continue;
            };
            let kind = match child_node.leaf {
                Some(leaf) => self.kind(leaf.value)?,
                None => ValueKind::Record,
            };
            let slot = match resolution.own_records().get(child as usize) {
                Some(&own_slot) if own && own_slot != Slot::NONE => own_slot,
                _ => slot,
            };
            let span = self.node_span(child_node, root);
            self.define_field(child_node.name, (slot, kind, span), false);
        }
        let base = self.item_stack.len();
        for &(slot, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(slot, kind), field_span);
            self.item_stack.push(load);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let span = self.node_span(node, root);
        Ok(self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span))
    }
}
