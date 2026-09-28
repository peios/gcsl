//! The evaluator: a recursive walk over [`Ir`].
//!
//! There is one walker per kind of value (see [`crate::value::ValueKind`]):
//! `Evaluator::int`, `Evaluator::float`, `Evaluator::bool` and
//! `Evaluator::string`, returning an `i64`, an `f64`, a `bool` and an
//! `Rc<str>`, and `Evaluator::record`, returning a [`Value`] that is
//! [`Value::Empty`] or [`Value::Record`], since both are records. Every IR
//! node states its kind, so the walker for a node is always known from its
//! parent and no runtime value is ever inspected to choose an operation.
//! `Evaluator::function` returns the closure of a function value. Each
//! typed walker handles the nodes of its kind and also loads, captures,
//! blocks, `if`s, field accesses, calls and awaits, which can be of any
//! kind: an `if` evaluates its condition through `bool` and then exactly
//! one branch through the walker it was called with, and a field access, a
//! call or an await produces a [`Value`] from which the walker extracts its
//! kind's. A general walker, `Evaluator::value`, handles every node, and
//! hands the typed nodes to the typed walkers, building a [`Value`] only
//! where a value of statically unknown kind is needed: at the root, in a
//! slot, in a record, in a closure's or a future's captures, as an
//! interpolated value, as a block's or a function body's result evaluated
//! through `value`, or where the value may be a future (see
//! [Futures](#futures)).
//!
//! A record is built from its record block's shared [`crate::value::Layout`]
//! and its field values in layout order, the one allocation for the values
//! and one for the reference-counted record. The empty record is unboxed,
//! so the statements that produce it allocate nothing. Reading a field finds
//! it by comparing interned names against the layout's.
//!
//! An `UpdateField` builds a new record and never changes the old one. It
//! clones the old record's values into a new boxed slice, one allocation,
//! with the updated field's value replaced or appended. A replacement
//! shares the old record's layout. An addition needs a new layout: the old
//! one's names and texts with the added name after them, whose text the
//! evaluator takes from [`Ir::name_text`], since it has no names table and
//! must never need one. Every addition of a name to a record of a given
//! layout makes an equal layout anew; caching the result per old layout and
//! name is the optimisation for a bytecode VM, which will have a place to
//! keep such a cache at each update site. Updating the empty record makes a
//! record of the one field.
//!
//! Typed equality, `RecordEq` and `RecordNe`, compares two records over the
//! [`crate::ir::Shape`] of their static type: for each field of the shape,
//! it looks the field up in both records by name, compares a record field
//! through its own shape and any other field with `==` on [`Value`], and
//! ignores every field the shape lacks. Nested records are compared through
//! a work list, as total equality compares them (see
//! [`crate::value::Record`]), so nesting does not recurse and comparing flat
//! records allocates nothing. Total equality, `RecordSame` and
//! `RecordNotSame`, is [`Value`]'s `PartialEq`.
//!
//! `and` and `or` short-circuit: the right operand is evaluated only when
//! the left does not decide the result. A string constant is a reference
//! to the IR's copy, so producing one increments a reference count and
//! copies nothing. Concatenation allocates exactly the length it needs, and
//! an interpolation builds its text in one buffer sized from its literal
//! pieces before sharing it.
//!
//! Variables live in frames on one value stack, a `Vec<Value>`: the top
//! level's frame at its bottom, sized by the checker's slot assignment, and
//! each call's frame above its caller's. A local [`Slot`] is an offset from
//! the current frame's base. A `Load` in a typed context reads its slot and
//! extracts the unboxed value, cloning a string's reference; a slot holding
//! a value of another type there is an internal error, since lowering typed
//! the load. Storing values in separate typed slots (`Vec<i64>`,
//! `Vec<f64>`) would avoid the tag check; that is a later optimisation for
//! a bytecode VM, not something this tree walker needs.
//!
//! # Calls
//!
//! A closure is built by `MakeClosure` from its captured values, read in
//! the creating frame into one boxed slice, and a reference to the IR,
//! which the closure keeps alive: two allocations. `LoadCapture` reads the
//! running closure's captures, and `LoadCallee` yields the running closure
//! itself, which the evaluator holds while it runs, so a recursive function
//! refers to itself without holding itself.
//!
//! A call evaluates its callee to a closure through the `function` walker,
//! checks the nesting budget, and grows the stack by the callee's frame
//! size with `Vec::resize`; then it evaluates each argument in the caller's
//! frame, in source order, into its parameter's slot of the new frame, runs
//! the body in a new `Evaluator` whose base is the new frame's, and finally
//! truncates the stack back, whether the body succeeded or failed. The
//! stack keeps its capacity, so once it has grown to a program's deepest
//! recursion, a call allocates nothing: not its frame, not its arguments'
//! slots, and nothing to look anything up, since every variable is a slot
//! or a capture index. `call` keeps only the closure, its frame's base and
//! the new evaluator across the body's evaluation.
//!
//! Calls nest on the Rust stack, one `call` and a few walker frames per
//! GCSL call, and so does every level of expression nesting between a
//! frame's body and a call in it; GCSL does not eliminate tail calls. So
//! calls and expressions share one budget of nesting levels,
//! [`MAX_DEPTH`], which keeps a program within [`crate::STACK_SIZE`]
//! whatever shape its recursion has. Each evaluator carries `levels`, the
//! nesting its enclosing frames have consumed where its frame's body
//! starts: 0 at the top level. A call at depth `d` in its frame's body (see
//! [`IrKind::Call`]), of a function whose body's height is `h` (see
//! [`crate::ir::FnCode::height`]), enters its callee's frame at
//! `entering = levels + d + CALL_LEVELS` levels, and fails with "call
//! nested too deeply" if `entering + h` exceeds `MAX_DEPTH`, before its
//! arguments are evaluated; otherwise the callee's body runs with
//! `levels = entering`. The whole height of the body is charged up front,
//! so that nothing inside the body, however deep, needs a check of its own:
//! the budget costs the evaluator three additions and a comparison per
//! call, and nothing per node.
//!
//! A function defined by an earlier input of a session can fail while a
//! later input runs; its error's spans then refer to the earlier input's
//! source. When an error comes out of the body of a closure whose IR is of
//! another input than the caller's, the call records that input on the
//! diagnostic, [`Diagnostic::input`], unless an inner call has already, so
//! the innermost foreign frame wins.
//!
//! # Futures
//!
//! `MakeFuture` builds a future as `MakeClosure` builds a closure: its
//! captured values in one boxed slice and a reference to the IR, two
//! allocations. Nothing runs. Only the general walker evaluates a node of
//! kind [`crate::value::ValueKind::Deferred`], whose value may be a future;
//! every typed walker treats one as mistyped, so no typed walker ever
//! inspects a value to see whether it is a future, and code that never
//! awaits pays nothing for futures. An `Await` node, placed by the checker
//! wherever such a value is used as its bits, evaluates its value through
//! `value`, and while it is a future, runs it: the future equivalent of a
//! call, `run_future`, charged to the nesting budget as a call of a
//! parameterless function would be, the await's depth in its frame's body
//! plus [`CALL_LEVELS`] plus the body's height, and one more level for each
//! level of record nesting a deep await has descended to reach it. Its first
//! await runs the body in a frame of its own, on the value stack above the
//! awaiting frame's, and records the value, dropping the captures; every
//! later await is a borrow and a clone of that value. A deep await then
//! awaits the lazy fields its shape lists, recursively, building a new
//! record, sharing the layout and the untouched values, only if a field
//! changed. An error inside a body leaves the future unrun, with its
//! captures, so a later await runs it again, and gets the context label
//! "awaited here" on the await that ran it, when both are of the same
//! session input; otherwise the future's input is stamped on it, as a call
//! stamps a closure's. A future is marked as running while its body runs,
//! so that an await of it from inside its own body, which the checker never
//! lets through, is an error rather than an endless loop.
//!
//! Operands are evaluated left to right, so when both would fail, the left
//! one's error is reported. Items are evaluated in order, and so are a
//! call's arguments, whatever parameters they fill. Evaluation holds no
//! per-node state and allocates nothing beyond the stack, except the
//! strings, records and closures the program builds and the diagnostic
//! when it fails. A runtime error points at the operator and labels each
//! operand with the value it had, so the user sees what actually
//! overflowed or divided by zero.

use std::fmt::Write as _;
use std::num::NonZeroI64;
use std::rc::Rc;

use crate::ast::FnId;
use crate::check::Slot;
use crate::diagnostic::Diagnostic;
use crate::ir::{
    Cmp, Ir, IrId, IrKind, IrNode, IrRange, LayoutId, LazyField, LazyShapeId, Operands, ShapeId,
    ShapedOperands,
};
use crate::names::Name;
use crate::source::Span;
use crate::value::{
    Begin, Closure, Future, FutureState, Layout, Record, Value, ValueKind, int_to_float,
};
use crate::{CALL_LEVELS, MAX_DEPTH};

/// The room an interpolation reserves for each interpolated value's text,
/// on top of its literal pieces' lengths.
const INTERPOLATED_VALUE_ESTIMATE: usize = 8;

/// Evaluates `ir` to a value, on a fresh stack.
///
/// # Errors
///
/// Returns a runtime error at the failing operator's span: "integer
/// overflow" for `Int` arithmetic that leaves the range of `i64`, and
/// "division by zero" for `//` or `%` by zero; and at a call's or an
/// await's span, "call nested too deeply" for a call, or an await running a
/// future, that would take the program's nesting, calls included, past
/// [`MAX_DEPTH`] levels (see the module documentation). An error inside a
/// future's body is reported where it happens, as one inside a function
/// is. `Float` arithmetic never fails.
pub fn eval(ir: &Rc<Ir>) -> Result<Value, Diagnostic> {
    // The top level's frame; calls grow the stack beyond it.
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    eval_in(ir, &mut stack)
}

/// Evaluates `ir` to a value with an existing `stack`, which must hold at
/// least [`Ir::frame_size`] values: the top level's frame, at its bottom.
/// Slots the program does not write keep their values, which is how a
/// [`crate::session::Session`] keeps its variables from one input to the
/// next. On return, whether evaluation succeeded or failed, the stack is
/// truncated to the frame size, dropping whatever calls left above it; its
/// capacity is kept, so a later evaluation's calls need not allocate it
/// again.
///
/// # Errors
///
/// As [`eval`]; and an internal error if `stack` is too small.
pub fn eval_in(ir: &Rc<Ir>, stack: &mut Vec<Value>) -> Result<Value, Diagnostic> {
    let root = ir.root();
    let frame_size = ir.frame_size() as usize;
    if stack.len() < frame_size {
        return Err(internal(
            "frame is smaller than the IR needs",
            ir.get(root).span,
        ));
    }
    let result = Evaluator {
        ir,
        stack,
        base: 0,
        captures: &[],
        callee: None,
        levels: 0,
    }
    .value(root);
    stack.truncate(frame_size);
    result
}

/// The state of one frame's evaluation: the top level's, or a call's.
struct Evaluator<'a> {
    /// The IR being run: the program's, or the running closure's.
    ir: &'a Rc<Ir>,
    /// The value stack, holding every frame.
    stack: &'a mut Vec<Value>,
    /// Where this frame starts on the stack: slot `n` is `stack[base + n]`.
    base: usize,
    /// The running closure's captured values; none at the top level.
    captures: &'a [Value],
    /// The running closure; `None` at the top level.
    callee: Option<&'a Rc<Closure>>,
    /// The levels of nesting the enclosing frames have consumed where this
    /// frame's body starts: 0 at the top level. See the module
    /// documentation.
    levels: usize,
}

// The walkers `value`, `int`, `float`, `bool`, `string` and `record` are
// the recursion of evaluation: an expression nested N deep puts about N of
// their frames on the stack. Each is therefore only a dispatcher. The work
// of each operator is in a helper of its own, and every error is returned
// by a call in tail position, so a walker's frame does not hold the
// temporaries of every operator; an unoptimised build gives each of those
// a stack slot of its own, which would make the walkers' frames several
// kilobytes and `crate::STACK_SIZE` too small at `crate::MAX_DEPTH`.
impl Evaluator<'_> {
    /// Evaluates a node of any kind.
    fn value(&mut self, id: IrId) -> Result<Value, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty => Ok(Value::Empty),
            IrKind::Load(slot, _) => self.load_value(slot, node),
            IrKind::Store(slot, value) => self.store(slot, value, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.value(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.value(branch)
            }
            IrKind::MakeRecord { layout, values } => self.make_record(layout, values),
            IrKind::Field { record, name, .. } => self.field(record, name, node),
            IrKind::UpdateField {
                record,
                name,
                value,
            } => self.update_field((record, name, value), node),
            IrKind::LoadCapture(index, _) => self.capture_value(index, node),
            IrKind::LoadCallee | IrKind::MakeClosure { .. } => self.closure_value(node),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call((callee, args, depth), node),
            IrKind::MakeFuture { function, captures } => self.make_future(function, captures),
            IrKind::Await { .. } => self.await_node(node),
            IrKind::Arg { .. } => mistyped(node, "an argument outside a call"),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => self.int(id).map(Value::Int),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => self.float(id).map(Value::Float),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => self.bool(id).map(Value::Bool),
            IrKind::StrConst(_) | IrKind::StrConcat(_) => self.string(id).map(Value::Str),
            // Directly rather than through `string`: an interpolation
            // evaluates its parts through `value`, so interpolations nested
            // in each other then recurse two frames per level, not three.
            IrKind::Interp(items) => self.interpolate(items, node.span).map(Value::Str),
        }
    }

    /// Evaluates a block's items, in order, for their effects.
    fn items(&mut self, items: IrRange) -> Result<(), Diagnostic> {
        for &item in self.ir.items(items) {
            self.value(item)?;
        }
        Ok(())
    }

    /// Evaluates the condition of an `if` and returns the branch it selects.
    fn branch(&mut self, cond: IrId, then: IrId, else_: IrId) -> Result<IrId, Diagnostic> {
        Ok(if self.bool(cond)? { then } else { else_ })
    }

    /// Evaluates `value` and writes it to `slot`, for the `Store` `node`.
    fn store(&mut self, slot: Slot, value: IrId, node: &IrNode) -> Result<Value, Diagnostic> {
        let value = self.value(value)?;
        let target = self
            .stack
            .get_mut(self.base + slot.index())
            .ok_or_else(|| internal("store to a slot outside the frame", node.span))?;
        *target = value;
        Ok(Value::Empty)
    }

    /// The captured value number `index` of the running closure, for the
    /// `LoadCapture` `node`.
    fn capture(&self, index: u32, node: &IrNode) -> Result<&Value, Diagnostic> {
        self.captures
            .get(index as usize)
            .ok_or_else(|| internal("load of a capture the closure lacks", node.span))
    }

    /// A copy of the value in `slot`, of any kind, for the `Load` `node`.
    fn load_value(&self, slot: Slot, node: &IrNode) -> Result<Value, Diagnostic> {
        self.load(slot, node).cloned()
    }

    /// A copy of the captured value number `index`, of any kind, for the
    /// `LoadCapture` `node`.
    fn capture_value(&self, index: u32, node: &IrNode) -> Result<Value, Diagnostic> {
        self.capture(index, node).cloned()
    }

    /// The value in `slot`, read by the `Load` `node` as `extract` accepts
    /// it. The walkers read slots through this and `capture_as`, returning
    /// the call, so that they hold no temporaries for the reads.
    fn slot_as<T>(&self, slot: Slot, node: &IrNode, extract: Extract<T>) -> Result<T, Diagnostic> {
        extract(self.load(slot, node)?, node)
    }

    /// The captured value number `index`, read by the `LoadCapture` `node`
    /// as `extract` accepts it.
    fn capture_as<T>(
        &self,
        index: u32,
        node: &IrNode,
        extract: Extract<T>,
    ) -> Result<T, Diagnostic> {
        extract(self.capture(index, node)?, node)
    }

    /// The closure the `LoadCallee` or `MakeClosure` `node` yields, as a
    /// value.
    fn closure_value(&mut self, node: &IrNode) -> Result<Value, Diagnostic> {
        match node.kind {
            IrKind::LoadCallee => self.callee(node).map(Value::Fn),
            IrKind::MakeClosure { function, captures } => {
                self.make_closure(function, captures).map(Value::Fn)
            }
            _ => mistyped(node, "a node that is not a closure where one was expected"),
        }
    }

    /// A new reference to the running closure, for the `LoadCallee`
    /// `node`.
    fn callee(&self, node: &IrNode) -> Result<Rc<Closure>, Diagnostic> {
        self.callee
            .map(Rc::clone)
            .ok_or_else(|| internal("load of the callee outside a function", node.span))
    }

    /// Builds a closure of `function` capturing the values of `captures`,
    /// read in this frame: one allocation for the captured values, sized
    /// exactly (none if there are none), and one for the closure, which
    /// shares the IR.
    fn make_closure(
        &mut self,
        function: FnId,
        captures: IrRange,
    ) -> Result<Rc<Closure>, Diagnostic> {
        let items = self.ir.items(captures);
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.value(item)?);
        }
        Ok(Rc::new(Closure {
            code: Rc::clone(self.ir),
            function,
            captures: values.into_boxed_slice(),
        }))
    }

    /// Builds a future of `function` capturing the values of `captures`,
    /// read in this frame, as [`Evaluator::make_closure`] builds a closure:
    /// one allocation for the captured values and one for the future, which
    /// shares the IR. Nothing is run.
    fn make_future(&mut self, function: FnId, captures: IrRange) -> Result<Value, Diagnostic> {
        let items = self.ir.items(captures);
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.value(item)?);
        }
        Ok(Value::Future(Rc::new(Future::new(
            Rc::clone(self.ir),
            function,
            values.into_boxed_slice(),
        ))))
    }

    /// Evaluates the `Await` `node` (see the module documentation) as a
    /// value, which must be of the kind the node states.
    fn await_node(&mut self, node: &IrNode) -> Result<Value, Diagnostic> {
        let value = self.await_value(node)?;
        match node.kind {
            IrKind::Await { kind, .. } if value.kind() == kind => Ok(value),
            _ => mistyped(
                node,
                "an await produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `Await` `node` as a value that `extract` accepts: one
    /// of the kind the node states.
    fn await_as<T>(
        &mut self,
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Result<T, Diagnostic> {
        match extract(self.await_value(node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "an await produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `Await` `node`: its value, through the general walker,
    /// awaited fully and then, if it has a lazy shape, deeply.
    fn await_value(&mut self, node: &IrNode) -> Result<Value, Diagnostic> {
        let IrKind::Await {
            value,
            shape,
            depth,
            ..
        } = node.kind
        else {
            return mistyped(node, "a node that is not an await awaited");
        };
        let awaited = self.value(value)?;
        let awaited = self.await_fully(awaited, depth, 0, node)?;
        if shape == LazyShapeId::NONE {
            Ok(awaited)
        } else {
            // The fields of the record awaited are one level of nesting
            // below it.
            self.await_deep(awaited, shape, depth, 1, node)
        }
    }

    /// Awaits `value` fully, for the await `node` at depth `depth` in this
    /// frame's body, `level` levels of record nesting below the value it
    /// awaits: while it is a future, runs it and continues with what it
    /// yields.
    fn await_fully(
        &mut self,
        value: Value,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Result<Value, Diagnostic> {
        let mut value = value;
        loop {
            match value {
                Value::Future(future) => value = self.run_future(&future, depth, level, node)?,
                present => return Ok(present),
            }
        }
    }

    /// Awaits the lazy fields of `value`, a record, as lazy shape `shape`
    /// lists them, for the await `node` at depth `depth` in this frame's
    /// body, the record being `level` levels of record nesting below the
    /// value the await awaits: each field that is lazy is awaited fully and
    /// then, if its shape says so, deeply, and each present record field
    /// with lazy fields inside is awaited deeply. If any field changed, the
    /// result is a new record sharing the layout and the values that did
    /// not; otherwise it is the record as it was. The empty record has no
    /// fields to await.
    fn await_deep(
        &mut self,
        value: Value,
        shape: LazyShapeId,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Result<Value, Diagnostic> {
        let Value::Record(record) = &value else {
            return Ok(value);
        };
        let ir = self.ir;
        let Some(lazy) = ir.find_lazy_shape(shape) else {
            return mistyped(node, "an await of a lazy shape its IR lacks");
        };
        let mut changed: Option<Vec<Value>> = None;
        for &(name, field) in &lazy.fields {
            let found = record.layout.names.iter().position(|&field| field == name);
            let Some((index, old)) =
                found.and_then(|index| Some((index, record.values.get(index)?)))
            else {
                return mistyped(node, "a record lacks a field of its lazy shape");
            };
            let new = match field {
                LazyField::Await(inner) => {
                    let awaited = self.await_fully(old.clone(), depth, level, node)?;
                    if inner == LazyShapeId::NONE {
                        awaited
                    } else {
                        self.await_deep(awaited, inner, depth, level + 1, node)?
                    }
                }
                LazyField::Within(inner) => {
                    self.await_deep(old.clone(), inner, depth, level + 1, node)?
                }
            };
            let differs = match (old, &new) {
                (Value::Future(_), _) => true,
                (Value::Record(old), Value::Record(new)) => !Rc::ptr_eq(old, new),
                _ => false,
            };
            if differs {
                let values = changed.get_or_insert_with(|| record.values.to_vec());
                if let Some(slot) = values.get_mut(index) {
                    *slot = new;
                }
            }
        }
        match changed {
            Some(values) => Ok(Value::Record(Rc::new(Record {
                layout: Rc::clone(&record.layout),
                values: values.into_boxed_slice(),
            }))),
            None => Ok(value),
        }
    }

    /// Runs `future` for the await `node` at depth `depth` in this frame's
    /// body, `level` levels of record nesting below the value the await
    /// awaits (see the module documentation), and returns what it yields:
    /// the value it recorded, if it has run; otherwise its body's value,
    /// run in a frame of its own, which it then records. The future
    /// equivalent of [`Evaluator::call`]: it keeps only the future, its
    /// frame's base and the new evaluator across the body's evaluation.
    fn run_future(
        &mut self,
        future: &Rc<Future>,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Result<Value, Diagnostic> {
        let Some(code) = future.code.find_function(future.function) else {
            return mistyped(node, "a future of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + level + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node));
        }
        let captures = match future.begin() {
            Some(Begin::Done(value)) => return Ok(value),
            Some(Begin::Running) => return Err(depends_on_itself(node)),
            Some(Begin::Pending(captures)) => captures,
            None => return mistyped(node, "a future's state is in use"),
        };
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        let result = Evaluator {
            ir: &future.code,
            stack: self.stack,
            base,
            captures: &captures,
            callee: None,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        match result {
            Ok(value) => {
                // The captures are dropped on return: a future that has
                // run holds only its value.
                if future.finish(FutureState::Done(value.clone())) {
                    Ok(value)
                } else {
                    mistyped(node, "a future's state is in use")
                }
            }
            Err(error) => {
                // Left unrun, so that a later await runs it again.
                if future.finish(FutureState::Pending(captures)) {
                    Err(self.awaited_error(error, future, node))
                } else {
                    mistyped(node, "a future's state is in use")
                }
            }
        }
    }

    /// `error`, raised while `future` ran for the await `node`, as it is
    /// reported: labelled "awaited here" at the await when the future's
    /// code is of the same session input as this frame's, and stamped with
    /// the future's input when it is not, unless an inner frame has stamped
    /// an input on it already.
    fn awaited_error(&self, mut error: Diagnostic, future: &Future, node: &IrNode) -> Diagnostic {
        if error.input.is_some() {
            return error;
        }
        if future.code.input() == self.ir.input() {
            error.with_context(node.span, "awaited here")
        } else {
            error.input = Some(future.code.input());
            error
        }
    }

    /// Evaluates the `Call` `node` of `callee` with the arguments `args`,
    /// at depth `depth` in this frame's body (see the module documentation),
    /// and returns the body's value.
    fn call(
        &mut self,
        (callee, args, depth): (IrId, IrRange, u16),
        node: &IrNode,
    ) -> Result<Value, Diagnostic> {
        let closure = self.function(callee)?;
        let Some(code) = closure.code.find_function(closure.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node));
        }
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        if let Err(error) = self.arguments(args, base) {
            self.stack.truncate(base);
            return Err(error);
        }
        let result = Evaluator {
            ir: &closure.code,
            stack: self.stack,
            base,
            captures: &closure.captures,
            callee: Some(&closure),
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        match result {
            Err(mut error) if error.input.is_none() && closure.code.input() != self.ir.input() => {
                error.input = Some(closure.code.input());
                Err(error)
            }
            result => result,
        }
    }

    /// Evaluates the arguments `args` of a call, `Arg` nodes, in order, in
    /// this frame, writing each to its parameter's slot of the frame that
    /// starts at `base`.
    fn arguments(&mut self, args: IrRange, base: usize) -> Result<(), Diagnostic> {
        for &arg in self.ir.items(args) {
            let node = self.ir.get(arg);
            let IrKind::Arg { param, value } = node.kind else {
                return mistyped(node, "a call's argument is not an argument");
            };
            let value = self.value(value)?;
            let Some(slot) = self.stack.get_mut(base + param as usize) else {
                return mistyped(node, "an argument for a parameter outside the frame");
            };
            *slot = value;
        }
        Ok(())
    }

    /// Evaluates the `Call` `node` of `callee` with the arguments `args`, at
    /// depth `depth`, as a value that `extract` accepts: one of the kind the
    /// node states.
    fn call_as<T>(
        &mut self,
        call: (IrId, IrRange, u16),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Result<T, Diagnostic> {
        match extract(self.call(call, node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "a call returned a value of another kind than its node",
            ),
        }
    }

    /// Evaluates a function-typed node to its closure.
    fn function(&mut self, id: IrId) -> Result<Rc<Closure>, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_FUNCTION),
            IrKind::Await { .. } => self.await_as(node, |value| match value {
                Value::Fn(closure) => Some(closure),
                _ => None,
            }),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_function),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_function),
            IrKind::LoadCallee => self.callee(node),
            IrKind::MakeClosure { function, captures } => self.make_closure(function, captures),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.function(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.function(branch)
            }
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Fn(closure) => Some(closure),
                    _ => None,
                })
            }
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, |value| match value {
                Value::Fn(closure) => Some(closure),
                _ => None,
            }),
            _ => mistyped(node, "a node of another kind where a function was expected"),
        }
    }

    /// Builds a record of the values of `values`, in the order of `layout`'s
    /// names: the one allocation for its values, sized exactly, and one for
    /// the record, which shares the layout.
    fn make_record(&mut self, layout: LayoutId, values: IrRange) -> Result<Value, Diagnostic> {
        let items = self.ir.items(values);
        let mut fields = Vec::with_capacity(items.len());
        for &item in items {
            fields.push(self.value(item)?);
        }
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::clone(self.ir.layout(layout)),
            values: fields.into_boxed_slice(),
        })))
    }

    /// Evaluates `record` and returns its field `name`, for the `Field`
    /// `node`. The checker guarantees the field is there, so a missing
    /// one, or an empty record, is an internal error.
    fn field(&mut self, record: IrId, name: Name, node: &IrNode) -> Result<Value, Diagnostic> {
        match self.record(record)? {
            Value::Record(fields) => match fields.get(name) {
                Some(value) => Ok(value.clone()),
                None => mistyped(node, "a record lacks a field the checker found"),
            },
            _ => mistyped(node, "a field of the empty record"),
        }
    }

    /// Evaluates the `Field` `node`, reading field `name` of `record`, as a
    /// value that `extract` accepts: a field of the kind the node states.
    fn field_as<T>(
        &mut self,
        (record, name): (IrId, Name),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Result<T, Diagnostic> {
        match extract(self.field(record, name, node)?) {
            Some(value) => Ok(value),
            None => mistyped(node, "a field holds a value of another kind than its node"),
        }
    }

    /// The value in `slot` of this frame.
    fn load(&self, slot: Slot, node: &IrNode) -> Result<&Value, Diagnostic> {
        self.stack
            .get(self.base + slot.index())
            .ok_or_else(|| internal("load from a slot outside the frame", node.span))
    }

    /// Evaluates an `Int`-typed node.
    fn int(&mut self, id: IrId) -> Result<i64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::IntConst(value) => Ok(value),
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_INT),
            IrKind::Await { .. } => self.await_as(node, |value| match value {
                Value::Int(value) => Some(value),
                _ => None,
            }),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_int),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_int),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, |value| match value {
                Value::Int(value) => Some(value),
                _ => None,
            }),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.int(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.int(branch)
            }
            IrKind::IntNeg(operand) => self.int_neg(node, operand),
            IrKind::IntAdd(operands) => self.int_arithmetic(node, operands, i64::checked_add),
            IrKind::IntSub(operands) => self.int_arithmetic(node, operands, i64::checked_sub),
            IrKind::IntMul(operands) => self.int_arithmetic(node, operands, i64::checked_mul),
            IrKind::IntFloorDiv(operands) => self.int_division(node, operands, floor_div),
            IrKind::IntRem(operands) => {
                self.int_division(node, operands, |lhs, rhs| Some(floor_mod(lhs, rhs)))
            }
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Int(value) => Some(value),
                    _ => None,
                })
            }
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where an Int was expected"),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where an Int was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where an Int was expected")
            }
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => mistyped(node, "record node where an Int was expected"),
            IrKind::MakeClosure { .. } | IrKind::LoadCallee | IrKind::Arg { .. } => {
                mistyped(node, "function node where an Int was expected")
            }
        }
    }

    /// Evaluates both `Int` operands, left to right.
    fn ints(&mut self, operands: Operands) -> Result<(i64, i64), Diagnostic> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        Ok((lhs, rhs))
    }

    /// Evaluates the `IntNeg` `node` of `operand`.
    fn int_neg(&mut self, node: &IrNode, operand: IrId) -> Result<i64, Diagnostic> {
        let value = self.int(operand)?;
        value
            .checked_neg()
            .ok_or_else(|| self.unary_error(node, operand, OVERFLOW, value))
    }

    /// Evaluates the `+`, `-` or `*` `node`, which `op` computes, `None`
    /// meaning overflow.
    fn int_arithmetic(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, i64) -> Option<i64>,
    ) -> Result<i64, Diagnostic> {
        // Not through `ints`: this is on the recursion path of every
        // arithmetic chain, so it saves that frame.
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        op(lhs, rhs).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
    }

    /// Evaluates the `//` or `%` `node`, which `op` computes for a nonzero
    /// divisor, `None` meaning overflow.
    fn int_division(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, NonZeroI64) -> Option<i64>,
    ) -> Result<i64, Diagnostic> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        let Some(divisor) = NonZeroI64::new(rhs) else {
            return Err(self.binary_error(node, operands, DIVISION_BY_ZERO, lhs, rhs));
        };
        op(lhs, divisor).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs))
    }

    /// Evaluates a `Float`-typed node. IEEE 754 semantics; never fails except
    /// through an `Int` operand of `IntToFloat`, an item of a block or the
    /// condition of an `if`.
    fn float(&mut self, id: IrId) -> Result<f64, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::FloatConst(value) => Ok(value),
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_FLOAT),
            IrKind::Await { .. } => self.await_as(node, |value| match value {
                Value::Float(value) => Some(value),
                _ => None,
            }),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_float),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_float),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, |value| match value {
                Value::Float(value) => Some(value),
                _ => None,
            }),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.float(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.float(branch)
            }
            IrKind::FloatNeg(operand) => self.float_neg(operand),
            IrKind::FloatAdd(o) => self.float_arithmetic(o, |lhs, rhs| lhs + rhs),
            IrKind::FloatSub(o) => self.float_arithmetic(o, |lhs, rhs| lhs - rhs),
            IrKind::FloatMul(o) => self.float_arithmetic(o, |lhs, rhs| lhs * rhs),
            IrKind::FloatDiv(o) => self.float_arithmetic(o, |lhs, rhs| lhs / rhs),
            IrKind::IntToFloat(operand) => self.int(operand).map(int_to_float),
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Float(value) => Some(value),
                    _ => None,
                })
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a Float was expected"),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a Float was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where a Float was expected")
            }
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => {
                mistyped(node, "record node where a Float was expected")
            }
            IrKind::MakeClosure { .. } | IrKind::LoadCallee | IrKind::Arg { .. } => {
                mistyped(node, "function node where a Float was expected")
            }
        }
    }

    /// Evaluates a `FloatNeg` of `operand`.
    fn float_neg(&mut self, operand: IrId) -> Result<f64, Diagnostic> {
        Ok(-self.float(operand)?)
    }

    /// Evaluates a `Float` operator that `op` computes.
    fn float_arithmetic(
        &mut self,
        operands: Operands,
        op: fn(f64, f64) -> f64,
    ) -> Result<f64, Diagnostic> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(op(lhs, rhs))
    }

    /// Evaluates a `Bool`-typed node. `and` and `or` evaluate their right
    /// operand only when the left does not decide the result.
    fn bool(&mut self, id: IrId) -> Result<bool, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::BoolConst(value) => Ok(value),
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_BOOL),
            IrKind::Await { .. } => self.await_as(node, |value| match value {
                Value::Bool(value) => Some(value),
                _ => None,
            }),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_bool),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_bool),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, |value| match value {
                Value::Bool(value) => Some(value),
                _ => None,
            }),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.bool(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.bool(branch)
            }
            IrKind::IntCmp(cmp, o) => self.int_cmp(cmp, o),
            IrKind::FloatCmp(cmp, o) => self.float_cmp(cmp, o),
            IrKind::StrCmp(cmp, o) => self.str_cmp(cmp, o),
            IrKind::BoolEq(equal, o) => self.bool_eq(equal, o),
            IrKind::RecordEq(o) => self.record_eq(o, true, node),
            IrKind::RecordNe(o) => self.record_eq(o, false, node),
            IrKind::RecordSame(o) => self.record_same(o, true),
            IrKind::RecordNotSame(o) => self.record_same(o, false),
            IrKind::And(o) => self.logic(o, false),
            IrKind::Or(o) => self.logic(o, true),
            IrKind::Not(operand) => self.not(operand),
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Bool(value) => Some(value),
                    _ => None,
                })
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a Bool was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a Bool was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where a Bool was expected")
            }
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => mistyped(node, "record node where a Bool was expected"),
            IrKind::MakeClosure { .. } | IrKind::LoadCallee | IrKind::Arg { .. } => {
                mistyped(node, "function node where a Bool was expected")
            }
        }
    }

    /// Evaluates an `IntCmp` testing `cmp`.
    fn int_cmp(&mut self, cmp: Cmp, operands: Operands) -> Result<bool, Diagnostic> {
        let (lhs, rhs) = self.ints(operands)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `FloatCmp` testing `cmp`.
    fn float_cmp(&mut self, cmp: Cmp, operands: Operands) -> Result<bool, Diagnostic> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `StrCmp` testing `cmp`.
    fn str_cmp(&mut self, cmp: Cmp, operands: Operands) -> Result<bool, Diagnostic> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        Ok(cmp.test(&*lhs, &*rhs))
    }

    /// Evaluates a `BoolEq`: `==` if `equal`, `!=` otherwise.
    fn bool_eq(&mut self, equal: bool, operands: Operands) -> Result<bool, Diagnostic> {
        let lhs = self.bool(operands.lhs)?;
        let rhs = self.bool(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates the typed record comparison `node`, a `RecordEq` if `equal`
    /// and a `RecordNe` otherwise: the operands compare over their shape
    /// (see [`record_eq_shaped`]).
    fn record_eq(
        &mut self,
        operands: ShapedOperands,
        equal: bool,
        node: &IrNode,
    ) -> Result<bool, Diagnostic> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        match record_eq_shaped(self.ir, operands.shape, &lhs, &rhs) {
            Some(same) => Ok(same == equal),
            None => mistyped(node, "a record lacks a field of its shape"),
        }
    }

    /// Evaluates a total record comparison, a `RecordSame` if `equal` and a
    /// `RecordNotSame` otherwise: records compare whole, deeply and by field
    /// name; empty records are all equal.
    fn record_same(&mut self, operands: Operands, equal: bool) -> Result<bool, Diagnostic> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates `and`, which a `false` left operand decides, or `or`, which
    /// a `true` one decides, as `decisive` says. The right operand is only
    /// evaluated when the left does not decide.
    fn logic(&mut self, operands: Operands, decisive: bool) -> Result<bool, Diagnostic> {
        let lhs = self.bool(operands.lhs)?;
        if lhs == decisive {
            Ok(lhs)
        } else {
            self.bool(operands.rhs)
        }
    }

    /// Evaluates a `Not` of `operand`.
    fn not(&mut self, operand: IrId) -> Result<bool, Diagnostic> {
        Ok(!self.bool(operand)?)
    }

    /// Evaluates a `String`-typed node.
    fn string(&mut self, id: IrId) -> Result<Rc<str>, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::StrConst(constant) => Ok(Rc::clone(self.ir.string(constant))),
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_STRING),
            IrKind::Await { .. } => self.await_as(node, |value| match value {
                Value::Str(text) => Some(text),
                _ => None,
            }),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_string),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_string),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, |value| match value {
                Value::Str(text) => Some(text),
                _ => None,
            }),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.string(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.string(branch)
            }
            IrKind::StrConcat(operands) => self.concat(operands),
            IrKind::Interp(items) => self.interpolate(items, node.span),
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Str(text) => Some(text),
                    _ => None,
                })
            }
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a String was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a String was expected"),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a String was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. } => {
                mistyped(node, "record node where a String was expected")
            }
            IrKind::MakeClosure { .. } | IrKind::LoadCallee | IrKind::Arg { .. } => {
                mistyped(node, "function node where a String was expected")
            }
        }
    }

    /// Evaluates a record node: [`Value::Empty`] or a [`Value::Record`].
    fn record(&mut self, id: IrId) -> Result<Value, Diagnostic> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty => Ok(Value::Empty),
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_RECORD),
            IrKind::Await { .. } => self.await_as(node, |value| match value {
                Value::Empty | Value::Record(_) => Some(value),
                _ => None,
            }),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_record),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_record),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, |value| match value {
                Value::Empty | Value::Record(_) => Some(value),
                _ => None,
            }),
            IrKind::Store(slot, value) => self.store(slot, value, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.record(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.record(branch)
            }
            IrKind::MakeRecord { layout, values } => self.make_record(layout, values),
            IrKind::Field { record, name, .. } => {
                self.field_as((record, name), node, |value| match value {
                    Value::Empty | Value::Record(_) => Some(value),
                    _ => None,
                })
            }
            IrKind::UpdateField {
                record,
                name,
                value,
            } => self.update_field((record, name, value), node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_) => mistyped(node, "Int node where a record was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a record was expected"),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => mistyped(node, "Bool node where a record was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) => {
                mistyped(node, "String node where a record was expected")
            }
            IrKind::MakeClosure { .. } | IrKind::LoadCallee | IrKind::Arg { .. } => {
                mistyped(node, "function node where a record was expected")
            }
        }
    }

    /// Evaluates the `UpdateField` `node`, setting field `name` of `record`
    /// to `value`: a new record, the old one unchanged. See the module
    /// documentation.
    fn update_field(
        &mut self,
        (record, name, value): (IrId, Name, IrId),
        node: &IrNode,
    ) -> Result<Value, Diagnostic> {
        let old = self.record(record)?;
        let value = self.value(value)?;
        let old = match old {
            Value::Record(old) => old,
            Value::Empty => {
                let Some(text) = self.ir.name_text(name) else {
                    return mistyped(node, "an update's name has no text");
                };
                return Ok(Value::Record(Rc::new(Record {
                    layout: Rc::new(Layout {
                        names: Box::new([name]),
                        texts: Box::new([Box::from(text)]),
                    }),
                    values: Box::new([value]),
                })));
            }
            Value::Int(_)
            | Value::Float(_)
            | Value::Bool(_)
            | Value::Str(_)
            | Value::Fn(_)
            | Value::Future(_) => {
                return mistyped(node, "an update of a value that is not a record");
            }
        };
        if let Some(index) = old.layout.names.iter().position(|&field| field == name) {
            // The same fields: the layout is shared.
            let mut values = old.values.clone();
            if let Some(slot) = values.get_mut(index) {
                *slot = value;
            }
            return Ok(Value::Record(Rc::new(Record {
                layout: Rc::clone(&old.layout),
                values,
            })));
        }
        let Some(text) = self.ir.name_text(name) else {
            return mistyped(node, "an update's name has no text");
        };
        let len = old.values.len() + 1;
        let mut names = Vec::with_capacity(len);
        names.extend_from_slice(&old.layout.names);
        names.push(name);
        let mut texts = Vec::with_capacity(len);
        texts.extend(old.layout.texts.iter().cloned());
        texts.push(Box::from(text));
        let mut values = Vec::with_capacity(len);
        values.extend(old.values.iter().cloned());
        values.push(value);
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: names.into_boxed_slice(),
                texts: texts.into_boxed_slice(),
            }),
            values: values.into_boxed_slice(),
        })))
    }

    /// Evaluates a `StrConcat`, allocating exactly the length the result
    /// needs.
    fn concat(&mut self, operands: Operands) -> Result<Rc<str>, Diagnostic> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        let mut text = String::with_capacity(lhs.len() + rhs.len());
        text.push_str(&lhs);
        text.push_str(&rhs);
        Ok(Rc::from(text))
    }

    /// Builds the text of an interpolation, spanning `span`, from its
    /// `items`: a `StrConst` is literal text, pushed as it is, and any other
    /// item is a value shown with its `Display`. The text is built in one
    /// buffer, reserved for the literal pieces and a little per value.
    fn interpolate(&mut self, items: IrRange, span: Span) -> Result<Rc<str>, Diagnostic> {
        let items = self.ir.items(items);
        let estimate = items.iter().fold(0_usize, |total, &item| {
            let len = match self.ir.get(item).kind {
                IrKind::StrConst(constant) => self.ir.string(constant).len(),
                _ => INTERPOLATED_VALUE_ESTIMATE,
            };
            total.saturating_add(len)
        });
        let mut text = String::with_capacity(estimate);
        for &item in items {
            if let IrKind::StrConst(constant) = self.ir.get(item).kind {
                text.push_str(self.ir.string(constant));
            } else {
                let value = self.value(item)?;
                write!(text, "{value}")
                    .map_err(|_| internal("showing a value as text failed", span))?;
            }
        }
        Ok(Rc::from(text))
    }

    /// A runtime error in a binary operator: the message points at the
    /// operator, and each operand is labelled with its value.
    fn binary_error(
        &self,
        node: &IrNode,
        operands: Operands,
        message: &'static str,
        lhs: i64,
        rhs: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operands.lhs).span, lhs.to_string())
            .with_context(self.ir.get(operands.rhs).span, rhs.to_string())
    }

    /// A runtime error in a unary operator, labelling the operand with its
    /// value.
    fn unary_error(
        &self,
        node: &IrNode,
        operand: IrId,
        message: &'static str,
        value: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operand).span, value.to_string())
    }
}

const OVERFLOW: &str = "integer overflow";
const DIVISION_BY_ZERO: &str = "division by zero";

// The internal errors of a typed walker meeting a node whose value may be a
// future, which only the general walker evaluates.
const DEFERRED_INT: &str = "a node that may be a future where an Int was expected";
const DEFERRED_FLOAT: &str = "a node that may be a future where a Float was expected";
const DEFERRED_BOOL: &str = "a node that may be a future where a Bool was expected";
const DEFERRED_STRING: &str = "a node that may be a future where a String was expected";
const DEFERRED_RECORD: &str = "a node that may be a future where a record was expected";
const DEFERRED_FUNCTION: &str = "a node that may be a future where a function was expected";

/// Floor division: the quotient rounded toward negative infinity. Returns
/// `None` on overflow, which only `i64::MIN // -1` causes.
///
/// Derivation. Rust's `/` truncates toward zero and `%` takes the sign of
/// the dividend, so `a == b * q + r` with `q = a / b`, `r = a % b` and
/// `|r| < |b|`. If `r == 0` the division is exact and `q` is already the
/// floor. Otherwise the exact quotient `a / b` lies strictly between `q` and
/// the next integer away from zero, and it is negative exactly when `r` and
/// `b` have opposite signs. In that case truncation rounded *up*, so the
/// floor is `q - 1`. When the signs agree the exact quotient is positive,
/// truncation rounded down, and `q` is the floor.
///
/// `q - 1` cannot overflow: it only happens when the exact quotient is
/// negative and not whole, so `q <= 0` and `q > i64::MIN`.
///
/// This is not `i64::div_euclid`, which rounds so that the remainder is
/// never negative, and so differs from floor division when `b < 0`
/// (`7.div_euclid(-2) == -3`, but `7 // -2 == -4`).
fn floor_div(a: i64, b: NonZeroI64) -> Option<i64> {
    let b = b.get();
    let q = a.checked_div(b)?;
    // Cannot wrap: `checked_div` has already rejected `i64::MIN / -1`.
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        Some(q - 1)
    } else {
        Some(q)
    }
}

/// The remainder matching [`floor_div`]: `a - b * floor_div(a, b)`, which
/// has the sign of the divisor `b` (or is zero).
///
/// Derivation. With `q`, `r` the truncating quotient and remainder as in
/// [`floor_div`], the floored quotient is `q - 1` exactly when `r != 0` and
/// `r`, `b` have opposite signs. Then the floored remainder is
/// `a - b * (q - 1) = (a - b * q) + b = r + b`, which has the sign of `b`
/// because `|r| < |b|`. Otherwise it is `r`, which is zero or already has
/// the sign of `b`. `r + b` cannot overflow, since `r` and `b` have opposite
/// signs.
///
/// Never overflows: `i64::MIN % -1` is mathematically `0`, which is what
/// `wrapping_rem` returns for it.
fn floor_mod(a: i64, b: NonZeroI64) -> i64 {
    let b = b.get();
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    }
}

/// Typed equality: whether records `a` and `b` are equal over shape
/// `shape`, comparing, for each field of the shape, the two records' fields
/// of that name, a record field through its own shape and any other with
/// `==` on [`Value`]. Fields the shape lacks are ignored, so a record with
/// no fields in its shape is equal to any other.
///
/// Nested records are compared through a work list rather than recursion,
/// as [`Record`]'s total equality does, and the list only grows when a
/// shape has a record field, so comparing flat records allocates nothing.
///
/// `None` if a record lacks a field of its shape, or holds something other
/// than a record in a field the shape has as a record: the checker
/// guarantees neither happens, so either is an internal error.
fn record_eq_shaped(ir: &Ir, shape: ShapeId, a: &Value, b: &Value) -> Option<bool> {
    let mut pending: Vec<(ShapeId, &Value, &Value)> = Vec::new();
    let (mut shape, mut a, mut b) = (shape, a, b);
    loop {
        let fields = &ir.shape(shape).fields;
        if !fields.is_empty() {
            let (Value::Record(a), Value::Record(b)) = (a, b) else {
                return None;
            };
            for &(name, nested) in fields {
                let (mine, theirs) = (a.get(name)?, b.get(name)?);
                match nested {
                    Some(nested) => pending.push((nested, mine, theirs)),
                    None if mine == theirs => {}
                    None => return Some(false),
                }
            }
        }
        let Some(next) = pending.pop() else {
            return Some(true);
        };
        (shape, a, b) = next;
    }
}

fn internal(message: &'static str, span: Span) -> Diagnostic {
    crate::diagnostic::InternalError { message, span }.into()
}

/// The error for the call `node`, which would take the program's nesting,
/// calls included, past [`MAX_DEPTH`] levels.
fn nested_too_deeply(node: &IrNode) -> Diagnostic {
    Diagnostic::error(
        format!(
            "call nested too deeply: the limit is {MAX_DEPTH} levels of nesting, calls included"
        ),
        node.span,
    )
    .with_note(
        "every level of expression nesting around a call counts, and so does the body of the \
         function called; GCSL does not eliminate tail calls, so recursion this deep is not \
         supported",
    )
}

/// The error for the await `node` of a future whose body is running: an
/// await of the future from inside its own body.
fn depends_on_itself(node: &IrNode) -> Diagnostic {
    Diagnostic::error(
        "this deferred expression depends on its own value",
        node.span,
    )
}

/// Extracts a value of one kind from a slot or capture, for the node
/// reading it: [`as_int`], [`as_float`], and so on.
type Extract<T> = fn(&Value, &IrNode) -> Result<T, Diagnostic>;

/// The `Int` in `value`, read by the load or capture read `node`.
fn as_int(value: &Value, node: &IrNode) -> Result<i64, Diagnostic> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => mistyped(node, "Int load from a slot holding another type"),
    }
}

/// The `Float` in `value`, read by the load or capture read `node`.
fn as_float(value: &Value, node: &IrNode) -> Result<f64, Diagnostic> {
    match value {
        Value::Float(value) => Ok(*value),
        _ => mistyped(node, "Float load from a slot holding another type"),
    }
}

/// The `Bool` in `value`, read by the load or capture read `node`.
fn as_bool(value: &Value, node: &IrNode) -> Result<bool, Diagnostic> {
    match value {
        Value::Bool(value) => Ok(*value),
        _ => mistyped(node, "Bool load from a slot holding another type"),
    }
}

/// A new reference to the `String` in `value`, read by the load or capture
/// read `node`: not a copy.
fn as_string(value: &Value, node: &IrNode) -> Result<Rc<str>, Diagnostic> {
    match value {
        Value::Str(text) => Ok(Rc::clone(text)),
        _ => mistyped(node, "String load from a slot holding another type"),
    }
}

/// A new reference to the record in `value`, read by the load or capture
/// read `node`: not a copy.
fn as_record(value: &Value, node: &IrNode) -> Result<Value, Diagnostic> {
    match value {
        Value::Empty | Value::Record(_) => Ok(value.clone()),
        _ => mistyped(node, "record load from a slot holding another type"),
    }
}

/// A new reference to the closure in `value`, read by the load or capture
/// read `node`.
fn as_function(value: &Value, node: &IrNode) -> Result<Rc<Closure>, Diagnostic> {
    match value {
        Value::Fn(closure) => Ok(Rc::clone(closure)),
        _ => mistyped(node, "function load from a slot holding another type"),
    }
}

/// Fails with the internal error `message` at `node`: a node of a type its
/// walker does not handle. Returned directly by the walkers' error arms, so
/// that they hold no temporary for it (see the note on `Evaluator`).
fn mistyped<T>(node: &IrNode, message: &'static str) -> Result<T, Diagnostic> {
    Err(internal(message, node.span))
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::lower::lower;
    use crate::parser::parse;
    use crate::source::SourceFile;
    use crate::types::TypeTable;
    use crate::value::ValueKind;

    fn compile(text: &str) -> Rc<Ir> {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, strings, &source).unwrap();
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        Rc::new(lower(&ast, &checked, &types, 0).unwrap())
    }

    fn boolean(text: &str) -> bool {
        match run(text).unwrap() {
            Value::Bool(value) => value,
            other => panic!("{text}: expected a Bool, got {other:?}"),
        }
    }

    fn string(text: &str) -> String {
        match run(text) {
            Ok(Value::Str(value)) => value.to_string(),
            other => panic!("{text}: expected a String, got {other:?}"),
        }
    }

    /// Checks that each program evaluates to the Bool given.
    fn booleans(cases: &[(&str, bool)]) {
        for &(text, expected) in cases {
            assert_eq!(boolean(text), expected, "{text}");
        }
    }

    fn run(text: &str) -> Result<Value, Diagnostic> {
        eval(&compile(text))
    }

    fn int(text: &str) -> i64 {
        match run(text).unwrap() {
            Value::Int(value) => value,
            other => panic!("{text}: expected an Int, got {other:?}"),
        }
    }

    fn float(text: &str) -> f64 {
        match run(text).unwrap() {
            Value::Float(value) => value,
            other => panic!("{text}: expected a Float, got {other:?}"),
        }
    }

    fn error(text: &str) -> (String, Span) {
        let diagnostic = run(text).unwrap_err();
        (diagnostic.message, diagnostic.span)
    }

    /// `i64::MIN`, which cannot be written as a literal.
    const MIN: &str = "(-9223372036854775807 - 1)";
    const MAX: &str = "9223372036854775807";

    fn nz(value: i64) -> NonZeroI64 {
        NonZeroI64::new(value).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(int("1 + 2 * 3"), 7);
        assert_eq!(int("8 - 3 - 2"), 3);
        assert_eq!(int("2 * 3 // 4"), 1);
        assert_eq!(int("-2 * 3"), -6);
        assert_eq!(int("-(-2)"), 2);
        assert_eq!(int("2 * -3"), -6);
        assert_eq!(float("7 / 2"), 3.5);
        assert_eq!(float("7 / 2.0"), 3.5);
        assert_eq!(float("(1 + 2) * 3.0"), 9.0);
        assert_eq!(float("1.5 - 2"), -0.5);
        assert_eq!(float("-1.5"), -1.5);
        assert_eq!(float("(7 // 2) / 2"), 1.5);
    }

    #[test]
    fn floor_division_and_modulo_in_all_sign_combinations() {
        assert_eq!((int("7 // 2"), int("7 % 2")), (3, 1));
        assert_eq!((int("-7 // 2"), int("-7 % 2")), (-4, 1));
        assert_eq!((int("7 // -2"), int("7 % -2")), (-4, -1));
        assert_eq!((int("-7 // -2"), int("-7 % -2")), (3, -1));
    }

    #[test]
    fn exact_division_needs_no_adjustment() {
        assert_eq!((int("6 // 2"), int("6 % 2")), (3, 0));
        assert_eq!((int("-6 // 2"), int("-6 % 2")), (-3, 0));
        assert_eq!((int("6 // -2"), int("6 % -2")), (-3, 0));
        assert_eq!((int("-6 // -2"), int("-6 % -2")), (3, 0));
        assert_eq!((int("0 // -5"), int("0 % -5")), (0, 0));
    }

    #[test]
    fn floor_division_identity_holds() {
        for a in [-9, -7, -1, 0, 1, 7, 9, i64::MIN, i64::MAX] {
            for b in [-7, -2, -1, 1, 2, 7, i64::MIN, i64::MAX] {
                let Some(q) = floor_div(a, nz(b)) else {
                    assert_eq!((a, b), (i64::MIN, -1));
                    continue;
                };
                let r = floor_mod(a, nz(b));
                assert_eq!(i128::from(b) * i128::from(q) + i128::from(r), i128::from(a));
                assert!(r == 0 || (r < 0) == (b < 0), "{a} % {b} = {r}");
                assert!(r.unsigned_abs() < b.unsigned_abs());
            }
        }
    }

    #[test]
    fn differs_from_euclidean_division() {
        assert_eq!(7_i64.div_euclid(-2), -3);
        assert_eq!(floor_div(7, nz(-2)), Some(-4));
        assert_eq!(7_i64.rem_euclid(-2), 1);
        assert_eq!(floor_mod(7, nz(-2)), -1);
    }

    #[test]
    fn overflow_on_each_operator() {
        let overflow = "integer overflow".to_owned();
        // The error points at the operator.
        assert_eq!(
            error(&format!("{MAX} + 1")),
            (overflow.clone(), Span::new(20, 21))
        );
        assert_eq!(error(&format!("{MIN} - 1")).0, overflow);
        assert_eq!(error(&format!("{MAX} * 2")).0, overflow);
        assert_eq!(
            error(&format!("-{MIN}")),
            (overflow.clone(), Span::new(0, 1))
        );
        assert_eq!(error(&format!("{MIN} // -1")).0, overflow);
        assert_eq!(error(&format!("{MIN} * -1")).0, overflow);
    }

    #[test]
    fn runtime_errors_label_operands_with_their_values() {
        let diagnostic = run("2 * (9223372036854775807 - -1)").unwrap_err();
        assert_eq!(diagnostic.message, "integer overflow");
        assert_eq!(diagnostic.span, Span::new(25, 26));
        let labels: Vec<(Span, &str)> = diagnostic
            .labels
            .iter()
            .map(|label| (label.span, label.text.as_str()))
            .collect();
        assert_eq!(
            labels,
            vec![
                (Span::new(5, 24), "9223372036854775807"),
                (Span::new(27, 29), "-1")
            ]
        );
        let diagnostic = run("-(-9223372036854775807 - 1)").unwrap_err();
        assert_eq!(diagnostic.span, Span::new(0, 1));
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].text, "-9223372036854775808");
        // The parenthesised operand is labelled without its parentheses.
        assert_eq!(diagnostic.labels[0].span, Span::new(2, 26));
    }

    #[test]
    fn i64_min_cases() {
        assert_eq!(int(MIN), i64::MIN);
        assert_eq!(int(&format!("{MIN} % -1")), 0);
        assert_eq!(int(&format!("{MIN} // 1")), i64::MIN);
        assert_eq!(int(&format!("{MIN} // 2")), i64::MIN / 2);
        assert_eq!(int(&format!("{MIN} // {MAX}")), -2);
        assert_eq!(int(&format!("{MIN} % {MAX}")), i64::MAX - 1);
        assert_eq!(int(&format!("{MIN} % 2")), 0);
        assert_eq!(int(&format!("-({MIN} + 1)")), i64::MAX);
        assert_eq!(float(&format!("{MIN} / -1")), 9_223_372_036_854_775_808.0);
    }

    #[test]
    fn division_by_zero() {
        let message = "division by zero".to_owned();
        assert_eq!(error("7 // 0"), (message.clone(), Span::new(2, 4)));
        assert_eq!(error("7 % 0"), (message.clone(), Span::new(2, 3)));
        assert_eq!(error("1 + 7 % (2 - 2)"), (message, Span::new(6, 7)));
        let diagnostic = run("1 + 7 % (2 - 2)").unwrap_err();
        assert_eq!(diagnostic.labels[1].text, "0");
        assert_eq!(diagnostic.labels[1].span, Span::new(9, 14));
    }

    #[test]
    fn float_division_by_zero_is_ieee() {
        assert_eq!(float("1.0 / 0.0"), f64::INFINITY);
        assert_eq!(float("-1.0 / 0.0"), f64::NEG_INFINITY);
        assert!(float("0.0 / 0.0").is_nan());
        assert_eq!(float("1 / 0"), f64::INFINITY);
        assert!(float("1e308 * 10.0").is_infinite());
        assert!(float("(1.0 / 0.0) - (1.0 / 0.0)").is_nan());
    }

    #[test]
    fn errors_are_reported_left_to_right() {
        assert_eq!(
            error("(1 // 0) + (9223372036854775807 + 1)"),
            ("division by zero".to_owned(), Span::new(3, 5))
        );
        assert_eq!(
            error("(9223372036854775807 + 1) + (1 // 0)"),
            ("integer overflow".to_owned(), Span::new(21, 22))
        );
    }

    #[test]
    fn root_value_type_follows_the_ir() {
        assert_eq!(run("1").unwrap().kind(), ValueKind::Int);
        assert_eq!(run("1 / 1").unwrap().kind(), ValueKind::Float);
        assert_eq!(run("").unwrap().kind(), ValueKind::Record);
        assert_eq!(run("x = 1").unwrap().kind(), ValueKind::Record);
    }

    #[test]
    fn blocks_yield_their_result() {
        assert_eq!(int("let x = { let y = 1; y + 1 }\nx"), 2);
        assert_eq!(int("{ { { 3 } } }"), 3);
        assert_eq!(int("{ 1 } + { 2 } * 3"), 7);
        assert_eq!(int("{ = 4 }"), 4);
        assert_eq!(int("let x = 1\n= { let x = 5\n= x } - x"), 4);
        assert_eq!(float("{ 1.5 } * 2"), 3.0);
        assert_eq!(float("{ let h = 0.5\nh } / { 2 }"), 0.25);
    }

    #[test]
    fn shadowing() {
        assert_eq!(
            int("let x = 1\nlet x = x + 1\n{ let x = x * 10; x } + x"),
            22
        );
        // A shadowing `let` may change the type.
        assert_eq!(float("let x = 1\nlet x = 2.5\nx"), 2.5);
        // The outer variable is intact after an inner block shadows it.
        assert_eq!(int("let x = 1\n{ let x = 2 }\nx"), 1);
    }

    #[test]
    fn rebinding() {
        assert_eq!(int("let x = 1\nx := x + 1\nx := x * 3\nx"), 6);
        // The literal adapts to the variable's type.
        assert_eq!(float("let x: Float = 1\nx := 2\nx"), 2.0);
        assert_eq!(int("{ let n = 1; n := n + 1; n }"), 2);
    }

    #[test]
    fn ascription() {
        assert_eq!(float("1 + 2 : Float"), 3.0);
        assert_eq!(float("-5 : Float"), -5.0);
        assert_eq!(float("let x: Float = 5\nx / 2"), 2.5);
        assert_eq!(float("let x = 5 : Float\nx * 2"), 10.0);
        assert_eq!(float("{ 5 } : Float"), 5.0);
        assert_eq!(int("let x = 3\nx : Int"), 3);
    }

    #[test]
    fn nested_blocks_reuse_slots() {
        let text = "let a = { let t = 1; t + 1 }\n\
                    let b = { let u = 10; u + a }\n\
                    let c = { let v = 100; { let w = v; w } }\n\
                    a + b + c";
        assert_eq!(int(text), 114);
        // Each temporary shares its slot with the variable declared after
        // it; only the innermost `w` needs a slot of its own.
        assert_eq!(compile(text).frame_size(), 4);
    }

    #[test]
    fn empty_record_results() {
        for text in [
            "",
            "{}",
            "let x = 1",
            "{ let x = 1 }",
            "let u = {}\nu",
            "{ {}; {} }",
            "let x = 1\nx := 2",
            "= {}",
            "if true {}",
            "{ e = {} }.e",
        ] {
            assert_eq!(run(text).unwrap(), Value::Empty, "{text}");
        }
    }

    #[test]
    fn items_run_in_order_and_their_errors_are_reported() {
        assert_eq!(
            error("let x = 1 // 0\n5"),
            ("division by zero".to_owned(), Span::new(10, 12))
        );
        assert_eq!(
            error("{ let y = 9223372036854775807 + 1 }\n1"),
            ("integer overflow".to_owned(), Span::new(30, 31))
        );
        // The first failing item wins.
        assert_eq!(
            error("let a = 1 // 0\nlet b = 9223372036854775807 + 1\nb").0,
            "division by zero"
        );
    }

    #[test]
    fn eval_in_uses_the_given_frame() {
        let ir = compile("let x = 7\nlet y = x + 1");
        let mut frame = vec![Value::Empty; 3];
        assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
        // The stack is truncated to the frame on return.
        assert_eq!(frame, vec![Value::Int(7), Value::Int(8)]);
        frame.truncate(1);
        let error = eval_in(&ir, &mut frame).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: frame is smaller than the IR needs"
        );
    }

    #[test]
    fn int_comparisons() {
        booleans(&[
            ("1 == 1", true),
            ("1 == 2", false),
            ("1 != 2", true),
            ("1 != 1", false),
            ("1 < 2", true),
            ("2 < 1", false),
            ("1 < 1", false),
            ("1 <= 1", true),
            ("2 <= 1", false),
            ("2 > 1", true),
            ("1 > 1", false),
            ("1 >= 1", true),
            ("1 >= 2", false),
            ("-5 < 3", true),
            ("(-9223372036854775807 - 1) < 9223372036854775807", true),
            ("let x = 7\nx // 2 == 3", true),
        ]);
    }

    #[test]
    fn float_comparisons_follow_ieee_754() {
        booleans(&[
            ("1.5 == 1.5", true),
            ("1.5 != 1.5", false),
            ("1.5 < 2.5", true),
            ("2.5 <= 2.5", true),
            ("2.5 > 1", true),
            ("1 >= 1.5", false),
            ("0.1 + 0.2 == 0.3", false),
            ("-0.0 == 0.0", true),
            ("-0.0 != 0.0", false),
            ("-0.0 < 0.0", false),
            ("-0.0 >= 0.0", true),
            ("1.0 / 0.0 > 1e308", true),
            ("-1.0 / 0.0 < -1e308", true),
        ]);
        // NaN is unequal and unordered to everything, itself included.
        let nan = "let nan = 0.0 / 0.0\n";
        for (comparison, expected) in [
            ("nan == nan", false),
            ("nan != nan", true),
            ("nan < nan", false),
            ("nan <= nan", false),
            ("nan > nan", false),
            ("nan >= nan", false),
            ("nan == 1.0", false),
            ("nan != 1.0", true),
            ("nan < 1.0", false),
            ("1.0 < nan", false),
            ("nan >= 1.0", false),
            ("1.0 >= nan", false),
        ] {
            assert_eq!(
                boolean(&format!("{nan}{comparison}")),
                expected,
                "{comparison}"
            );
        }
    }

    #[test]
    fn string_comparisons_are_bytewise() {
        booleans(&[
            ("\"a\" < \"b\"", true),
            ("\"Z\" < \"a\"", true),
            ("\"\" < \"a\"", true),
            ("\"ab\" < \"b\"", true),
            ("\"b\" < \"ab\"", false),
            ("\"a\" <= \"a\"", true),
            ("\"a\" > \"A\"", true),
            ("\"abc\" >= \"abd\"", false),
            ("\"é\" > \"z\"", true),
            ("\"a\" == \"a\"", true),
            ("\"a\" == \"b\"", false),
            ("\"a\" != \"b\"", true),
            ("\"\" == \"\"", true),
            ("\"a\" == \"a \"", false),
            // Equality compares content, however the strings were built.
            ("\"{1}{2}\" == \"12\"", true),
            ("\"a\" + \"b\" == r\"ab\"", true),
            ("\"\"\"a\"\"\" == \"a\"", true),
        ]);
    }

    #[test]
    fn bool_and_empty_record_equality() {
        booleans(&[
            ("true == true", true),
            ("false == false", true),
            ("true == false", false),
            ("true != false", true),
            ("false != false", false),
            ("(1 < 2) == (2 > 1)", true),
            ("{} == {}", true),
            ("{} != {}", false),
            ("let u = {}\nu == { let x = 1 }", true),
        ]);
        // Empty record operands are still evaluated.
        assert_eq!(
            error("{ let y = 1 // 0 } == {}"),
            ("division by zero".to_owned(), Span::new(12, 14))
        );
    }

    #[test]
    fn records_are_built_and_read() {
        assert_eq!(int("{ x = 1; y = 2 }.y"), 2);
        assert_eq!(float("{ a = { n = 1.5 } }.a.n"), 1.5);
        assert!(boolean("{ \"a b\" = true }.\"a b\""));
        assert_eq!(string("{ s = \"hi\" }.s"), "hi");
        // Fields see earlier fields and private `let`s; later `let`s do not
        // change a field.
        assert_eq!(int("{ x = 1; let t = x * 10; y = t + x }.y"), 11);
        assert_eq!(
            int("let r = { x = 1; let x = 5; y = x }\nr.x * 10 + r.y"),
            15
        );
        // Stored in a variable and read back, whole and field by field.
        assert_eq!(
            run("let p = { x = 1; y = \"a\" }\nlet q = p\nq")
                .unwrap()
                .to_string(),
            "{x = 1; y = \"a\"}"
        );
        assert_eq!(int("let p = { x = 3; y = 4 }\np.x * p.x + p.y * p.y"), 25);
        // A record's field can be a record, read at any depth, and a record
        // can be rebuilt from another's fields.
        assert_eq!(
            run("let p = { a = { b = { c = 7 } } }\n{ c = p.a.b.c; d = p.a }")
                .unwrap()
                .to_string(),
            "{c = 7; d = {b = {c = 7}}}"
        );
        // An empty record as a field.
        assert_eq!(run("{ e = {} }").unwrap().to_string(), "{e = {}}");
        // Checked against an annotation, a literal is built at the field's
        // type.
        assert_eq!(float("let p: {x: Float} = { x = 1 }\np.x / 2"), 0.5);
        // A file of fields is a record.
        assert_eq!(
            run("x = 1\ny = \"a\"").unwrap().to_string(),
            "{x = 1; y = \"a\"}"
        );
    }

    #[test]
    fn records_display_as_gcsl_syntax() {
        let value = run("{ x = 1; y = \"a\\n\"; inner = { n = 2.0 }; \"a b\" = true }").unwrap();
        let shown = "{x = 1; y = \"a\\n\"; inner = {n = 2.0}; \"a b\" = true}";
        assert_eq!(value.to_string(), shown);
        assert_eq!(value.repr().to_string(), shown);
    }

    #[test]
    fn record_equality_is_deep_and_by_name() {
        booleans(&[
            ("{ x = 1; y = \"a\" } == { x = 1; y = \"a\" }", true),
            ("{ x = 1; y = \"a\" } == { x = 1; y = \"b\" }", false),
            ("{ x = 1; y = \"a\" } != { x = 2; y = \"a\" }", true),
            // Fields in a different order, of the same type.
            ("{ x = 1; y = 2 } == { y = 2; x = 1 }", true),
            ("{ x = 1; y = 2 } == { y = 1; x = 2 }", false),
            // Nested.
            ("{ a = { n = 1 } } == { a = { n = 1 } }", true),
            ("{ a = { n = 1 } } == { a = { n = 2 } }", false),
            ("{ e = {} } == { e = {} }", true),
            // Floats compare as IEEE 754 does, so NaN is unequal to itself.
            ("let r = { f = 0.0 / 0.0 }\nr == r", false),
            ("let r = { f = 0.0 / 0.0 }\nr != r", true),
            ("{ f = -0.0 } == { f = 0.0 }", true),
            // Through variables.
            ("let p = { x = 1 }\nlet q = { x = 1 }\np == q", true),
        ]);
        // Operands are evaluated left to right.
        assert_eq!(
            error("{ x = 1 // 0 } == { x = 9223372036854775807 + 1 }"),
            ("division by zero".to_owned(), Span::new(8, 10))
        );
    }

    #[test]
    fn if_selects_records() {
        assert_eq!(
            run("if true { { x = 1; y = 2 } } else { { x = 3 } }")
                .unwrap()
                .to_string(),
            "{x = 1; y = 2}"
        );
        assert_eq!(
            int("let r = if 1 > 2 { { x = 1; y = 2 } } else { { x = 3 } }\nr.x"),
            3
        );
        assert_eq!(
            int("(if false { { x = 1 } } else { { x = 2; z = 0 } }).x"),
            2
        );
    }

    #[test]
    fn records_in_interpolations_show_their_repr() {
        assert_eq!(
            string("let p = { x = 1; s = \"a\" }\n\"p is {p}, {{}} is { {} }\""),
            "p is {x = 1; s = \"a\"}, {} is {}"
        );
    }

    #[test]
    fn logic() {
        booleans(&[
            ("true and true", true),
            ("true and false", false),
            ("false and true", false),
            ("true or false", true),
            ("false or false", false),
            ("not true", false),
            ("not false", true),
            ("not not true", true),
            ("not 1 == 2 and 3 > 2 or false", true),
        ]);
    }

    #[test]
    fn and_and_or_short_circuit() {
        // The right operand would fail, so it is never evaluated.
        booleans(&[
            ("false and 1 // 0 == 0", false),
            ("true or 1 // 0 == 0", true),
            ("not (false and 1 // 0 == 0)", true),
            ("false and (true or 1 // 0 == 0)", false),
        ]);
        // When the left operand does not decide, it is.
        assert_eq!(
            error("true and 1 // 0 == 0"),
            ("division by zero".to_owned(), Span::new(11, 13))
        );
        assert_eq!(
            error("false or 1 // 0 == 0"),
            ("division by zero".to_owned(), Span::new(11, 13))
        );
    }

    #[test]
    fn if_evaluates_exactly_one_branch() {
        assert_eq!(int("if true { 1 } else { 1 // 0 }"), 1);
        assert_eq!(int("if false { 1 // 0 } else { 2 }"), 2);
        assert_eq!(
            int("if 1 > 2 { 1 // 0 } else if 2 > 1 { 3 } else { 1 // 0 }"),
            3
        );
        assert_eq!(float("if true { 1.5 } else { 1.0 / 0.0 }"), 1.5);
        assert!(!boolean("if false { true } else { false }"));
        assert_eq!(string("if 1 < 2 { \"yes\" } else { \"no\" }"), "yes");
        assert_eq!(
            run("if true {} else { let x = 1 // 0 }").unwrap(),
            Value::Empty
        );
        assert_eq!(int("if false { let x = 1 // 0 }\n5"), 5);
        // The branch taken, and the condition, are evaluated.
        assert_eq!(
            error("if true { 1 // 0 } else { 2 }"),
            ("division by zero".to_owned(), Span::new(12, 14))
        );
        assert_eq!(
            error("if 1 // 0 == 0 { 1 } else { 2 }"),
            ("division by zero".to_owned(), Span::new(5, 7))
        );
        assert_eq!(
            error("if true { let y = 1 // 0 }"),
            ("division by zero".to_owned(), Span::new(20, 22))
        );
        // Adapted and defaulted branches.
        assert_eq!(float("if true { 1 } else { 2.5 }"), 1.0);
        assert_eq!(float("(if false { 1 } else { 2 }) * 0.5"), 1.0);
        assert_eq!(int("let x = if true { 7 } else { 8 }\nx + 1"), 8);
    }

    #[test]
    fn interpolating_every_type() {
        assert_eq!(
            string(
                "\"{1} {2.5} {3.0} {true} {\"s\"} { {} } {1 == 2} {1.0 / 0.0} {0.0 / 0.0} {-0.0}\""
            ),
            "1 2.5 3.0 true s {} false inf nan -0.0"
        );
        assert_eq!(
            string("let n = 3\nlet s = \"x\"\nlet b = n > 2\n\"{s}{n}{s} {b}\""),
            "x3x true"
        );
        assert_eq!(string("\"{ { let a = 2; a * 3 } }\""), "6");
        assert_eq!(string("\"{\"\"}\""), "");
        assert_eq!(string("\"say \\\"{\"hi\"}\\\" {{ok}}\""), "say \"hi\" {ok}");
        assert_eq!(
            string("\"status: {if 1 > 2 { \"ok\" } else { \"failed\" }}\""),
            "status: failed"
        );
    }

    #[test]
    fn nested_interpolation() {
        assert_eq!(string("\"a{ \"b{ \"c{1}\" }\" }d\""), "abc1d");
        assert_eq!(string("\"{ \"{ \"{ 1 + 1 }\" }\" }\""), "2");
        assert_eq!(
            string("let who = \"world\"\n\"<{ \"{who} and {\"{who}!\"}\" }>\""),
            "<world and world!>"
        );
    }

    #[test]
    fn interpolation_errors_come_from_its_values() {
        assert_eq!(
            error("\"a{1 // 0}\""),
            ("division by zero".to_owned(), Span::new(5, 7))
        );
    }

    #[test]
    fn concatenation() {
        assert_eq!(string("\"a\" + \"b\""), "ab");
        assert_eq!(string("\"\" + \"\""), "");
        assert_eq!(string("\"x\" + \"{1 + 1}\" + \"y\""), "x2y");
        assert_eq!(string("let s = \"ab\"\ns + s + s"), "ababab");
        assert_eq!(string("\"é\" + \"😀\""), "é😀");
    }

    #[test]
    fn strings_round_trip_through_variables_and_blocks() {
        assert_eq!(
            string("let s = \"hello\"\nlet t = { let u = s; u }\nt"),
            "hello"
        );
        assert_eq!(string("let s = \"a\"\ns := s + \"b\"\n{ s }"), "ab");
        assert!(boolean("let s = \"x\"\n{ s } == s"));
        assert_eq!(string("let s = \"q\"\n= { = { s } }"), "q");
        assert_eq!(run("\"a\"").unwrap().kind(), ValueKind::String);
        assert_eq!(run("true").unwrap().kind(), ValueKind::Bool);
    }

    #[test]
    fn loading_a_string_shares_it() {
        let ir = compile("let s = \"abc\"\nlet t = s\nlet u = s + \"\"");
        let mut frame = vec![Value::Empty; 3];
        assert_eq!(eval_in(&ir, &mut frame).unwrap(), Value::Empty);
        let [Value::Str(s), Value::Str(t), Value::Str(u)] = &frame[..] else {
            panic!("expected strings, got {frame:?}");
        };
        // A load copies the reference, and concatenation makes a new string.
        assert!(Rc::ptr_eq(s, t));
        assert!(!Rc::ptr_eq(s, u));
        assert_eq!(u.as_ref(), "abc");
    }

    #[test]
    fn a_load_of_the_wrong_type_is_an_internal_error() {
        // `x + 1` with `x` loaded as an Int, from a frame holding a Float.
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let load = builder.alloc(IrKind::Load(Slot::new(0), ValueKind::Int), Span::new(0, 1));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
        let add = builder.alloc(
            IrKind::IntAdd(Operands {
                lhs: load,
                rhs: one,
                op_start: 2,
            }),
            Span::new(0, 5),
        );
        let ir = Rc::new(builder.finish(add, 1, Vec::new()));
        assert_eq!(
            eval_in(&ir, &mut vec![Value::Int(2)]).unwrap(),
            Value::Int(3)
        );
        let error = eval_in(&ir, &mut vec![Value::Float(1.5)]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: Int load from a slot holding another type"
        );
        assert_eq!(error.span, Span::new(0, 1));
    }

    /// The record in `value`, which must be one.
    fn record_of(value: &Value) -> &Rc<Record> {
        match value {
            Value::Record(record) => record,
            other => panic!("expected a record, got {other:?}"),
        }
    }

    #[test]
    fn dotted_fields_merge_into_records() {
        assert_eq!(
            run("server.host = \"localhost\"\nserver.port = 8080\nserver.tls.enabled = true")
                .unwrap()
                .to_string(),
            "{server = {host = \"localhost\"; port = 8080; tls = {enabled = true}}}"
        );
        // Fields in the order of first mention, at every level.
        assert_eq!(
            run("{ a.b.c = 1; d = 2; a.e = 3; a.b.f = 4 }")
                .unwrap()
                .to_string(),
            "{a = {b = {c = 1; f = 4}; e = 3}; d = 2}"
        );
        // Later items see the record built so far.
        assert_eq!(
            run("{ a.x = 1; let p = a; a.y = a.x + 1; q = p }")
                .unwrap()
                .to_string(),
            "{a = {x = 1; y = 2}; q = {x = 1}}"
        );
        assert_eq!(int("{ a.x = 1; a.y = 2 }.a.y"), 2);
    }

    #[test]
    fn an_update_that_replaces_a_field_shares_the_layout() {
        let ir = compile(
            "let a = { x = 1; s = \"text\" }\n\
             let b = a\n\
             let b.x = 5\n\
             let c = a\n\
             let c.z = true",
        );
        let mut frame = vec![Value::Empty; 5];
        eval_in(&ir, &mut frame).unwrap();
        let [a, _, replaced, _, added] = &frame[..] else {
            panic!("expected five slots, got {frame:?}");
        };
        // The old record is intact.
        assert_eq!(a.to_string(), "{x = 1; s = \"text\"}");
        assert_eq!(replaced.to_string(), "{x = 5; s = \"text\"}");
        let (a, replaced) = (record_of(a), record_of(replaced));
        assert!(Rc::ptr_eq(&a.layout, &replaced.layout));
        assert!(!Rc::ptr_eq(a, replaced));
        // An addition makes a new layout, with the name at the end.
        assert_eq!(added.to_string(), "{x = 1; s = \"text\"; z = true}");
        let added = record_of(added);
        assert!(!Rc::ptr_eq(&a.layout, &added.layout));
        assert_eq!(added.layout.names.len(), 3);
        assert_eq!(added.layout.names[..2], a.layout.names[..]);
        assert!(!a.layout.names.contains(&added.layout.names[2]));
        assert_eq!(
            &*added.layout.texts,
            &[Box::from("x"), Box::from("s"), Box::from("z")]
        );
        // The unchanged fields are shared, not copied.
        let shared = |record: &Record| match &record.values[1] {
            Value::Str(text) => Rc::clone(text),
            other => panic!("expected a string, got {other:?}"),
        };
        assert!(Rc::ptr_eq(&shared(a), &shared(replaced)));
        assert!(Rc::ptr_eq(&shared(a), &shared(added)));
    }

    #[test]
    fn updates_nest_and_create_records() {
        assert_eq!(
            run("let a = { b = { c = 1; d = 2 } }\nlet a.b.c = 10\na")
                .unwrap()
                .to_string(),
            "{b = {c = 10; d = 2}}"
        );
        assert_eq!(
            run("let a = { x = 1 }\nlet a.b.c = 2\na")
                .unwrap()
                .to_string(),
            "{x = 1; b = {c = 2}}"
        );
        assert_eq!(
            run("let a.b.c = \"s\"\na").unwrap().to_string(),
            "{b = {c = \"s\"}}"
        );
        assert_eq!(
            run("let a = { x = 1 }\na.x := a.x + 1\nlet a.y.z = false\na.y.z := true\na")
                .unwrap()
                .to_string(),
            "{x = 2; y = {z = true}}"
        );
        // A leaf replaced by a value of another type.
        assert_eq!(
            run("let a = { b = { c = 1 } }\nlet a.b = 2.5\na")
                .unwrap()
                .to_string(),
            "{b = 2.5}"
        );
        // A field the static type has as `{}` may hold the empty record.
        assert_eq!(
            run("let a = { e = {} }\nlet a.e.x = 1\na")
                .unwrap()
                .to_string(),
            "{e = {x = 1}}"
        );
        // A field hidden by subtyping is replaced where the path creates a
        // record, since the static type is the truth.
        assert_eq!(
            run("let p: {x: Int} = { x = 1; y = 2 }\nlet p.y.z = 3\np")
                .unwrap()
                .to_string(),
            "{x = 1; y = {z = 3}}"
        );
        // The value is evaluated after the record, and its errors reported.
        assert_eq!(
            error("let a = { x = 1 }\nlet a.y = 1 // 0").0,
            "division by zero"
        );
    }

    #[test]
    fn an_update_leaves_the_old_value_intact() {
        assert_eq!(
            string("let a = { x = 1; n = { m = 2 } }\nlet old = a\nlet a.n.m = 3\n\"{old} {a}\""),
            "{x = 1; n = {m = 2}} {x = 1; n = {m = 3}}"
        );
        assert_eq!(
            string("let a = { x = 1 }\nlet old = a\na.x := 2\n\"{old} {a}\""),
            "{x = 1} {x = 2}"
        );
        // Through a field: the field's record is extended in its slot, and
        // what was read from it before is unchanged.
        assert_eq!(
            run("{ a.x = 1; let p = a; a.y = 2; q = p }")
                .unwrap()
                .to_string(),
            "{a = {x = 1; y = 2}; q = {x = 1}}"
        );
    }

    #[test]
    fn typed_equality_compares_the_static_fields() {
        let narrowed = "let q: {x: Int} = { x = 1; y = 2 }\n";
        booleans(&[
            (&format!("{narrowed}q == {{ x = 1 }}"), true),
            (&format!("{narrowed}q === {{ x = 1 }}"), false),
            (&format!("{narrowed}q != {{ x = 1 }}"), false),
            (&format!("{narrowed}q !== {{ x = 1 }}"), true),
            (&format!("{narrowed}q == {{ x = 2 }}"), false),
            (&format!("{narrowed}q === q"), true),
            (&format!("{narrowed}q == q"), true),
        ]);
        // Nested records compare over their static fields too.
        let nested = "let q: {a: {n: Int}} = { a = { n = 1; m = 2 }; b = 3 }\n";
        booleans(&[
            (&format!("{nested}q == {{ a = {{ n = 1 }} }}"), true),
            (&format!("{nested}q === {{ a = {{ n = 1 }} }}"), false),
            (&format!("{nested}q == {{ a = {{ n = 2 }} }}"), false),
            (&format!("{nested}q != {{ a = {{ n = 2 }} }}"), true),
            (&format!("{nested}q.a == {{ n = 1 }}"), true),
        ]);
        // A record typed `{}` equals every other.
        booleans(&[
            ("let u: {} = { x = 1 }\nu == {}", true),
            ("let u: {} = { x = 1 }\nu === {}", false),
            ("let u: {} = { x = 1 }\nu != {}", false),
            ("let e: {e: {}} = { e = { x = 1 } }\ne == { e = {} }", true),
        ]);
        // Floats compare as IEEE 754 does either way.
        booleans(&[
            ("let r = { f = 0.0 / 0.0 }\nr == r", false),
            ("let r = { f = 0.0 / 0.0 }\nr === r", false),
            ("let r = { f = 0.0 / 0.0 }\nr != r", true),
            ("{ f = -0.0 } == { f = 0.0 }", true),
            ("{ f = -0.0 } === { f = 0.0 }", true),
        ]);
        // Scalars are the same under both.
        booleans(&[
            ("1 === 1", true),
            ("1 !== 1", false),
            ("\"a\" === \"b\"", false),
            ("true !== false", true),
            ("1.5 === 1.5", true),
        ]);
        // Operands are evaluated left to right.
        assert_eq!(
            error("{ x = 1 // 0 } == { x = 9223372036854775807 + 1 }"),
            ("division by zero".to_owned(), Span::new(8, 10))
        );
        assert_eq!(
            error("{ x = 1 // 0 } === { x = 9223372036854775807 + 1 }"),
            ("division by zero".to_owned(), Span::new(8, 10))
        );
    }

    /// A record nested `depth` deep through the field named `inner`,
    /// ending in `leaf`.
    fn nested_record(inner: Name, depth: usize, leaf: Value) -> Value {
        let layout = Rc::new(Layout {
            names: Box::new([inner]),
            texts: Box::new([Box::from("inner")]),
        });
        let mut value = leaf;
        for _ in 0..depth {
            value = Value::Record(Rc::new(Record {
                layout: Rc::clone(&layout),
                values: Box::new([value]),
            }));
        }
        value
    }

    #[test]
    fn typed_equality_of_deep_records_does_not_recurse() {
        // Far deeper than a test thread's stack could take recursively: a
        // shape for each level, the last comparing the leaf as a value.
        const DEPTH: usize = 1_000_000;
        let mut names = crate::names::Names::new();
        let inner = names.intern("inner");
        let mut builder = crate::ir::IrBuilder::with_capacity(1);
        let first = builder.push_shape(crate::ir::Shape {
            fields: Box::default(),
        });
        let mut shape = first;
        for _ in 1..DEPTH {
            let next = builder.push_shape(crate::ir::Shape {
                fields: Box::default(),
            });
            builder.set_shape(
                shape,
                crate::ir::Shape {
                    fields: Box::new([(inner, Some(next))]),
                },
            );
            shape = next;
        }
        builder.set_shape(
            shape,
            crate::ir::Shape {
                fields: Box::new([(inner, None)]),
            },
        );
        let root = builder.alloc(IrKind::Empty, Span::new(0, 0));
        let ir = builder.finish(root, 0, Vec::new());
        let a = nested_record(inner, DEPTH, Value::Int(0));
        let b = nested_record(inner, DEPTH, Value::Int(0));
        let c = nested_record(inner, DEPTH, Value::Int(1));
        assert_eq!(record_eq_shaped(&ir, first, &a, &b), Some(true));
        assert_eq!(record_eq_shaped(&ir, first, &a, &c), Some(false));
        // Shallower records lack a field the shape has at the bottom.
        let shallow = nested_record(inner, DEPTH - 1, Value::Int(0));
        assert_eq!(record_eq_shaped(&ir, first, &a, &shallow), None);
    }

    #[test]
    fn a_record_lacking_a_field_of_its_shape_is_an_internal_error() {
        // `r == r` built by hand, with `r` holding a record without the
        // shape's `y`.
        let mut names = crate::names::Names::new();
        let (x, y) = (names.intern("x"), names.intern("y"));
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let shape = builder.push_shape(crate::ir::Shape {
            fields: Box::new([(y, None)]),
        });
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Record),
            Span::new(0, 1),
        );
        let eq = builder.alloc(
            IrKind::RecordEq(ShapedOperands {
                shape,
                lhs: load,
                rhs: load,
            }),
            Span::new(0, 6),
        );
        let ir = Rc::new(builder.finish(eq, 1, Vec::new()));
        let record = Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: Box::new([x]),
                texts: Box::new([Box::from("x")]),
            }),
            values: Box::new([Value::Int(1)]),
        }));
        let error = eval_in(&ir, &mut vec![record]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a record lacks a field of its shape"
        );
        assert_eq!(error.span, Span::new(0, 6));
        // So is the empty record, where the shape has fields.
        let error = eval_in(&ir, &mut vec![Value::Empty]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a record lacks a field of its shape"
        );
    }

    #[test]
    fn an_update_without_its_name_text_is_an_internal_error() {
        let mut names = crate::names::Names::new();
        let x = names.intern("x");
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let nothing = builder.alloc(IrKind::Empty, Span::new(0, 1));
        let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
        let update = builder.alloc(
            IrKind::UpdateField {
                record: nothing,
                name: x,
                value: one,
            },
            Span::new(0, 5),
        );
        let ir = Rc::new(builder.finish(update, 0, Vec::new()));
        let error = eval(&ir).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: an update's name has no text"
        );
        assert_eq!(error.span, Span::new(0, 5));
    }

    #[test]
    fn a_missing_field_is_an_internal_error() {
        // `r.y` built by hand, with `r` holding a record without `y`.
        let mut names = crate::names::Names::new();
        let (x, y) = (names.intern("x"), names.intern("y"));
        let mut builder = crate::ir::IrBuilder::with_capacity(2);
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Record),
            Span::new(0, 1),
        );
        let field = builder.alloc(
            IrKind::Field {
                record: load,
                name: y,
                kind: ValueKind::Int,
            },
            Span::new(0, 3),
        );
        let ir = Rc::new(builder.finish(field, 1, Vec::new()));
        let record = |value: Value| {
            Value::Record(Rc::new(Record {
                layout: Rc::new(crate::value::Layout {
                    names: Box::new([x]),
                    texts: Box::new([Box::from("x")]),
                }),
                values: Box::new([value]),
            }))
        };
        let error = eval_in(&ir, &mut vec![record(Value::Int(1))]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a record lacks a field the checker found"
        );
        assert_eq!(error.span, Span::new(0, 3));
        let error = eval_in(&ir, &mut vec![Value::Empty]).unwrap_err();
        assert_eq!(error.message, "internal error: a field of the empty record");
    }

    // ----- Functions -----

    /// Runs `f` on a thread with [`crate::STACK_SIZE`] of stack, as the
    /// pipeline is meant to be run. Test threads are far smaller by default.
    fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(crate::STACK_SIZE)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn calls_of_every_kind_of_result() {
        // Through each typed walker.
        assert_eq!(int("fn f(): Int { 41 }\nf() + 1"), 42);
        assert_eq!(float("fn f(x: Float): Float { x / 2 }\nf(3) * 2"), 3.0);
        assert!(boolean("fn f(x: Int): Bool { x > 1 }\nnot f(1)"));
        assert_eq!(
            string("fn f(s: String): String { s + s }\nf(\"ab\") + \"!\""),
            "abab!"
        );
        assert_eq!(int("fn f(): {x: Int} { x = 1 }\nf().x"), 1);
        assert!(boolean("fn f(): {x: Int} { x = 1 }\nf() == { x = 1 }"));
        assert_eq!(run("fn f() -> {} {}\nf()").unwrap(), Value::Empty);
        assert_eq!(int("fn f(): fn(): Int { fn(): Int { 7 } }\nf()()"), 7);
        // And through the general walker.
        assert_eq!(
            string("fn f(n: Int): Int { n * 2 }\n\"{f(2)} {f(3)}\""),
            "4 6"
        );
        assert_eq!(
            run("fn f(n: Int): {n: Int} { n = n }\n{ a = f(1); b = f(2) }")
                .unwrap()
                .to_string(),
            "{a = {n = 1}; b = {n = 2}}"
        );
        // A call in a condition, an operand, and an argument.
        assert_eq!(
            int(
                "fn even(n: Int): Bool { n % 2 == 0 }\nfn half(n: Int): Int { n // 2 }\n\
                 if even(4) { half(half(8)) } else { 0 }"
            ),
            2
        );
        // Captured values of each kind, read by each walker.
        let text = "let i = 1\nlet x = 1.5\nlet b = true\nlet s = \"s\"\nlet r = { v = 2 }\n\
                    let g = fn(): Int { 3 }\n\
                    let f = fn(): String { \"{i + 1} {x * 2} {not b} {s + s} {r.v} {g()} {r}\" }\n\
                    f()";
        assert_eq!(string(text), "2 3.0 false ss 2 3 {v = 2}");
    }

    #[test]
    fn recursion() {
        assert_eq!(
            int("fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\nfact(20)"),
            2_432_902_008_176_640_000
        );
        assert_eq!(
            int(
                "let fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
                 fib(20)"
            ),
            6765
        );
        // Through a field, and through a callee captured by a closure.
        assert_eq!(
            int(
                "let m = { sum = fn(n: Int): Int { if n == 0 { 0 } else { n + sum(n - 1) } } }\n\
                 m.sum(10)"
            ),
            55
        );
        assert_eq!(
            int("fn down(n: Int): fn(): Int {\n\
                 \x20 fn(): Int { if n == 0 { 0 } else { down(n - 1)() + 1 } }\n\
                 }\n\
                 down(5)()"),
            5
        );
        // An overflow deep in a recursion is reported where it happens.
        let diagnostic = run(
            "fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\n\
                              fact(21)",
        )
        .unwrap_err();
        assert_eq!(diagnostic.message, "integer overflow");
        // The `*` of the outermost multiplication, `21 * fact(20)`.
        assert_eq!(diagnostic.span, Span::new(48, 49));
        assert_eq!(diagnostic.labels[0].text, "21");
        assert_eq!(diagnostic.input, None);
    }

    #[test]
    fn closures_capture_by_value() {
        assert_eq!(int("let x = 1\nlet f = fn(): Int { x }\nx := 2\nf()"), 1);
        assert_eq!(
            int("let x = 1\nlet f = fn(): Int { x }\nlet x = 5\nf() + x"),
            6
        );
        // Each closure holds what was in scope when it was made.
        assert_eq!(
            string(
                "fn make(n: Int): fn(): Int { fn(): Int { n } }\n\
                    let a = make(1)\nlet b = make(2)\n\"{a()} {b()} {a()}\""
            ),
            "1 2 1"
        );
        // A record captured is the record as it was.
        assert_eq!(
            string("let r = { x = 1 }\nlet f = fn(): {x: Int} { r }\nlet r.x = 2\n\"{f()} {r}\""),
            "{x = 1} {x = 2}"
        );
        // A dotted `let` of a captured record inside the function updates
        // the captured value into a new local, leaving the capture and the
        // outer variable as they were.
        assert_eq!(
            string(
                "let r = { a = 1 }\n\
                 let f = fn(): {a: Int, b: Int} { let r.b = 2\nr }\n\
                 \"{f()} {f()} {r}\""
            ),
            "{a = 1; b = 2} {a = 1; b = 2} {a = 1}"
        );
    }

    #[test]
    fn nested_closures_and_closures_called_later() {
        assert_eq!(
            // The innermost function measures every parameter, through the
            // pure functions around it, so each is present.
            int("fn adder(a: Int): fn(Int): fn(Int): Int {\n\
                   fn(b: Int): fn(Int): Int { fn(c: Int): Int { a * 100 + b * 10 + c } }\n\
                 }\n\
                 adder(1)(2)(3)"),
            123
        );
        assert_eq!(
            int(
                "fn counter(start: Int): fn(): Int { fn(): Int { start + 1 } }\n\
                 let c = counter(41)\nlet unrelated = counter(0)()\nc()"
            ),
            42
        );
        // A function-valued field, called through `.`.
        assert_eq!(
            int("let math = { square = fn(v: Int): Int { v * v }; k = 3 }\nmath.square(math.k)"),
            9
        );
        assert_eq!(
            int(
                "fn tools(): {twice: fn(Int): Int} { twice = fn(v: Int): Int { v * 2 } }\n\
                 tools().twice(21)"
            ),
            42
        );
        // Passed as an argument, and chosen by an `if`.
        assert_eq!(
            int("fn apply(f: fn(Int): Int, v: Int): Int { f(v) }\n\
                 apply(if true { fn(n: Int): Int { n + 1 } } else { fn(n: Int): Int { n } }, 1)"),
            2
        );
    }

    #[test]
    fn arguments_are_evaluated_in_source_order() {
        let f = "fn f(a: Int, b: Int): Int { a + b }\n";
        assert_eq!(int(&format!("{f}f(b = 1, a = 2)")), 3);
        // The first-written argument fails first, whatever it fills.
        assert_eq!(
            error(&format!("{f}f(b = 1 // 0, a = 9223372036854775807 + 1)")).0,
            "division by zero"
        );
        assert_eq!(
            error(&format!("{f}f(b = 9223372036854775807 + 1, a = 1 // 0)")).0,
            "integer overflow"
        );
        // The trailing block comes last.
        let g = "fn g(a: Int, r: {x: Int}): Int { a + r.x }\n";
        assert_eq!(int(&format!("{g}g(1) {{ x = 2 }}")), 3);
        assert_eq!(int(&format!("{g}g(a = 1) {{ x = 2 }}")), 3);
        assert_eq!(
            error(&format!("{g}g(1 // 0) {{ x = 9223372036854775807 + 1 }}")).0,
            "division by zero"
        );
        assert_eq!(
            error(&format!("{g}g(9223372036854775807 + 1) {{ x = 1 // 0 }}")).0,
            "integer overflow"
        );
        // The callee is evaluated before the arguments.
        assert_eq!(
            error("fn f(x: Int): fn(Int): Int { fn(y: Int): Int { y } }\nf(1 // 0)(9223372036854775807 + 1)").0,
            "division by zero"
        );
    }

    /// Evaluates `ir` without `eval_in`'s truncation, returning the result
    /// and the stack's length afterwards.
    fn run_on(ir: &Rc<Ir>, stack: &mut Vec<Value>) -> (Result<Value, Diagnostic>, usize) {
        let result = Evaluator {
            ir,
            stack,
            base: 0,
            captures: &[],
            callee: None,
            levels: 0,
        }
        .value(ir.root());
        (result, stack.len())
    }

    #[test]
    fn a_call_truncates_the_stack_whether_it_succeeds_or_fails() {
        let ir = compile(
            "fn f(n: Int): Int { let m = n + 1\nlet k = m * 2\nk }\n\
             fn g(n: Int): Int { let p = f(n)\nf(p) }\nlet a = g(1)\n= g(a)",
        );
        let frame = ir.frame_size() as usize;
        let mut stack = vec![Value::Empty; frame];
        let (result, len) = run_on(&ir, &mut stack);
        // g(1) = f(f(1)) = 10, and g(10) = f(f(10)) = 46.
        assert_eq!(result.unwrap(), Value::Int(46));
        assert_eq!(len, frame);
        // A failure inside a nested call leaves the stack at the frame too.
        let ir = compile(
            "fn f(n: Int): Int { let m = n // 0\nm }\n\
             fn g(n: Int): Int { let p = n\nf(p) }\nlet a = 1\n= g(a)",
        );
        let frame = ir.frame_size() as usize;
        let mut stack = vec![Value::Empty; frame];
        let (result, len) = run_on(&ir, &mut stack);
        assert_eq!(result.unwrap_err().message, "division by zero");
        assert_eq!(len, frame);
        // So does a failure in an argument, after the frame has grown.
        let ir = compile("fn f(a: Int, b: Int): Int { a }\nf(1, 1 // 0)");
        let mut stack = vec![Value::Empty; ir.frame_size() as usize];
        let (result, len) = run_on(&ir, &mut stack);
        assert_eq!(result.unwrap_err().message, "division by zero");
        assert_eq!(len, ir.frame_size() as usize);
    }

    #[test]
    fn a_warm_stack_does_not_grow_again() {
        let ir = compile(
            "let fib = fn(n: Int): Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
             fib(15)",
        );
        let frame = ir.frame_size() as usize;
        let mut stack = vec![Value::Empty; frame];
        assert_eq!(eval_in(&ir, &mut stack).unwrap(), Value::Int(610));
        let capacity = stack.capacity();
        // Calls grew it: one slot per call in progress at the deepest.
        assert!(capacity >= frame + 15, "{capacity}");
        assert_eq!(stack.len(), frame);
        assert_eq!(eval_in(&ir, &mut stack).unwrap(), Value::Int(610));
        assert_eq!(stack.capacity(), capacity);
        assert_eq!(stack.len(), frame);
    }

    const BUDGET_ERROR: &str =
        "call nested too deeply: the limit is 10000 levels of nesting, calls included";
    const BUDGET_NOTE: &str = "every level of expression nesting around a call counts, and so \
                               does the body of the function called; GCSL does not eliminate \
                               tail calls, so recursion this deep is not supported";

    #[test]
    fn the_first_call_past_the_nesting_budget_fails() {
        on_big_stack(|| {
            let recurse = "fn f(n: Int): Int { if n == 0 { 0 } else { f(n - 1) + 1 } }\n";
            // The shape of the IR does not depend on the argument.
            let ir = compile(&format!("{recurse}f(0)"));
            let [(outer, _)] = ir.calls_in(ir.root())[..] else {
                panic!("expected one call at the top level");
            };
            let code = ir.function(FnId::new(0));
            let [(inner, inner_span)] = ir.calls_in(code.body)[..] else {
                panic!("expected one call in the body");
            };
            let (outer, inner) = (usize::from(outer), usize::from(inner));
            let height = code.height as usize;
            // The first call enters its frame at `outer + CALL_LEVELS`
            // levels, and each nested call `inner + CALL_LEVELS` further in;
            // each is made while the body it enters fits within the budget.
            let allowed = 1 + (MAX_DEPTH - height - outer - CALL_LEVELS) / (inner + CALL_LEVELS);
            // `f(n)` makes n + 1 calls: exactly the allowed number succeeds.
            let last = i64::try_from(allowed - 1).unwrap();
            assert_eq!(
                run(&format!("{recurse}f({last})")).unwrap(),
                Value::Int(last)
            );
            // One more fails, at the innermost call, which would exceed it.
            let text = format!("{recurse}f({})", last + 1);
            let diagnostic = run(&text).unwrap_err();
            assert_eq!(diagnostic.message, BUDGET_ERROR);
            assert_eq!(
                diagnostic.message,
                format!(
                    "call nested too deeply: the limit is {MAX_DEPTH} levels of nesting, calls \
                     included"
                )
            );
            assert_eq!(diagnostic.notes, vec![BUDGET_NOTE]);
            assert_eq!(diagnostic.span, inner_span);
            assert_eq!(diagnostic.span, Span::new(43, 51));
            assert!(diagnostic.labels.is_empty());
            // Nothing is left on the stack by the failure.
            let ir = compile(&text);
            let mut stack = vec![Value::Empty; ir.frame_size() as usize];
            let (result, len) = run_on(&ir, &mut stack);
            assert_eq!(result.unwrap_err().message, BUDGET_ERROR);
            assert_eq!(len, ir.frame_size() as usize);
        });
    }

    #[test]
    fn the_nesting_around_a_top_level_call_counts() {
        on_big_stack(|| {
            // `f()` as the left operand of `terms` additions, which put it
            // `terms + 1` levels deep in the file's body.
            let program =
                |terms: usize| format!("fn f(): Int {{ 1 }}\nf(){}", " + 1".repeat(terms));
            let ir = compile(&program(1));
            let [(depth, _)] = ir.calls_in(ir.root())[..] else {
                panic!("expected one call at the top level");
            };
            assert_eq!(depth, 2);
            let height = ir.function(FnId::new(0)).height as usize;
            // The deepest the call may be, and so the most additions.
            let terms = MAX_DEPTH - CALL_LEVELS - height - 1;
            let ir = compile(&program(terms));
            let [(depth, _)] = ir.calls_in(ir.root())[..] else {
                panic!("expected one call at the top level");
            };
            assert_eq!(usize::from(depth), terms + 1);
            let sum = i64::try_from(terms + 1).unwrap();
            assert_eq!(eval(&ir).unwrap(), Value::Int(sum));
            let diagnostic = run(&program(terms + 1)).unwrap_err();
            assert_eq!(diagnostic.message, BUDGET_ERROR);
            assert_eq!(diagnostic.span, Span::new(18, 21));
        });
    }

    #[test]
    fn function_values_and_their_internal_errors() {
        // A closure shows as its type, its parameters' laziness inferred.
        let value = run("fn(x: Int, s: String): {x: Int} { x = x }").unwrap();
        assert_eq!(value.kind(), ValueKind::Fn);
        assert_eq!(
            value.to_string(),
            "fn(x: Int, s: String | Future<String>): {x: Int}"
        );
        assert_eq!(
            value.repr().to_string(),
            "fn(x: Int, s: String | Future<String>): {x: Int}"
        );
        // An impure function's bare types are the either shape.
        let value = run("fn(x: Int, s: String) -> {x: Int} { x = x }").unwrap();
        assert_eq!(
            value.to_string(),
            "fn(x: Int | Future<Int>, s: String | Future<String>) -> {x: Int | Future<Int>} | \
             Future<{x: Int | Future<Int>}>"
        );
        assert_eq!(
            string("let f = fn(): Int { 1 }\n\"f is {f}\""),
            "f is fn(): Int"
        );
        // The callee outside a function, and an argument outside a call.
        let mut builder = crate::ir::IrBuilder::with_capacity(2);
        let callee = builder.alloc(IrKind::LoadCallee, Span::new(0, 1));
        let ir = Rc::new(builder.finish(callee, 0, Vec::new()));
        assert_eq!(
            eval(&ir).unwrap_err().message,
            "internal error: load of the callee outside a function"
        );
        let mut builder = crate::ir::IrBuilder::with_capacity(2);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
        let arg = builder.alloc(
            IrKind::Arg {
                param: 0,
                value: one,
            },
            Span::new(0, 1),
        );
        let ir = Rc::new(builder.finish(arg, 0, Vec::new()));
        assert_eq!(
            eval(&ir).unwrap_err().message,
            "internal error: an argument outside a call"
        );
        // A closure of a function its IR does not have.
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let empty = builder.push_item_list(&[]);
        let closure = builder.alloc(
            IrKind::MakeClosure {
                function: FnId::new(3),
                captures: empty,
            },
            Span::new(0, 1),
        );
        let call = builder.alloc(
            IrKind::Call {
                callee: closure,
                args: empty,
                kind: ValueKind::Int,
                depth: 0,
            },
            Span::new(0, 3),
        );
        let ir = Rc::new(builder.finish(call, 0, Vec::new()));
        assert_eq!(
            eval(&ir).unwrap_err().message,
            "internal error: a closure of a function its IR lacks"
        );
    }

    // ----- Futures -----

    /// The labels of a diagnostic as `(span, text)` pairs.
    fn labels(diagnostic: &Diagnostic) -> Vec<(Span, &str)> {
        diagnostic
            .labels
            .iter()
            .map(|label| (label.span, label.text.as_str()))
            .collect()
    }

    /// Runs `text` on a frame of its own and returns the frame afterwards,
    /// holding the value of each top-level variable.
    fn frame_after(text: &str) -> Vec<Value> {
        let ir = compile(text);
        let mut frame = vec![Value::Empty; ir.frame_size() as usize];
        eval_in(&ir, &mut frame).unwrap();
        frame
    }

    #[test]
    fn a_lazy_runs_nothing_until_awaited() {
        assert_eq!(int("let x = lazy 1 // 0\n5"), 5);
        assert_eq!(run("lazy 1 // 0").unwrap().to_string(), "Future<Int>");
        let text = "let x = lazy 1 // 0\nawait x";
        let diagnostic = run(text).unwrap_err();
        assert_eq!(diagnostic.message, "division by zero");
        // At the failing operator in the body, with "awaited here" on the
        // await that ran it.
        assert_eq!(diagnostic.span, Span::new(15, 17));
        assert_eq!(
            labels(&diagnostic),
            vec![
                (Span::new(13, 14), "1"),
                (Span::new(18, 19), "0"),
                (Span::new(20, 27), "awaited here")
            ]
        );
        assert_eq!(diagnostic.input, None);
        // Through an implicit await, on the expression awaited.
        let diagnostic = run("let x = lazy 1 // 0\nx + 1").unwrap_err();
        assert_eq!(diagnostic.span, Span::new(15, 17));
        assert_eq!(labels(&diagnostic)[2], (Span::new(20, 21), "awaited here"));
        // An await that runs another's body labels its own await too.
        let diagnostic = run("let x = lazy 1 // 0\nlet y = lazy x + 1\nawait y").unwrap_err();
        assert_eq!(
            labels(&diagnostic)[2..],
            [
                (Span::new(33, 34), "awaited here"),
                (Span::new(39, 46), "awaited here")
            ]
        );
    }

    #[test]
    fn a_future_captures_by_value_and_runs_once() {
        assert_eq!(int("let x = 1\nlet f = lazy x\nx := 2\nawait f"), 1);
        assert_eq!(int("let x = 1\nlet f = lazy x\nlet x = 5\nf + x"), 6);
        // A future shared by a variable and a closure's capture is one
        // future: its value is the same record, built once.
        let frame = frame_after(
            "let f = lazy { a = 1 }\n\
             let g = fn() -> {a: Int} { await f }\n\
             let p = await f\n\
             let q = g()",
        );
        let [Value::Future(future), _, Value::Record(p), Value::Record(q)] = &frame[..] else {
            panic!("unexpected frame {frame:?}");
        };
        assert!(Rc::ptr_eq(p, q));
        assert!(future.is_done());
        // Awaited twice, directly and through a record field.
        assert_eq!(
            string("let f = lazy 20 + 1\nlet r = { a = f }\n\"{f} {r.a} {await f}\""),
            "21 21 21"
        );
    }

    #[test]
    fn an_update_of_a_future_record_runs_it_once() {
        // The update awaits the future, and a later await of the same future
        // finds the record that one run built: the update shares its
        // untouched field with it.
        let frame = frame_after(
            "let r = lazy { a = 1; b = { c = 2 } }\n\
             let old = r\n\
             let r.a = 5\n\
             let s = await old",
        );
        let [
            Value::Future(future),
            Value::Future(old),
            Value::Record(updated),
            Value::Record(read),
        ] = &frame[..]
        else {
            panic!("unexpected frame {frame:?}");
        };
        assert!(Rc::ptr_eq(future, old));
        assert!(future.is_done());
        assert_eq!(frame[2].to_string(), "{a = 5; b = {c = 2}}");
        assert_eq!(frame[3].to_string(), "{a = 1; b = {c = 2}}");
        let (Value::Record(updated), Value::Record(read)) = (&updated.values[1], &read.values[1])
        else {
            panic!("expected records");
        };
        assert!(Rc::ptr_eq(updated, read));
        // Every record along the path, by `let` and `:=`.
        assert_eq!(
            string("let r = lazy { a = lazy { b = 1; c = 2 } }\nr.a.b := 5\n\"{r}\""),
            "{a = {b = 5; c = 2}}"
        );
        assert_eq!(
            string(
                "fn f(conf: {a: {b: Int}}) -> {a: {b: Int}} { let conf.a.b = conf.a.b + 1\nconf }\n\
                 let x = f(lazy { a = lazy { b = 1 } })\n\
                 let y = f({ a = { b = 10 } })\n\
                 \"{await x} {await y}\""
            ),
            "{a = {b = 2}} {a = {b = 11}}"
        );
        // A failing future fails where the update awaits it.
        let diagnostic = run("let r = lazy { a = 1 // 0 }\nlet r.a = 2").unwrap_err();
        assert_eq!(diagnostic.message, "division by zero");
        assert_eq!(
            labels(&diagnostic).last(),
            Some(&(Span::new(32, 33), "awaited here"))
        );
    }

    #[test]
    fn awaits_through_every_walker() {
        let text = "let i = lazy 20\n\
                    let x = lazy 1.5\n\
                    let b = lazy true\n\
                    let s = lazy \"s\"\n\
                    let r = lazy { v = 2 }\n\
                    let g = lazy fn(): Int { 3 }\n\
                    \"{i + 1} {x * 2.0} {not b} {s + \"!\"} {r.v} {g()} {await i} {i} {r}\"";
        assert_eq!(string(text), "21 3.0 false s! 2 3 20 20 {v = 2}");
        // Each typed walker, at the root.
        assert_eq!(int("let i = lazy 20\nawait i"), 20);
        assert_eq!(float("let x = lazy 1.5\nx * 2"), 3.0);
        assert!(boolean("let b = lazy false\nnot b"));
        assert_eq!(string("let s = lazy \"a\"\ns + s"), "aa");
        assert_eq!(int("let r = lazy { v = 2 }\nr.v"), 2);
        assert_eq!(int("let g = lazy fn(): Int { 3 }\ng()"), 3);
        assert_eq!(
            run("let r = lazy { v = 2 }\nawait r").unwrap().to_string(),
            "{v = 2}"
        );
        // A future of a future, awaited fully.
        assert_eq!(int("let x = lazy lazy 7\nawait x + 1"), 8);
        assert_eq!(int("let x = lazy lazy 7\nx + 1"), 8);
        // An impure function taking and returning values that may be
        // futures, and a pure one passing one through.
        assert_eq!(int("fn f(x: Int) -> Int { x * 2 }\nf(lazy 20) + f(1)"), 42);
        assert_eq!(
            int(
                "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }\n\
                 let k = keep(lazy 40)\n\
                 k.v + 2"
            ),
            42
        );
        // An `if` of futures and present values.
        assert_eq!(
            int("let x = lazy 1\nlet y = if true { x } else { 2 }\ny + 1"),
            2
        );
        assert_eq!(
            int("let x = lazy 1\nlet y = if false { x } else { 2 }\ny + 1"),
            3
        );
    }

    #[test]
    fn a_deep_await_shares_what_it_does_not_change() {
        let frame = frame_after(
            "fn f(r: {a: Int, b: String}): {a: Int, b: String} { r }\n\
             let r = { a = lazy 1; b = \"text\" }\n\
             let s = f(r)",
        );
        let [_, Value::Record(r), Value::Record(s)] = &frame[..] else {
            panic!("unexpected frame {frame:?}");
        };
        // A new record, sharing the layout and the untouched field.
        assert!(!Rc::ptr_eq(r, s));
        assert!(Rc::ptr_eq(&r.layout, &s.layout));
        let (Value::Str(old), Value::Str(new)) = (&r.values[1], &s.values[1]) else {
            panic!("expected strings");
        };
        assert!(Rc::ptr_eq(old, new));
        assert_eq!(s.values[0], Value::Int(1));
        // The original still holds the future, now run.
        let Value::Future(future) = &r.values[0] else {
            panic!("expected a future");
        };
        assert!(future.is_done());
        // A record with nothing lazy in it is passed through as it is.
        let frame = frame_after(
            "let r: {a: Int | Future<Int>, b: {c: Int | Future<Int>}} = { a = 1; b = { c = 2 } }\n\
             let s: {a: Int, b: {c: Int}} = r",
        );
        let [Value::Record(r), Value::Record(s)] = &frame[..] else {
            panic!("unexpected frame {frame:?}");
        };
        assert!(Rc::ptr_eq(r, s));
        // Nested records are rebuilt only where they change.
        let frame = frame_after(
            "let r = { a = { b = lazy 1 }; c = { d = 2 }; e = lazy { f = lazy 3 } }\n\
             let s: {a: {b: Int}, c: {d: Int}, e: {f: Int}} = r",
        );
        assert_eq!(
            frame[1].to_string(),
            "{a = {b = 1}; c = {d = 2}; e = {f = 3}}"
        );
        // (The record block's temporaries leave values in later slots.)
        let [Value::Record(r), Value::Record(s), ..] = &frame[..] else {
            panic!("unexpected frame {frame:?}");
        };
        let (Value::Record(old), Value::Record(new)) = (&r.values[1], &s.values[1]) else {
            panic!("expected records");
        };
        assert!(Rc::ptr_eq(old, new));
        let (Value::Record(old), Value::Record(new)) = (&r.values[0], &s.values[0]) else {
            panic!("expected records");
        };
        assert!(!Rc::ptr_eq(old, new));
        // Equality and interpolation await deeply too.
        assert!(boolean(
            "fn f(r: {a: Int}) -> Bool { r == { a = 1 } }\nlet r = { a = lazy 1 }\nawait f(r)"
        ));
        assert_eq!(
            string("let r = { a = lazy 1; b = { c = lazy \"x\" } }\n\"{r}\""),
            "{a = 1; b = {c = \"x\"}}"
        );
    }

    #[test]
    fn a_future_displays_as_its_type() {
        let frame =
            frame_after("let f = lazy 1\nlet r = { a = f; b = lazy \"s\" }\nlet v = await f");
        let [Value::Future(future), record, Value::Int(1)] = &frame[..] else {
            panic!("unexpected frame {frame:?}");
        };
        // Whether it has run or not.
        assert!(future.is_done());
        assert_eq!(frame[0].to_string(), "Future<Int>");
        assert_eq!(frame[0].repr().to_string(), "Future<Int>");
        assert_eq!(frame[0].kind(), ValueKind::Deferred);
        assert_eq!(record.to_string(), "{a = Future<Int>; b = Future<String>}");
    }

    #[test]
    fn a_failed_await_truncates_the_stack() {
        let ir = compile("let x = lazy { let a = 1\nlet b = a // 0\nb }\nlet y = await x");
        let frame = ir.frame_size() as usize;
        let mut stack = vec![Value::Empty; frame];
        let (result, len) = run_on(&ir, &mut stack);
        assert_eq!(result.unwrap_err().message, "division by zero");
        assert_eq!(len, frame);
    }

    #[test]
    fn a_future_that_depends_on_itself_is_an_error() {
        // A future whose body stores the future, which it captured, in a
        // slot of its frame and awaits it: the checker never lets this
        // through, since no name is in scope in its own `lazy`.
        let mut builder = crate::ir::IrBuilder::with_capacity(8);
        let capture = builder.alloc(
            IrKind::LoadCapture(0, ValueKind::Deferred),
            Span::new(10, 11),
        );
        let store = builder.alloc(IrKind::Store(Slot::new(0), capture), Span::new(10, 11));
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Deferred),
            Span::new(20, 21),
        );
        let inner = builder.alloc(
            IrKind::Await {
                value: load,
                kind: ValueKind::Int,
                shape: LazyShapeId::NONE,
                depth: 0,
            },
            Span::new(14, 21),
        );
        let items = builder.push_item_list(&[store]);
        let body = builder.alloc(
            IrKind::Block {
                items,
                result: inner,
            },
            Span::new(8, 23),
        );
        let root_load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Deferred),
            Span::new(30, 31),
        );
        let root = builder.alloc(
            IrKind::Await {
                value: root_load,
                kind: ValueKind::Int,
                shape: LazyShapeId::NONE,
                depth: 0,
            },
            Span::new(24, 31),
        );
        let mut functions = vec![crate::ir::FnCode {
            body,
            kind: crate::ir::FnKind::Future,
            frame_size: 1,
            param_count: 0,
            capture_count: 1,
            height: 0,
            name: None,
            display: Box::from("Future<Int>"),
            slot_names: Box::default(),
        }];
        builder.measure(root, &mut functions);
        let ir = Rc::new(builder.finish_program(root, (1, Vec::new()), functions, 0));
        let future = Rc::new(Future::new(Rc::clone(&ir), FnId::new(0), Box::default()));
        // The future captures itself.
        assert!(
            future.finish(FutureState::Pending(Box::new([Value::Future(Rc::clone(
                &future
            ))])))
        );
        let mut stack = vec![Value::Future(Rc::clone(&future))];
        let diagnostic = eval_in(&ir, &mut stack).unwrap_err();
        assert_eq!(
            diagnostic.message,
            "this deferred expression depends on its own value"
        );
        assert_eq!(diagnostic.span, Span::new(14, 21));
        assert_eq!(
            labels(&diagnostic),
            vec![(Span::new(24, 31), "awaited here")]
        );
        assert_eq!(stack.len(), 1);
        // Left unrun; the cycle is broken so that nothing leaks.
        assert!(!future.is_done());
        assert!(future.finish(FutureState::Pending(Box::default())));
    }

    #[test]
    fn the_nesting_budget_covers_awaits() {
        on_big_stack(|| {
            // Each call awaits a future whose body makes the next call.
            let recurse = "fn f(n: Int) -> Int {\n\
                           \x20 let x = lazy { if n == 0 { 0 } else { f(n - 1) + 1 } }\n\
                           \x20 await x\n\
                           }\n";
            let ir = compile(&format!("{recurse}f(0)"));
            let [(outer, _)] = ir.calls_in(ir.root())[..] else {
                panic!("expected one call at the top level");
            };
            // The lazy is function 0, finished before `f`, function 1.
            let (future, function) = (ir.function(FnId::new(0)), ir.function(FnId::new(1)));
            let [(inner, call_span)] = ir.calls_in(future.body)[..] else {
                panic!("expected one call in the future's body");
            };
            let [(awaited, await_span)] = ir.awaits_in(function.body)[..] else {
                panic!("expected one await in the function's body");
            };
            let (future_height, function_height) =
                (future.height as usize, function.height as usize);
            // Walk the chain as the evaluator charges it: each call enters
            // its frame at its depth plus `CALL_LEVELS` further in, and the
            // body's await enters the future's frame the same way. `f(n)`
            // makes n + 1 calls and runs as many futures; count how many
            // pairs fit.
            let mut pairs = 0_usize;
            let mut levels = 0;
            let mut depth = usize::from(outer);
            loop {
                let entering = levels + depth + CALL_LEVELS;
                if entering + function_height > MAX_DEPTH {
                    break;
                }
                let running = entering + usize::from(awaited) + CALL_LEVELS;
                if running + future_height > MAX_DEPTH {
                    break;
                }
                pairs += 1;
                levels = running;
                depth = usize::from(inner);
            }
            assert!(pairs > 100, "{pairs}");
            let last = i64::try_from(pairs - 1).unwrap();
            assert_eq!(
                run(&format!("{recurse}f({last})")).unwrap(),
                Value::Int(last)
            );
            let diagnostic = run(&format!("{recurse}f({})", last + 1)).unwrap_err();
            assert_eq!(diagnostic.message, BUDGET_ERROR);
            assert_eq!(diagnostic.notes, vec![BUDGET_NOTE]);
            assert!(
                diagnostic.span == call_span || diagnostic.span == await_span,
                "{diagnostic:?}"
            );
        });
    }

    /// A kind of nesting to wrap a recursive call in, for
    /// `the_nesting_budget_fits_the_stack_for_awaits`: its name, the return
    /// type of the function whose call it wraps, how it wraps an
    /// expression in 60 levels, and the value of a recursion making the
    /// number of calls given.
    type AwaitNesting = (
        &'static str,
        &'static str,
        fn(&str) -> String,
        fn(i64) -> Value,
    );

    /// The levels of nesting each kind of `AwaitNesting` wraps a call in.
    const AWAIT_WRAP: usize = 60;

    /// The costly kinds of nesting, as the end-to-end tests list them for
    /// calls.
    fn await_nesting_kinds() -> [AwaitNesting; 5] {
        [
            (
                "additions",
                "Int",
                |inner| format!("{inner}{}", " + 1".repeat(AWAIT_WRAP)),
                |calls| Value::Int(calls * i64::try_from(AWAIT_WRAP).unwrap() - 1),
            ),
            (
                "negations",
                "Int",
                |inner| {
                    format!(
                        "{}{inner}{}",
                        "-(".repeat(AWAIT_WRAP),
                        ")".repeat(AWAIT_WRAP)
                    )
                },
                |_| Value::Int(-1),
            ),
            (
                "interpolated strings",
                "String",
                |inner| {
                    format!(
                        "{}{inner}{}",
                        "\"{".repeat(AWAIT_WRAP),
                        "}\"".repeat(AWAIT_WRAP)
                    )
                },
                |_| Value::Str("-1".into()),
            ),
            (
                "nested blocks",
                "Int",
                |inner| {
                    format!(
                        "{}{inner}{}",
                        "{ ".repeat(AWAIT_WRAP),
                        " }".repeat(AWAIT_WRAP)
                    )
                },
                |_| Value::Int(-1),
            ),
            (
                "blocks with a declaration each",
                "Int",
                |inner| {
                    format!(
                        "{}{inner}{}{}",
                        "{ let v = 1\n".repeat(AWAIT_WRAP / 2),
                        " + v".repeat(AWAIT_WRAP / 2),
                        "\n}".repeat(AWAIT_WRAP / 2)
                    )
                },
                |calls| Value::Int(calls * i64::try_from(AWAIT_WRAP / 2).unwrap() - 1),
            ),
        ]
    }

    /// `STACK_SIZE` covers the nesting budget when it is spent on awaits as
    /// well as calls, unoptimised, whatever kind of nesting they are made
    /// from: for each costly kind, a recursion through futures, each call
    /// awaiting a future whose body makes the next call wrapped in that
    /// nesting, runs as many calls as the budget allows, which takes the
    /// deepest future's body to exactly `MAX_DEPTH` levels, and one more
    /// call fails.
    #[test]
    fn the_nesting_budget_fits_the_stack_for_awaits() {
        on_big_stack(|| {
            let wrap = AWAIT_WRAP;
            for (kind, ret, wrap_in, value) in await_nesting_kinds() {
                // `f(n)` makes n + 1 calls, each running a future.
                let program = |calls: usize, padding: usize| {
                    format!(
                        "fn f(n: Int) -> {ret} {{\n\
                         \x20 await lazy {{ if n == 0 {{ {} }} else {{ {} }} }}\n\
                         }}\n{}f({}){}",
                        wrap_in("-(-(n - 1))"),
                        wrap_in("f(n - 1)"),
                        "{ ".repeat(padding),
                        calls - 1,
                        " }".repeat(padding)
                    )
                };
                // The shape of the IR does not depend on the count, and the
                // padding only moves the first call deeper.
                let ir = compile(&program(1, 0));
                let [(outer, _)] = ir.calls_in(ir.root())[..] else {
                    panic!("{kind}: expected one call at the top level");
                };
                let (future, function) = (ir.function(FnId::new(0)), ir.function(FnId::new(1)));
                let [(inner, _)] = ir.calls_in(future.body)[..] else {
                    panic!("{kind}: expected one call in the future's body");
                };
                let [(awaited, _)] = ir.awaits_in(function.body)[..] else {
                    panic!("{kind}: expected one await in the function's body");
                };
                let (outer, inner, awaited) =
                    (usize::from(outer), usize::from(inner), usize::from(awaited));
                assert!(inner > wrap, "{kind}: {inner}");
                let (future_height, function_height) =
                    (future.height as usize, function.height as usize);
                // Each call enters its frame at its depth plus `CALL_LEVELS`
                // further in, and its body's await enters the future's frame
                // the same way; the deepest future's body is the tallest
                // part. With the padding making up the remainder, it
                // reaches exactly `MAX_DEPTH`.
                let first = outer + CALL_LEVELS + awaited + CALL_LEVELS + future_height;
                let step = inner + CALL_LEVELS + awaited + CALL_LEVELS;
                let room = MAX_DEPTH - first;
                let (calls, padding) = (1 + room / step, room % step);
                assert!(calls > 50, "{kind}: {calls}");
                // Every call's frame fits too.
                let mut entering = padding + outer + CALL_LEVELS;
                for _ in 0..calls {
                    assert!(entering + function_height <= MAX_DEPTH, "{kind}");
                    let running = entering + awaited + CALL_LEVELS;
                    assert!(running + future_height <= MAX_DEPTH, "{kind}");
                    entering = running + inner + CALL_LEVELS;
                }
                let expected = value(i64::try_from(calls).unwrap());
                assert_eq!(run(&program(calls, padding)), Ok(expected), "{kind}");
                // One more call, or one more level of padding, is past the
                // budget: it is used exactly.
                for past in [program(calls + 1, padding), program(calls, padding + 1)] {
                    let diagnostic = run(&past).unwrap_err();
                    assert_eq!(diagnostic.message, BUDGET_ERROR, "{kind}");
                }
            }
        });
    }

    #[test]
    fn a_deep_await_of_a_deep_record() {
        on_big_stack(|| {
            // Record nesting is limited by the parser's, two levels a
            // record; the deep await descends it all, one frame a level,
            // and runs the future at the bottom.
            let depth = 4_000;
            let text = format!(
                "let r = {}lazy 1{}\n\"{{r}}\"",
                "{ a = ".repeat(depth),
                " }".repeat(depth)
            );
            let shown = string(&text);
            assert!(shown.starts_with("{a = {a = "));
            assert!(shown.ends_with(&format!("{{a = 1{}", "}".repeat(depth))));
            assert_eq!(shown.len(), depth * "{a = }".len() + 1);
        });
    }

    #[test]
    fn mistyped_awaits_are_internal_errors() {
        // An await whose value is of another kind than it states, through
        // the general walker and a typed one.
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
        let awaited = builder.alloc(
            IrKind::Await {
                value: one,
                kind: ValueKind::Bool,
                shape: LazyShapeId::NONE,
                depth: 0,
            },
            Span::new(0, 7),
        );
        let ir = Rc::new(builder.finish(awaited, 0, Vec::new()));
        assert_eq!(
            eval(&ir).unwrap_err().message,
            "internal error: an await produced a value of another kind than its node"
        );
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let one = builder.alloc(IrKind::IntConst(1), Span::new(0, 1));
        let awaited = builder.alloc(
            IrKind::Await {
                value: one,
                kind: ValueKind::Bool,
                shape: LazyShapeId::NONE,
                depth: 0,
            },
            Span::new(0, 7),
        );
        let not = builder.alloc(IrKind::Not(awaited), Span::new(0, 11));
        let ir = Rc::new(builder.finish(not, 0, Vec::new()));
        assert_eq!(
            eval(&ir).unwrap_err().message,
            "internal error: an await produced a value of another kind than its node"
        );
        // A typed walker meeting a node that may be a future, whatever the
        // slot holds.
        let mut builder = crate::ir::IrBuilder::with_capacity(3);
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Deferred),
            Span::new(0, 1),
        );
        let one = builder.alloc(IrKind::IntConst(1), Span::new(4, 5));
        let add = builder.alloc(
            IrKind::IntAdd(Operands {
                lhs: load,
                rhs: one,
                op_start: 2,
            }),
            Span::new(0, 5),
        );
        let ir = Rc::new(builder.finish(add, 1, Vec::new()));
        let error = eval_in(&ir, &mut vec![Value::Int(2)]).unwrap_err();
        assert_eq!(
            error.message,
            "internal error: a node that may be a future where an Int was expected"
        );
        assert_eq!(error.span, Span::new(0, 1));
        // A deep await of a record lacking a field of its shape.
        let mut names = crate::names::Names::new();
        let (x, y) = (names.intern("x"), names.intern("y"));
        let mut builder = crate::ir::IrBuilder::with_capacity(2);
        let shape = builder.push_lazy_shape(crate::ir::LazyShape {
            fields: Box::new([(y, crate::ir::LazyField::Await(LazyShapeId::NONE))]),
        });
        let load = builder.alloc(
            IrKind::Load(Slot::new(0), ValueKind::Record),
            Span::new(0, 1),
        );
        let awaited = builder.alloc(
            IrKind::Await {
                value: load,
                kind: ValueKind::Record,
                shape,
                depth: 0,
            },
            Span::new(0, 1),
        );
        let ir = Rc::new(builder.finish(awaited, 1, Vec::new()));
        let record = Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: Box::new([x]),
                texts: Box::new([Box::from("x")]),
            }),
            values: Box::new([Value::Int(1)]),
        }));
        assert_eq!(
            eval_in(&ir, &mut vec![record]).unwrap_err().message,
            "internal error: a record lacks a field of its lazy shape"
        );
        // The empty record has no fields to await.
        assert_eq!(eval_in(&ir, &mut vec![Value::Empty]).unwrap(), Value::Empty);
    }
}
