//! Lists: persistent ropes, so that joining two shares their elements.
//!
//! A non-empty list is a tree of [`Node`]s: a leaf holds elements in order,
//! and a join holds two lists, the elements of the left then of the right.
//! Joins are balanced by height, as an AVL tree is ([`List::concat`]): a
//! join's children differ in height by at most one, so a list's depth is
//! logarithmic in the number of its nodes, and joining two lists costs a
//! walk down one of them, never a copy of their elements, but where two
//! short ones make one leaf. Reading a list as a slice ([`List::values`])
//! flattens a join once and keeps the slice in it; its length and an
//! element at a position ([`List::get`]) walk the tree instead. Replacing an
//! element ([`List::with`]) copies the path to it and shares the rest.
//!
//! Dropping a list frees what its elements hold through the value module's
//! work list, never recursion, however deeply lists are nested in lists; a
//! rope's own depth is logarithmic, and its nodes go on the same list.

use std::cell::OnceCell;
use std::rc::Rc;

use super::{Nested, Pair, Value, deep_equal, free_nested, take_nested};

/// The most elements two lists joined into one leaf hold: a join of two
/// lists this short or shorter is a leaf of their elements, so that
/// building a list a few elements at a time makes no tree of tiny leaves.
const SHORT: usize = 32;

/// What [`List::child`] gives of a leaf.
const NO_CHILD: &List = &List::EMPTY;

/// A list: its elements, in order, or nothing for the empty list, which
/// therefore needs no allocation. Cloning a list shares its elements, and
/// so does joining two (see the module documentation).
#[derive(Clone, Debug, Default)]
pub struct List(Option<Rc<Node>>);

/// The elements of a non-empty list.
#[derive(Debug)]
enum Node {
    /// Elements, at least one, in order.
    Leaf(Box<[Value]>),
    /// Two lists, neither empty, the left's elements then the right's.
    Join(Join),
}

/// Two lists joined, and what is kept of them.
#[derive(Debug)]
struct Join {
    /// The first elements.
    left: List,
    /// The elements after them.
    right: List,
    /// The number of elements of both.
    len: usize,
    /// One more than the greater of its children's heights; a leaf's is 0.
    height: u32,
    /// The elements in one slice, once something has read them as one.
    flat: OnceCell<Rc<[Value]>>,
}

impl List {
    /// The empty list.
    pub const EMPTY: List = List(None);

    /// The list of `values`, in order: one leaf of them, or none if there
    /// are none.
    #[must_use]
    pub fn from_vec(values: Vec<Value>) -> Self {
        if values.is_empty() {
            List::EMPTY
        } else {
            List(Some(Rc::new(Node::Leaf(values.into_boxed_slice()))))
        }
    }

    /// The list of the values `values` yields, in order, which must be an
    /// iterator of exactly known length, such as a map over a range, so
    /// that the leaf is allocated once, at its size, and filled in place;
    /// no allocation if it yields none.
    #[must_use]
    pub fn from_exact(values: impl ExactSizeIterator<Item = Value>) -> Self {
        if values.len() == 0 {
            List::EMPTY
        } else {
            List(Some(Rc::new(Node::Leaf(values.collect()))))
        }
    }

    /// The elements, in order, as one slice: a leaf's own, or a join's
    /// flattened the first time it is read so and kept for the next.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        match self.0.as_deref() {
            None => &[],
            Some(Node::Leaf(values)) => values,
            Some(Node::Join(join)) => join.flat.get_or_init(|| {
                let mut values = Vec::with_capacity(join.len);
                self.extend_into(&mut values);
                Rc::from(values)
            }),
        }
    }

    /// Appends the elements, in order, to `values`, walking the tree
    /// through a work list: a join's kept slice if it has one.
    fn extend_into(&self, values: &mut Vec<Value>) {
        let mut pending: Vec<&List> = vec![self];
        while let Some(list) = pending.pop() {
            match list.0.as_deref() {
                None => {}
                Some(Node::Leaf(leaf)) => values.extend_from_slice(leaf),
                Some(Node::Join(join)) => {
                    if let Some(flat) = join.flat.get() {
                        values.extend_from_slice(flat);
                    } else {
                        pending.push(&join.right);
                        pending.push(&join.left);
                    }
                }
            }
        }
    }

    /// The number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        match self.0.as_deref() {
            None => 0,
            Some(Node::Leaf(values)) => values.len(),
            Some(Node::Join(join)) => join.len,
        }
    }

    /// Whether the list has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// The element at `index`, if there is one, found by walking the tree:
    /// no slice is made.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Value> {
        let (mut list, mut index) = (self, index);
        loop {
            match list.0.as_deref()? {
                Node::Leaf(values) => return values.get(index),
                Node::Join(join) => {
                    if let Some(flat) = join.flat.get() {
                        return flat.get(index);
                    }
                    let left = join.left.len();
                    if index < left {
                        list = &join.left;
                    } else {
                        list = &join.right;
                        index -= left;
                    }
                }
            }
        }
    }

    /// Whether this list and `other` are the same list, sharing their
    /// elements, as a clone does: two empty lists always are.
    #[must_use]
    pub fn shares(&self, other: &List) -> bool {
        match (&self.0, &other.0) {
            (Some(mine), Some(theirs)) => Rc::ptr_eq(mine, theirs),
            (None, None) => true,
            _ => false,
        }
    }

    /// The list of this list's elements and then `other`'s, sharing both:
    /// two short lists make one leaf (see [`SHORT`]); otherwise their join,
    /// balanced as an AVL tree is, at a cost of a walk down the taller one.
    #[must_use]
    pub fn concat(&self, other: &List) -> List {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        if self.len() + other.len() <= SHORT {
            let mut values = Vec::with_capacity(self.len() + other.len());
            self.extend_into(&mut values);
            other.extend_into(&mut values);
            return List::from_vec(values);
        }
        let (mine, theirs) = (self.height(), other.height());
        if mine > theirs + 1 {
            self.join_right(other)
        } else if theirs > mine + 1 {
            other.join_left(self)
        } else {
            List::join(self.clone(), other.clone())
        }
    }

    /// This list, the taller by more than one, joined with `right`: down
    /// its right spine to a list `right` may be joined beside, then back up,
    /// rotating where a join would lean by two.
    fn join_right(&self, right: &List) -> List {
        let Some(Node::Join(join)) = self.0.as_deref() else {
            return List::join(self.clone(), right.clone());
        };
        let (left, middle) = (&join.left, &join.right);
        let joined = if middle.height() <= right.height() + 1 {
            List::join(middle.clone(), right.clone())
        } else {
            middle.join_right(right)
        };
        if joined.height() <= left.height() + 1 {
            List::join(left.clone(), joined)
        } else {
            // `joined` leans right by two over `left`; if it leans left
            // inside, turn it first.
            let joined = if joined.child(true).height() > joined.child(false).height() {
                joined.rotate_right()
            } else {
                joined
            };
            List::join(left.clone(), joined).rotate_left()
        }
    }

    /// This list, the taller by more than one, joined after `left`, as
    /// [`List::join_right`] mirrored.
    fn join_left(&self, left: &List) -> List {
        let Some(Node::Join(join)) = self.0.as_deref() else {
            return List::join(left.clone(), self.clone());
        };
        let (middle, right) = (&join.left, &join.right);
        let joined = if middle.height() <= left.height() + 1 {
            List::join(left.clone(), middle.clone())
        } else {
            middle.join_left(left)
        };
        if joined.height() <= right.height() + 1 {
            List::join(joined, right.clone())
        } else {
            let joined = if joined.child(false).height() > joined.child(true).height() {
                joined.rotate_left()
            } else {
                joined
            };
            List::join(joined, right.clone()).rotate_right()
        }
    }

    /// The join of `left` and `right`, both non-empty, as it is.
    fn join(left: List, right: List) -> List {
        let height = left.height().max(right.height()) + 1;
        let len = left.len() + right.len();
        List(Some(Rc::new(Node::Join(Join {
            left,
            right,
            len,
            height,
            flat: OnceCell::new(),
        }))))
    }

    /// The height of the list's tree: a leaf's, or the empty list's, is 0.
    fn height(&self) -> u32 {
        match self.0.as_deref() {
            Some(Node::Join(join)) => join.height,
            _ => 0,
        }
    }

    /// A join's left child if `left`, else its right; the empty list for a
    /// leaf.
    fn child(&self, left: bool) -> &List {
        match self.0.as_deref() {
            Some(Node::Join(join)) if left => &join.left,
            Some(Node::Join(join)) => &join.right,
            _ => NO_CHILD,
        }
    }

    /// `a` joined with `b` joined with `c`, rejoined as (`a` with `b`) with
    /// `c`; the list itself if it is no join of a join on its right.
    fn rotate_left(&self) -> List {
        let (a, right) = (self.child(true), self.child(false));
        if right.height() == 0 {
            return self.clone();
        }
        let (b, c) = (right.child(true), right.child(false));
        List::join(List::join(a.clone(), b.clone()), c.clone())
    }

    /// (`a` with `b`) with `c`, rejoined as `a` with (`b` with `c`); the
    /// list itself if it is no join of a join on its left.
    fn rotate_right(&self) -> List {
        let (left, c) = (self.child(true), self.child(false));
        if left.height() == 0 {
            return self.clone();
        }
        let (a, b) = (left.child(true), left.child(false));
        List::join(a.clone(), List::join(b.clone(), c.clone()))
    }

    /// This list with the element at `index` replaced by `value`, sharing
    /// the rest: the path to it copied, and a leaf longer than [`SHORT`]
    /// split into leaves of that many first, so that the next replacement
    /// copies one of those; `None` if there is no element at `index`.
    #[must_use]
    pub fn with(&self, index: usize, value: Value) -> Option<List> {
        if index >= self.len() {
            return None;
        }
        Some(self.replaced(index, value))
    }

    /// [`List::with`], `index` known to be in range.
    fn replaced(&self, index: usize, value: Value) -> List {
        match self.0.as_deref() {
            None => List::EMPTY,
            Some(Node::Leaf(values)) if values.len() <= SHORT => {
                let mut values = values.to_vec();
                values[index] = value;
                List::from_vec(values)
            }
            Some(Node::Leaf(values)) => List::chunked(values).replaced(index, value),
            // A child whose leaf was split is taller than it was, so the
            // two are joined again, balanced.
            Some(Node::Join(join)) => {
                let left = join.left.len();
                if index < left {
                    join.left.replaced(index, value).concat(&join.right)
                } else {
                    join.left.concat(&join.right.replaced(index - left, value))
                }
            }
        }
    }

    /// The list of `values`, at least one, as a balanced tree of leaves of
    /// at most [`SHORT`] elements.
    fn chunked(values: &[Value]) -> List {
        let leaves: Vec<List> = values
            .chunks(SHORT)
            .map(|chunk| List::from_vec(chunk.to_vec()))
            .collect();
        List::balanced(&leaves)
    }

    /// The join of `lists`, each a leaf, in order, halved recursively, so
    /// that the two sides of every join are of heights at most one apart.
    ///
    /// Recurses once per level of the tree it makes, logarithmic in the
    /// number of leaves.
    fn balanced(lists: &[List]) -> List {
        match lists {
            [] => List::EMPTY,
            [only] => only.clone(),
            _ => {
                let (left, right) = lists.split_at(lists.len() / 2);
                List::join(List::balanced(left), List::balanced(right))
            }
        }
    }

    /// Moves what this list holds that nothing else does onto `pending`,
    /// for the value module's iterative drop: a leaf's records, lists,
    /// closures and futures, and a join's two lists and kept slice.
    pub(super) fn take_held(&mut self, pending: &mut Vec<Nested>) {
        let Some(node) = self.0.as_mut().and_then(Rc::get_mut) else {
            return;
        };
        match node {
            Node::Leaf(values) => take_nested(values, pending),
            Node::Join(join) => {
                if let Some(flat) = join.flat.get_mut().and_then(Rc::get_mut) {
                    take_nested(flat, pending);
                }
                for list in [&mut join.left, &mut join.right] {
                    let list = std::mem::take(list);
                    if !list.is_empty() {
                        pending.push(Nested::List(list));
                    }
                }
            }
        }
    }
}

/// Dropping a list frees its nested records, lists, closures and futures
/// iteratively, as dropping a record does, so a list nested in a list as
/// deeply as a program runs is freed without recursion; so are the lists a
/// join holds. A list whose elements are shared elsewhere is merely
/// released.
impl Drop for List {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        self.take_held(&mut pending);
        if !pending.is_empty() {
            free_nested(pending);
        }
    }
}

/// Lists compare element by element, in order, deeply, as [`Value`]'s
/// equality compares each: two lists are equal when they have the same
/// length and equal elements at every index. Nested records and lists are
/// compared through a work list, as a record's fields are (see
/// [`super::Record`]'s equality).
impl PartialEq for List {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && deep_equal(Pair::Lists(self.values(), other.values()))
    }
}

#[cfg(test)]
mod tests;
