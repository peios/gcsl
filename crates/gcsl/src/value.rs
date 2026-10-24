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
//! the type table shows it (`fn(x: Float): Int` for `int`, and
//! `fn(message: String): Never` for `panic`), or as `builtin len`
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
use crate::diagnostic::Diagnostic;
use crate::ir::Ir;
use crate::names::Name;
use crate::strings::{FieldName, Quoted};

pub use list::List;

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
    /// A `Duration` or a `Size`, [`Value::Quantity`].
    Quantity(Quantity),
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

/// A quantity with a unit: a duration, of time in whole nanoseconds, or a
/// size, of data in whole bytes. Each is an `i64` of its smallest unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quantity {
    /// A `Duration`, of nanoseconds.
    Duration,
    /// A `Size`, of bytes.
    Size,
}

impl Quantity {
    /// What it is, in prose: `duration` or `size`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Quantity::Duration => "duration",
            Quantity::Size => "size",
        }
    }

    /// The name of its type: `Duration` or `Size`.
    #[must_use]
    pub const fn type_name(self) -> &'static str {
        match self {
            Quantity::Duration => "Duration",
            Quantity::Size => "Size",
        }
    }

    /// Its smallest unit, in the plural: `nanoseconds` or `bytes`.
    #[must_use]
    pub const fn smallest(self) -> &'static str {
        match self {
            Quantity::Duration => "nanoseconds",
            Quantity::Size => "bytes",
        }
    }
}

/// A unit of a duration or a size literal: its name, its quantity, and
/// how many of the smallest unit, a nanosecond or a byte, it is.
pub(crate) struct Unit {
    /// Its name, as written after the number.
    pub(crate) name: &'static str,
    /// What it is a unit of.
    pub(crate) quantity: Quantity,
    /// How many of the smallest unit it is.
    pub(crate) factor: u128,
}

/// A duration's units, from the largest to the smallest, the order its
/// components are written in. A day is exactly 24 hours.
pub(crate) const DURATION_UNITS: [Unit; 7] = [
    unit("d", Quantity::Duration, 86_400_000_000_000),
    unit("h", Quantity::Duration, 3_600_000_000_000),
    unit("m", Quantity::Duration, 60_000_000_000),
    unit("s", Quantity::Duration, 1_000_000_000),
    unit("ms", Quantity::Duration, 1_000_000),
    unit("us", Quantity::Duration, 1_000),
    unit("ns", Quantity::Duration, 1),
];

/// A size's units: the byte, the decimal ones, powers of 1000, and the
/// binary ones, powers of 1024. There are none of exabytes: `1E5` is a
/// float.
pub(crate) const SIZE_UNITS: [Unit; 11] = [
    unit("B", Quantity::Size, 1),
    unit("KB", Quantity::Size, 1_000),
    unit("MB", Quantity::Size, 1_000_000),
    unit("GB", Quantity::Size, 1_000_000_000),
    unit("TB", Quantity::Size, 1_000_000_000_000),
    unit("PB", Quantity::Size, 1_000_000_000_000_000),
    unit("KiB", Quantity::Size, 1 << 10),
    unit("MiB", Quantity::Size, 1 << 20),
    unit("GiB", Quantity::Size, 1 << 30),
    unit("TiB", Quantity::Size, 1 << 40),
    unit("PiB", Quantity::Size, 1 << 50),
];

/// The unit `name` of `quantity`, `factor` of its smallest unit.
const fn unit(name: &'static str, quantity: Quantity, factor: u128) -> Unit {
    Unit {
        name,
        quantity,
        factor,
    }
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
    free_nested(pending);
}

/// Frees what is on `pending`, and all it holds in turn, through the work
/// list [`drop_nested`] starts from: what a list's drop starts from too.
fn free_nested(mut pending: Vec<Nested>) {
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
                list.take_held(&mut pending);
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

    /// The attributes of the closure's function, in source order: what a
    /// host reads of how to run a call of it (see the parser's
    /// [Attributes](crate::parser::attributes#attributes)). Empty for a function that
    /// is no function item, or has none.
    #[must_use]
    pub fn attributes(&self) -> &[crate::check::Attribute] {
        self.code
            .find_function(self.function)
            .map_or(&[], |code| &code.attributes)
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
    /// A step's result that a host has yet to give, the step its node of
    /// the action graph, by number (see [Graph mode](crate::eval::graph#graph-mode)).
    Host(u32),
    /// A step's failure, which a host gave, and which every await raises.
    Failed(Box<Diagnostic>),
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
    /// The future is a step's result that a host has yet to give, the step
    /// its node, by number.
    Host(u32),
    /// The future is a step's failure.
    Failed(Diagnostic),
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
    /// The context it was made in, which its body runs with, whoever awaits
    /// it (see [`crate::eval`], "Context").
    pub(crate) context: Option<Rc<ContextFrame>>,
}

/// A frame of the context, what one `with` binds (see [`crate::eval`],
/// "Context"): each context it binds, by its declaration's key, with its
/// value, over the context it was evaluated in.
#[derive(Debug)]
pub(crate) struct ContextFrame {
    /// Its identity, new for each `with` evaluated, by which a memoised
    /// call tells whether a read of a context is of its binding.
    pub(crate) id: u64,
    /// What it binds.
    pub(crate) binds: Box<[(Name, Value)]>,
    /// The context it was evaluated in.
    pub(crate) outer: Option<Rc<ContextFrame>>,
}

impl ContextFrame {
    /// The nearest binding of the context of the declaration key `key` in
    /// the context `context`: the identity of the frame binding it and its
    /// value; `None` if no frame does, so that its default is read.
    pub(crate) fn find(context: Option<&Rc<Self>>, key: Name) -> Option<(u64, &Value)> {
        let mut at = context;
        while let Some(frame) = at {
            if let Some((_, value)) = frame.binds.iter().find(|&&(bound, _)| bound == key) {
                return Some((frame.id, value));
            }
            at = frame.outer.as_ref();
        }
        None
    }

    /// Every context the context `context` binds, by its declaration's key,
    /// with the value of its nearest binding, in order of key.
    pub(crate) fn flattened(context: Option<&Rc<Self>>) -> Vec<(Name, Value)> {
        let mut found: Vec<(Name, Value)> = Vec::new();
        let mut at = context;
        while let Some(frame) = at {
            for (key, value) in &frame.binds {
                if !found.iter().any(|(seen, _)| seen == key) {
                    found.push((*key, value.clone()));
                }
            }
            at = frame.outer.as_ref();
        }
        found.sort_by_key(|&(key, _)| key);
        found
    }
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
        Self::made_by(code, function, captures, (Caller::default(), None))
    }

    /// [`Future::new`], made by a frame whose caller, helper flag and
    /// origin are `made`, in the context `context`, both of which its body
    /// runs with.
    pub(crate) fn made_by(
        code: Rc<Ir>,
        function: FnId,
        captures: Box<[Value]>,
        (made, context): (Caller, Option<Rc<ContextFrame>>),
    ) -> Self {
        Self {
            code,
            function,
            state: RefCell::new(FutureState::Pending(captures)),
            made,
            context,
        }
    }

    /// A future that stands for the result of the step that node number
    /// `node` of an action graph is, a call of the function `function` of
    /// `code`, made by `made` in `context`, until a host gives it (see
    /// [Graph mode](crate::eval::graph#graph-mode)).
    pub(crate) fn host(
        code: Rc<Ir>,
        function: FnId,
        node: u32,
        (made, context): (Caller, Option<Rc<ContextFrame>>),
    ) -> Self {
        Self {
            code,
            function,
            state: RefCell::new(FutureState::Host(node)),
            made,
            context,
        }
    }

    /// The number of the node of an action graph whose result the future
    /// stands for, while a host has yet to give it.
    #[must_use]
    pub(crate) fn host_node(&self) -> Option<u32> {
        match self.state.try_borrow().as_deref() {
            Ok(FutureState::Host(node)) => Some(*node),
            _ => None,
        }
    }

    /// Gives the future, a step's result, what its step came to: its value,
    /// which every await then yields, or its failure, which every await
    /// raises. Returns `false`, changing nothing, if the future is no step's
    /// waiting result.
    pub(crate) fn resolve(&self, outcome: Result<Value, Diagnostic>) -> bool {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return false;
        };
        if !matches!(*state, FutureState::Host(_)) {
            return false;
        }
        *state = match outcome {
            Ok(value) => FutureState::Done(value),
            Err(error) => FutureState::Failed(Box::new(error)),
        };
        true
    }

    /// The values the future holds while it has not run, its captures, for
    /// a walk of a value through it; none once it has run or for a step's.
    pub(crate) fn with_captures<T>(&self, f: impl FnOnce(&[Value]) -> T) -> Option<T> {
        match self.state.try_borrow().as_deref() {
            Ok(FutureState::Pending(captures)) => Some(f(captures)),
            _ => None,
        }
    }

    /// The failure the future raised, if it has run and failed.
    pub(crate) fn failure(&self) -> Option<Diagnostic> {
        match self.state.try_borrow().as_deref() {
            Ok(FutureState::Failed(error)) => Some((**error).clone()),
            _ => None,
        }
    }

    /// The value the future yielded, if it has run.
    pub(crate) fn value(&self) -> Option<Value> {
        match self.state.try_borrow().as_deref() {
            Ok(FutureState::Done(value)) => Some(value.clone()),
            _ => None,
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
        match &*state {
            FutureState::Host(node) => return Some(Begin::Host(*node)),
            FutureState::Failed(error) => return Some(Begin::Failed((**error).clone())),
            _ => {}
        }
        Some(match std::mem::replace(&mut *state, FutureState::Running) {
            FutureState::Pending(captures) => Begin::Pending(captures),
            FutureState::Running => Begin::Running,
            // Handled above; put back unchanged if they were reached.
            FutureState::Host(node) => {
                *state = FutureState::Host(node);
                Begin::Host(node)
            }
            FutureState::Failed(error) => {
                let raised = (*error).clone();
                *state = FutureState::Failed(error);
                Begin::Failed(raised)
            }
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
            FutureState::Running | FutureState::Host(_) | FutureState::Failed(_) => &mut [],
        }
    }

    /// The name of the future's state, for its debug form.
    fn state_name(&self) -> &'static str {
        match self.state.try_borrow().as_deref() {
            Ok(FutureState::Pending(_)) => "pending",
            Ok(FutureState::Running) | Err(_) => "running",
            Ok(FutureState::Done(_)) => "done",
            Ok(FutureState::Host(_)) => "waiting",
            Ok(FutureState::Failed(_)) => "failed",
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
    /// A `Duration`, of nanoseconds, or a `Size`, of bytes.
    Quantity(Quantity, i64),
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
    /// A variant of an enum as a value, `Toolchain::C`: of kind
    /// [`ValueKind::Fn`], since a call of it makes a value of the enum, and
    /// compared by which variant it is (see the checker's "Enums").
    Tag(Rc<Tag>),
    /// A value of an enum: a variant and its payload, compared by both.
    Variant(Rc<Variant>),
    /// A secret: a value that never leaks, shown as `***` (see
    /// [`crate::eval`], "Secrets"), compared by its identity alone.
    Secret(Rc<Secret>),
    /// A file: its path and the content identity a host gives it (see
    /// [Files](crate::eval::files#files)), compared by both.
    File(Rc<File>),
    /// A directory, a tree of files: its path and the content identity a
    /// host gives it, as a file's (see [Files](crate::eval::files#files)),
    /// compared by both.
    Dir(Rc<File>),
    /// A sandbox: a live copy of a directory, which commands run in as
    /// their `/`, with the working directory and environment they are
    /// given by default (see [Sandboxes](crate::eval::sandbox#sandboxes)),
    /// compared by its copy alone.
    Sandbox(Rc<Sandbox>),
}

/// A sandbox, as values carry it (see
/// [Sandboxes](crate::eval::sandbox#sandboxes)).
#[derive(Debug)]
pub struct Sandbox {
    /// The copy, its `/`, as an absolute path of the host's: a directory of
    /// the run's, its own.
    pub root: Rc<str>,
    /// The working directory of its commands by default, absolute, inside
    /// it.
    pub cwd: Rc<str>,
    /// The environment of its commands by default, the whole of it.
    pub env: Box<[(Rc<str>, Rc<str>)]>,
}

/// Two sandboxes are equal if they are one copy: each has its own.
impl PartialEq for Sandbox {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
    }
}

impl Eq for Sandbox {}

impl std::hash::Hash for Sandbox {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.root.hash(state);
    }
}

/// A file or a directory, as values carry it (see
/// [Files](crate::eval::files#files)): [`Value::File`] or [`Value::Dir`]
/// says which.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct File {
    /// Its path, absolute.
    pub path: Rc<str>,
    /// The identity of its content, as a host records it, a directory's of
    /// its whole tree: `None` under plain GCSL, which never looks at a
    /// file's content to make one.
    pub identity: Option<Rc<str>>,
}

/// A secret, as values carry it (see [`crate::eval`], "Secrets").
#[derive(Debug)]
pub struct Secret {
    /// Its value, never a secret itself.
    pub value: Value,
    /// Its public identity, what a host's receipts record of it.
    pub identity: Rc<str>,
    /// Whether it is revealed, `Revealed<T>`, which a program may be given.
    pub revealed: bool,
}

/// Two secrets are equal if their identities are, whatever their values,
/// and they are both revealed or both not: a comparison that is not itself
/// a secret, total equality's of fields a type does not name or a memoised
/// call's of its arguments, must not tell of a secret's value (see
/// [`crate::eval`], "Secrets"), and whether one is revealed is public, its
/// type's. A comparison of secrets' values is the checker's, of their
/// values, into a secret.
impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity && self.revealed == other.revealed
    }
}

/// A variant of an enum, as values carry it (see the checker's "Enums").
#[derive(Debug, PartialEq, Eq)]
pub struct Tag {
    /// Its key, its identity ([`crate::names::variant_key`]).
    pub key: Name,
    /// Its enum's key ([`crate::names::enum_key`]).
    pub of: Name,
    /// How it is shown, `Toolchain::C`.
    pub shown: Rc<str>,
}

/// A value of an enum: its variant and its payload.
#[derive(Debug, PartialEq)]
pub struct Variant {
    /// The variant.
    pub tag: Rc<Tag>,
    /// The payload.
    pub payload: Value,
}

impl Value {
    /// The kind of the value: [`ValueKind::Deferred`] for a future.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Value::Int(_) => ValueKind::Int,
            &Value::Quantity(quantity, _) => ValueKind::Quantity(quantity),
            Value::Float(_) => ValueKind::Float,
            Value::Bool(_) => ValueKind::Bool,
            Value::Str(_) => ValueKind::String,
            Value::Empty | Value::Record(_) => ValueKind::Record,
            Value::List(_) => ValueKind::List,
            Value::Fn(_) | Value::Builtin(_) | Value::Tag(_) => ValueKind::Fn,
            Value::Future(_) => ValueKind::Deferred,
            // A value of an enum is of no kind of its own: present, of the
            // kind a node of an enum's type has; and so is a secret.
            Value::Variant(_)
            | Value::Secret(_)
            | Value::File(_)
            | Value::Dir(_)
            | Value::Sandbox(_) => ValueKind::Mixed,
        }
    }

    /// This value written as the derivation of a secret computed on it,
    /// every secret in it as its identity (see [`Derivation`]).
    #[must_use]
    pub fn derivation(&self) -> Derivation<'_> {
        Derivation(self)
    }

    /// The identities of the secrets this value holds, in a field, an
    /// element or a payload at any depth, in the order met, each once:
    /// what a host records of a value's secrets, never their values (see
    /// [`crate::eval`], "Secrets"). A closure's or a future's captures are
    /// not looked into.
    #[must_use]
    pub fn secret_identities(&self) -> Vec<Rc<str>> {
        let mut found: Vec<Rc<str>> = Vec::new();
        let mut pending: Vec<&Value> = vec![self];
        while let Some(value) = pending.pop() {
            match value {
                Value::Secret(secret) => {
                    if !found.contains(&secret.identity) {
                        found.push(Rc::clone(&secret.identity));
                    }
                }
                Value::Record(record) => pending.extend(record.values.iter().rev()),
                Value::List(list) => pending.extend(list.values().iter().rev()),
                Value::Variant(variant) => pending.push(&variant.payload),
                _ => {}
            }
        }
        found
    }

    /// The files this value holds, in a field, an element or a payload at
    /// any depth, in the order met, each once: what a host takes into a
    /// step's sandbox of its arguments, and back out of its result (see
    /// [Files](crate::eval::files#files)). A closure's or a future's captures are not
    /// looked into, nor a secret's value. A directory is no file: see
    /// [`Value::dirs`].
    #[must_use]
    pub fn files(&self) -> Vec<Rc<File>> {
        self.found(|value| match value {
            Value::File(file) => Some(file),
            _ => None,
        })
    }

    /// The directories this value holds, as [`Value::files`] finds its
    /// files: each a tree a host takes into a step's sandbox whole.
    #[must_use]
    pub fn dirs(&self) -> Vec<Rc<File>> {
        self.found(|value| match value {
            Value::Dir(dir) => Some(dir),
            _ => None,
        })
    }

    /// What `pick` picks of the values this value holds, in a field, an
    /// element or a payload at any depth, in the order met, each once.
    fn found(&self, pick: impl Fn(&Value) -> Option<&Rc<File>>) -> Vec<Rc<File>> {
        let mut found: Vec<Rc<File>> = Vec::new();
        let mut pending: Vec<&Value> = vec![self];
        while let Some(value) = pending.pop() {
            if let Some(picked) = pick(value) {
                if !found.contains(picked) {
                    found.push(Rc::clone(picked));
                }
                continue;
            }
            match value {
                Value::Record(record) => pending.extend(record.values.iter().rev()),
                Value::List(list) => pending.extend(list.values().iter().rev()),
                Value::Variant(variant) => pending.push(&variant.payload),
                _ => {}
            }
        }
        found
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

    /// The amount of the duration the value is, if it is one, as
    /// [`Value::into_int`] takes an `Int`.
    #[must_use]
    pub(crate) fn into_duration(self) -> Option<i64> {
        match self {
            Value::Quantity(Quantity::Duration, amount) => Some(amount),
            _ => None,
        }
    }

    /// The amount of the size the value is, if it is one, as
    /// [`Value::into_int`] takes an `Int`.
    #[must_use]
    pub(crate) fn into_size(self) -> Option<i64> {
        match self {
            Value::Quantity(Quantity::Size, amount) => Some(amount),
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
/// A duration or a size shows as its canonical literal ([`write_quantity`]):
/// `1m30s`, `2GiB`.
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
            &Value::Quantity(quantity, amount) => write_quantity(f, quantity, amount),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Str(text) => f.write_str(text),
            Value::Empty => f.write_str("{}"),
            Value::Fn(closure) => f.write_str(closure.display()),
            Value::Builtin(builtin) => f.write_str(builtin.display()),
            Value::Tag(tag) => f.write_str(&tag.shown),
            Value::Variant(variant) => {
                write!(f, "{}({})", variant.tag.shown, variant.payload.repr())
            }
            // The one way a secret is shown, wherever it is.
            Value::Secret(_) => f.write_str("***"),
            // As the call that makes it, which reads back as an equal file.
            Value::File(file) => write!(f, "file({})", Quoted(&file.path)),
            Value::Dir(dir) => write!(f, "dir({})", Quoted(&dir.path)),
            // No call makes one again: it is a live directory, not data.
            Value::Sandbox(sandbox) => write!(f, "<sandbox {}>", sandbox.root),
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

/// A [`Value`] written as the derivation of a secret computed on it (see
/// [`crate::eval`], "Secrets"): as [`Repr`] writes it, but every secret in
/// it as `$(identity)`, its identity with `\` and `)` escaped, so that two
/// derivations are written alike only if they are alike. Returned by
/// [`Value::derivation`].
#[derive(Clone, Copy, Debug)]
pub struct Derivation<'a>(&'a Value);

impl fmt::Display for Derivation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Secret(secret) => {
                f.write_str("$(")?;
                for c in secret.identity.chars() {
                    if c == '\\' || c == ')' {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
                f.write_str(")")
            }
            Value::Record(record) => {
                f.write_str("{")?;
                for (index, (text, value)) in
                    record.layout.texts.iter().zip(&record.values).enumerate()
                {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{} = {}", FieldName(text), Derivation(value))?;
                }
                f.write_str("}")
            }
            Value::List(list) => {
                f.write_str("[")?;
                for (index, value) in list.values().iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    Derivation(value).fmt(f)?;
                }
                f.write_str("]")
            }
            Value::Variant(variant) => {
                write!(f, "{}({})", variant.tag.shown, Derivation(&variant.payload))
            }
            value => value.repr().fmt(f),
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

mod list;

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests;

/// Writes the canonical literal of the `quantity` of `amount` of its
/// smallest unit, a literal of the same value: a duration's components that
/// are not zero, from days down to nanoseconds, as in `1m30s` and
/// `1s500ms`, or `0s`; a size in the largest unit, decimal or binary, that
/// divides it exactly, as in `2GiB`, `1500B` and `1MB`, or `0B`. A negative
/// one is `-` and the literal of its magnitude.
pub(crate) fn write_quantity(
    f: &mut fmt::Formatter<'_>,
    quantity: Quantity,
    amount: i64,
) -> fmt::Result {
    if amount < 0 {
        f.write_str("-")?;
    }
    let magnitude = u128::from(amount.unsigned_abs());
    match quantity {
        Quantity::Duration if magnitude == 0 => f.write_str("0s"),
        Quantity::Duration => {
            let mut rest = magnitude;
            for unit in &DURATION_UNITS {
                let count = rest / unit.factor;
                rest %= unit.factor;
                if count > 0 {
                    write!(f, "{count}{}", unit.name)?;
                }
            }
            Ok(())
        }
        // Every unit divides zero: it is in bytes.
        Quantity::Size if magnitude == 0 => f.write_str("0B"),
        Quantity::Size => {
            let unit = SIZE_UNITS
                .iter()
                .filter(|unit| magnitude.is_multiple_of(unit.factor))
                .max_by_key(|unit| unit.factor)
                .unwrap_or(&SIZE_UNITS[0]);
            write!(f, "{}{}", magnitude / unit.factor, unit.name)
        }
    }
}
