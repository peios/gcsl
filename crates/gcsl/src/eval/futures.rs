//! Evaluating futures: making them, awaiting them at the top or deeply, and
//! running them.
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
//! level of record or list nesting a deep await has descended to reach it.
//! Its first await runs the body in a frame of its own, on the value stack
//! above the
//! awaiting frame's, and records the value, dropping the captures; every
//! later await is a borrow and a clone of that value. A deep await then
//! awaits the lazy fields its shape lists, recursively, building a new
//! record, sharing the layout and the untouched values, only if a field
//! changed; and the elements of a list, each as a list's lazy shape says,
//! in order, building a new list, sharing the untouched elements, only if an
//! element changed. Each element's future runs at most once, however many
//! lists hold it, since a future is shared. An error inside a body leaves
//! the future unrun, with its
//! captures, so a later await runs it again, and gets the context label
//! "awaited here" on the await that ran it, when both are of the same
//! session input; otherwise the future's input is stamped on it, as a call
//! stamps a closure's. A future is marked as running while its body runs,
//! so that an await of it from inside its own body, which the checker never
//! lets through, is an error rather than an endless loop.

use std::rc::Rc;

use crate::{CALL_LEVELS, MAX_DEPTH};
use crate::ast::FnId;
use crate::diagnostic::Diagnostic;
use crate::ir::{IrKind, IrNode, IrRange, LazyField, LazyShapeId};
use crate::value::{Begin, Future, FutureState, List, Record, Value};

use super::{Evaluator, Flow, MIXED, depends_on_itself, mistyped, nested_too_deeply, replaced};
use super::kinds::{mixed, of_kind};
use super::memo::Ambient;

impl Evaluator<'_> {
    /// Builds a future of `function` capturing the values of `captures`,
    /// read in this frame, as [`Evaluator::make_closure`] builds a closure:
    /// one allocation for the captured values and one for the future, which
    /// shares the IR. Nothing is run. The future captures this frame's
    /// caller, helper flag and origin, which its body runs with (see
    /// [Callers and helpers](crate::eval::callers#callers-and-helpers)). Making it is a read of
    /// this frame's caller, recorded on the running memoised calls of its
    /// origin, if the body can read the caller it captures (`reads_caller`,
    /// the `lazy`'s mark) or this frame's helper flag is set, so that its
    /// body's calls pass that caller on (see [Memoised
    /// functions](crate::eval::memo#memoised-functions), "Ambient inputs").
    pub(super) fn make_future(
        &mut self,
        function: FnId,
        captures: IrRange,
        reads_caller: bool,
    ) -> Flow<Value> {
        let items = self.ir.items(captures);
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.value(item)?);
        }
        let made = self.shared_caller();
        if reads_caller || made.helper {
            self.memo.depend(made.origin, &Ambient::Caller(made.dir));
        }
        // Its body may read any context it captures (see [Context](crate::eval::context#context)).
        self.memo.depend_contexts();
        let context = self.shared.context.borrow().clone();
        Ok(Value::Future(Rc::new(Future::made_by(
            Rc::clone(self.ir),
            function,
            values.into_boxed_slice(),
            (made, context),
        ))))
    }

    /// Evaluates the `Await` `node` (see the module documentation) as a
    /// value, which must be of the kind the node states, or, for
    /// [`ValueKind::Mixed`](super::ValueKind::Mixed), of any kind but a future's.
    pub(super) fn await_node(&mut self, node: &IrNode) -> Flow<Value> {
        let value = self.await_value(node)?;
        match node.kind {
            IrKind::Await { kind, .. } if of_kind(&value, kind) => Ok(value),
            _ => mistyped(
                node,
                "an await produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `Await` `node` as a value that `extract` accepts: one
    /// of the kind the node states.
    pub(super) fn await_as<T>(
        &mut self,
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
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
    pub(super) fn await_value(&mut self, node: &IrNode) -> Flow<Value> {
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
            self.await_deep(awaited, shape, (depth, 1, false), node)
        }
    }

    /// Awaits `value` fully, for the await `node` at depth `depth` in this
    /// frame's body, `level` levels of record nesting below the value it
    /// awaits: while it is a future, runs it and continues with what it
    /// yields.
    pub(super) fn await_fully(
        &mut self,
        value: Value,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Flow<Value> {
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
    /// body, the record's fields being `level` levels of record and list
    /// nesting below the value the await awaits: each field that is lazy is
    /// awaited fully and then, if its shape says so, deeply, and each
    /// present record or list field with lazy values inside is awaited
    /// deeply. If any field changed, the result is a new record sharing the
    /// layout and the values that did not; otherwise it is the record as it
    /// was. The empty record has no fields to await. A list's elements are
    /// awaited by [`Evaluator::await_elements`] instead. A union's shape
    /// applies each of its members' in turn, `tolerant`ly: a field the
    /// record lacks, or a shape of another kind than the value's, is passed
    /// over, there and at every level below (see [`Ir::lazy_union`](super::Ir::lazy_union)), where
    /// otherwise it is an internal error.
    pub(super) fn await_deep(
        &mut self,
        value: Value,
        shape: LazyShapeId,
        (depth, level, tolerant): (u16, usize, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let ir = self.ir;
        if let Some(members) = ir.lazy_union(shape) {
            let mut value = value;
            for &member in members {
                value = self.await_deep(value, member, (depth, level, true), node)?;
            }
            return Ok(value);
        }
        if matches!(value, Value::List(_)) {
            return self.await_elements(value, shape, (depth, level, tolerant), node);
        }
        let Value::Record(record) = &value else {
            return Ok(value);
        };
        let Some(lazy) = ir.find_lazy_shape(shape) else {
            return mistyped(node, "an await of a lazy shape its IR lacks");
        };
        let mut changed: Option<Vec<Value>> = None;
        for &(name, field) in &lazy.fields {
            let found = record.layout.names.iter().position(|&field| field == name);
            let Some((index, old)) =
                found.and_then(|index| Some((index, record.values.get(index)?)))
            else {
                if tolerant {
                    continue;
                }
                return mistyped(node, "a record lacks a field of its lazy shape");
            };
            let new = self.await_field(old, field, (depth, level, tolerant), node)?;
            if replaced(old, &new) {
                let values = changed.get_or_insert_with(|| record.values.to_vec());
                if let Some(slot) = values.get_mut(index) {
                    *slot = new;
                }
            }
        }
        // Every field the type does not name, as its rest-field says.
        if let Some(rest) = &lazy.rest {
            for (index, (name, old)) in record.layout.names.iter().zip(&record.values).enumerate() {
                if rest.named.binary_search(name).is_ok() {
                    continue;
                }
                let new = self.await_field(old, rest.field, (depth, level, tolerant), node)?;
                if replaced(old, &new) {
                    let values = changed.get_or_insert_with(|| record.values.to_vec());
                    if let Some(slot) = values.get_mut(index) {
                        *slot = new;
                    }
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

    /// Awaits `old`, a field of a record or an element of a list, as
    /// `field` says, for the await `node` at depth `depth` in this frame's
    /// body, `level` levels of record and list nesting below the value the
    /// await awaits: fully and then, if its shape says so, deeply, or, if
    /// it is present, deeply (see [`Evaluator::await_deep`]).
    pub(super) fn await_field(
        &mut self,
        old: &Value,
        field: LazyField,
        (depth, level, tolerant): (u16, usize, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        match field {
            LazyField::Await(inner) => {
                let awaited = self.await_fully(old.clone(), depth, level, node)?;
                if inner == LazyShapeId::NONE {
                    Ok(awaited)
                } else {
                    self.await_deep(awaited, inner, (depth, level + 1, tolerant), node)
                }
            }
            LazyField::Within(inner) => {
                self.await_deep(old.clone(), inner, (depth, level + 1, tolerant), node)
            }
        }
    }

    /// Awaits the elements of `value`, a list, as the list's lazy shape
    /// `shape` says to await each, for the await `node` at depth `depth` in
    /// this frame's body, the elements being `level` levels of record and
    /// list nesting below the value the await awaits, as
    /// [`Evaluator::await_deep`] awaits a record's fields: each element is
    /// awaited fully and then, if the shape says so, deeply, or, if it is
    /// present, deeply. If any element changed, the result is a new list,
    /// sharing the elements that did not; otherwise it is the list as it
    /// was. A shape that is not a list's is passed over if `tolerant`, as
    /// a member's shape of a union's is (see [`Evaluator::await_deep`]).
    pub(super) fn await_elements(
        &mut self,
        value: Value,
        shape: LazyShapeId,
        (depth, level, tolerant): (u16, usize, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let Value::List(list) = &value else {
            return Ok(value);
        };
        let Some(each) = self.ir.lazy_list(shape) else {
            if tolerant {
                return Ok(value);
            }
            return mistyped(node, "an await of a list's lazy shape its IR lacks");
        };
        let mut changed: Option<Vec<Value>> = None;
        for (index, old) in list.values().iter().enumerate() {
            let new = self.await_field(old, each, (depth, level, tolerant), node)?;
            if replaced(old, &new) {
                let values = changed.get_or_insert_with(|| list.values().to_vec());
                if let Some(slot) = values.get_mut(index) {
                    *slot = new;
                }
            }
        }
        match changed {
            Some(values) => Ok(Value::List(List::from_vec(values))),
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
    pub(super) fn run_future(
        &mut self,
        future: &Rc<Future>,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Flow<Value> {
        let Some(code) = future.code.find_function(future.function) else {
            return mistyped(node, "a future of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + level + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let captures = match future.begin() {
            Some(Begin::Done(value)) => return Ok(value),
            Some(Begin::Running) => return Err(depends_on_itself(node).into()),
            Some(Begin::Pending(captures)) => captures,
            None => return mistyped(node, "a future's state is in use"),
        };
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        // The body runs as if where it was written, with the caller, the
        // helper flag and the origin of the frame that made the future, in
        // the context it was made in.
        let outer = self.shared.context.replace(future.context.clone());
        let result = Evaluator {
            ir: &future.code,
            stack: self.stack,
            base,
            captures: &captures,
            callee: None,
            memo: self.memo,
            shared: self.shared,
            caller: future.made,
            levels: entering,
        }
        .value(code.body);
        self.shared.context.replace(outer);
        self.stack.truncate(base);
        // A `return` for the `lazy` is its value, recorded as any is, here,
        // inside the running mark; no other stop leaves its body: the
        // checker sees to it.
        match result.or_else(|unwind| unwind.returned(node.span)) {
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
                    Err(self.awaited_error(error, future, node).into())
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
    pub(super) fn awaited_error(
        &self,
        mut error: Diagnostic,
        future: &Future,
        node: &IrNode,
    ) -> Diagnostic {
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
}
