//! Evaluating loops: `for` and `while`, the stops that leave them, and the
//! values they leave with.
//!
//! # Stops
//!
//! Every walker returns a `Flow`: a value, or an `Unwind`, the one road out
//! of a node that does not yield one. An `Unwind` is an error, a
//! [`Diagnostic`](super::Diagnostic), which ends the evaluation, or a *stop*, a `Stop`: a
//! `break` or a `continue` on its way to its loop, with the number of loops
//! it is still to leave, or a `return` on its way to what it leaves, named
//! by the number of its [`CatchId`](super::CatchId) in the same `u32`, each with its value,
//! if any, and what it does there (`StopKind`). So a stop is as small as it
//! was before `return`, and a loop handles a `break` or `continue` as it
//! did. A stop travels
//! as an error does, through every `?`, so every node between a `break` and
//! its loop is left at once, with nothing after the `break` evaluated: in
//! `{ if c { break }; 1 } + g()`, `g` is not called. (A flag that the loop
//! checks would let the `+` and its call run.) A `Break`, `Continue` or
//! `Return` node evaluates its value through the general walker and then
//! unwinds (`Evaluator::leave`); every walker, typed or general, hands it
//! such a node, since a node of kind [`ValueKind::Never`](super::ValueKind::Never) may stand where
//! one of any kind is expected. A catch (see below) may stand under any
//! walker too, and so may a loop that cannot end, whose node is of kind
//! `Never` where it stands, which the walker runs. So each typed walker has
//! one arm for all of them, handing them to one helper
//! (`Evaluator::stopping`), and pays nothing else; the general walker has
//! one more arm, for a catch (`Evaluator::catch`). These helpers, and the
//! record walker's for its loops (`Evaluator::record_loop`), are kept out
//! of line, so that no walker's frame, on the recursion path of every
//! expression, grows: with an arm each in every typed walker, inlined, the
//! new nodes cost a loop of a few million iterations two or three per
//! cent. `leave` and `caught` are left to the compiler, as they were.
//!
//! Only a loop catches a `break` or `continue`, and only one that comes out
//! of its body (`caught`): a stop out of its iterable, guard, condition or
//! items after its condition is for a loop around it, and goes on with
//! `?`. A stop with no loops left to leave is this loop's: its value, if
//! any, is collected, and the loop ends, for a `break`, or goes on with its
//! items after the condition and its next test, for a `continue`. Any other
//! goes on, one loop nearer its own, and this loop's list is dropped with
//! it. A `return` goes through every loop as it is, the loop's list and
//! after-items with it. Its target catches it, three ways:
//!
//! - a `Catch`, around a labelled block or a loop that a `return` leaves,
//!   catches one that names its number, coming out of the block or loop, and
//!   yields its value (`Evaluator::catch`); any other stop goes on;
//! - a call, a memoised call and an await running a future catch one that names
//!   [`CatchId::FUNCTION`](super::CatchId::FUNCTION) coming out of the body,
//!   the value being the body's (`Unwind::returned`). The catch is on the error
//!   road only, a [`Result::or_else`] of the body's result, so that a body that
//!   gives a value takes exactly the road it took before `return` existed
//!   (written as a function of the whole result, matched on every call, it cost
//!   a call-heavy program a few per cent). It is inside the memoised call's
//!   running entry and the future's running mark: the value returned is then
//!   recorded by `Evaluator::memo_end`, and resolves the future, by the road
//!   any value of a body takes, rather than reaching them as an `Err`, which
//!   clears the entry or leaves the future unrun.
//!
//! The checker lets no other stop out of a function, a `lazy` or a memoised
//! call, nor any out of the top level, so one that reaches such a boundary
//! is an internal error there (`Unwind::into_error`), never a diagnostic,
//! and never caught by a loop or catch of another frame. A catch's number
//! is unique in its program, and no node holds itself within one frame, so
//! a catch never takes another's `return`.
//!
//! Since evaluation goes on after a stop, nothing may assume that an `Err`
//! ends it all and skip restoring what it changed. What was found, and done:
//!
//! - The nesting budget is a field of each frame's `Evaluator`, `levels`,
//!   set when the frame is made and never changed, and charged statically:
//!   nothing is counted up and down around a node, so nothing can leak.
//! - The value stack: a call grows it by the callee's frame before its
//!   arguments are evaluated, and an argument may now stop, as in `f({
//!   continue })`, or return from the function the call is in, as in
//!   `f(g({ return 1 }))`: `Evaluator::call` and `Evaluator::memo_begin`
//!   truncate it back on any `Err` of an argument, as they did for an
//!   error, and the stop goes on. A call, a memoised call and a future
//!   truncate it after the body whatever the body gave, a `return` for it
//!   included, and `eval_with` at the top.
//! - A future's running mark: `Evaluator::run_future` puts the future back
//!   unrun on any `Err` of its body; a `return` for the `lazy` is its value,
//!   caught before, and recorded as any value is; no other stop can come
//!   out of a body, and is made an internal error first.
//! - A memoised call's running entry: `Evaluator::memo_end` removes it on
//!   any `Err` of the body; a `return` for the function is its value,
//!   caught before, and recorded as any value is, so that a function that
//!   returns early is memoised; any other stop is made an internal error
//!   first. The memo tables' checkpoint is rolled back only by a
//!   [`crate::session::Session`], and only when a whole input fails: a stop
//!   in the body of a `->>` function is caught by its loop or catch, or by
//!   its call, in the body, and reaches neither the call's caller nor the
//!   session, so the entries the body's calls rightly recorded stay.
//! - A `return` out of the value of an update (`r.a := { return 1 }`) or
//!   out of a position of its path leaves before anything is stored, the
//!   records the walk read dropped with it; one out of an interpolation
//!   drops the text built so far, one out of an `if`'s narrowed branch
//!   leaves the slot the `is` wrote, which belongs to the `if`.
//! - The input stamped on an error from another input's code, and the
//!   "awaited here" and "called here with the same arguments" labels, are
//!   added to errors only.
//! - What a node holds while it evaluates its children (a list being built,
//!   the records an update passes through, a loop's collected values) is
//!   local to its helper's frame, and dropped as the stop passes.
//! - Slots written for a node, a loop's variable and an `is`'s narrowed
//!   value, are not restored: they belong to that node's scope, which no
//!   code after it reads without writing first.
//!
//! An `UpdateField`, a dotted field's, builds a new record and never
//! changes the old one, as a step of an `Update` that sets a field does
//! (see [Updates](crate::eval::updates#updates)). It clones the old record's values into a new
//! boxed slice, one allocation, with the updated field's value replaced or
//! appended. A replacement shares the old record's layout. An addition
//! needs a new layout: the old one's names and texts with the added name
//! after them, whose text the evaluator takes from [`Ir::name_text`](super::Ir::name_text), since
//! it has no names table and must never need one. Every addition of a name
//! to a record of a given layout makes an equal layout anew; caching the
//! result per old layout and name is the optimisation for a bytecode VM,
//! which will have a place to keep such a cache at each update site.
//! Updating the empty record makes a record of the one field.
//!
//! Typed equality, `RecordEq` and `RecordNe`, compares two records over the
//! [`crate::ir::Shape`] of their static type: for each field of the shape,
//! it looks the field up in both records by name, compares a record field
//! through its own shape and any other field with `==` on [`Value`], and
//! ignores every field the shape lacks. Nested records are compared through
//! a work list, as total equality compares them (see
//! [`crate::value::Record`]), so nesting does not recurse and comparing flat
//! records allocates nothing. Total equality, `RecordSame` and
//! `RecordNotSame`, is [`Value`]'s `PartialEq`. `ListEq` and `ListNe`
//! compare two lists' lengths and then their elements in order, each
//! through the shape of the element type, if it is a record or a list, as
//! the list's [`crate::ir::ListShape`] says, and with `==` otherwise, in the
//! same work list; `ListSame` and `ListNotSame` are [`Value`]'s `PartialEq`.
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
//! A slot may be written many times: a mutable variable's by each of its
//! rebinds, from any block of its function, a `for`'s body included, as well
//! as a `for`'s variable's by each iteration. A `Load` reads the slot when
//! it is evaluated, and evaluation is in source order, so a read sees the
//! last write before it: in `let mut x = 1; let y = x + { x := 5; x }`, `y`
//! is 6. A `Load` copies the value, a reference-count increment, and never
//! moves it out of the slot, and nothing caches a slot's value, so a `for`'s
//! guard reads what its body last wrote, and a closure or a future copies
//! the value a slot holds when it is made, which later writes do not
//! change. A value already read is held, as any operand is, while what
//! follows it is evaluated: `r.a := e` evaluates `e` first, which may
//! rebind `r` itself, and then reads `r`, as it then is, and stores the
//! update of that record (see [Updates](crate::eval::updates#updates)); and a `for` iterates
//! over the list its iterable was when it began, whatever its body
//! rebinds.

use crate::check::Slot;
use crate::ir::{IrKind, IrNode, IrRange};
use crate::value::{List, Value};

use super::{Evaluator, FOR_RESERVE, Flow, MIXED, Stop, StopKind, Unwind, caught, internal, mistyped};
use super::kinds::mixed;

impl Evaluator<'_> {
    /// Evaluates the `For` `node`, whose variable is in `slot` and whose
    /// parts are `parts` (see [`IrKind::For`]): its iterable once, and then,
    /// for each element in order, the element written to the slot, the
    /// guard, if it has one, and if that is true or absent, the body. If
    /// `collect`, the body's values are collected, in a vector sized up
    /// front to the number of elements when there is no guard to skip
    /// some, and then copied into the list's allocation, so that the whole
    /// takes time linear in the list's length; otherwise they are dropped
    /// as they come, and the `for` is `{}`. The first error stops the loop.
    /// Each iteration evaluates the body afresh at the same depth, so none
    /// nests in another.
    pub(super) fn for_loop(
        &mut self,
        (slot, parts, collect): (Slot, IrRange, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let (iterable, guard, body) = match *self.ir.items(parts) {
            [iterable, body] => (iterable, None, body),
            [iterable, guard, body] => (iterable, Some(guard), body),
            _ => return mistyped(node, "a `for` without its iterable and body"),
        };
        let elements = self.elements(iterable)?;
        let mut values = Vec::new();
        if collect && guard.is_none() {
            // A range of any length may be iterated over: no more than this
            // is reserved up front, and the vector grows past it only as
            // the body's values come.
            values.reserve_exact(elements.remaining().min(FOR_RESERVE));
        }
        let at = self.base + slot.index();
        for element in elements {
            if self.past_deadline() {
                return self.timed_out(node);
            }
            let Some(variable) = self.stack.get_mut(at) else {
                return Err(internal("a `for`'s variable is outside the frame", node.span).into());
            };
            *variable = element;
            // A stop out of the guard is for a loop around this one.
            if let Some(guard) = guard
                && !self.bool(guard)?
            {
                continue;
            }
            match self.value(body) {
                Ok(value) if collect => values.push(value),
                Ok(_) => {}
                Err(unwind) => {
                    if caught(unwind, &mut values, collect)? {
                        break;
                    }
                }
            }
        }
        Ok(if collect {
            Value::List(List::from_vec(values))
        } else {
            Value::Empty
        })
    }

    /// Evaluates the loop `node`, a `For` or a `Loop`, by the helper for its
    /// kind: the walkers only call this.
    pub(super) fn looped(&mut self, node: &IrNode) -> Flow<Value> {
        match node.kind {
            IrKind::For {
                slot,
                parts,
                collect,
                ..
            } => self.for_loop((slot, parts, collect), node),
            IrKind::Loop {
                parts,
                collect,
                test_first,
            } => self.condition_loop((parts, collect, test_first), node),
            _ => mistyped(node, "a node that is not a loop looped"),
        }
    }

    /// Evaluates the `Loop` `node`, whose parts are `parts`, a condition
    /// loop or a `do` loop (see [`IrKind::Loop`]): while its condition is
    /// true, tested first only if `test_first`, its body, and then the items
    /// after its condition, each for its effects. If `collect`, what each
    /// iteration contributes is collected, in a vector that grows as it
    /// comes, its length not being known up front, and then copied into the
    /// list's allocation; otherwise it is dropped, and the loop is `{}`. A
    /// stop out of its body is caught as [`caught`] says; one out of its
    /// condition or an item after it is for a loop around this one. Each
    /// iteration evaluates the body afresh at the same depth, so none nests
    /// in another.
    pub(super) fn condition_loop(
        &mut self,
        (parts, collect, test_first): (IrRange, bool, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let ir = self.ir;
        let Some((&cond, rest)) = ir.items(parts).split_first() else {
            return mistyped(node, "a loop without its condition");
        };
        let Some((&body, after)) = rest.split_first() else {
            return mistyped(node, "a loop without its body");
        };
        let mut values = Vec::new();
        let mut test = test_first;
        loop {
            if self.past_deadline() {
                return self.timed_out(node);
            }
            if test && !self.bool(cond)? {
                break;
            }
            test = true;
            match self.value(body) {
                Ok(value) if collect => values.push(value),
                Ok(_) => {}
                Err(unwind) => {
                    if caught(unwind, &mut values, collect)? {
                        break;
                    }
                }
            }
            for &item in after {
                self.value(item)?;
            }
        }
        Ok(if collect {
            Value::List(List::from_vec(values))
        } else {
            Value::Empty
        })
    }

    /// Evaluates the `Break`, `Continue` or `Return` `node`: its value, if
    /// it has one, through the general walker, and then the stop that
    /// carries it to its loop, or to what it returns from, unwinding every
    /// node on the way, as an error does. Every walker hands such a node
    /// here, the typed walkers through [`Evaluator::stopping`], since one
    /// of kind [`ValueKind::Never`](super::ValueKind::Never) may stand where a value of any kind is
    /// expected. A `return` names its catch by number in the `u32` a
    /// `break` counts its loops in (see [`Stop`]).
    pub(super) fn leave<T>(&mut self, node: &IrNode) -> Flow<T> {
        let (kind, to, value) = match node.kind {
            IrKind::Break { loops, value } => (StopKind::Break, loops, value),
            IrKind::Continue { loops, value } => (StopKind::Continue, loops, value),
            IrKind::Return { to, value } => (StopKind::Return, to.number(), value),
            _ => {
                return mistyped(
                    node,
                    "a node that is not a `break`, `continue` or `return` left a loop",
                );
            }
        };
        let value = match value {
            Some(value) => Some(self.value(value)?),
            None => None,
        };
        Err(Unwind::Stop(Stop { to, kind, value }))
    }

    /// Evaluates the `Catch` `node` (see [`IrKind::Catch`]): its block or
    /// loop, through the general walker, whose value it yields, or the
    /// value of a `return` for it that comes out of it, `{}` for one
    /// without. Any other stop, and an error, go on as they are. Never
    /// inlined into the general walker, whose frame, on the recursion path
    /// of every expression, it would grow.
    #[inline(never)]
    pub(super) fn catch(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::Catch { body, id, .. } = node.kind else {
            return mistyped(node, "a node that is not a catch caught");
        };
        match self.value(body) {
            Err(Unwind::Stop(Stop {
                kind: StopKind::Return,
                to,
                value,
            })) if to == id.number() => Ok(value.unwrap_or(Value::Empty)),
            flow => flow,
        }
    }

    /// Evaluates, for a typed walker, a node that stops or may, as a value
    /// that `extract` accepts, of the kind the walker wants: a `break`,
    /// `continue` or `return` unwinds as [`Evaluator::leave`] says; a
    /// catch's value must be one `extract` accepts, of the kind the node
    /// states; and a loop where a value of another kind than a loop's is
    /// wanted is one that cannot end, of type `Never` (see [`crate::ir`]),
    /// which runs until something leaves it, and is mistyped if it ends.
    /// One arm of each typed walker hands them all here, out of line, so
    /// that what a program without them never meets grows no walker.
    #[inline(never)]
    pub(super) fn stopping<T>(
        &mut self,
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        match node.kind {
            IrKind::Catch { .. } if mixed(node) => mistyped(node, MIXED),
            IrKind::Catch { .. } => match extract(self.catch(node)?) {
                Some(value) => Ok(value),
                None => mistyped(
                    node,
                    "a catch yielded a value of another kind than its node",
                ),
            },
            IrKind::For { .. } | IrKind::Loop { .. } => {
                self.looped(node)?;
                mistyped(node, "a loop ended where it cannot")
            }
            _ => self.leave(node),
        }
    }

    /// Evaluates the loop `node` where a record is expected: one that
    /// discards what its iterations contribute, which is `{}`, as a
    /// statement or a loop that cannot end is; one that collects them is a
    /// list, and mistyped here. Out of line, as [`Evaluator::stopping`] is.
    #[inline(never)]
    pub(super) fn record_loop(&mut self, node: &IrNode) -> Flow<Value> {
        match node.kind {
            IrKind::For { collect: true, .. } | IrKind::Loop { collect: true, .. } => {
                mistyped(node, "list node where a record was expected")
            }
            _ => self.looped(node),
        }
    }
}
