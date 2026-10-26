//! Evaluating lists: list literals, ranges, spreads, indexes and comparisons of
//! lists.
//!
//! # Lists
//!
//! A list node, of [`crate::value::ValueKind::List`], has no typed walker
//! of its own: `Evaluator::list` evaluates it through the general walker
//! and takes the [`List`] out of the value, which is one match on its tag,
//! as a load extracts a slot's value. A list is only ever read whole, by an
//! index, a spread, `len`, a comparison or an update (see
//! [Updates](crate::eval::updates#updates)), each of which is a helper and
//! not on the recursion path of anything but its own operands, so a walker
//! of its own would save nothing measurable, and would add another large
//! match to keep in step with every kind of node; the list nodes that can
//! be of any kind, indexes, are handled by every typed walker, as field
//! accesses are.
//!
//! A `MakeList` without spreads evaluates its items in order straight into
//! the list's one allocation, sized to the number of items; the empty list
//! allocates nothing. One with spreads collects its elements in a vector,
//! each spread's list's elements cloned in its place (reference-count
//! increments), in time linear in the result's length, and then copies
//! them into the list's allocation. An `Index` evaluates its list, then its
//! position, and fails with "index out of range" at the index, labelled
//! with the position and the list's length, for a position outside the
//! list, a negative one included; nothing wraps around from the end.
//!
//! A `Range`, inclusive or not, used as a value evaluates its bounds,
//! left to right, and builds the list of its integers in one allocation,
//! sized exactly and filled in place; the empty range, one whose left bound
//! is not below its right one (or, inclusive, is above it), allocates
//! nothing. Its length is computed in 128 bits, so that no pair of bounds
//! overflows it, `i64::MIN..=i64::MAX` included, and a range of more
//! integers than a list can hold ([`MAX_LIST_LEN`]) fails with "range too
//! large" at the range, labelled with its length, rather than ask for an
//! allocation that cannot exist. There is no other limit on a list's
//! length: how large a list a program builds is its own business, and one
//! too large for the machine fails as any allocation does.
//!
//! A `For` is evaluated by a helper, `Evaluator::for_loop`, which the
//! walkers only call: it evaluates the iterable once, and for each element
//! in order writes it to the variable's slot of the current frame, then
//! evaluates the guard, if there is one, and if that is true or absent the
//! body, each through its walker, so an error stops the loop and is
//! reported as any error in the body is. Every iteration evaluates the body
//! from the `for`, at the same depth, so iterations do not nest, and the
//! nesting budget, charged statically by the body's height, is charged for
//! one. A closure or a `lazy` the body makes reads the slot when it is
//! made, so it captures that iteration's element. A collecting `for`
//! gathers the body's values in a vector, sized up front to the number of
//! elements when there is no guard to skip some (at most `FOR_RESERVE`
//! up front), and copies them into the list's allocation, in time
//! linear in its length; one that discards them, a statement whose value
//! nothing would hold, drops each as it comes and is `{}`. A `for` whose
//! iterable is directly a range (the parentheses around it are gone by
//! then) does not build the range's list: it evaluates the bounds and
//! counts from one to the other, the last included for `..=`, stopping at
//! the last rather than stepping past it, so that counting up to
//! `i64::MAX` never overflows; so it has no limit on the range's length.
//!
//! A `Loop`, a condition loop's or a `do` loop's, is evaluated by
//! `Evaluator::condition_loop`: while its condition is true, tested first
//! unless it is a `do` loop's, its body, and then the items after its
//! condition, each for its effects; it collects or discards as a `For`
//! does, but reserves nothing, its length not being known up front. Both
//! helpers are reached through `Evaluator::looped`. A `break` or `continue`
//! in a body ends its iteration, or its loop, through the stop channel (see
//! [Stops](crate::eval::loops#stops)).

use crate::ir::{IrId, IrKind, IrNode, IrRange, Operands, ShapedOperands};
use crate::value::{List, Value};

use super::{
    Elements, Evaluator, Flow, MAX_LIST_LEN, MIXED, mistyped, out_of_range, range_len,
    range_too_large, record_eq_shaped,
};
use super::kinds::mixed;

impl Evaluator<'_> {
    /// Evaluates a list node, through the general walker, to its list (see
    /// the module documentation).
    pub(super) fn list(&mut self, id: IrId) -> Flow<List> {
        match self.value(id)? {
            Value::List(list) => Ok(list),
            _ => mistyped(
                self.ir.get(id),
                "a node of another kind where a list was expected",
            ),
        }
    }

    /// Builds a list of the values of `items`, in order, spreading the
    /// list of each `Spread` among them if `spreads` says there is one: one
    /// allocation, sized exactly and filled in place, if there is none,
    /// and none at all for the empty list.
    pub(super) fn make_list(&mut self, items: IrRange, spreads: bool) -> Flow<Value> {
        let ir = self.ir;
        let items = ir.items(items);
        if spreads {
            return self.make_spread_list(items);
        }
        // The slice is built at its known length as the items are
        // evaluated, in order; once one fails, the rest are not evaluated,
        // and the slice, filled with `{}`, is dropped.
        let mut failed = None;
        let list = List::from_exact((0..items.len()).map(|at| {
            if failed.is_some() {
                return Value::Empty;
            }
            let value = items
                .get(at)
                .map_or(Ok(Value::Empty), |&item| self.value(item));
            value.unwrap_or_else(|error| {
                failed = Some(error);
                Value::Empty
            })
        }));
        match failed {
            Some(error) => Err(error),
            None => Ok(Value::List(list)),
        }
    }

    /// Evaluates the range `node` of `operands`, its bounds, left to right,
    /// to the list of its integers, in order, up to the right bound,
    /// included if `inclusive`: one allocation, sized exactly and filled in
    /// place, and none for an empty range. A range of more than
    /// [`MAX_LIST_LEN`] integers, whose list cannot exist, is "range too
    /// large".
    pub(super) fn range(
        &mut self,
        operands: Operands,
        inclusive: bool,
        node: &IrNode,
    ) -> Flow<Value> {
        let (start, end) = self.ints(operands)?;
        let len = range_len(start, end, inclusive);
        let Some(len) = usize::try_from(len).ok().filter(|&len| len <= MAX_LIST_LEN) else {
            return Err(range_too_large(node, len).into());
        };
        // No integer of the range passes its end, so none overflows.
        let list = List::from_exact((0..len).map(|offset| {
            Value::Int(i64::try_from(offset).map_or(end, |offset| start.saturating_add(offset)))
        }));
        Ok(Value::List(list))
    }

    /// What a `for` whose iterable is `iterable` iterates over: if the
    /// iterable is directly a range, its integers, from its bounds,
    /// evaluated left to right, without building its list, so that
    /// [`MAX_LIST_LEN`] does not apply; otherwise the elements of the list
    /// it evaluates to.
    pub(super) fn elements(&mut self, iterable: IrId) -> Flow<Elements> {
        match self.ir.get(iterable).kind {
            IrKind::Range(inclusive, operands) => {
                let (start, end) = self.ints(operands)?;
                Ok(Elements::range(start, end, inclusive))
            }
            _ => Ok(Elements::List(self.list(iterable)?, 0)),
        }
    }

    /// Builds a list of the values of `items`, in order, of which some are
    /// `Spread`s, whose lists' elements are put in their places: each run
    /// of elements a leaf, and each spread list joined in as it is, sharing
    /// its elements (see [`List::concat`]), so that spreading a long list
    /// costs a walk down a tree, not a copy of its elements.
    pub(super) fn make_spread_list(&mut self, items: &[IrId]) -> Flow<Value> {
        let mut list = List::EMPTY;
        let mut run = Vec::new();
        for &item in items {
            if let IrKind::Spread(spread) = self.ir.get(item).kind {
                let spread = self.list(spread)?;
                if !run.is_empty() {
                    list = list.concat(&List::from_vec(std::mem::take(&mut run)));
                }
                list = list.concat(&spread);
            } else {
                run.push(self.value(item)?);
            }
        }
        if !run.is_empty() {
            list = list.concat(&List::from_vec(run));
        }
        Ok(Value::List(list))
    }

    /// Evaluates `list` and then `index`, and returns the element at the
    /// position, counting from 0, for the `Index` `node`; a position
    /// outside the list, a negative one included, is "index out of range".
    pub(super) fn index(&mut self, (list, index): (IrId, IrId), node: &IrNode) -> Flow<Value> {
        let list = self.list(list)?;
        let at = self.int(index)?;
        match usize::try_from(at)
            .ok()
            .and_then(|position| list.get(position))
        {
            Some(value) => Ok(value.clone()),
            None => Err(out_of_range(node.span, at, list.len()).into()),
        }
    }

    /// Evaluates the `Index` `node`, reading the element of `list` at
    /// `index`, as a value that `extract` accepts: an element of the kind
    /// the node states.
    pub(super) fn index_as<T>(
        &mut self,
        operands: (IrId, IrId),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.index(operands, node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "a list holds an element of another kind than its node",
            ),
        }
    }

    /// Evaluates a `ListLen` of `list`: the number of its elements.
    pub(super) fn list_len(&mut self, list: IrId) -> Flow<i64> {
        // A list has far fewer elements than `i64::MAX`.
        Ok(i64::try_from(self.list(list)?.len()).unwrap_or(i64::MAX))
    }

    /// Evaluates the typed list comparison `node`, a `ListEq` if `equal` and
    /// a `ListNe` otherwise: the operands compare element by element over
    /// the list's shape (see [`record_eq_shaped`]).
    pub(super) fn list_eq(
        &mut self,
        operands: ShapedOperands,
        equal: bool,
        node: &IrNode,
    ) -> Flow<bool> {
        let lhs = Value::List(self.list(operands.lhs)?);
        let rhs = Value::List(self.list(operands.rhs)?);
        match record_eq_shaped(self.ir, operands.shape, &lhs, &rhs) {
            Some(same) => Ok(same == equal),
            None => mistyped(node, "a list does not have its shape"),
        }
    }

    /// Evaluates a total list comparison, a `ListSame` if `equal` and a
    /// `ListNotSame` otherwise: lists compare whole, element by element,
    /// deeply.
    pub(super) fn list_same(&mut self, operands: Operands, equal: bool) -> Flow<bool> {
        let lhs = self.list(operands.lhs)?;
        let rhs = self.list(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }
}
