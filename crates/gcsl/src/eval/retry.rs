//! Retrying and timing calls: the attempts of a function with `#[retry]` and
//! the deadlines of one with `#[timeout]`.
//!
//! # Retry and timeout
//!
//! A call of a function with `#[retry]` or `#[timeout]` (see the checker's
//! [Attributes](crate::check::attributes#attributes)), whose code says so
//! ([`FnCode::retry`], [`FnCode::timeout`]), runs its body through
//! `Evaluator::guarded_body`, a frame between the call and the body that
//! costs [`GUARD_LEVELS`] more of the nesting budget; every other call runs
//! as it did. Each attempt runs the body once.
//!
//! **Retry.** An attempt that fails with an error that is neither a bug in
//! GCSL ([`ErrorMark::INTERNAL`]) nor the passing of an enclosing call's
//! deadline is run again, while retries are left: the frame's variables
//! emptied and its arguments, defaults included, kept, so the body starts
//! afresh, with its effects repeated. The last attempt's error is noted
//! "`f` was tried 3 times". A memoised call retries inside its memoisation,
//! which records only a success.
//!
//! **Timeout.** An attempt of a function with a timeout runs under a
//! *deadline*, kept in a stack in the run's shared state
//! ([`Shared`](super::Shared)), outermost first: its own instant, the attempt's
//! start plus the limit, and its effective one, the earlier of its own and the
//! enclosing deadline's, which a cell keeps for the innermost so that a check
//! reads one cell while none runs. The evaluator checks it at every call and
//! every iteration of a loop, so pure code that runs long is stopped too, and
//! at every `exec`, which runs its program until the deadline (`run_until`):
//! the program is looked at, at first often and then every
//! [`DEADLINE_POLL`](super::system::DEADLINE_POLL) at most, and killed when the
//! deadline passes (the program itself, not its own children). When one has
//! passed, the error is the outermost passed deadline's, "`f` timed out after
//! 30s", where the evaluation was, marked with the deadline's number
//! ([`ErrorMark`]): every call inside passes it on, unretried, and the call
//! that set the deadline clears the mark, which makes it that call's own
//! failure, which its own `#[retry]` may retry, timed afresh, and any call
//! around it too.

use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::MAX_DEPTH;
use crate::diagnostic::{Diagnostic, Mark as ErrorMark};
use crate::ir::{FnCode, IrNode};
use crate::value::{Caller, Closure, Value};

use super::{
    Deadline, Evaluator, Flow, GUARD_LEVELS, Unwind, mistyped, nested_too_deeply, shown_duration,
};

impl Evaluator<'_> {
    /// Finishes the call `node` of `closure`, whose function has `#[retry]`
    /// or `#[timeout]`, its frame at `base` holding its arguments, with the
    /// caller `caller`, at `levels` of nesting: runs its body as
    /// [`Evaluator::guarded_body`] does, then truncates the stack back and
    /// gives the result, as [`Evaluator::call`] does for any other call.
    #[inline(never)]
    pub(super) fn guarded_call(
        &mut self,
        closure: &Rc<Closure>,
        base: usize,
        (caller, levels): (Caller, usize),
        node: &IrNode,
    ) -> Flow<Value> {
        let Some(code) = closure.code.find_function(closure.function) else {
            self.stack.truncate(base);
            return mistyped(node, "a closure of a function its IR lacks");
        };
        let result = self.guarded_body((closure, code), base, (caller, levels), node);
        self.stack.truncate(base);
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Err(mut error) if error.input.is_none() && closure.code.input() != self.ir.input() => {
                error.input = Some(closure.code.input());
                Err(error.into())
            }
            result => result.map_err(Unwind::from),
        }
    }

    /// Runs the body of `closure`, whose code `code` has `#[retry]` or
    /// `#[timeout]`, in the frame at `base`, which holds its arguments, with
    /// the caller `caller`, at `levels` of nesting and one more for this
    /// frame (see [Retry and timeout](#retry-and-timeout)). Each attempt
    /// runs under the function's own deadline, if it has a timeout, and an
    /// error of its passing is turned into the call's own error, its mark
    /// cleared. An attempt that fails with an error that is neither a bug
    /// in GCSL nor an enclosing call's timeout is run again, its frame's
    /// variables emptied and its arguments kept, while retries are left;
    /// the last error is noted with how many times the call was tried. A
    /// `return` for the function is its value, as for any call.
    #[inline(never)]
    pub(super) fn guarded_body(
        &mut self,
        (closure, code): (&Rc<Closure>, &FnCode),
        base: usize,
        (caller, levels): (Caller, usize),
        node: &IrNode,
    ) -> Flow<Value> {
        let levels = levels + GUARD_LEVELS;
        if levels + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let shown = code
            .name_text
            .as_deref()
            .map_or_else(|| "the call".to_owned(), |name| format!("`{name}`"));
        let mut tried: u32 = 0;
        loop {
            tried = tried.saturating_add(1);
            if tried > 1 {
                let locals = base + code.param_count as usize..base + code.frame_size as usize;
                for slot in self.stack.get_mut(locals).into_iter().flatten() {
                    *slot = Value::Empty;
                }
            }
            let deadline = code.timeout.map(|limit| {
                self.push_deadline(
                    limit,
                    &format!("{shown} timed out after {}", shown_duration(limit)),
                )
            });
            let result = Evaluator {
                ir: &closure.code,
                stack: self.stack,
                base,
                captures: &closure.captures,
                callee: Some(closure),
                memo: self.memo,
                shared: self.shared,
                caller,
                levels,
            }
            .value(code.body);
            if let Some(number) = deadline {
                self.pop_deadline(number);
            }
            let mut error = match result.or_else(|unwind| unwind.returned(node.span)) {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };
            // Its own deadline's passing is its own error.
            if deadline.is_some() && error.mark.deadline_number() == deadline {
                error.mark = ErrorMark::NONE;
            }
            if error.mark == ErrorMark::NONE && tried <= code.retry {
                continue;
            }
            if tried > 1 && error.mark == ErrorMark::NONE {
                error.notes.push(format!("{shown} was tried {tried} times"));
            }
            return Err(error.into());
        }
    }

    /// Starts a deadline of `limit` from now, inside the running ones, its
    /// passing the error `message`: the effective instant is the earlier of
    /// its own and the enclosing one's. Returns its number.
    pub(super) fn push_deadline(&self, limit: Duration, message: &str) -> u32 {
        let now = Instant::now();
        let own = now
            .checked_add(limit)
            .unwrap_or(now + Duration::from_secs(u32::MAX.into()));
        let mut number = self.shared.deadlines_made.get().wrapping_add(1);
        if number == 0 || number == u32::MAX {
            number = 1;
        }
        self.shared.deadlines_made.set(number);
        let effective = self
            .shared
            .earliest
            .get()
            .map_or(own, |earliest| earliest.min(own));
        self.shared.deadlines.borrow_mut().push(Deadline {
            number,
            own,
            effective,
            message: Rc::from(message),
        });
        self.shared.earliest.set(Some(effective));
        number
    }

    /// Ends the innermost deadline, number `number`.
    pub(super) fn pop_deadline(&self, number: u32) {
        let mut deadlines = self.shared.deadlines.borrow_mut();
        if deadlines
            .last()
            .is_some_and(|deadline| deadline.number == number)
        {
            deadlines.pop();
        }
        self.shared
            .earliest
            .set(deadlines.last().map(|deadline| deadline.effective));
    }

    /// Whether a running deadline has passed: one look at a cell while none
    /// runs, and the clock's while one does.
    pub(super) fn past_deadline(&self) -> bool {
        self.shared
            .earliest
            .get()
            .is_some_and(|earliest| Instant::now() >= earliest)
    }

    /// The error of the passing of the outermost running deadline that has
    /// passed, at `node`, where the evaluation was when it passed: its
    /// message, marked with its number, so that only its call turns it into
    /// its own error and no call inside retries it.
    #[cold]
    #[inline(never)]
    pub(super) fn timed_out<T>(&self, node: &IrNode) -> Flow<T> {
        let now = Instant::now();
        let deadlines = self.shared.deadlines.borrow();
        let Some(passed) = deadlines.iter().find(|deadline| deadline.own <= now) else {
            return mistyped(node, "a deadline passed that no running call has");
        };
        let mut error = Diagnostic::error(&*passed.message, node.span);
        error.mark = ErrorMark::deadline(passed.number);
        Err(error.into())
    }
}
