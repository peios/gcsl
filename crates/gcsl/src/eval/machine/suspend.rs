//! Suspending the machine: a unit in graph mode that waits for a node's
//! result, or for another unit's run, stops where it is, and its next run
//! takes up there.
//!
//! When the machine stops for a [`Wait`], what the unit's evaluation holds
//! is taken out of what the run shares into a [`Suspended`], which the run
//! leaves to its host ([`GraphRun::suspend`]):
//!
//! - the machine's frames, activations and calls ([`Parked`]), and what to
//!   do first when it takes up again ([`Resume`]);
//! - the value stack above where the unit's own slots begin: every call's
//!   frame, and a prepared call's own top-level slots, which another
//!   prepared call's run would overwrite;
//! - its running memoised calls, whose entries stay running, owned by the
//!   unit, so that another unit's call of one waits for it;
//! - its context, deadlines, the failures its `recover`s caught, the
//!   identities its operations on secrets computed, and its foreign-call
//!   sites;
//! - where it is in its graph-mode scopes, and the node it met last;
//! - the calls the profile had running for it.
//!
//! The next run of the unit puts them all back, its frames rebased if its
//! slots begin elsewhere on the stack now, and carries on. Time spent
//! stopped is none of the unit's: its deadlines and its profiled calls are
//! moved on by it, so that `#[timeout]` measures running time, as with
//! restarts, and the profile counts no waiting.

use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ir::{Ir, IrId};
use crate::source::Span;
use crate::value::{Caller, ContextFrame, Future, Value};

use super::super::failure::Caught;
use super::super::graph::{GraphRun, NodeId, RunState, StepCall, Wait, pending};
use super::super::memo::{MemoTables, Running, RunningAt};
use super::super::profile;
use super::super::{Deadline, Flow, Shared, Unwind};
use crate::diagnostic::Mark;
use super::futures::AwaitState;
use super::{Activation, Frame, Machine, Next, Parked, Ran, fail};

/// What a stopped machine does first when it takes up again.
pub(in crate::eval) enum Resume {
    /// The await `state` of `future`, again: the future has run, or the
    /// node it stands for has its result, or not yet.
    Await(AwaitState, Rc<Future>),
    /// The application of a built-in, its arguments evaluated, again: its
    /// turn to make an effect may have come.
    Builtin(Box<super::builtins::BuiltinCall>),
    /// The deep await `state` of the value, again: the future of one of its
    /// fields has run, or the node it stands for has its result, or not
    /// yet.
    Deep(AwaitState, Value, Rc<Future>),
    /// The application of a node to its operands, again: the future met on
    /// its update's path has run, or the node it stands for has its result,
    /// or not yet.
    Apply(Box<super::data::OperandsState>, Rc<Future>),
    /// The outcome of the effect node `NodeId`, made by the call node
    /// `IrId`.
    Effect(IrId, NodeId),
    /// The lookup of the memoised call on top of the machine's calls,
    /// again, its arguments still in its frame; with the running entry it
    /// found, another task's or another unit's, if it found one.
    Lookup(Option<RunningAt>),
    /// The evaluation of the context read node, again: its turn to read a
    /// context the run watches may have come.
    Read(IrId),
}

/// A unit's evaluation, stopped where it waits (see the module
/// documentation).
pub(crate) struct Suspended {
    /// The machine's frames, activations and calls, and the unit's other
    /// tasks.
    parked: Parked,
    /// The value stack from where the unit's own slots begin.
    segment: Vec<Value>,
    /// Where on the stack `segment` began.
    base: usize,
    /// What the run shared that is the unit's own.
    task: Task,
    /// The unit's running memoised calls, outermost first.
    running: Vec<Running>,
    /// Where it is in its graph-mode scopes.
    run: RunState,
    /// The calls the profile had running for it.
    profile: Option<profile::Suspended>,
    /// When it stopped.
    since: Instant,
}

/// Releases what the unit `unit`, which ended or was given up without
/// ending them, was running: its memoised calls' entries go, as a failed
/// call's do, and its `lazy`s are left unrun, so that a unit waiting for
/// one runs it itself (see [Across
/// units](super::tasks#across-units)). `parked` holds its frames.
fn release(parked: &mut Parked, memo: &mut MemoTables, unit: u32) {
    super::tasks::release_futures(parked, memo, unit);
    memo.release(unit);
}

/// Gives up the unit `unit`, whose evaluation is `suspended`: what it was
/// running is released (see [`release`]).
pub(crate) fn give_up(suspended: Suspended, memo: &mut MemoTables, unit: u32) {
    let mut parked = suspended.parked;
    release(&mut parked, memo, unit);
}

impl Suspended {
    /// What its tasks are: the machine's frames, activations and calls,
    /// the root's place and status and the other tasks, and the root's
    /// running memoised calls and `lazy`s.
    pub(super) fn parts(&self) -> (&Parked, &[Running], &[Rc<Future>]) {
        (&self.parked, &self.running, &self.task.futures)
    }
}

impl fmt::Debug for Suspended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Suspended")
            .field("frames", &self.parked.frames.len())
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// What a run's [`Shared`] holds that is its unit's own.
struct Task {
    /// The context it is evaluated in.
    context: Option<Rc<ContextFrame>>,
    /// The deadlines of its calls with `#[timeout]` running.
    deadlines: Vec<Deadline>,
    /// The number of the last deadline made.
    made: u32,
    /// The failures its `recover`s running their fallbacks caught.
    failures: Vec<Rc<Caught>>,
    /// The identities its operations on secrets have computed on.
    identities: Vec<Rc<str>>,
    /// Whether the failure unwinding to its innermost operation on secrets
    /// is an operand's.
    operand_failed: bool,
    /// Its calls from the workspace's own code into foreign code.
    sites: Vec<(u32, Span)>,
    /// The `lazy`s it is running.
    futures: Vec<Rc<crate::value::Future>>,
}

/// Where a run of the machine started: the stack's length from which the
/// unit's own slots and frames begin, and how many memoised calls and
/// profiled calls were running then.
#[derive(Clone, Copy)]
struct Start {
    /// The stack's length.
    base: usize,
    /// The memoised calls running.
    memo: usize,
    /// The profiled calls running.
    profiled: usize,
}

/// What a run of the machine starts from, when it takes up nothing
/// suspended.
#[derive(Clone, Copy)]
pub(in crate::eval) enum Entry<'a> {
    /// The root of the code, run with this caller.
    Root(&'a Rc<Ir>, Caller),
    /// The body of a step, run as a unit of its own (see
    /// [`run_step_call`](super::super::graph::run_step_call)).
    Step(&'a StepCall),
}

/// Evaluates `entry`, the root of code or a step's body, on the machine, over `stack`
/// and `memo`, sharing `shared`, as [`crate::eval::eval_rooted`] does in
/// graph mode: taking up the unit's suspended evaluation, if its run has
/// one, or else from the root; and suspending it, and stopping with the
/// error marked [`Mark::PENDING`](crate::diagnostic::Mark::PENDING), if it
/// stops for something it waits for. `own` is where the unit's own
/// top-level slots begin, for a prepared call's, whose slots others share;
/// a file's own slots are its cells, which no other run writes.
pub(in crate::eval) fn run_machine(
    entry: Entry<'_>,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    shared: &Shared<'_>,
    own: Option<usize>,
) -> Flow<Value> {
    let graph = shared.graph;
    let resumed = graph.and_then(GraphRun::take_suspended);
    let (start, ran, parked) = if let Some(suspended) = resumed {
        let (start, parked) = take_up(suspended, (stack, memo), shared, own);
        // Its profiled calls are back above where its run started.
        let mut machine = Machine::parked((stack, memo), shared, parked, start.profiled);
        let next = machine.take_up_tasks();
        let ran = machine.run_from(next);
        (start, ran, machine.into_parked())
    } else {
        let start = Start {
            base: own.map_or(stack.len(), |own| own.min(stack.len())),
            memo: memo.running.len(),
            profiled: shared.profile.map_or(0, profile::Profile::running),
        };
        let (ran, parked) = match entry {
            Entry::Root(ir, caller) => {
                let mut machine = Machine::new((stack, memo), shared, Activation::top(ir, caller));
                let ran = machine.run(ir.root());
                (ran, machine.into_parked())
            }
            Entry::Step(call) => Machine::run_step(call, (stack, memo), shared),
        };
        (start, ran, parked)
    };
    let span = match entry {
        Entry::Root(ir, _) => ir.get(ir.root()).span,
        Entry::Step(call) => call.span(),
    };
    match (ran, graph) {
        // Graph mode stops only on the machine, never in a walker, which
        // could only restart the unit.
        (Ran::Done(Err(Unwind::Error(error))), Some(_)) if error.mark == Mark::PENDING => Err(
            super::super::internal("a unit stopped where the machine could not", error.span).into(),
        ),
        // A failure that left frames, an internal error, did not unwind
        // them: what the unit was running, which another unit may wait
        // for, is released.
        (Ran::Done(flow), Some(run)) if flow.is_err() && !parked.is_empty() => {
            let mut parked = parked;
            release(&mut parked, memo, run.unit());
            memo.running.truncate(start.memo.min(memo.running.len()));
            flow
        }
        (Ran::Done(flow), _) => flow,
        (Ran::Blocked(waits), Some(run)) => {
            let suspended = set_aside(parked, start, (stack, memo), shared);
            run.suspend(suspended);
            for wait in waits {
                run.stopped(wait);
            }
            Err(pending(span).into())
        }
        (Ran::Blocked(..), None) => {
            Err(super::super::internal("a plain run stopped to wait", span).into())
        }
    }
}

/// Takes what the machine stopped with out of `stack`, `memo` and `shared`
/// into the unit's [`Suspended`], `start` being where its run started.
fn set_aside(
    parked: Parked,
    start: Start,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    shared: &Shared<'_>,
) -> Suspended {
    let segment = stack.split_off(start.base.min(stack.len()));
    let running = memo.running.split_off(start.memo.min(memo.running.len()));
    let profile = shared
        .profile
        .map(|profile| profile.suspend(start.profiled));
    let task = Task {
        context: shared.context.take(),
        deadlines: shared.deadlines.take(),
        made: shared.deadlines_made.get(),
        failures: shared.failures.take(),
        identities: shared.identities.take(),
        operand_failed: shared.operand_failed.get(),
        sites: shared.sites.take(),
        futures: shared.futures.take(),
    };
    shared.earliest.set(None);
    let run = shared
        .graph
        .map_or_else(RunState::default, GraphRun::take_state);
    Suspended {
        parked,
        segment,
        base: start.base,
        task,
        running,
        run,
        profile,
        since: Instant::now(),
    }
}

/// Puts `suspended` back into `stack`, `memo` and `shared`, its frames
/// rebased if its slots begin elsewhere now, and returns where this run
/// started and its machine's parts. `own` is where the
/// unit's own top-level slots begin, if it is a prepared call.
fn take_up(
    suspended: Suspended,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    shared: &Shared<'_>,
    own: Option<usize>,
) -> (Start, Parked) {
    let Suspended {
        mut parked,
        segment,
        base,
        task,
        running,
        run,
        profile,
        since,
    } = suspended;
    let blocked = since.elapsed();
    // A prepared call's own slots are its alone while it runs: whatever
    // another left there goes. A file's slots are followed by the cells of
    // files that ran since, which stay.
    let at = match own {
        Some(own) => {
            stack.truncate(own);
            own
        }
        None => stack.len().max(base),
    };
    stack.resize(at, Value::Empty);
    stack.extend(segment);
    if at != base {
        parked.rebase(base, at.wrapping_sub(base));
    }
    let start = Start {
        base: at,
        memo: memo.running.len(),
        profiled: shared.profile.map_or(0, profile::Profile::running),
    };
    memo.running.extend(running);
    if let (Some(profile), Some(suspended)) = (shared.profile, profile) {
        profile.resume(suspended, blocked);
    }
    restore_task(task, shared, blocked);
    if let Some(graph) = shared.graph {
        graph.restore_state(run);
    }
    (start, parked)
}

/// Puts `task` back into `shared`, its deadlines moved on by `blocked`,
/// the time its unit was stopped.
fn restore_task(task: Task, shared: &Shared<'_>, blocked: Duration) {
    let mut deadlines = task.deadlines;
    for deadline in &mut deadlines {
        deadline.own = deadline.own.checked_add(blocked).unwrap_or(deadline.own);
        deadline.effective = deadline
            .effective
            .checked_add(blocked)
            .unwrap_or(deadline.effective);
    }
    shared
        .earliest
        .set(deadlines.last().map(|deadline| deadline.effective));
    shared.deadlines.replace(deadlines);
    shared.deadlines_made.set(task.made);
    shared.context.replace(task.context);
    shared.failures.replace(task.failures);
    shared.identities.replace(task.identities);
    shared.operand_failed.set(task.operand_failed);
    shared.sites.replace(task.sites);
    shared.futures.replace(task.futures);
}

impl Parked {
    /// Moves every frame from `from` on, those set aside with the unit, by
    /// `delta` slots up the stack, wrapping, so that a negative move is a
    /// wrapped subtraction; the top level's frame, below, stays.
    fn rebase(&mut self, from: usize, delta: usize) {
        for activation in &mut self.activations {
            if activation.base >= from {
                activation.base = activation.base.wrapping_add(delta);
            }
        }
        for call in &mut self.calls {
            call.base = call.base.wrapping_add(delta);
            if let Some(marks) = call.pieces.begun.marks_mut() {
                marks.rebase(delta);
            }
        }
        for frame in &mut self.frames {
            match frame {
                Frame::Future(run) => run.rebase(delta),
                Frame::Default(run) => run.rebase(delta),
                Frame::ArgDefault(run) => run.base = run.base.wrapping_add(delta),
                frame => frame.rebase_marks(delta),
            }
        }
    }
}

impl Resume {
    /// Where the await it takes up again is, if it is one's.
    pub(super) fn await_span(&self) -> Option<Span> {
        match self {
            Self::Await(state, _) | Self::Deep(state, ..) => Some(state.span()),
            _ => None,
        }
    }

    /// The run it waits for, if it looks a memoised call up or awaits a
    /// future again (see [Across units](super::tasks#across-units)).
    pub(super) fn waits_for(&self) -> Option<super::tasks::Run> {
        match self {
            Self::Await(_, future) | Self::Deep(_, _, future) | Self::Apply(_, future) => {
                Some(super::tasks::Run::Future(Rc::clone(future)))
            }
            Self::Lookup(at) => at.map(super::tasks::Run::Entry),
            Self::Builtin(_) | Self::Effect(..) | Self::Read(_) => None,
        }
    }
}

impl Machine<'_> {
    /// What the machine does first when it takes up after `resume`.
    pub(super) fn resume_next(&mut self, resume: Resume) -> Next {
        match resume {
            Resume::Await(state, future) => self.await_fully(state, Value::Future(future)),
            Resume::Deep(state, value, _) => self.await_deep(state, value),
            Resume::Builtin(call) => self.builtin_apply(call),
            Resume::Apply(operands, _) => self.apply_again(operands),
            Resume::Effect(node, id) => {
                let outcome = self.walker(|walker| walker.node_outcome(id, walker.ir.get(node)));
                match outcome {
                    Ok(Some(value)) => Next::Give(Ok(value)),
                    Ok(None) => Next::Block(Wait::Node(id), Resume::Effect(node, id)),
                    Err(unwind) => fail(unwind),
                }
            }
            Resume::Lookup(_) => match self.calls.pop() {
                Some(state) => self.enter_body(state),
                None => self.lost_call(),
            },
            Resume::Read(id) => Next::Eval(id),
        }
    }
}
