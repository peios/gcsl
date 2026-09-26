//! The type checker and name resolver.
//!
//! One pass over the tree both types every expression and resolves every
//! variable. Its results are side tables indexed by the AST's ids:
//! [`Types`], the type of every expression, and [`Resolution`], the storage
//! slot of every variable use and declaration. Both are returned together as
//! [`Checked`].
//!
//! # Types
//!
//! `Int` and `Float` never mix implicitly. The one exception is that integer
//! literals adapt to their context: a literal starts as
//! [`Ty::UntypedInt`], and an operator that needs a concrete type for it
//! decides one. The checker works in two directions:
//!
//! - **Up**: [`Checker::check_expr`] computes each node's type from its
//!   children's. Operators whose operands are all untyped produce
//!   `UntypedInt` themselves where the rules allow, so `1 + 2` stays untyped,
//!   and so does a block whose result is untyped.
//! - **Down**: when an untyped subtree meets a concrete type, as in
//!   `(1 + 2) * 3.0` or `let x: Float = 5`, [`Checker::coerce`] walks the
//!   untyped subtree and rewrites every entry in it to the decided type. It
//!   stops at nodes that are already concrete, so each node is rewritten at
//!   most once and the whole check stays linear.
//!
//! A variable must have a concrete type, so an untyped initialiser defaults
//! to `Int` where it is bound: after `let x = 5`, `x` is an `Int`, and
//! `x + 2.5` is an error. Finalisation defaults an untyped file result to
//! `Int` in the same way. Every untyped node is either coerced by the
//! nearest concrete context or defaulted where it is bound or used as a
//! result, so afterwards the table contains no `UntypedInt`.
//!
//! A node whose operator rejects its operands is reported once and given
//! [`Ty::Error`]. Every operator accepts `Error` operands silently and
//! produces `Error` itself, so the mistake does not cascade into a spurious
//! error at each enclosing operator. A variable whose initialiser has an
//! error is still declared, with type `Error`, and a use of an undefined
//! variable has type `Error`, so neither cascades either. Independent
//! mistakes elsewhere in the tree are still reported, so one run shows every
//! real error.
//!
//! # Scopes
//!
//! Every block, including the file's own body, is a scope. A `let` declares
//! a variable visible from the next item to the end of its block; a later
//! `let` of the same name shadows it, in the same block or an inner one. A
//! `:=` is a new declaration too, allowed only for a name already declared
//! in the same block.
//!
//! The scopes in effect form a stack, kept as one flat vector of bindings
//! with the innermost block's bindings last: entering a block records where
//! its bindings start, and leaving it truncates the vector back. Lookup
//! searches the vector from the end, which visits the innermost block first
//! and, within a block, the latest declaration first, so shadowing needs no
//! extra work. Blocks are short, so a linear search beats a hash map, and
//! the one vector means entering a block allocates nothing.
//!
//! # Slots
//!
//! Every variable is stored in a [`Slot`] of one flat frame, assigned
//! stack-fashion, as in Lua and clox: entering a block records the next free
//! slot; each declaration (`let`, and `:=`, which is a new binding) takes
//! the next free slot; leaving the block restores the next free slot to what
//! it was on entry, so the slots of a finished block are reused by whatever
//! comes after it. The highest slot count reached is the frame size. A
//! variable's slot is therefore known statically and never changes, and
//! evaluation needs no environment lookups and no allocation per block.
//!
//! # Sessions
//!
//! A [`Checker`] can start from an existing outermost [`Scope`], which it
//! returns extended by the new input's top-level declarations. This is what
//! lets a [`crate::session::Session`] check each input in the context of the
//! variables its earlier inputs declared, with the same code that checks a
//! file.

use std::fmt;

use crate::ast::{
    Annotate, Ast, BinaryOp, ExprId, ExprKind, ItemId, ItemKind, ItemRange, TypeExprId,
    TypeExprKind, UnaryOp,
};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::names::Name;
use crate::source::Span;
use crate::types::Ty;

/// The type of every expression in an [`Ast`], indexed by [`ExprId`].
///
/// A `Types` returned by [`check`] contains neither [`Ty::UntypedInt`] nor
/// [`Ty::Error`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    types: Vec<Ty>,
}

impl Types {
    /// Wraps a raw table. Only the checker (and tests) build tables.
    pub(crate) fn new(types: Vec<Ty>) -> Self {
        Self { types }
    }

    /// The type of expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn get(&self, id: ExprId) -> Ty {
        self.types[id.index()]
    }

    /// Number of entries, equal to the number of nodes in the [`Ast`].
    pub(crate) fn len(&self) -> usize {
        self.types.len()
    }
}

/// Writes ` : Type` after each node, for `--dump-types`.
impl Annotate for Types {
    fn annotate(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, " : {}", self.get(id))
    }
}

/// The index of a variable's storage in the evaluator's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot(u32);

impl Slot {
    /// The absence of a slot, in [`Resolution`] entries for nodes that are
    /// not variables.
    pub const NONE: Slot = Slot(u32::MAX);

    /// The slot numbered `number`. Slots are normally assigned by the
    /// checker; this is for building IR by hand in tests.
    #[cfg(test)]
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The slot's number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The slot as an index into a frame.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Where every variable of an [`Ast`] is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The slot each `Name` expression reads, indexed by [`ExprId`];
    /// [`Slot::NONE`] for every other expression.
    expr_slots: Vec<Slot>,
    /// The slot each `Let` and `Rebind` item writes, indexed by [`ItemId`];
    /// [`Slot::NONE`] for every other item.
    item_slots: Vec<Slot>,
    /// The number of slots the frame needs.
    frame_size: u32,
    /// For each slot, the name of the last variable assigned to it.
    slot_names: Vec<Name>,
}

impl Resolution {
    /// The slot read by expression `id`, or [`Slot::NONE`] if it is not a
    /// variable use.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn expr_slot(&self, id: ExprId) -> Slot {
        self.expr_slots[id.index()]
    }

    /// The slot written by item `id`, or [`Slot::NONE`] if it is not a
    /// declaration.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn item_slot(&self, id: ItemId) -> Slot {
        self.item_slots[id.index()]
    }

    /// The number of slots the frame needs: one more than the highest slot
    /// used.
    #[must_use]
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// For each slot, the name of the last variable assigned to it. Only
    /// for showing slots to people, as `--dump-ir` does.
    #[must_use]
    pub fn slot_names(&self) -> &[Name] {
        &self.slot_names
    }

    /// A resolution for a tree with `exprs` expressions and `items` items and
    /// no variables, for building checked trees by hand in tests.
    #[cfg(test)]
    pub(crate) fn without_variables(exprs: usize, items: usize) -> Self {
        Self {
            expr_slots: vec![Slot::NONE; exprs],
            item_slots: vec![Slot::NONE; items],
            frame_size: 0,
            slot_names: Vec::new(),
        }
    }

    /// Number of expression entries, equal to the number of expressions in
    /// the [`Ast`].
    pub(crate) fn expr_len(&self) -> usize {
        self.expr_slots.len()
    }

    /// Number of item entries, equal to the number of items in the [`Ast`].
    pub(crate) fn item_len(&self) -> usize {
        self.item_slots.len()
    }
}

/// The result of a successful check: the types and the resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The type of every expression.
    pub types: Types,
    /// Where every variable is stored.
    pub resolution: Resolution,
}

/// A variable in scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    name: Name,
    slot: Slot,
    ty: Ty,
    /// The `let` or `:=` item that declared it, or `None` if it was declared
    /// by an earlier input of a session, whose tree is gone.
    declared: Option<ItemId>,
}

/// The variables of an outermost scope, with their slots and types: what a
/// [`crate::session::Session`] carries from one input to the next.
///
/// Only a [`Checker`] produces a non-empty scope. Its bindings occupy slots
/// `0..n` in declaration order, since the outermost block's declarations are
/// the first to take slots and no block is left open around them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    bindings: Vec<Binding>,
}

impl Scope {
    /// An empty scope, as at the start of a file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of variables declared, counting shadowed ones, which is
    /// also the number of slots they occupy.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether no variable has been declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Type-checks and resolves `ast` as a file, starting from an empty scope.
///
/// # Errors
///
/// Returns every type and scope error found. Checking continues after an
/// error by giving the offending node [`Ty::Error`], which every enclosing
/// operator accepts silently, so one run reports each independent error
/// exactly once.
pub fn check(ast: &Ast) -> Result<Checked, Vec<Diagnostic>> {
    Checker::new(ast).check().map(|(checked, _)| checked)
}

/// The type names every program can use, as `(name, type)` pairs. A name
/// the program never mentions has no [`Name`].
type BuiltinTypes = [(Option<Name>, Ty); 3];

/// A check in progress. See the module documentation.
pub struct Checker<'a> {
    ast: &'a Ast,
    types: Vec<Ty>,
    expr_slots: Vec<Slot>,
    item_slots: Vec<Slot>,
    /// Every variable in scope, outermost block first.
    bindings: Vec<Binding>,
    /// Index in `bindings` of the innermost block's first binding.
    block_start: usize,
    /// The next free slot.
    next_slot: u32,
    /// The highest `next_slot` reached.
    frame_size: u32,
    slot_names: Vec<Name>,
    builtin_types: BuiltinTypes,
    errors: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    /// A checker for `ast` as a file, starting from an empty scope.
    #[must_use]
    pub fn new(ast: &'a Ast) -> Self {
        Self::with_scope(ast, Scope::new())
    }

    /// A checker for `ast` whose outermost block starts with the variables
    /// of `scope` already declared. `scope` must come from checking earlier
    /// input interned into the same [`crate::names::Names`] as `ast`.
    #[must_use]
    pub fn with_scope(ast: &'a Ast, scope: Scope) -> Self {
        let names = ast.names();
        let slot_names: Vec<Name> = scope.bindings.iter().map(|binding| binding.name).collect();
        // The scope's bindings occupy exactly slots `0..len`.
        let next_slot = u32::try_from(slot_names.len()).unwrap_or(u32::MAX);
        Self {
            ast,
            // Placeholder only: every node is written by `check_expr` before
            // it is read, and the final invariant check catches any that
            // are not.
            types: vec![Ty::Error; ast.len()],
            expr_slots: vec![Slot::NONE; ast.len()],
            item_slots: vec![Slot::NONE; ast.item_count()],
            bindings: scope.bindings,
            block_start: 0,
            next_slot,
            frame_size: next_slot,
            slot_names,
            builtin_types: [
                (names.lookup("Int"), Ty::Int),
                (names.lookup("Float"), Ty::Float),
                (names.lookup("Unit"), Ty::Unit),
            ],
            errors: Vec::new(),
        }
    }

    /// Checks the file's body. On success, returns the tables and the
    /// outermost scope extended with the body's top-level declarations.
    ///
    /// # Errors
    ///
    /// As [`check`].
    pub fn check(mut self) -> Result<(Checked, Scope), Vec<Diagnostic>> {
        let root = self.ast.root();
        self.check_body(root);
        // Finalisation: an integer result with no context defaults to Int.
        self.coerce_result(root, Ty::Int);
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        if self
            .types
            .iter()
            .any(|&ty| matches!(ty, Ty::Error | Ty::UntypedInt))
        {
            let error = InternalError {
                message: "a type table entry is untyped or erroneous after a successful check",
                span: self.ast.root_span(),
            };
            return Err(vec![error.into()]);
        }
        // Item ids are meaningless once this tree is gone.
        let bindings = self
            .bindings
            .into_iter()
            .map(|binding| Binding {
                declared: None,
                ..binding
            })
            .collect();
        let checked = Checked {
            types: Types::new(self.types),
            resolution: Resolution {
                expr_slots: self.expr_slots,
                item_slots: self.item_slots,
                frame_size: self.frame_size,
                slot_names: self.slot_names,
            },
        };
        Ok((checked, Scope { bindings }))
    }

    /// Checks a nested block's body in a scope of its own, returning its
    /// type.
    fn check_block(&mut self, body: ItemRange) -> Ty {
        let outer = (self.block_start, self.next_slot);
        self.block_start = self.bindings.len();
        let ty = self.check_body(body);
        self.bindings.truncate(self.block_start);
        (self.block_start, self.next_slot) = outer;
        ty
    }

    /// Checks the items of a body in the current scope, returning the type
    /// of its result: the `= EXPR` or final expression statement, or `Unit`
    /// if there is none.
    fn check_body(&mut self, body: ItemRange) -> Ty {
        let items = self.ast.items(body);
        let mut result = Ty::Unit;
        for (index, &id) in items.iter().enumerate() {
            let next = items.get(index + 1).copied();
            let item = *self.ast.item(id);
            match item.kind {
                ItemKind::Let {
                    name, ty, value, ..
                } => self.check_let(id, name, ty, value),
                ItemKind::Rebind {
                    name,
                    name_span,
                    value,
                } => self.check_rebind(id, name, name_span, value),
                ItemKind::Result(value) => {
                    let ty = self.check_expr(value);
                    match next {
                        None => result = ty,
                        Some(next) => {
                            self.settle(value, ty);
                            let eq = Span::new(item.span.start, item.span.start + 1);
                            self.errors.push(
                                Diagnostic::error(
                                    "the result must be the last item in its block",
                                    eq,
                                )
                                .with_context(self.ast.item(next).span, "this item comes after it"),
                            );
                        }
                    }
                }
                ItemKind::Expr(value) => {
                    let ty = self.check_expr(value);
                    if next.is_none() {
                        result = ty;
                    } else if !matches!(ty, Ty::Unit | Ty::Error) {
                        self.settle(value, ty);
                        self.errors.push(
                            Diagnostic::error(
                                format!("unused value of type {ty}"),
                                self.ast.get(value).span,
                            )
                            .with_note(
                                "make it the block's result with `= ...`, bind it with `let`, \
                                 or remove it",
                            ),
                        );
                    }
                }
            }
        }
        result
    }

    /// Checks `let NAME (: TYPE)? = VALUE` and declares the variable.
    fn check_let(&mut self, id: ItemId, name: Name, annotation: Option<TypeExprId>, value: ExprId) {
        let annotation = annotation.map(|ty| (ty, self.resolve_type(ty)));
        let found = self.check_expr(value);
        let ty = match annotation {
            None => self.settle(value, found),
            Some((ty_id, expected)) => {
                let context = self.ast.type_expr(ty_id).span;
                self.expect(
                    value,
                    found,
                    expected,
                    context,
                    "expected because of this annotation",
                );
                if found.is_error() {
                    Ty::Error
                } else {
                    expected
                }
            }
        };
        self.declare(name, ty, id);
    }

    /// Checks `NAME := VALUE` and declares the new binding. The binding is
    /// declared even when the rebind is rejected, so that later uses of the
    /// name do not cascade into "undefined variable" errors.
    fn check_rebind(&mut self, id: ItemId, name: Name, name_span: Span, value: ExprId) {
        match self.lookup(name) {
            Some((index, _)) if index >= self.block_start => {}
            Some((_, binding)) => {
                let text = self.ast.names().get(name);
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}` here: it is declared in an enclosing block"),
                    name_span,
                );
                // A binding from an earlier session input has no item here
                // to point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(self.ast.item(declared).span, "declared here");
                }
                let error = error.with_note(format!("shadow it with `let {text} = ...` instead"));
                self.errors.push(error);
            }
            None => {
                let text = self.ast.names().get(name);
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: no variable named `{text}` is declared"),
                        name_span,
                    )
                    .with_note(format!("declare it with `let {text} = ...`")),
                );
            }
        }
        let found = self.check_expr(value);
        let ty = self.settle(value, found);
        self.declare(name, ty, id);
    }

    /// Declares a variable in the innermost block, in the next free slot.
    fn declare(&mut self, name: Name, ty: Ty, id: ItemId) {
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        match self.slot_names.get_mut(slot.index()) {
            Some(entry) => *entry = name,
            // Slots are taken in order, so a slot beyond the table is the
            // next one.
            None => self.slot_names.push(name),
        }
        self.item_slots[id.index()] = slot;
        self.bindings.push(Binding {
            name,
            slot,
            ty,
            declared: Some(id),
        });
    }

    /// The innermost, latest binding of `name`, with its index in
    /// `bindings`.
    fn lookup(&self, name: Name) -> Option<(usize, Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .rev()
            .find(|(_, binding)| binding.name == name)
            .map(|(index, &binding)| (index, binding))
    }

    /// The type a type expression denotes, or `Error` (reported) if it names
    /// no type.
    fn resolve_type(&mut self, id: TypeExprId) -> Ty {
        let ty = self.ast.type_expr(id);
        match ty.kind {
            TypeExprKind::Named(name) => {
                if let Some(&(_, builtin)) = self
                    .builtin_types
                    .iter()
                    .find(|(builtin, _)| *builtin == Some(name))
                {
                    return builtin;
                }
                let text = self.ast.names().get(name);
                self.errors
                    .push(Diagnostic::error(format!("unknown type `{text}`"), ty.span));
                Ty::Error
            }
        }
    }

    /// Checks that `expr`, of type `found`, can have type `expected`: an
    /// untyped integer is coerced to `Int` or `Float`, and any other type
    /// must match exactly. A mismatch is reported at `expr` with a context
    /// label on `context`, the source of the expectation.
    fn expect(&mut self, expr: ExprId, found: Ty, expected: Ty, context: Span, why: &str) {
        match (found, expected) {
            (Ty::Error, _) => {}
            // The expected type is unknown and has been reported.
            (_, Ty::Error) => {
                self.settle(expr, found);
            }
            (Ty::UntypedInt, Ty::Int | Ty::Float) => self.coerce(expr, expected),
            _ if found == expected => {}
            _ => {
                self.settle(expr, found);
                self.errors.push(
                    Diagnostic::error("mismatched types", self.ast.get(expr).span)
                        .with_label(format!("expected {expected}, found {found}"))
                        .with_context(context, why),
                );
            }
        }
    }

    /// Computes, records and returns the type of `id`, bottom-up.
    fn check_expr(&mut self, id: ExprId) -> Ty {
        let expr = self.ast.get(id);
        let ty = match expr.kind {
            ExprKind::Int(_) => Ty::UntypedInt,
            ExprKind::Float(_) => Ty::Float,
            ExprKind::Paren(inner) => self.check_expr(inner),
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => {
                // Negation accepts every numeric type and preserves it.
                let operand_ty = self.check_expr(operand);
                match operand_ty {
                    Ty::Int | Ty::Float | Ty::UntypedInt | Ty::Error => operand_ty,
                    Ty::Unit => {
                        let span = expr.operator_span().unwrap_or(expr.span);
                        self.errors.push(
                            Diagnostic::error(
                                format!("cannot {} {operand_ty}", UnaryOp::Neg.verb()),
                                span,
                            )
                            .with_context(self.ast.get(operand).span, operand_ty.to_string()),
                        );
                        Ty::Error
                    }
                }
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match op {
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                        self.arithmetic(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
                    }
                    BinaryOp::Div => self.division(id, (lhs, lhs_ty), (rhs, rhs_ty)),
                    BinaryOp::FloorDiv | BinaryOp::Rem => {
                        self.integer_only(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
                    }
                }
            }
            ExprKind::Name(name) => self.check_name(id, name, expr.span),
            ExprKind::Block(body) => self.check_block(body),
            ExprKind::Ascribe { expr: inner, ty } => {
                let found = self.check_expr(inner);
                let expected = self.resolve_type(ty);
                let context = self.ast.type_expr(ty).span;
                self.expect(inner, found, expected, context, "ascribed here");
                if found.is_error() {
                    Ty::Error
                } else {
                    expected
                }
            }
        };
        self.types[id.index()] = ty;
        ty
    }

    /// Resolves a use of the variable `name` at expression `id`, recording
    /// its slot, and returns its type.
    fn check_name(&mut self, id: ExprId, name: Name, span: Span) -> Ty {
        if let Some((_, binding)) = self.lookup(name) {
            self.expr_slots[id.index()] = binding.slot;
            return binding.ty;
        }
        let text = self.ast.names().get(name);
        let mut error = Diagnostic::error(format!("undefined variable `{text}`"), span);
        if text.contains('-') {
            error = error.with_note(
                "names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction",
            );
        }
        self.errors.push(error);
        Ty::Error
    }

    /// Types `+`, `-` and `*`: both operands must end up the same numeric
    /// type, with untyped operands adapting to a concrete one.
    fn arithmetic(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, Ty),
        (rhs, rhs_ty): (ExprId, Ty),
    ) -> Ty {
        match (lhs_ty, rhs_ty) {
            // An erroneous operand has been reported already. The other
            // operand still needs a concrete type in case it is untyped.
            (Ty::Error, _) => {
                self.coerce(rhs, Ty::Int);
                Ty::Error
            }
            (_, Ty::Error) => {
                self.coerce(lhs, Ty::Int);
                Ty::Error
            }
            (Ty::UntypedInt, Ty::UntypedInt) => Ty::UntypedInt,
            (Ty::UntypedInt, concrete @ (Ty::Int | Ty::Float)) => {
                self.coerce(lhs, concrete);
                concrete
            }
            (concrete @ (Ty::Int | Ty::Float), Ty::UntypedInt) => {
                self.coerce(rhs, concrete);
                concrete
            }
            (Ty::Int, Ty::Int) => Ty::Int,
            (Ty::Float, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) | (Ty::Unit, _) | (_, Ty::Unit) => {
                self.coerce(lhs, Ty::Int);
                self.coerce(rhs, Ty::Int);
                self.operand_error(
                    id,
                    format!("cannot {} {lhs_ty} and {rhs_ty}", op.verb()),
                    (lhs, lhs_ty),
                    (rhs, rhs_ty),
                );
                Ty::Error
            }
        }
    }

    /// Types `/`: it accepts any mix of numeric operands and always yields
    /// `Float`. Untyped operands become `Float` literals; concrete `Int`
    /// operands are converted by lowering.
    fn division(
        &mut self,
        id: ExprId,
        (lhs, lhs_ty): (ExprId, Ty),
        (rhs, rhs_ty): (ExprId, Ty),
    ) -> Ty {
        self.coerce(lhs, Ty::Float);
        self.coerce(rhs, Ty::Float);
        if lhs_ty.is_error() || rhs_ty.is_error() {
            return Ty::Error;
        }
        if lhs_ty == Ty::Unit || rhs_ty == Ty::Unit {
            self.operand_error(
                id,
                format!("cannot {} {lhs_ty} and {rhs_ty}", BinaryOp::Div.verb()),
                (lhs, lhs_ty),
                (rhs, rhs_ty),
            );
            return Ty::Error;
        }
        Ty::Float
    }

    /// Reports an operator rejecting its operands: the message points at the
    /// operator, and each operand is labelled with its type.
    fn operand_error(
        &mut self,
        id: ExprId,
        message: String,
        (lhs, lhs_ty): (ExprId, Ty),
        (rhs, rhs_ty): (ExprId, Ty),
    ) {
        let expr = self.ast.get(id);
        let span = expr.operator_span().unwrap_or(expr.span);
        self.errors.push(
            Diagnostic::error(message, span)
                .with_context(self.ast.get(lhs).span, lhs_ty.to_string())
                .with_context(self.ast.get(rhs).span, rhs_ty.to_string()),
        );
    }

    /// Types `//` and `%`: both operands must be integers, and the result is
    /// a concrete `Int`. Untyped operands are coerced to `Int`; they never
    /// adapt to `Float`, since these operators do not accept it.
    fn integer_only(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, Ty),
        (rhs, rhs_ty): (ExprId, Ty),
    ) -> Ty {
        // Whatever the outcome, untyped operands settle to Int: these
        // operators never accept Float, so nothing else could decide them.
        self.coerce(lhs, Ty::Int);
        self.coerce(rhs, Ty::Int);
        if lhs_ty.is_error() || rhs_ty.is_error() {
            return Ty::Error;
        }
        let rejected = [lhs_ty, rhs_ty]
            .into_iter()
            .find(|ty| matches!(ty, Ty::Float | Ty::Unit));
        if let Some(rejected) = rejected {
            self.operand_error(
                id,
                format!("cannot {} {rejected}", op.verb()),
                (lhs, lhs_ty),
                (rhs, rhs_ty),
            );
            return Ty::Error;
        }
        Ty::Int
    }

    /// Gives an untyped expression its default type, `Int`, where it is
    /// bound or discarded rather than used in a typed context. Returns the
    /// resulting type.
    fn settle(&mut self, id: ExprId, ty: Ty) -> Ty {
        if ty == Ty::UntypedInt {
            self.coerce(id, Ty::Int);
            Ty::Int
        } else {
            ty
        }
    }

    /// Pushes the concrete type `ty` down into the untyped subtree rooted at
    /// `id`. Does nothing if `id` already has a concrete type (or is an
    /// error).
    ///
    /// Only literals, parentheses, negation, `+ - *` and blocks can be
    /// untyped, and an untyped node's children are untyped or concrete of
    /// the same kind, so the walk only ever meets those nodes. Names and
    /// ascriptions always have a concrete type.
    fn coerce(&mut self, id: ExprId, ty: Ty) {
        if self.types[id.index()] != Ty::UntypedInt {
            return;
        }
        self.types[id.index()] = ty;
        match self.ast.get(id).kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Name(_)
            | ExprKind::Ascribe { .. } => {}
            ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
                self.coerce(inner, ty);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.coerce(lhs, ty);
                self.coerce(rhs, ty);
            }
            ExprKind::Block(body) => self.coerce_result(body, ty),
        }
    }

    /// Coerces the result expression of `body`, if it has one, to `ty`.
    fn coerce_result(&mut self, body: ItemRange, ty: Ty) {
        let Some(&last) = self.ast.items(body).last() else {
            return;
        };
        match self.ast.item(last).kind {
            ItemKind::Result(value) | ItemKind::Expr(value) => self.coerce(value, ty),
            ItemKind::Let { .. } | ItemKind::Rebind { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{tokenize, tokenize_with};
    use crate::names::Names;
    use crate::parser::parse;
    use crate::source::SourceFile;

    fn parse_text(text: &str) -> Ast {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names) = tokenize(&source).unwrap();
        parse(&tokens, names, &source).unwrap()
    }

    /// The expression of the file's final result or expression item.
    fn result_expr(ast: &Ast) -> ExprId {
        let last = *ast.items(ast.root()).last().unwrap();
        match ast.item(last).kind {
            ItemKind::Result(value) | ItemKind::Expr(value) => value,
            other => panic!("the last item has no result: {other:?}"),
        }
    }

    /// The type of the file's result.
    fn result_type(text: &str) -> Ty {
        let ast = parse_text(text);
        check(&ast).unwrap().types.get(result_expr(&ast))
    }

    fn diagnostics(text: &str) -> Vec<Diagnostic> {
        check(&parse_text(text)).unwrap_err()
    }

    fn errors(text: &str) -> Vec<(String, Span)> {
        diagnostics(text)
            .into_iter()
            .map(|d| (d.message, d.span))
            .collect()
    }

    fn messages(text: &str) -> Vec<String> {
        errors(text).into_iter().map(|(m, _)| m).collect()
    }

    /// Every node's type in allocation order (children before parents).
    fn all_types(text: &str) -> Vec<Ty> {
        check(&parse_text(text)).unwrap().types.types
    }

    /// The labels of a diagnostic as `(span, text)` pairs.
    fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
        diagnostic
            .labels
            .iter()
            .map(|label| (label.span, label.text.as_str()))
            .collect()
    }

    // A concrete Int operand is written `(7 // 2)`: `//` always produces a
    // concrete Int, so the typing rows with a concrete Int are reachable
    // without variables.

    #[test]
    fn literals() {
        assert_eq!(result_type("1"), Ty::Int);
        assert_eq!(result_type("1.5"), Ty::Float);
    }

    #[test]
    fn untyped_op_untyped_stays_untyped_until_the_root() {
        for op in ["+", "-", "*"] {
            let text = format!("1 {op} 2");
            assert_eq!(all_types(&text), vec![Ty::Int; 3], "{text}");
        }
    }

    #[test]
    fn untyped_op_int_is_int() {
        for op in ["+", "-", "*"] {
            assert_eq!(result_type(&format!("1 {op} (7 // 2)")), Ty::Int);
            assert_eq!(result_type(&format!("(7 // 2) {op} 1")), Ty::Int);
        }
    }

    #[test]
    fn untyped_op_float_is_float() {
        for op in ["+", "-", "*"] {
            let text = format!("1 {op} 2.0");
            assert_eq!(all_types(&text), vec![Ty::Float; 3], "{text}");
            let text = format!("2.0 {op} 1");
            assert_eq!(all_types(&text), vec![Ty::Float; 3], "{text}");
        }
    }

    #[test]
    fn same_concrete_types() {
        assert_eq!(result_type("(7 // 2) + (7 // 2)"), Ty::Int);
        assert_eq!(result_type("1.0 * 2.0"), Ty::Float);
    }

    #[test]
    fn int_and_float_do_not_mix() {
        assert_eq!(
            errors("(7 // 2) + 1.5"),
            vec![("cannot add Int and Float".to_owned(), Span::new(9, 10))]
        );
        assert_eq!(
            messages("1.5 - (7 // 2)"),
            vec!["cannot subtract Float and Int"]
        );
        assert_eq!(
            messages("(7 % 2) * 1.5"),
            vec!["cannot multiply Int and Float"]
        );
    }

    #[test]
    fn negation_preserves_type() {
        assert_eq!(all_types("-1"), vec![Ty::Int; 2]);
        assert_eq!(all_types("-1.5"), vec![Ty::Float; 2]);
        assert_eq!(result_type("-(7 // 2)"), Ty::Int);
        // An untyped negation adapts like a literal.
        assert_eq!(all_types("-1 + 2.0"), vec![Ty::Float; 4]);
    }

    #[test]
    fn division_accepts_any_mix_and_yields_float() {
        assert_eq!(all_types("7 / 2"), vec![Ty::Float; 3]);
        assert_eq!(all_types("7 / 2.0"), vec![Ty::Float; 3]);
        assert_eq!(all_types("7.0 / 2"), vec![Ty::Float; 3]);
        assert_eq!(result_type("(7 // 2) / 2.0"), Ty::Float);
        assert_eq!(result_type("2.0 / (7 // 2)"), Ty::Float);
        assert_eq!(result_type("(7 // 2) / (7 // 2)"), Ty::Float);
    }

    #[test]
    fn division_leaves_concrete_int_operands_int() {
        let ast = parse_text("(7 // 2) / 2");
        let types = check(&ast).unwrap().types;
        let ExprKind::Binary { lhs, rhs, .. } = ast.get(result_expr(&ast)).kind else {
            panic!("expected binary");
        };
        assert_eq!(types.get(lhs), Ty::Int);
        assert_eq!(types.get(rhs), Ty::Float);
    }

    #[test]
    fn floor_division_and_modulo_need_integers() {
        assert_eq!(all_types("7 // 2"), vec![Ty::Int; 3]);
        assert_eq!(all_types("7 % 2"), vec![Ty::Int; 3]);
        assert_eq!(result_type("(7 // 2) // 2"), Ty::Int);
        assert_eq!(
            errors("7.0 // 2"),
            vec![("cannot floor-divide Float".to_owned(), Span::new(4, 6))]
        );
        assert_eq!(messages("7 // 2.0"), vec!["cannot floor-divide Float"]);
        assert_eq!(messages("7.0 % 2"), vec!["cannot take modulo of Float"]);
        assert_eq!(messages("7 % 2.0"), vec!["cannot take modulo of Float"]);
        assert_eq!(messages("7.0 % 2.0"), vec!["cannot take modulo of Float"]);
    }

    #[test]
    fn floor_division_result_does_not_adapt_to_float() {
        assert_eq!(messages("(7 // 2) + 1.0"), vec!["cannot add Int and Float"]);
    }

    #[test]
    fn untyped_subtree_is_coerced_all_the_way_down() {
        // Allocation order: 1, 2, +, paren, 3.0, *.
        assert_eq!(all_types("(1 + 2) * 3.0"), vec![Ty::Float; 6]);
        assert_eq!(all_types("3.0 * -(1 - -2)"), vec![Ty::Float; 8]);
    }

    #[test]
    fn division_makes_untyped_siblings_float() {
        // `2 / 3` is Float, so the untyped `1` beside it adapts to Float.
        assert_eq!(all_types("1 + 2 / 3"), vec![Ty::Float; 5]);
    }

    #[test]
    fn untyped_root_defaults_to_int_throughout() {
        assert_eq!(all_types("-(1 + 2) * 3"), vec![Ty::Int; 7]);
        assert_eq!(all_types("= 1 + 2"), vec![Ty::Int; 3]);
    }

    #[test]
    fn multiple_errors_in_one_run() {
        assert_eq!(
            errors("(7.0 // 2) + (1 % 2.5)"),
            vec![
                ("cannot floor-divide Float".to_owned(), Span::new(5, 7)),
                ("cannot take modulo of Float".to_owned(), Span::new(16, 17)),
            ]
        );
    }

    #[test]
    fn operands_are_labelled_with_their_types() {
        let concrete = diagnostics("(7 // 2) + 1.5");
        assert_eq!(
            labels(&concrete[0]),
            vec![(Span::new(0, 8), "Int"), (Span::new(11, 14), "Float")]
        );
        // An untyped literal is labelled as the Int it would default to.
        let untyped = diagnostics("1 % 2.5");
        let texts: Vec<&str> = labels(&untyped[0]).into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, vec!["Int", "Float"]);
    }

    #[test]
    fn errors_do_not_cascade() {
        // The failed `//` becomes an error type that every enclosing operator
        // accepts, whatever the other operand is.
        for text in [
            "(1.0 // 2) + (7 // 2)",
            "(1.0 // 2) + 1.5",
            "(1.0 // 2) + 1",
            "1.5 * (1.0 // 2)",
            "-(1.0 // 2) // 2",
            "(1.0 // 2) / 2.5",
            "2 % (1.0 // 2)",
            "(1.0 // 2) : Float",
            "{ 1.0 // 2 } + 1.5",
        ] {
            assert_eq!(messages(text), vec!["cannot floor-divide Float"], "{text}");
        }
    }

    #[test]
    fn unit_is_rejected_by_every_operator() {
        assert_eq!(
            errors("{} + 1"),
            vec![("cannot add Unit and Int".to_owned(), Span::new(3, 4))]
        );
        assert_eq!(messages("1.5 * {}"), vec!["cannot multiply Float and Unit"]);
        assert_eq!(messages("{} / 2"), vec!["cannot divide Unit and Int"]);
        assert_eq!(messages("{} // 2"), vec!["cannot floor-divide Unit"]);
        assert_eq!(messages("7 % {}"), vec!["cannot take modulo of Unit"]);
        assert_eq!(messages("1.5 // {}"), vec!["cannot floor-divide Float"]);
        let diagnostics = diagnostics("-{}");
        assert_eq!(diagnostics[0].message, "cannot negate Unit");
        assert_eq!(diagnostics[0].span, Span::new(0, 1));
        assert_eq!(labels(&diagnostics[0]), vec![(Span::new(1, 3), "Unit")]);
    }

    #[test]
    fn error_type_never_escapes_a_successful_check() {
        // Every node of a successful check has a concrete type.
        for text in [
            "1",
            "1.5",
            "(1 + 2) * 3.0",
            "7 / 2",
            "(7 // 2) / 2",
            "let x = 5\nx + 1",
            "let y = { 1 + 2 }\n{ 3 } : Float",
            "= { let a = 1\n= a } * 2",
            "{}",
        ] {
            for ty in all_types(text) {
                assert!(!ty.is_error() && ty != Ty::UntypedInt, "{text}: {ty:?}");
            }
        }
    }

    #[test]
    fn annotation_renders_types() {
        let ast = parse_text("1 + 2.0");
        let types = check(&ast).unwrap().types;
        assert_eq!(
            ast.pretty_with(&types).to_string(),
            "(file\n  (expr @0..7\n    (+ : Float @0..7\n      (int 1 : Float @0..1)\n      (float 2.0 : Float @4..7))))"
        );
        let ast = parse_text("let x = { 1 }\nx : Float");
        let types = check(&ast);
        assert!(types.is_err());
        let ast = parse_text("let x = { 1 }\n{}");
        let types = check(&ast).unwrap().types;
        assert_eq!(
            ast.pretty_with(&types).to_string(),
            "(file\n\
             \x20 (let x @0..13\n\
             \x20   (block : Int @8..13\n\
             \x20     (expr @10..11\n\
             \x20       (int 1 : Int @10..11))))\n\
             \x20 (expr @14..16\n\
             \x20   (block : Unit @14..16)))"
        );
    }

    #[test]
    fn variables_have_the_type_they_were_bound_with() {
        assert_eq!(result_type("let x = 5\nx"), Ty::Int);
        assert_eq!(result_type("let x = 5.5\nx"), Ty::Float);
        assert_eq!(result_type("let x = {}\nx"), Ty::Unit);
        // The untyped literal defaults where it is bound.
        assert_eq!(
            errors("let x = 5\nx + 2.5"),
            vec![("cannot add Int and Float".to_owned(), Span::new(12, 13))]
        );
        assert_eq!(result_type("let x = 5 : Float\nx + 2.5"), Ty::Float);
        assert_eq!(result_type("let x: Float = 5\nx + 2.5"), Ty::Float);
    }

    #[test]
    fn a_let_is_visible_from_the_next_item() {
        assert_eq!(
            errors("let x = x"),
            vec![("undefined variable `x`".to_owned(), Span::new(8, 9))]
        );
        // The initialiser sees the binding being shadowed.
        assert_eq!(result_type("let x = 1.5\nlet x = x + 1\nx"), Ty::Float);
    }

    #[test]
    fn shadowing_in_the_same_block() {
        assert_eq!(result_type("let x = 1\nlet x = 2.5\nx"), Ty::Float);
        let ast = parse_text("let x = 1\nlet x = 2.5\nx");
        let resolution = check(&ast).unwrap().resolution;
        assert_eq!(resolution.expr_slot(result_expr(&ast)), Slot(1));
    }

    #[test]
    fn shadowing_across_blocks() {
        assert_eq!(result_type("let x = 1\n{ let x = 2.5; x }"), Ty::Float);
        // The outer variable is visible in an inner block until shadowed.
        assert_eq!(result_type("let x = 1\n{ let y = x; y }"), Ty::Int);
        // And is back in scope after the block.
        assert_eq!(result_type("let x = 1\n{ let x = 2.5 }\nx"), Ty::Int);
    }

    #[test]
    fn variables_are_not_visible_after_their_block() {
        assert_eq!(
            errors("{ let y = 1 }\ny"),
            vec![("undefined variable `y`".to_owned(), Span::new(14, 15))]
        );
        assert_eq!(
            messages("let z = { let inner = 1; inner }\ninner"),
            vec!["undefined variable `inner`"]
        );
    }

    #[test]
    fn undefined_variables() {
        let diagnostic = &diagnostics("x + 1")[0];
        assert_eq!(diagnostic.message, "undefined variable `x`");
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert!(diagnostic.notes.is_empty());
        let diagnostic = &diagnostics("let x = 2\nx-1")[0];
        assert_eq!(diagnostic.message, "undefined variable `x-1`");
        assert_eq!(diagnostic.span, Span::new(10, 13));
        assert_eq!(
            diagnostic.notes,
            vec!["names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction"]
        );
    }

    #[test]
    fn rebinding_in_the_same_block() {
        assert_eq!(result_type("let x = 1\nx := 2.5\nx"), Ty::Float);
        assert_eq!(result_type("let x = 1\nx := x + 1\nx := x * 2\nx"), Ty::Int);
        // In an inner block, of a variable declared in that block, even one
        // shadowing an outer variable.
        assert_eq!(
            result_type("let x = 1\n{ let x = 2; x := 3.5; x }"),
            Ty::Float
        );
        // The new value sees the old binding.
        let ast = parse_text("let x = 1\nx := x + 1\nx");
        let resolution = check(&ast).unwrap().resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(0));
        // `:=` takes a fresh slot.
        assert_eq!(resolution.item_slot(items[1]), Slot(1));
        assert_eq!(resolution.expr_slot(result_expr(&ast)), Slot(1));
    }

    #[test]
    fn rebinding_an_undeclared_name() {
        let diagnostic = &diagnostics("x := 1")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `x`: no variable named `x` is declared"
        );
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert!(diagnostic.labels.is_empty());
        assert_eq!(diagnostic.notes, vec!["declare it with `let x = ...`"]);
        // A later declaration does not count.
        assert_eq!(
            messages("x := 1\nlet x = 2"),
            vec!["cannot rebind `x`: no variable named `x` is declared"]
        );
    }

    #[test]
    fn rebinding_a_name_from_an_enclosing_block() {
        let diagnostic = &diagnostics("let x = 1\n{\n  x := 2\n}")[0];
        assert_eq!(
            diagnostic.message,
            "cannot rebind `x` here: it is declared in an enclosing block"
        );
        assert_eq!(diagnostic.span, Span::new(14, 15));
        assert_eq!(labels(diagnostic), vec![(Span::new(0, 9), "declared here")]);
        assert_eq!(
            diagnostic.notes,
            vec!["shadow it with `let x = ...` instead"]
        );
        // The label points at the latest declaration.
        let diagnostic = &diagnostics("let x = 1\nx := 2\n{ x := 3 }")[0];
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(10, 16), "declared here")]
        );
    }

    #[test]
    fn a_rejected_rebind_still_declares_the_variable() {
        assert_eq!(
            messages("y := 1\ny + 1"),
            vec!["cannot rebind `y`: no variable named `y` is declared"]
        );
        assert_eq!(
            messages("let x = 1\n{ x := 2.5; x * 1.5 }"),
            vec!["cannot rebind `x` here: it is declared in an enclosing block"]
        );
    }

    #[test]
    fn annotations() {
        let ast = parse_text("let x: Float = 5");
        assert_eq!(check(&ast).unwrap().types.types, vec![Ty::Float]);
        assert_eq!(result_type("let x: Int = 5\nx"), Ty::Int);
        assert_eq!(result_type("let x: Float = 5\nx"), Ty::Float);
        assert_eq!(result_type("let x: Float = 2.5\nx"), Ty::Float);
        assert_eq!(result_type("let x: Unit = {}\nx"), Ty::Unit);
        assert_eq!(result_type("let x: Float = -(1 + 2)\nx"), Ty::Float);
        assert_eq!(result_type("let x: Int = 7 // 2\nx"), Ty::Int);
    }

    #[test]
    fn annotation_mismatches() {
        let diagnostic = &diagnostics("let x: Int = 5.0")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(13, 16));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found Float")
        );
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(7, 10), "expected because of this annotation")]
        );
        assert_eq!(
            diagnostics("let x: Float = 7 // 2")[0].label.as_deref(),
            Some("expected Float, found Int")
        );
        assert_eq!(
            diagnostics("let x: Unit = 1")[0].label.as_deref(),
            Some("expected Unit, found Int")
        );
        assert_eq!(
            diagnostics("let x: Int = {}")[0].label.as_deref(),
            Some("expected Int, found Unit")
        );
        // The variable has the annotated type, so uses do not cascade.
        assert_eq!(
            messages("let x: Int = 5.0\nlet y = x + 1\n= x // y"),
            vec!["mismatched types"]
        );
    }

    #[test]
    fn unknown_types() {
        assert_eq!(
            errors("let x: Foo = 1"),
            vec![("unknown type `Foo`".to_owned(), Span::new(7, 10))]
        );
        assert_eq!(
            errors("1 : int"),
            vec![("unknown type `int`".to_owned(), Span::new(4, 7))]
        );
        // A variable is not a type.
        assert_eq!(
            messages("let x = 1\nlet y: x = 2"),
            vec!["unknown type `x`"]
        );
        // Neither the variable nor the ascription cascades.
        assert_eq!(
            messages("let x: Foo = 1.0 // 2\nx + 1.5\n(1 : Bar) + 2.5"),
            vec![
                "unknown type `Foo`",
                "cannot floor-divide Float",
                "unknown type `Bar`"
            ]
        );
    }

    #[test]
    fn ascriptions() {
        assert_eq!(all_types("5 : Float"), vec![Ty::Float; 2]);
        assert_eq!(result_type("-5 : Float"), Ty::Float);
        assert_eq!(result_type("1 + 2 : Float"), Ty::Float);
        assert_eq!(result_type("let x = 1\nx : Int"), Ty::Int);
        assert_eq!(result_type("{} : Unit"), Ty::Unit);
        assert_eq!(result_type("1 : Int : Int"), Ty::Int);
        // An untyped block adapts like a literal.
        assert_eq!(all_types("{ 5 } : Float"), vec![Ty::Float; 3]);
        let diagnostic = &diagnostics("5.0 : Int")[0];
        assert_eq!(diagnostic.message, "mismatched types");
        assert_eq!(diagnostic.span, Span::new(0, 3));
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("expected Int, found Float")
        );
        assert_eq!(labels(diagnostic), vec![(Span::new(6, 9), "ascribed here")]);
        assert_eq!(
            diagnostics("let x = 1\nx : Float")[0].label.as_deref(),
            Some("expected Float, found Int")
        );
        // Chained ascriptions are each checked in turn.
        assert_eq!(
            errors("1 : Int : Float"),
            vec![("mismatched types".to_owned(), Span::new(0, 7))]
        );
    }

    #[test]
    fn block_types() {
        assert_eq!(result_type("{}"), Ty::Unit);
        assert_eq!(result_type("{ 1.5 }"), Ty::Float);
        assert_eq!(result_type("{ = 1.5 }"), Ty::Float);
        assert_eq!(result_type("{ let a = 1 }"), Ty::Unit);
        assert_eq!(result_type("{ let a = 1; a }"), Ty::Int);
        assert_eq!(result_type("{ {} }"), Ty::Unit);
        assert_eq!(result_type("{ 1 } + 2.5"), Ty::Float);
        assert_eq!(result_type("let x = { 1 }\nx"), Ty::Int);
    }

    #[test]
    fn unused_values() {
        let diagnostic = &diagnostics("1 + 2\n3")[0];
        assert_eq!(diagnostic.message, "unused value of type Int");
        assert_eq!(diagnostic.span, Span::new(0, 5));
        assert_eq!(
            diagnostic.notes,
            vec!["make it the block's result with `= ...`, bind it with `let`, or remove it"]
        );
        assert_eq!(messages("1.5; 2"), vec!["unused value of type Float"]);
        assert_eq!(
            messages("let x = 1\nx\nx"),
            vec!["unused value of type Int"]
        );
        assert_eq!(messages("{ 1; 2 }"), vec!["unused value of type Int"]);
        // Unit statements are fine anywhere.
        assert_eq!(result_type("{}\n{ let a = 1 }\n{ {} }\n2"), Ty::Int);
        // An erroneous statement is reported once, not also as unused.
        assert_eq!(messages("1.0 // 2\n3"), vec!["cannot floor-divide Float"]);
    }

    #[test]
    fn the_result_must_be_the_last_item() {
        let diagnostic = &diagnostics("= 1\nlet y = 2")[0];
        assert_eq!(
            diagnostic.message,
            "the result must be the last item in its block"
        );
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(
            labels(diagnostic),
            vec![(Span::new(4, 13), "this item comes after it")]
        );
        // A second result is caught by the same rule, at the first.
        assert_eq!(
            errors("{ = 1; = 2 }"),
            vec![(
                "the result must be the last item in its block".to_owned(),
                Span::new(2, 3)
            )]
        );
        // A final expression after a misplaced result is not also unused.
        assert_eq!(
            messages("= 1\n2"),
            vec!["the result must be the last item in its block"]
        );
    }

    /// Every `let` in `body` and the blocks nested in it, in the order their
    /// declarations complete.
    fn lets(ast: &Ast, body: ItemRange, out: &mut Vec<ItemId>) {
        fn visit(ast: &Ast, id: ExprId, out: &mut Vec<ItemId>) {
            match ast.get(id).kind {
                ExprKind::Block(body) => lets(ast, body, out),
                ExprKind::Binary { lhs, rhs, .. } => {
                    visit(ast, lhs, out);
                    visit(ast, rhs, out);
                }
                ExprKind::Paren(inner)
                | ExprKind::Unary { operand: inner, .. }
                | ExprKind::Ascribe { expr: inner, .. } => visit(ast, inner, out),
                ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Name(_) => {}
            }
        }
        for &id in ast.items(body) {
            match ast.item(id).kind {
                ItemKind::Let { value, .. } => {
                    visit(ast, value, out);
                    out.push(id);
                }
                ItemKind::Rebind { value, .. }
                | ItemKind::Result(value)
                | ItemKind::Expr(value) => {
                    visit(ast, value, out);
                }
            }
        }
    }

    #[test]
    fn slots_are_assigned_stack_fashion() {
        let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
        let ast = parse_text(text);
        let resolution = check(&ast).unwrap().resolution;
        let names = ast.names();
        let mut all_lets = Vec::new();
        lets(&ast, ast.root(), &mut all_lets);
        let slot_of = |text: &str| -> Vec<u32> {
            let wanted = names.lookup(text).unwrap();
            all_lets
                .iter()
                .filter(|&&id| matches!(ast.item(id).kind, ItemKind::Let { name, .. } if name == wanted))
                .map(|&id| resolution.item_slot(id).get())
                .collect()
        };
        // Each initialiser's temporary takes the slot its variable then
        // reuses, since the variable is declared after its initialiser's
        // block has ended.
        assert_eq!(slot_of("t"), vec![0]);
        assert_eq!(slot_of("a"), vec![0]);
        assert_eq!(slot_of("u"), vec![1]);
        assert_eq!(slot_of("b"), vec![1]);
        assert_eq!(slot_of("v"), vec![2]);
        assert_eq!(slot_of("w"), vec![3]);
        assert_eq!(slot_of("c"), vec![2]);
        assert_eq!(resolution.frame_size(), 4);
        let slot_names: Vec<&str> = resolution
            .slot_names()
            .iter()
            .map(|&name| names.get(name))
            .collect();
        assert_eq!(slot_names, vec!["a", "b", "c", "w"]);
        // Uses read the slot of the binding in scope.
        let ExprKind::Binary { lhs, rhs, .. } = ast.get(result_expr(&ast)).kind else {
            panic!("expected a sum");
        };
        assert_eq!(resolution.expr_slot(rhs), Slot(2));
        let ExprKind::Binary { lhs: a, rhs: b, .. } = ast.get(lhs).kind else {
            panic!("expected a sum");
        };
        assert_eq!(resolution.expr_slot(a), Slot(0));
        assert_eq!(resolution.expr_slot(b), Slot(1));
        // Nodes that are not variables have no slot.
        assert_eq!(resolution.expr_slot(lhs), Slot::NONE);
    }

    #[test]
    fn slots_after_a_block_are_reused() {
        let ast = parse_text("let a = 1\n{ let b = 2; let c = 3 }\nlet d = 4\nd");
        let resolution = check(&ast).unwrap().resolution;
        let items = ast.items(ast.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(0));
        assert_eq!(resolution.item_slot(items[1]), Slot::NONE);
        // `b` was in slot 1; `d` reuses it.
        assert_eq!(resolution.item_slot(items[2]), Slot(1));
        assert_eq!(resolution.frame_size(), 3);
        assert_eq!(resolution.expr_slot(result_expr(&ast)), Slot(1));
        assert_eq!(Slot(3).to_string(), "#3");
    }

    #[test]
    fn a_program_without_variables_needs_no_frame() {
        let ast = parse_text("1 + { 2 }");
        let resolution = check(&ast).unwrap().resolution;
        assert_eq!(resolution.frame_size(), 0);
        assert!(resolution.slot_names().is_empty());
    }

    #[test]
    fn recovery_does_not_cascade() {
        // A `let` whose initialiser fails is still declared, as Error.
        assert_eq!(
            messages("let x = 1.0 // 2\nlet y = x + 1\n= x * 2.5 + y"),
            vec!["cannot floor-divide Float"]
        );
        // An undefined variable is reported at each use, and nothing more.
        assert_eq!(
            messages("let y = z + 1.5\nz * 2\ny // 2"),
            vec!["undefined variable `z`", "undefined variable `z`"]
        );
        // Independent errors are all reported.
        assert_eq!(
            messages("let x = 1.0 // 2\nlet y: Int = 2.5\nq"),
            vec![
                "cannot floor-divide Float",
                "mismatched types",
                "undefined variable `q`"
            ]
        );
    }

    #[test]
    fn checking_continues_from_an_existing_scope() {
        let mut names = Names::new();
        let parse_with = |text: &str, names: &mut Names| {
            let source = SourceFile::new("test.gcsl", text).unwrap();
            let tokens = tokenize_with(&source, names).unwrap();
            parse(&tokens, names.clone(), &source).unwrap()
        };
        let first = parse_with("let a = 1\nlet b = 2.5", &mut names);
        let (_, scope) = Checker::new(&first).check().unwrap();
        assert_eq!(scope.len(), 2);

        let second = parse_with("a := a + 1\nlet c = b * 2.0\n= a", &mut names);
        let (checked, extended) = Checker::with_scope(&second, scope.clone()).check().unwrap();
        assert_eq!(extended.len(), 4);
        assert_eq!(checked.types.get(result_expr(&second)), Ty::Int);
        let resolution = &checked.resolution;
        let items = second.items(second.root());
        assert_eq!(resolution.item_slot(items[0]), Slot(2));
        assert_eq!(resolution.item_slot(items[1]), Slot(3));
        assert_eq!(resolution.frame_size(), 4);
        let slot_names: Vec<&str> = resolution
            .slot_names()
            .iter()
            .map(|&name| second.names().get(name))
            .collect();
        assert_eq!(slot_names, vec!["a", "b", "a", "c"]);

        // A variable from an earlier input is in the enclosing block of an
        // inner one, with no item to point at.
        let third = parse_with("{ b := 1.5 }", &mut names);
        let diagnostics = Checker::with_scope(&third, scope).check().unwrap_err();
        assert_eq!(
            diagnostics[0].message,
            "cannot rebind `b` here: it is declared in an enclosing block"
        );
        assert!(diagnostics[0].labels.is_empty());
    }
}
