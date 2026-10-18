//! Runtime values.
//!
//! Scalars are unboxed; strings, records, lists, closures and futures are
//! reference-counted, so copying a value, into a variable, a record, a list
//! or a closure's captures, is at most a reference-count increment. Records,
//! lists, closures and futures can nest as deeply as a program runs (a
//! record holding a record, a list holding a list, a closure capturing a
//! closure, a future capturing a future), so dropping one frees what it
//! uniquely owns iteratively, through a work list, rather than recursively
//! (see [`Record`], [`List`], [`Closure`] and [`Future`]), and comparing two
//! compares nested records and lists through a work list too.
//!
//! A [`List`] is its elements in one reference-counted slice, allocated
//! once; the empty list is no allocation at all, so every empty list is the
//! same value, made for nothing.
//!
//! A [`Future`] is a memoised thunk: the function of a `lazy` with the
//! values it captured, run by its first await, after which it holds the
//! value it yielded and has dropped its captures. It is shared by
//! reference, so a future stored in two places is one future, run at most
//! once. Its state changes behind a [`RefCell`], which is only ever
//! borrowed briefly by the evaluator, never across running the body.
//!
//! A built-in function, a member of the `builtins` module, is a value of
//! its own kind, [`Value::Builtin`]: the [`Builtin`] it is, unboxed, with
//! no code or captures, since it is implemented in Rust. It is a function
//! ([`ValueKind::Fn`]): a call of it through a value, a parameter say, is
//! applied by the evaluator (see [`crate::eval`]). It shows as its type, as
//! the type table shows it (`fn(argv: List<String>) -> {status: Int,
//! stderr: String, stdout: String}` for `exec`, its record's fields by
//! name, and `fn(message: String): Never` for `panic`), or as `builtin len`
//! for one whose typing is special, and two compare equal when they are the
//! same built-in, which no program can ask, since the checker rejects
//! comparing functions. The record `exec` returns is an ordinary
//! [`Record`], whose [`Layout`] (`status`, `stdout`, `stderr`, in that
//! order) the driver of a run builds once and every such record shares.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::ast::FnId;
use crate::check::Builtin;
use crate::ir::Ir;
use crate::names::Name;
use crate::strings::{FieldName, Quoted};

/// The kind of value an expression produces: all that the IR and the
/// evaluator need to know of its type. Every record, the empty record
/// included, is [`ValueKind::Record`]. See
/// [`crate::types::TypeTable::value_kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueKind {
    /// An `Int`, [`Value::Int`].
    Int,
    /// A `Float`, [`Value::Float`].
    Float,
    /// A `Bool`, [`Value::Bool`].
    Bool,
    /// A `String`, [`Value::Str`].
    String,
    /// A record: [`Value::Empty`] or [`Value::Record`].
    Record,
    /// A list, [`Value::List`].
    List,
    /// A function, [`Value::Fn`].
    Fn,
    /// A value whose static type is a `Future` or `T | Future<T>`, so which
    /// may be a [`Value::Future`] or a present value of `T`'s kind. Only
    /// the evaluator's general walker evaluates such nodes; see
    /// [`crate::ir::Ir::kind_of`].
    Deferred,
    /// A present value whose static type is a union of types of different
    /// kinds, as `Int | String` is, so which may be of any of their kinds,
    /// but is never a [`Value::Future`]. Only the evaluator's general
    /// walker evaluates such nodes, as it does [`ValueKind::Deferred`] ones.
    Mixed,
    /// No value: the kind of a node of type `Never`, which never produces
    /// one, a `break` or a `continue`, or a block or an `if` that always
    /// ends in one. Every walker evaluates such a node, which always
    /// unwinds (see [`crate::eval`]); no [`Value`] is of it.
    Never,
}

/// The field names of a record, in definition order. Built once per record
/// block when a program is lowered and shared by every record that block
/// builds, so building a record never copies names; a functional update
/// that replaces a field shares its record's layout too, and one that adds
/// a field builds a new layout with the name at the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The name of each field, in definition order. Never empty and never
    /// repeating a name.
    pub names: Box<[Name]>,
    /// The text of each name, in the same order, so that a record can be
    /// shown without the names table.
    pub texts: Box<[Box<str>]>,
}

/// A record with at least one field. The empty record is [`Value::Empty`].
#[derive(Clone, Debug)]
pub struct Record {
    /// The record's field names, shared with every record built by the
    /// same record block.
    pub layout: Rc<Layout>,
    /// The value of each field, in the layout's order.
    pub values: Box<[Value]>,
}

impl Record {
    /// The value of field `name`, or `None` if the record has no such
    /// field.
    ///
    /// A linear scan of the layout, comparing interned names: records are
    /// small. A bytecode VM would cache each access site's field index
    /// against the layout it last saw instead.
    #[must_use]
    pub fn get(&self, name: Name) -> Option<&Value> {
        let index = self.layout.names.iter().position(|&field| field == name)?;
        self.values.get(index)
    }
}

/// Dropping a record frees its nested records and closures iteratively, not
/// recursively.
///
/// The default drop would recurse once per level of nesting, and a record
/// nested through variables (`let a = { x = a }` repeated) can be nested as
/// deeply as the program is long, far beyond any stack; so can a closure
/// capturing a closure. Instead, the nested records and closures this
/// record uniquely owns are moved onto a work list and freed level by level
/// (see `drop_nested`). A record with none allocates nothing here: the list
/// only grows when there is something to put on it. A nested value that is
/// still shared elsewhere is merely released.
impl Drop for Record {
    fn drop(&mut self) {
        drop_nested(&mut self.values);
    }
}

/// A list: its elements, in order, in one reference-counted slice, or
/// nothing for the empty list, which therefore needs no allocation. Cloning
/// a list shares its elements. See the module documentation.
#[derive(Clone, Debug, Default)]
pub struct List(Option<Rc<[Value]>>);

impl List {
    /// The empty list.
    pub const EMPTY: List = List(None);

    /// The list of `values`, in order: one allocation, which copies them
    /// out of the vector, or none if there are none.
    #[must_use]
    pub fn from_vec(values: Vec<Value>) -> Self {
        if values.is_empty() {
            List::EMPTY
        } else {
            List(Some(Rc::from(values)))
        }
    }

    /// The list of the values `values` yields, in order, which must be an
    /// iterator of exactly known length, such as a map over a range, so
    /// that the slice is allocated once, at its size, and filled in place;
    /// no allocation if it yields none.
    #[must_use]
    pub fn from_exact(values: impl ExactSizeIterator<Item = Value>) -> Self {
        if values.len() == 0 {
            List::EMPTY
        } else {
            List(Some(values.collect()))
        }
    }

    /// The elements, in order.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        self.0.as_deref().unwrap_or_default()
    }

    /// The number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values().len()
    }

    /// Whether the list has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
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

    /// The elements, if nothing else holds them, for dropping them.
    fn unique_mut(&mut self) -> Option<&mut [Value]> {
        self.0.as_mut().and_then(Rc::get_mut)
    }
}

/// Dropping a list frees its nested records, lists, closures and futures
/// iteratively, as dropping a record does, so a list nested in a list as
/// deeply as a program runs is freed without recursion. A list whose
/// elements are shared elsewhere is merely released.
impl Drop for List {
    fn drop(&mut self) {
        if let Some(values) = self.unique_mut() {
            drop_nested(values);
        }
    }
}

/// Lists compare element by element, in order, deeply, as [`Value`]'s
/// equality compares each: two lists are equal when they have the same
/// length and equal elements at every index. Nested records and lists are
/// compared through a work list, as a record's fields are (see [`Record`]'s
/// equality).
impl PartialEq for List {
    fn eq(&self, other: &Self) -> bool {
        deep_equal(Pair::Lists(self.values(), other.values()))
    }
}

/// A uniquely owned value that holds other values: what the iterative drop
/// of records, lists, closures and futures frees level by level.
enum Nested {
    /// A record, holding its fields.
    Record(Rc<Record>),
    /// A list, holding its elements.
    List(List),
    /// A closure, holding its captured values.
    Closure(Rc<Closure>),
    /// A future, holding its captured values or the value it yielded.
    Future(Rc<Future>),
}

/// Frees the records, lists, closures and futures among `values` that
/// nothing else holds, and all those they hold in turn, through a work list
/// rather than recursion: each is emptied of the records, lists, closures
/// and futures it holds, which go on the list, before it is dropped, so no
/// drop recurses.
fn drop_nested(values: &mut [Value]) {
    let mut pending = Vec::new();
    take_nested(values, &mut pending);
    while let Some(nested) = pending.pop() {
        match nested {
            Nested::Record(record) => {
                if let Ok(mut record) = Rc::try_unwrap(record) {
                    // Emptied, it drops without adding to the list or
                    // recursing.
                    take_nested(&mut record.values, &mut pending);
                }
            }
            Nested::List(mut list) => {
                // Emptied, its own drop finds nothing left to free.
                if let Some(values) = list.unique_mut() {
                    take_nested(values, &mut pending);
                }
            }
            Nested::Closure(closure) => {
                if let Ok(mut closure) = Rc::try_unwrap(closure) {
                    take_nested(&mut closure.captures, &mut pending);
                }
            }
            Nested::Future(future) => {
                if let Ok(mut future) = Rc::try_unwrap(future) {
                    take_nested(future.held_mut(), &mut pending);
                }
            }
        }
    }
}

/// Moves the records, lists, closures and futures among `values` onto
/// `pending`, leaving `{}` in their places. An empty list holds nothing, and
/// stays.
fn take_nested(values: &mut [Value], pending: &mut Vec<Nested>) {
    for value in values {
        let holds = match value {
            Value::Record(_) | Value::Fn(_) | Value::Future(_) => true,
            Value::List(list) => !list.is_empty(),
            _ => false,
        };
        if !holds {
            continue;
        }
        match std::mem::replace(value, Value::Empty) {
            Value::Record(record) => pending.push(Nested::Record(record)),
            Value::List(list) => pending.push(Nested::List(list)),
            Value::Fn(closure) => pending.push(Nested::Closure(closure)),
            Value::Future(future) => pending.push(Nested::Future(future)),
            _ => {}
        }
    }
}

/// A function value: a function of a program's IR with the values it
/// captured when its function expression was evaluated.
pub struct Closure {
    /// The IR the function belongs to. Holding it keeps the code alive: a
    /// session drops each input's IR once the input has run, but a closure
    /// made by that input may be called by a later one.
    pub code: Rc<Ir>,
    /// Which of `code`'s functions this is.
    pub function: FnId,
    /// The captured values, in the function's capture order.
    pub captures: Box<[Value]>,
}

impl Closure {
    /// The closure's type as text, `fn(x: Int): Int`: what it displays as.
    /// Empty if `function` is not one of `code`'s, which the evaluator
    /// never builds.
    #[must_use]
    pub fn display(&self) -> &str {
        self.code
            .find_function(self.function)
            .map_or("", |code| &code.display)
    }
}

/// Shows the function and what it captured, not the IR it belongs to.
impl fmt::Debug for Closure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Closure")
            .field("function", &self.function)
            .field("type", &self.display())
            .field("captures", &self.captures)
            .finish_non_exhaustive()
    }
}

/// Closures compare by identity: two are equal only if they are the same
/// closure. No program can compare functions, since the checker rejects
/// `==`, `!=`, `===` and `!==` on any type containing one; this exists for
/// [`Value`]'s equality, which a closure inside a record would otherwise
/// have none for.
impl PartialEq for Closure {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// Dropping a closure frees its captured records and closures iteratively,
/// as dropping a record does, so a chain of closures each capturing the
/// last is freed without recursion.
impl Drop for Closure {
    fn drop(&mut self) {
        drop_nested(&mut self.captures);
    }
}

/// Where a [`Future`] is in its life.
#[derive(Debug)]
pub(crate) enum FutureState {
    /// Not yet run: the values its body captured, until its first await
    /// succeeds.
    Pending(Box<[Value]>),
    /// Its body is running, for an await in progress.
    Running,
    /// Run: the value it yielded, which every later await yields.
    Done(Value),
}

/// What an await finds when it starts to await a [`Future`], from
/// [`Future::begin`].
pub(crate) enum Begin {
    /// The future has yielded this value already.
    Done(Value),
    /// The future's body is running: the await depends on itself.
    Running,
    /// The future had not run: its captured values, taken out, and it is
    /// now running. The await must end it with [`Future::finish`].
    Pending(Box<[Value]>),
}

/// A future: the function of a `lazy` in a program's IR, with the values it
/// captured when the `lazy` was evaluated, run once, by its first await,
/// after which it holds the value it yielded instead. See the module
/// documentation.
pub struct Future {
    /// The IR the function belongs to, kept alive by the future as by a
    /// closure.
    pub code: Rc<Ir>,
    /// Which of `code`'s functions this is.
    pub function: FnId,
    /// Where the future is in its life. Borrowed only briefly, and never
    /// across running the body.
    state: RefCell<FutureState>,
    /// The caller, the helper flag and the origin of the frame that made
    /// the future, which its body runs with, whoever awaits it (see
    /// [`crate::eval`], "Callers and helpers").
    pub(crate) made: Caller,
}

/// What a running frame has of its caller (see [`crate::eval`], "Callers
/// and helpers"), and what a `lazy` captures of the frame that makes it:
/// the caller, the directory of the file whose code made the call the frame
/// runs for, by its index in the run's table of directories; whether
/// `fileroot.helper()` has run in the frame, so that the calls it makes pass
/// its caller on; and the caller's *origin*, a number its run gives the
/// frame whose caller was set from its calling code's file once it is
/// needed, which every frame it passed the caller on to, through helpers
/// and `lazy`s, shares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Caller {
    /// The caller's directory, by its index in the run's table.
    pub(crate) dir: u32,
    /// Whether `fileroot.helper()` has run in the frame.
    pub(crate) helper: bool,
    /// The caller's origin; 0 until one is needed, which no frame shares.
    pub(crate) origin: u64,
}

impl Future {
    /// A future of `function` of `code`, not yet run, holding the values
    /// its body captured, made by a frame of caller 0, the root of its
    /// run's table of directories, and an origin no frame has.
    #[must_use]
    pub fn new(code: Rc<Ir>, function: FnId, captures: Box<[Value]>) -> Self {
        Self::made_by(code, function, captures, Caller::default())
    }

    /// [`Future::new`], made by a frame whose caller, helper flag and
    /// origin are `made`, which its body runs with.
    pub(crate) fn made_by(
        code: Rc<Ir>,
        function: FnId,
        captures: Box<[Value]>,
        made: Caller,
    ) -> Self {
        Self {
            code,
            function,
            state: RefCell::new(FutureState::Pending(captures)),
            made,
        }
    }

    /// The future's type as text, `Future<Int>`: what it displays as,
    /// whether it has run or not. Empty if `function` is not one of
    /// `code`'s, which the evaluator never builds.
    #[must_use]
    pub fn display(&self) -> &str {
        self.code
            .find_function(self.function)
            .map_or("", |code| &code.display)
    }

    /// Whether the future has run and holds its value.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.state.try_borrow().as_deref(), Ok(FutureState::Done(_)))
    }

    /// Starts an await of the future: a copy of its value if it has run; a
    /// note that it is running; or, if it has not run, its captured values,
    /// taken out, leaving it running. `None` only if the state is borrowed,
    /// which never happens, since no borrow outlives a call of this or of
    /// [`Future::finish`].
    pub(crate) fn begin(&self) -> Option<Begin> {
        let mut state = self.state.try_borrow_mut().ok()?;
        if let FutureState::Done(value) = &*state {
            return Some(Begin::Done(value.clone()));
        }
        Some(match std::mem::replace(&mut *state, FutureState::Running) {
            FutureState::Pending(captures) => Begin::Pending(captures),
            FutureState::Running => Begin::Running,
            // Handled above; put back unchanged if it were reached.
            FutureState::Done(value) => {
                *state = FutureState::Done(value.clone());
                Begin::Done(value)
            }
        })
    }

    /// Ends the await that [`Future::begin`] started on a pending future:
    /// `Done` with the value its body yielded, or `Pending` with its
    /// captured values back if the body failed, so that a later await runs
    /// it again. Returns `false`, changing nothing, only if the state is
    /// borrowed, which never happens.
    pub(crate) fn finish(&self, state: FutureState) -> bool {
        match self.state.try_borrow_mut() {
            Ok(mut current) => {
                *current = state;
                true
            }
            Err(_) => false,
        }
    }

    /// The values the future holds, for dropping: its captures if it has
    /// not run, the value it yielded if it has, and none while it runs.
    fn held_mut(&mut self) -> &mut [Value] {
        match self.state.get_mut() {
            FutureState::Pending(captures) => captures,
            FutureState::Done(value) => std::slice::from_mut(value),
            FutureState::Running => &mut [],
        }
    }

    /// The name of the future's state, for its debug form.
    fn state_name(&self) -> &'static str {
        match self.state.try_borrow().as_deref() {
            Ok(FutureState::Pending(_)) => "pending",
            Ok(FutureState::Running) | Err(_) => "running",
            Ok(FutureState::Done(_)) => "done",
        }
    }
}

/// Shows the function, its type and its state, not the IR it belongs to or
/// the values it holds.
impl fmt::Debug for Future {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Future")
            .field("function", &self.function)
            .field("type", &self.display())
            .field("state", &self.state_name())
            .finish_non_exhaustive()
    }
}

/// Futures compare by identity, as closures do: two are equal only if they
/// are the same future. No program can compare futures, since the checker
/// awaits or rejects them first; this exists for [`Value`]'s equality.
impl PartialEq for Future {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// Dropping a future frees what it holds, its captures or its value,
/// iteratively, as dropping a record does, so a chain of futures each
/// capturing the last is freed without recursion.
impl Drop for Future {
    fn drop(&mut self) {
        drop_nested(self.held_mut());
    }
}

/// Deep equality by field name: two records are equal when they have the
/// same number of fields and every field of one is in the other with an
/// equal value. Field order does not matter, and `Float` fields compare as
/// IEEE 754 does, so a record holding NaN is not equal to itself.
///
/// Nested records and lists are compared iteratively through a work list,
/// for the same reason [`Record`]'s drop is iterative: nesting is bounded
/// only by the program's length. The list only grows when both records
/// have a nested record or list in the same field, so comparing flat
/// records allocates nothing.
impl PartialEq for Record {
    fn eq(&self, other: &Self) -> bool {
        deep_equal(Pair::Records(self, other))
    }
}

/// Two records or two lists being compared by [`deep_equal`].
enum Pair<'a> {
    /// Two records, compared by field name.
    Records(&'a Record, &'a Record),
    /// The elements of two lists, compared in order.
    Lists(&'a [Value], &'a [Value]),
}

/// Whether the two records or lists of `first` are equal, deeply, comparing
/// the records and lists nested in them through a work list rather than
/// recursion.
fn deep_equal(first: Pair<'_>) -> bool {
    let mut pending = Vec::new();
    let mut next = Some(first);
    while let Some(pair) = next {
        let equal = match pair {
            Pair::Records(mine, theirs) => fields_equal(mine, theirs, &mut pending),
            Pair::Lists(mine, theirs) => {
                mine.len() == theirs.len()
                    && mine
                        .iter()
                        .zip(theirs)
                        .all(|(mine, theirs)| values_equal(mine, theirs, &mut pending))
            }
        };
        if !equal {
            return false;
        }
        next = pending.pop();
    }
    true
}

/// Whether `mine` and `theirs` are equal, as far as can be told without
/// looking inside nested records and lists: a pair of those is deferred to
/// `pending` and counts as equal here, and every other pair of values is
/// compared with `==`.
fn values_equal<'a>(mine: &'a Value, theirs: &'a Value, pending: &mut Vec<Pair<'a>>) -> bool {
    match (mine, theirs) {
        (Value::Record(a), Value::Record(b)) => {
            pending.push(Pair::Records(a, b));
            true
        }
        (Value::List(a), Value::List(b)) => {
            pending.push(Pair::Lists(a.values(), b.values()));
            true
        }
        _ => mine == theirs,
    }
}

/// Compares the fields of `mine` and `theirs`, deferring pairs of nested
/// records and lists to `pending` instead of recursing into them.
fn fields_equal<'a>(mine: &'a Record, theirs: &'a Record, pending: &mut Vec<Pair<'a>>) -> bool {
    if mine.values.len() != theirs.values.len() {
        return false;
    }
    let same_layout =
        Rc::ptr_eq(&mine.layout, &theirs.layout) || mine.layout.names == theirs.layout.names;
    for (index, (&name, value)) in mine.layout.names.iter().zip(&mine.values).enumerate() {
        let other = if same_layout {
            theirs.values.get(index)
        } else {
            theirs.get(name)
        };
        let Some(other) = other else {
            return false;
        };
        if !values_equal(value, other, pending) {
            return false;
        }
    }
    true
}

/// The result of evaluating an expression.
///
/// Strings, records and lists have value semantics but are shared: cloning
/// a `Value::Str`, a `Value::Record` or a `Value::List` increments a
/// reference count, if any, and never copies the text, the fields or the
/// elements. So does cloning a `Value::Fn`, whose
/// closure is immutable, and a `Value::Future`, which is shared by
/// reference: every copy is the same future, run at most once.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An `Int`.
    Int(i64),
    /// A `Float`.
    Float(f64),
    /// A `Bool`.
    Bool(bool),
    /// A `String`.
    Str(Rc<str>),
    /// `{}`, the empty record. Unboxed, so that the many statements whose
    /// value it is produce it without allocating.
    Empty,
    /// A record with at least one field; the empty record is always
    /// [`Value::Empty`].
    Record(Rc<Record>),
    /// A list, the empty one included.
    List(List),
    /// A function: a closure over the values it captured.
    Fn(Rc<Closure>),
    /// A future: a deferred computation, run by its first await.
    Future(Rc<Future>),
    /// A built-in function, a member of the `builtins` module: a function
    /// implemented in Rust, with nothing captured. It is of kind
    /// [`ValueKind::Fn`], as a closure is, and compares by which built-in
    /// it is. Only the driver of a workspace makes one, writing each into
    /// its cell as it lays out the frame (see [`crate::workspace`]).
    Builtin(Builtin),
}

impl Value {
    /// The kind of the value: [`ValueKind::Deferred`] for a future.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Value::Int(_) => ValueKind::Int,
            Value::Float(_) => ValueKind::Float,
            Value::Bool(_) => ValueKind::Bool,
            Value::Str(_) => ValueKind::String,
            Value::Empty | Value::Record(_) => ValueKind::Record,
            Value::List(_) => ValueKind::List,
            Value::Fn(_) | Value::Builtin(_) => ValueKind::Fn,
            Value::Future(_) => ValueKind::Deferred,
        }
    }

    /// The value as it would be written in a program, for the REPL and the
    /// built-in `show` (see [`crate::eval`], "The built-in functions"): a
    /// string double-quoted with its special characters escaped (see
    /// [`Quoted`]), and every other value as its `Display` form.
    #[must_use]
    pub const fn repr(&self) -> Repr<'_> {
        Repr(self)
    }

    /// The `Int` the value is, if it is one: what the evaluator's `Int`
    /// walker takes from a value read as a whole, an element, a field, an
    /// awaited value or a call's result.
    #[must_use]
    pub(crate) fn into_int(self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(value),
            _ => None,
        }
    }

    /// The `Float` the value is, if it is one, as [`Value::into_int`] takes
    /// an `Int`.
    #[must_use]
    pub(crate) fn into_float(self) -> Option<f64> {
        match self {
            Value::Float(value) => Some(value),
            _ => None,
        }
    }

    /// The `Bool` the value is, if it is one, as [`Value::into_int`] takes
    /// an `Int`.
    #[must_use]
    pub(crate) fn into_bool(self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(value),
            _ => None,
        }
    }

    /// The `String` the value is, if it is one, as [`Value::into_int`]
    /// takes an `Int`: the same text, not a copy.
    #[must_use]
    pub(crate) fn into_string(self) -> Option<Rc<str>> {
        match self {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }
}

/// Shows the value only, never its type.
///
/// A `Float` always shows a decimal point or an exponent, so it cannot be
/// mistaken for an `Int`: `3.0`, `1.5`, `1e300`, `1e-7`. It uses the
/// shortest text that reads back as the same value. The non-finite values
/// show as `inf`, `-inf` and `nan`. A `Bool` shows as `true` or `false`, and
/// a `String` as its text with no quotes or escaping.
///
/// A record shows as GCSL syntax, the same as its [`Value::repr`]: its
/// fields in definition order, separated by `;` as items are, each value in
/// `repr` form, so strings are quoted, as in
/// `{x = 1; y = "a"; inner = {n = 2}}`; a field name that is not a plain
/// identifier is quoted too (see [`FieldName`]). The empty record is `{}`.
/// A list shows as its elements in order, in brackets, separated by `, `,
/// each in `repr` form, as in `[1, "a", {n = 2}]`; the empty list is `[]`.
///
/// A function shows as its type, `fn(x: Int): Int`, in both forms, since it
/// has no source form of its own to show, and so does a future, as
/// `Future<Int>`, whether it has run or not: showing it never awaits it. A
/// built-in function shows as its type too, `fn(x: Float): Int`, as the
/// type table shows it; one whose typing is special has no type to show,
/// and shows as `builtin len` (see [`Builtin::display`]).
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(value) => write!(f, "{value}"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Str(text) => f.write_str(text),
            Value::Empty => f.write_str("{}"),
            Value::Fn(closure) => f.write_str(closure.display()),
            Value::Builtin(builtin) => f.write_str(builtin.display()),
            Value::Future(future) => f.write_str(future.display()),
            Value::Record(record) => {
                f.write_str("{")?;
                for (index, (text, value)) in
                    record.layout.texts.iter().zip(&record.values).enumerate()
                {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{} = {}", FieldName(text), value.repr())?;
                }
                f.write_str("}")
            }
            Value::List(list) => {
                f.write_str("[")?;
                for (index, value) in list.values().iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    value.repr().fmt(f)?;
                }
                f.write_str("]")
            }
            // `{:?}` would show NaN as `NaN`.
            Value::Float(value) if value.is_nan() => f.write_str("nan"),
            // `{:?}` on f64 is the shortest round-trip representation and
            // always includes `.` or `e`; it shows infinities as `inf`/`-inf`.
            Value::Float(value) => write!(f, "{value:?}"),
        }
    }
}

/// A [`Value`] shown as it would be written in a program. Returned by
/// [`Value::repr`].
#[derive(Clone, Copy, Debug)]
pub struct Repr<'a>(&'a Value);

impl fmt::Display for Repr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Str(text) => Quoted(text).fmt(f),
            value => value.fmt(f),
        }
    }
}

/// Converts an `Int` to the nearest `Float` (ties to even), the meaning of
/// both an integer literal in `Float` context and an `IntToFloat` node.
#[expect(
    clippy::cast_precision_loss,
    reason = "rounding to the nearest f64 is the defined meaning of Int-to-Float conversion"
)]
pub(crate) const fn int_to_float(value: i64) -> f64 {
    value as f64
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests;
