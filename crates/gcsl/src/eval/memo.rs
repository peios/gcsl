//! The memo tables of memoised functions: their keys, entries and buckets, what
//! a running call depends on, and sweeping what a run no longer needs.
//!
//! # Memoised functions
//!
//! A function declared with `->>` is lowered to code of [`FnKind::Memo`](crate::ir::FnKind::Memo),
//! and each of its closures runs its body at most once per distinct list of
//! arguments, and of the values of the ambient inputs it reads (below).
//! Memoisation belongs to the closure, not to the call site: a
//! call decides by the callee's code, one branch per call, whatever the
//! static type of the callee (a memoised closure held as a `fn(Present<Int>)
//! -> Int`, or passed to another function as a callback, is still
//! memoised), so a call of any other function costs that one comparison
//! more and nothing else. Two closures of one function expression are two
//! functions, with a table each; a recursive call through the function's
//! own name is a call of the running closure, memoised in its table.
//!
//! **The key.** A call's key is its arguments, in parameter order: the
//! parameters' slots of the callee's frame, once the arguments have been
//! evaluated into them, in source order, as for any call. Keys compare by
//! *key equality*, which is [`Value`]'s equality, total equality: a
//! function and a future by identity, the same `Rc`, and every other value
//! as `===` compares it, records deeply, by field name, whatever the order
//! of their fields, and every field, those the static type hides included,
//! and lists deeply, element by element, in order;
//! floats as IEEE 754 does, so `0.0` and `-0.0` are equal, and NaN is equal
//! to nothing, itself included: a call with a NaN among its arguments never
//! finds an entry, and always runs its body, and makes no entry, which no
//! call could find. Its hash is consistent with
//! it: the zeros hash alike, a function and a future by address, a record
//! as the sum of its fields' hashes, which does not depend on their order,
//! and a list as its length and its elements' hashes in order, down to a
//! few levels of record and list nesting, below which a record contributes
//! only its field count and a list its length (equal keys still hash
//! equally, and hashing never recurses as deeply as a record or a list may
//! be nested).
//!
//! **The tables.** The tables belong to the execution context, not to the
//! closures: [`MemoTables`], which a [`crate::session::Session`] owns for
//! its lifetime, and [`eval`](super::eval) for its run, and which every frame of an
//! evaluation borrows. Held outside the closure, a table that holds the
//! closure itself, in a key or a result, makes no reference cycle. There
//! is one table per closure, found by the closure's address, and the table
//! holds its closure weakly, which keeps the allocation, and so the
//! address, from being reused by another closure while the table exists,
//! without keeping the closure alive. A table maps the hash of each key to
//! its entries, nearly always one, held without an allocation of its own;
//! each entry holds its key, the arguments copied into a boxed slice, and
//! either the value the body returned or a mark that the body is running.
//! Keys and results are held strongly, and an entry of a live closure is
//! never evicted, however many there are: that memory is the cost of
//! `->>`. A table of a dead closure can never be hit again, and is dropped
//! by a *sweep*, which runs when a table is about to be added for another
//! closure and the number of tables has reached a threshold; afterwards
//! the threshold is twice the number of tables that survived, and at least
//! `MIN_SWEEP_THRESHOLD`, so sweeping costs amortised constant time per
//! closure called. Dropping a table may drop the last reference to other
//! memoised closures, whose tables go at a later sweep; a closure held by
//! its own table's keys or results lives as long as the context. A sweep
//! never drops the table of a running call: the caller holds its closure.
//! The one other way an entry goes is with the evaluation that made it, if
//! its context says that evaluation never happened: a session takes a
//! [`MemoTables::checkpoint`] before each input and rolls back to it if
//! the input fails, since the names and the input number that the failed
//! input's records and closures hold are given to later inputs.
//!
//! **A call.** The call grows the stack by the callee's frame, evaluates
//! the arguments into it, and looks them up where they are, which
//! allocates nothing: it hashes them in place and compares them with the
//! keys of the entries of that hash. What it finds decides:
//!
//! - an entry with a value: the call's value is a copy of it, which for a
//!   record, a closure or a future shares it, so a future a memoised
//!   function returned is one future, run at most once, whoever awaits it.
//!   The body does not run, and the call charges nothing to the nesting
//!   budget;
//! - a running entry: the function is running with equal arguments further
//!   up the stack, and waiting for itself would never finish. The call
//!   fails with "this call depends on its own result", and the call that is
//!   running the function is labelled "called here with the same
//!   arguments" when the error reaches it, as the await running a future
//!   that awaits itself is labelled "awaited here", under the same
//!   condition: that it is of the same session input as the error;
//! - no entry: the call checks the nesting budget as any call does, copies
//!   the arguments into a new entry marked running, the one allocation the
//!   memoisation makes, runs the body as any call does, and then finds the
//!   entry again, by its *identity*, a number from a counter of
//!   [`MemoTables`], new for each entry, which tells it from every other
//!   entry of its bucket, whatever their keys compare equal to and however
//!   many are empty (every empty key, having no elements, has one address),
//!   and records the value. If the body fails, the entry is removed
//!   before the error goes on, so that a later call, in a later input of a
//!   session, runs the body again.
//!
//! An entry is found only if, besides its key, every ambient input it
//! recorded has the same value for the call (below); a running entry's are
//! those recorded so far, so the check that a call depends on its own result
//! keys as the lookup does.
//!
//! **Ambient inputs.** A memoised call's result may depend on more than its
//! arguments: on an *ambient input*, a value the call reads without taking
//! it as an argument. Captures are by value and fields immutable, so today
//! the one ambient input is the caller, `fileroot.caller` (see [Callers and
//! helpers](crate::eval::callers#callers-and-helpers)), but the mechanism is general: an entry
//! records its *dependencies*, each a kind of input with the value read
//! (`Ambient`), comparing values, so that two files of one directory, whose
//! callers are equal, share entries. A read of `fileroot.caller` in a frame
//! records the caller as a dependency of every running memoised call whose
//! result it can affect: the reading frame's own, and while the caller was
//! passed on by helpers, each one up that chain whose caller it is, as far
//! as the frame whose caller was set from its calling code's file, which is
//! a constant to the frames above it, which therefore do not depend on it.
//! That chain is the running calls of the reading frame's caller's
//! *origin*: [`MemoTables`] keeps the running memoised calls on a stack,
//! each with its caller's origin, and a read adds its dependency to each
//! entry of the reader's origin, once. A call then looks up only entries
//! whose dependencies its own caller, known before the body runs, holds; a
//! call that finds one passes the entry's dependencies on up the chain, as
//! a read would, if its callee would have shared its frame's origin, the
//! frame having run `fileroot.helper()`. A memoised function that never
//! reads the caller, itself or through a callee it passed its caller on to,
//! records nothing, and is found from every file.
//!
//! A future captures its maker's caller (see [Callers and
//! helpers](crate::eval::callers#callers-and-helpers)), so a result holding one whose body will
//! read it depends on that caller even if the future is awaited only after
//! the call that made it has returned, when nothing of it runs. So making a
//! future is itself a read of the making frame's caller, recorded as any
//! read is, when the body can read the caller it captures: when lowering
//! marked its `lazy` (`MakeFuture`'s `reads_caller`), its body, but for the
//! bodies of the functions it makes, whose callers are their own, reading
//! `fileroot.caller`, calling `fileroot.helper()` or making a `lazy` so
//! marked; or when the making frame's helper flag is set, so that the
//! body's calls pass the captured caller on to callees that may read it. A
//! future neither marked nor made after `fileroot.helper()` records
//! nothing, so a memoised function returning one is found from every file.
//! A marked future read when awaited, while its maker runs, records its
//! read again, which changes nothing.
//!
//! The lookup and the recording are helpers of `Evaluator::memo_call`, so
//! that its frame, on the recursion path of a memoised recursion, holds
//! little across the body.
//!
//! Operands are evaluated left to right, so when both would fail, the left
//! one's error is reported. Items are evaluated in order, and so are a
//! call's arguments, whatever parameters they fill. Evaluation holds no
//! per-node state and allocates nothing beyond the stack, except the
//! strings, records, lists and closures the program builds, the memo tables
//! and their entries, and the diagnostic when it fails. A runtime error points
//! at the operator and labels each operand with the value it had, so the
//! user sees what actually overflowed or divided by zero.

use std::collections::HashMap;
use std::collections::hash_map::Entry as MapEntry;
use std::hash::{BuildHasherDefault, DefaultHasher, Hash, Hasher};
use std::rc::{Rc, Weak};

use crate::names::Name;
use crate::value::{Caller, Closure, ContextFrame, Value};

/// The number of tables below which [`MemoTables`] never sweeps: the
/// threshold's minimum (see [Memoised functions](#memoised-functions)).
pub(super) const MIN_SWEEP_THRESHOLD: usize = 32;

/// How many levels of record and list nesting a memo key's hash descends
/// into; a record nested deeper contributes only its field count, and a
/// list only its length. Equal keys still hash equally, and the bound keeps
/// hashing from recursing as deeply as a record or list may be nested,
/// which is as deeply as a program runs.
pub(super) const KEY_HASH_DEPTH: usize = 4;

/// The memo tables of an execution context: one table per closure of a memoised
/// function that has been called, from lists of arguments to what the closure
/// returned for them. A [`crate::session::Session`] owns one for its lifetime,
/// and [`eval`](super::eval) and [`eval_in`](super::eval_in) one for their run.
/// See [Memoised functions](#memoised-functions).
#[derive(Clone, Debug)]
pub struct MemoTables {
    /// Each closure's table, found by the closure's address, which the
    /// table's weak reference to it keeps from being reused.
    pub(super) tables: HashMap<*const Closure, MemoTable>,
    /// The number of tables at which adding one for another closure first
    /// sweeps away the tables of dead closures.
    pub(super) threshold: usize,
    /// The number of checkpoints taken: every entry records the one it was
    /// made after (see [`MemoTables::checkpoint`]).
    pub(super) generation: u64,
    /// The last origin given to a caller (see [Callers and
    /// helpers](crate::eval::callers#callers-and-helpers)): each is new, 0 being no frame's.
    pub(super) origins: u64,
    /// The last identity given to an entry: each is new, so that it tells
    /// the entry apart from every other of its bucket, as the address of
    /// its key cannot, every empty key having the same one.
    pub(super) entries: u64,
    /// The memoised calls running, outermost first, each with the origin of
    /// its caller: those an ambient input read is a dependency of (see
    /// [Memoised functions](#memoised-functions), "Ambient inputs"). Empty
    /// between evaluations.
    pub(super) running: Vec<Running>,
    /// The last identity given to a frame of the context (see
    /// [Context](crate::eval::context#context)): each is new.
    pub(super) context_frames: u64,
}

/// The origin of a caller that has none yet (see [Callers and
/// helpers](crate::eval::callers#callers-and-helpers)): a frame whose caller was set from a file
/// is given one only when another frame shares it, or when it is a memoised
/// call's, since no running memoised call can share a caller nothing has
/// shared. [`MemoTables`] never gives it.
pub(super) const NO_ORIGIN: u64 = 0;

/// A memoised call that is running, on [`MemoTables`]'s stack of them: its
/// entry, by its closure, its arguments' hash and its identity, as [`Mark`]
/// finds it, the origin of its caller and the context it runs in.
#[derive(Clone, Debug)]
pub(super) struct Running {
    /// The address of the closure, which finds its table.
    pub(super) closure: *const Closure,
    /// The hash of the arguments.
    pub(super) hash: u64,
    /// The identity of the entry, which finds it in its bucket.
    pub(super) entry: u64,
    /// The origin of the call's caller.
    pub(super) origin: u64,
    /// The context it runs in (see [Context](crate::eval::context#context)).
    pub(super) context: Option<Rc<ContextFrame>>,
}

/// Where a running memoised call's entry is: its closure's address, its
/// key's hash and its identity.
pub(super) type RunningAt = (*const Closure, u64, u64);

impl Running {
    /// Where its entry is.
    pub(super) const fn at(&self) -> RunningAt {
        (self.closure, self.hash, self.entry)
    }
}

/// A read a memoised call found in its table replays where it is called, as
/// a read there would be ([`MemoTables::lookup`]), for the step whose unit
/// is running to note (see [Graph mode](crate::eval::graph#graph-mode), "Reads").
#[derive(Clone, Copy, Debug)]
pub(super) enum Replayed {
    /// A read of the context of this declaration key.
    Context(Name),
    /// A read of every context bound.
    All,
}

/// An ambient input of a memoised call (see [Memoised
/// functions](#memoised-functions), "Ambient inputs"): a kind of input the
/// call does not take as an argument but may read, with the value it read.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ambient {
    /// `fileroot.caller`: the caller's directory, by its index in the run's
    /// table, which is its value: two files of one directory have one.
    Caller(u32),
    /// A read of a context, `^arch` (see [Context](crate::eval::context#context)): its
    /// declaration's key, with the value its nearest binding gave it, or
    /// `None` for one read from no binding, whose default's value depends on
    /// what that reads, recorded as its own reads.
    Context(Name, Option<Value>),
    /// Every context bound, by its declaration's key, with its value, in
    /// order of key: what a future made in the call may read once it has
    /// returned.
    Contexts(Rc<[(Name, Value)]>),
}

impl Ambient {
    /// Whether a call whose caller is `caller`, in the context `context`,
    /// has this value of the input.
    pub(super) fn holds(&self, caller: Caller, context: Option<&Rc<ContextFrame>>) -> bool {
        match self {
            Ambient::Caller(dir) => *dir == caller.dir,
            Ambient::Context(key, value) => {
                ContextFrame::find(context, *key).map(|(_, found)| found) == value.as_ref()
            }
            Ambient::Contexts(all) => ContextFrame::flattened(context)[..] == all[..],
        }
    }
}

/// A point in the life of [`MemoTables`], to which
/// [`MemoTables::rollback`] returns them. See [`MemoTables::checkpoint`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoCheckpoint(pub(super) u64);

impl Default for MemoTables {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoTables {
    /// Tables holding nothing.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            threshold: MIN_SWEEP_THRESHOLD,
            generation: 0,
            origins: 0,
            entries: 0,
            running: Vec::new(),
            context_frames: 0,
        }
    }

    /// A new origin, for a caller set from its calling code's file (see
    /// [Callers and helpers](crate::eval::callers#callers-and-helpers)): no frame had it before.
    pub(super) fn new_origin(&mut self) -> u64 {
        self.origins = self.origins.wrapping_add(1);
        self.origins
    }

    /// A new identity, for an entry [`MemoTables::start`] makes: no entry
    /// had it before.
    pub(super) fn new_entry(&mut self) -> u64 {
        self.entries = self.entries.wrapping_add(1);
        self.entries
    }

    /// Records `ambient`, an ambient input a frame of origin `origin` read,
    /// as a dependency of every running memoised call of that origin, once
    /// each (see [Memoised functions](#memoised-functions), "Ambient
    /// inputs"): those whose caller the reading frame's is, passed on by
    /// helpers from the first of them, whose caller was set from a file.
    /// With no memoised call running, nothing is looked up.
    pub(super) fn depend(&mut self, origin: u64, ambient: &Ambient) {
        let found: Vec<RunningAt> = self
            .running
            .iter()
            .filter(|running| running.origin == origin)
            .map(Running::at)
            .collect();
        for at in found {
            self.record(at, ambient);
        }
    }

    /// Records a read of the context of the declaration key `key` in the
    /// context `context` as a dependency of every running memoised call
    /// that shares the read's binding of it: whose context's nearest
    /// binding of it is the same frame's, or, for a read of its default,
    /// none (see [Context](crate::eval::context#context)), with the value read, once each.
    pub(super) fn depend_context(&mut self, key: Name, context: Option<&Rc<ContextFrame>>) {
        if self.running.is_empty() {
            return;
        }
        let read = ContextFrame::find(context, key);
        let frame = read.map(|(frame, _)| frame);
        let ambient = Ambient::Context(key, read.map(|(_, value)| value.clone()));
        let found: Vec<RunningAt> = self
            .running
            .iter()
            .filter(|running| {
                ContextFrame::find(running.context.as_ref(), key).map(|(at, _)| at) == frame
            })
            .map(Running::at)
            .collect();
        for at in found {
            self.record(at, &ambient);
        }
    }

    /// Records, for every running memoised call, every context its own
    /// context binds as a dependency, once each: what a future made in it
    /// may read after it has returned (see [Context](crate::eval::context#context)).
    pub(super) fn depend_contexts(&mut self) {
        let found: Vec<(RunningAt, Ambient)> = self
            .running
            .iter()
            .map(|running| {
                let all = ContextFrame::flattened(running.context.as_ref());
                (running.at(), Ambient::Contexts(all.into()))
            })
            .collect();
        for (at, ambient) in found {
            self.record(at, &ambient);
        }
    }

    /// Adds `ambient` to the dependencies of the running entry `at`, unless
    /// it has it.
    pub(super) fn record(&mut self, (closure, hash, id): RunningAt, ambient: &Ambient) {
        let entry = self
            .tables
            .get_mut(&closure)
            .and_then(|table| table.buckets.get_mut(&hash))
            .and_then(|bucket| bucket.entries_mut().iter_mut().find(|entry| entry.id == id));
        if let Some(entry) = entry
            && !entry.deps.contains(ambient)
        {
            entry.deps = entry
                .deps
                .iter()
                .cloned()
                .chain([ambient.clone()])
                .collect();
        }
    }

    /// A new identity, for a frame of the context a `with` makes (see
    /// [Context](crate::eval::context#context)).
    pub(crate) fn new_context_frame(&mut self) -> u64 {
        self.context_frames = self.context_frames.wrapping_add(1);
        self.context_frames
    }

    /// Marks this point, before an evaluation whose values may not outlive
    /// it: a session input, whose names and number are forgotten if it
    /// fails, and with them the meaning of every record and closure it
    /// made. Costs nothing until it is rolled back to.
    pub fn checkpoint(&mut self) -> MemoCheckpoint {
        self.generation += 1;
        MemoCheckpoint(self.generation)
    }

    /// Forgets every entry made since `checkpoint` was taken, so that the
    /// calls they recorded run again, and every table left with none. Only
    /// between evaluations: no call is running. Visits every entry, which
    /// only a failed evaluation pays for.
    pub fn rollback(&mut self, checkpoint: MemoCheckpoint) {
        self.tables.retain(|_, table| {
            table
                .buckets
                .retain(|_, bucket| bucket.retain(|entry| entry.generation < checkpoint.0));
            !table.buckets.is_empty()
        });
    }

    /// The number of tables held: one for each closure called since the
    /// last sweep, and one for each that survived it.
    #[must_use]
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    /// Whether `closure` has returned for the arguments `key`, as a call of
    /// it made from code of the directory `dir`, by its index in the run's
    /// table of directories, with no helper passing a caller on, would find:
    /// an entry of its table whose key is equal by key equality and each of
    /// whose dependencies that caller has (see [Memoised
    /// functions](#memoised-functions)), holding a value. A call that
    /// failed has left no entry, and one that is running holds no value.
    /// Looks the table up as such a call would, but records nothing, passes
    /// nothing on and marks nothing re-entered.
    #[must_use]
    pub(crate) fn completed(&self, closure: &Rc<Closure>, key: &[Value], dir: u32) -> bool {
        let caller = Caller {
            dir,
            ..Caller::default()
        };
        let entry = self
            .tables
            .get(&Rc::as_ptr(closure))
            .and_then(|table| table.buckets.get(&key_hash(key)))
            .and_then(|bucket| {
                bucket.entries().iter().find(|entry| {
                    key_eq(&entry.key, key) && entry.deps.iter().all(|dep| dep.holds(caller, None))
                })
            });
        entry.is_some_and(|entry| matches!(entry.state, EntryState::Done(_)))
    }

    /// The number of entries recorded as running, for tests: none once an
    /// evaluation has returned.
    #[cfg(test)]
    pub(crate) fn running_count(&self) -> usize {
        self.tables
            .values()
            .flat_map(|table| table.buckets.values())
            .flat_map(Bucket::entries)
            .filter(|entry| matches!(entry.state, EntryState::Running { .. }))
            .count()
    }

    /// Looks `key`, a call's arguments in parameter order, up in
    /// `closure`'s table, for a call whose caller is `caller`: an entry
    /// matches if its key is equal by key equality (see [Memoised
    /// functions](#memoised-functions)) and the call has the value each of
    /// its dependencies recorded ("Ambient inputs" there), a running
    /// entry's those recorded so far; without allocating. Marks a running
    /// entry found as re-entered, for the error's label. A value found
    /// passes the entry's dependencies on to the running calls of
    /// `caller`'s origin if `inherited`, the caller passed on by a helper,
    /// as a read would, which allocates a copy of them if there are any.
    pub(super) fn lookup(
        &mut self,
        closure: &Rc<Closure>,
        key: &[Value],
        (caller, inherited): (Caller, bool),
        context: Option<&Rc<ContextFrame>>,
    ) -> Lookup {
        let hash = key_hash(key);
        let Some(bucket) = self
            .tables
            .get_mut(&Rc::as_ptr(closure))
            .and_then(|table| table.buckets.get_mut(&hash))
        else {
            return Lookup::Missing(hash);
        };
        let Some(entry) = bucket.entries_mut().iter_mut().find(|entry| {
            key_eq(&entry.key, key) && entry.deps.iter().all(|dep| dep.holds(caller, context))
        }) else {
            return Lookup::Missing(hash);
        };
        let (value, passed) = match &mut entry.state {
            EntryState::Done(value) => {
                // Copied only when they are passed on, which allocates: the
                // caller only by a helper, and a context always, as a read
                // of it here would be (see [Context](crate::eval::context#context)).
                let passed: Vec<Ambient> = entry
                    .deps
                    .iter()
                    .filter(|dep| inherited || !matches!(dep, Ambient::Caller(_)))
                    .cloned()
                    .collect();
                (value.clone(), passed)
            }
            EntryState::Running { reentered } => {
                *reentered = true;
                return Lookup::Running;
            }
        };
        let mut replayed = Vec::new();
        for dep in passed {
            match dep {
                Ambient::Caller(_) => self.depend(caller.origin, &dep),
                Ambient::Context(key, _) => {
                    self.depend_context(key, context);
                    replayed.push(Replayed::Context(key));
                }
                Ambient::Contexts(_) => {
                    self.depend_contexts();
                    replayed.push(Replayed::All);
                }
            }
        }
        Lookup::Done(value, replayed)
    }

    /// Records `key`, whose hash is `hash`, as running in `closure`'s
    /// table, which is made if the closure has none, after sweeping if the
    /// number of tables has reached the threshold. Returns the entry's
    /// identity, by which [`MemoTables::finish`] finds it again, whatever
    /// its key compares equal to, and however many other entries of its
    /// bucket have empty keys.
    pub(super) fn start(&mut self, closure: &Rc<Closure>, hash: u64, key: &[Value]) -> u64 {
        let address = Rc::as_ptr(closure);
        if self.tables.len() >= self.threshold && !self.tables.contains_key(&address) {
            self.sweep();
        }
        let id = self.new_entry();
        let table = self.tables.entry(address).or_insert_with(|| MemoTable {
            closure: Rc::downgrade(closure),
            buckets: HashMap::default(),
        });
        let entry = Entry {
            key: Box::from(key),
            state: EntryState::Running { reentered: false },
            deps: Box::default(),
            generation: self.generation,
            id,
        };
        match table.buckets.entry(hash) {
            MapEntry::Occupied(mut bucket) => bucket.get_mut().push(entry),
            MapEntry::Vacant(vacant) => {
                vacant.insert(Bucket::One(entry));
            }
        }
        id
    }

    /// Ends the call of `closure` that [`MemoTables::start`] recorded as
    /// `mark`: records `value` as what it returned, or, if it failed
    /// (`None`), removes its entry. Returns whether the running entry was
    /// found re-entered, or `None` if there is no such entry, which never
    /// happens: the closure is alive while it runs, so no sweep removes its
    /// table.
    pub(super) fn finish(
        &mut self,
        closure: &Rc<Closure>,
        mark: &Mark,
        value: Option<&Value>,
    ) -> Option<bool> {
        let table = self.tables.get_mut(&Rc::as_ptr(closure))?;
        let Some(value) = value else {
            return match table.remove(mark)? {
                EntryState::Running { reentered } => Some(reentered),
                EntryState::Done(_) => Some(false),
            };
        };
        let entry = table
            .buckets
            .get_mut(&mark.hash)?
            .entries_mut()
            .iter_mut()
            .find(|entry| entry.id == mark.entry)?;
        entry.state = EntryState::Done(value.clone());
        Some(false)
    }

    /// Drops the tables of dead closures, which no call can reach again,
    /// and sets the threshold of the next sweep to twice the number of
    /// tables that survive, or [`MIN_SWEEP_THRESHOLD`] if that is more.
    /// Dropping a table may drop the last reference to other memoised
    /// closures; their tables go at a later sweep.
    pub(super) fn sweep(&mut self) {
        self.tables
            .retain(|_, table| table.closure.strong_count() > 0);
        self.threshold = (self.tables.len() * 2).max(MIN_SWEEP_THRESHOLD);
    }
}

/// One closure's memo table.
#[derive(Clone, Debug)]
pub(super) struct MemoTable {
    /// The closure, held weakly: the table neither keeps it alive nor lets
    /// its address be reused while the table exists.
    pub(super) closure: Weak<Closure>,
    /// The entries, grouped by the hash of their keys.
    pub(super) buckets: HashMap<u64, Bucket, BuildHasherDefault<Prehashed>>,
}

impl MemoTable {
    /// Removes the entry `mark` recorded, found by its identity, and
    /// returns its state; `None` if there is none.
    pub(super) fn remove(&mut self, mark: &Mark) -> Option<EntryState> {
        let bucket = self.buckets.get_mut(&mark.hash)?;
        let index = bucket
            .entries()
            .iter()
            .position(|entry| entry.id == mark.entry)?;
        if let Bucket::Many(entries) = bucket
            && entries.len() > 1
        {
            return Some(entries.swap_remove(index).state);
        }
        // The bucket's only entry: the bucket goes with it.
        match self.buckets.remove(&mark.hash)? {
            Bucket::One(entry) => Some(entry.state),
            Bucket::Many(mut entries) => entries.pop().map(|entry| entry.state),
        }
    }
}

/// The entries of a memo table whose keys have one hash: nearly always
/// one, held without an allocation of its own.
#[derive(Clone, Debug)]
pub(super) enum Bucket {
    /// The one entry.
    One(Entry),
    /// Several entries, after a collision of hashes, or of keys that are
    /// never equal to anything, such as those holding NaN.
    Many(Vec<Entry>),
}

impl Bucket {
    /// The entries.
    pub(super) fn entries(&self) -> &[Entry] {
        match self {
            Bucket::One(entry) => std::slice::from_ref(entry),
            Bucket::Many(entries) => entries,
        }
    }

    /// The entries, to change their states.
    pub(super) fn entries_mut(&mut self) -> &mut [Entry] {
        match self {
            Bucket::One(entry) => std::slice::from_mut(entry),
            Bucket::Many(entries) => entries,
        }
    }

    /// Keeps only the entries `keep` accepts, and returns whether any is
    /// left.
    pub(super) fn retain(&mut self, keep: impl Fn(&Entry) -> bool) -> bool {
        match self {
            Bucket::One(entry) => keep(entry),
            Bucket::Many(entries) => {
                entries.retain(keep);
                !entries.is_empty()
            }
        }
    }

    /// Adds `entry`.
    pub(super) fn push(&mut self, entry: Entry) {
        let previous = std::mem::replace(self, Bucket::Many(Vec::new()));
        *self = Bucket::Many(match previous {
            Bucket::One(first) => vec![first, entry],
            Bucket::Many(mut entries) => {
                entries.push(entry);
                entries
            }
        });
    }
}

/// A memoised call: the arguments it was made with, and what it returned
/// or that it is running, with the ambient inputs it depends on.
#[derive(Clone, Debug)]
pub(super) struct Entry {
    /// The arguments, in parameter order.
    pub(super) key: Box<[Value]>,
    /// Whether the call is running or has returned, and what.
    pub(super) state: EntryState,
    /// The ambient inputs the call depends on, each with the value it read,
    /// once each (see [Memoised functions](#memoised-functions), "Ambient
    /// inputs"): empty, and allocating nothing, for a call that reads none,
    /// and copied afresh for each new one, which is rare.
    pub(super) deps: Box<[Ambient]>,
    /// The checkpoint the entry was made after (see
    /// [`MemoTables::checkpoint`]).
    pub(super) generation: u64,
    /// The entry's identity, new for each (see [`MemoTables::start`]): what
    /// a running call finds its entry by, since keys that compare equal may
    /// share a bucket, and empty keys share an address.
    pub(super) id: u64,
}

/// Where a memoised call is.
#[derive(Clone, Debug)]
pub(super) enum EntryState {
    /// The body is running further up the stack; `reentered` once a call
    /// inside it has found it so.
    Running {
        /// Whether a call inside the body has found this entry running.
        reentered: bool,
    },
    /// The body returned this value.
    Done(Value),
}

/// What looking a call's arguments up in a memo table finds.
pub(super) enum Lookup {
    /// The closure returned this value for equal arguments, the reads it
    /// replays here with it.
    Done(Value, Vec<Replayed>),
    /// The closure is running with equal arguments.
    Running,
    /// No entry: the arguments' hash, for recording them.
    Missing(u64),
}

/// How a memoised call begins, from [`Evaluator::memo_begin`](super::Evaluator::memo_begin).
pub(super) enum MemoBegin {
    /// The closure returned this value for equal arguments before.
    Found(Value),
    /// The call is recorded as running and its frame is on the stack.
    Started(Mark),
    /// The arguments equal nothing, so the call is not recorded: its frame
    /// is on the stack, and its body runs at this nesting, with this caller.
    Unkeyed(usize, Caller),
}

/// A running memoised call, as [`MemoTables::start`] recorded it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Mark {
    /// The hash of the arguments.
    pub(super) hash: u64,
    /// The identity of the entry, which finds it again in its bucket.
    pub(super) entry: u64,
    /// The nesting at which the body runs (see the module documentation).
    pub(super) entering: usize,
    /// The caller the body runs with (see [Callers and
    /// helpers](crate::eval::callers#callers-and-helpers)).
    pub(super) caller: Caller,
}

/// The hasher of a memo table's buckets, whose keys are hashes already: it
/// takes a `u64` as its hash.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Prehashed(pub(super) u64);

impl Hasher for Prehashed {
    fn finish(&self) -> u64 {
        self.0
    }

    /// Only a `u64` is ever hashed; any other bytes are folded in.
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

/// Whether the memo keys `a` and `b` are equal, by key equality (see
/// [Memoised functions](#memoised-functions)).
pub(super) fn key_eq(a: &[Value], b: &[Value]) -> bool {
    a == b
}

/// The hash of the memo key `key`, consistent with key equality: equal keys
/// hash equally. The standard library's [`DefaultHasher`], `SipHash` with
/// fixed keys: it needs no dependency, and its output is spread well
/// enough for the buckets to use as it is (see [`Prehashed`]).
pub(super) fn key_hash(key: &[Value]) -> u64 {
    let mut state = DefaultHasher::new();
    state.write_usize(key.len());
    for value in key {
        hash_value(value, &mut state, KEY_HASH_DEPTH);
    }
    state.finish()
}

/// Feeds `value` to `state`, consistently with key equality: a float's
/// zeros alike, a function and a future by address, a built-in function by
/// which it is, a record, `depth`
/// levels deep, as the sum of its fields' hashes, which does not depend on
/// their order, since total equality compares fields by name, and a list,
/// `depth` levels deep, as its elements in order, since total equality
/// compares elements by position.
pub(super) fn hash_value(value: &Value, state: &mut DefaultHasher, depth: usize) {
    match value {
        Value::Int(value) => {
            state.write_u8(0);
            state.write_i64(*value);
        }
        Value::Float(value) => {
            state.write_u8(1);
            let bits = value.to_bits();
            // `0.0` and `-0.0` are equal, and differ only in the sign bit.
            state.write_u64(if bits << 1 == 0 { 0 } else { bits });
        }
        Value::Bool(value) => {
            state.write_u8(2);
            state.write_u8(u8::from(*value));
        }
        Value::Quantity(quantity, amount) => {
            state.write_u8(10);
            quantity.hash(state);
            state.write_i64(*amount);
        }
        Value::Str(text) => {
            state.write_u8(3);
            text.hash(state);
        }
        Value::Empty => state.write_u8(4),
        Value::Tag(tag) => {
            state.write_u8(11);
            tag.key.hash(state);
        }
        Value::Variant(variant) => {
            state.write_u8(12);
            variant.tag.key.hash(state);
            if depth > 0 {
                hash_value(&variant.payload, state, depth - 1);
            }
        }
        // By its identity alone, as it is equal (see [Secrets](crate::eval::secrets#secrets)).
        Value::Secret(secret) => {
            state.write_u8(13);
            secret.identity.hash(state);
            secret.revealed.hash(state);
        }
        // By its path and identity, as it is equal (see
        // [Files](crate::eval::files#files)).
        Value::File(file) => {
            state.write_u8(14);
            file.hash(state);
        }
        Value::Dir(dir) => {
            state.write_u8(15);
            dir.hash(state);
        }
        Value::Sandbox(sandbox) => {
            state.write_u8(16);
            sandbox.hash(state);
        }
        Value::Record(record) => {
            state.write_u8(5);
            state.write_usize(record.values.len());
            if let Some(depth) = depth.checked_sub(1) {
                let mut sum: u64 = 0;
                for (name, value) in record.layout.names.iter().zip(&record.values) {
                    let mut field = DefaultHasher::new();
                    name.hash(&mut field);
                    hash_value(value, &mut field, depth);
                    sum = sum.wrapping_add(field.finish());
                }
                state.write_u64(sum);
            }
        }
        Value::List(list) => {
            state.write_u8(8);
            state.write_usize(list.len());
            if let Some(depth) = depth.checked_sub(1) {
                for value in list.values() {
                    hash_value(value, state, depth);
                }
            }
        }
        Value::Fn(closure) => {
            state.write_u8(6);
            Rc::as_ptr(closure).hash(state);
        }
        Value::Builtin(builtin) => {
            state.write_u8(9);
            builtin.hash(state);
        }
        Value::Future(future) => {
            state.write_u8(7);
            Rc::as_ptr(future).hash(state);
        }
    }
}
