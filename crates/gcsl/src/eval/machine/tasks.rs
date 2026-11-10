//! Tasks: branches of one unit's evaluation that wait apart (pekit2's
//! `design/suspension.md`, "Forking").
//!
//! # Tasks
//!
//! A unit's evaluation starts as one task, its root. When a task would
//! wait, for a node, another unit, its turn, a sibling or its own
//! children, the machine looks for the innermost iteration of a forkable
//! `for` (see [`crate::ir::IrKind::For`]) or piece of a list,
//! interpolation or call (see [Pieces](super::pieces#pieces)) on its
//! frames, inside a function's body; if it finds one, it splits the task
//! there: what the iteration or piece was doing becomes a *child* task,
//! which waits, and the task goes on with the next. So a loop of a hundred
//! fetches meets all of them before the host is asked for any. What was
//! split off from the loops, nodes and calls the child takes with it is
//! the child's to wait for.
//!
//! The task running has its frames, activations and calls in the
//! machine's fields, its value stack in the run's, and what the run shares
//! that is a task's own in the run's [`Shared`], memo tables, graph run and
//! profile: its context, deadlines, failures caught, secrets' identities
//! and foreign-call sites; its running memoised calls; its graph-mode
//! scopes; and its profiled calls ([`Own`]). Every other task keeps them in
//! its [`Task`], and a switch exchanges the two. A child's value stack is a
//! copy of its parent's up to where the iteration began, with the frames
//! above it moved, so that every frame's base is where it was. Each
//! iteration of a forkable `for` that can be split begins a graph-mode
//! scope of its own, by its number, so that the nodes it meets are named
//! the same whether it was split or not.
//!
//! A child ends at its bottom frame, which delivers what it came to to the
//! loop's [`Join`]. The loop, once it has no elements left, waits for its
//! children, and then takes their outcomes in iteration order, as a loop
//! run in order would: the first failure, or else every value collected in
//! its place. A failure, a child's or the loop's own, cancels every
//! later iteration still running: a cancelled task unwinds every frame with
//! an error that nothing catches, so that every frame's cleanup runs.
//!
//! **Program order.** A task's *path* is where its work is in the program:
//! its base, the path its parent was at when it split it off, and then the
//! iteration or piece that each loop whose iterations may be split off,
//! and each list, interpolation, call and block, on its frames is at,
//! whether or not anything was split off from it, so that paths taken at
//! different times compare as the program orders them; a loop's guard is at
//! its iteration, though it is never split off, and one waiting for its
//! children is past every iteration or piece. An effect
//! the host does not isolate ([`crate::eval::Effects::isolates`]) is made
//! only when no live task's path is earlier, so that such effects reach the
//! host in program order; and never once an iteration or piece split off
//! from one the task is in has failed, since a run in order would have
//! stopped there.
//!
//! A task that needs a memoised call or a `lazy` another task of the unit
//! is running waits for it; one that waits for its turn or for a sibling
//! is woken whenever a task ends, and one that waits for a sibling also
//! whenever a demanded run ends (see [Across units](#across-units)).
//!
//! # Wound-wait
//!
//! Tasks that wait for each other's memoised calls or `lazy`s can wait in
//! a circle that a run in order never meets: a later task begins a call
//! that an earlier one needs, and then, inside it, waits for its turn
//! behind the earlier task. So a task never waits for a later one's run:
//! it *wounds* it ([`Machine::wound`]). A task is later, for this, if it
//! began the call or `lazy` later, however far into it it has gone since.
//! The later task is cancelled as far
//! as the call or `lazy` it was running ([`Meta::wound`]), whose frame
//! catches the cancellation: the entry goes, or the `lazy` is left unrun,
//! every task that waits for a sibling's call or `lazy` is woken, and the
//! later one waits to look it up or await it again. Of the tasks woken to
//! look again, the earliest in program order runs first, so that the
//! earliest task that needs the call begins it again: were a later one to,
//! the earlier would only wound it again, and so on without end. That task
//! then runs it as a run in order would: it finishes it, or, if it truly
//! depends on itself, fails where a run in order fails. Waits then only
//! ever go from a later task to an earlier one, and cannot form a circle. A
//! wounded task's work inside the call is lost, as are its children's; an
//! effect the host does not isolate that it was waiting to make is never
//! made. A call abandoned far more often than this allows is reported as
//! an internal error ([`Machine::abandoned_too_often`]).
//!
//! # Across units
//!
//! Units in graph mode share their memoised calls and `lazy`s: a unit
//! that finds another unit's run of one waits for that unit
//! ([`Wait::Unit`]). Wound-wait keeps the waits within a unit from forming
//! a circle, but a unit cannot wound another's tasks, and units have no
//! common program order. A task that holds a run, and inside it waits its
//! turn behind earlier work of its unit outside it, could close a circle
//! through another unit that the earlier work waits for, and that needs
//! the run.
//!
//! So a run another unit waits for is *demanded*: the first such wait
//! marks it ([`MemoTables::demanded`](crate::eval::MemoTables::demanded)).
//! An effect the host does not isolate, or a step's context read, made
//! inside a demanded run waits only for the earlier tasks of its unit that
//! hold every demanded run the task holds: work inside the run keeps its
//! order, and work outside it no longer comes first. A task inside the
//! innermost demanded run holds every run around it, since a child takes
//! its parent's runs. Effects a host does not isolate are still made one
//! at a time: none is made while another task of the unit waits for one's
//! result. Nothing is abandoned, so a demanded run still runs once, and its
//! effects with it, but they reach the host out of its unit's program
//! order. A run of shared work is no longer ordered by the program of any
//! one unit that needs it: which unit ran it first was chance already.
//!
//! A demanded run is never wounded: its effects, made out of order, would
//! be made again. A task that needs it waits for it instead, woken when it
//! ends.
//!
//! The demand passes on within the unit ([`Machine::demand_waited`]): a
//! task holding a demanded run that waits for a run another task of the
//! unit holds marks that run demanded too, and so on, as the task parks
//! and as the unit is taken up. Otherwise the other task, inside the run
//! waited for, would still wait its turn behind work outside the first
//! run, which may wait for the other unit.
//!
//! With these rules, a circle of waits through another unit has to go
//! through a run inside itself, as each turn waited for stays inside the
//! demanded run it began in: the call depends on its own result. Within a
//! unit, a circle that wound-wait left alone, because a run in it is
//! demanded, is that too: once every task waits, and none for a node or
//! another unit, the task running a demanded run that an earlier task
//! waits for is wounded after all ([`Machine::unstick`]), and the earlier
//! task fails where a run in order fails.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::diagnostic::Mark;
use crate::source::Span;
use crate::value::{ContextFrame, List, Value};

use super::super::failure::Caught;
use super::super::graph::{GraphRun, NodeId, RunState, Wait};
use super::super::memo::{Running, RunningAt};
use super::super::profile;
use super::super::{Deadline, Unwind, internal};
use super::loops::ForState;
use super::suspend::Resume;
use super::{Activation, CallState, Frame, Machine, Next, Outcome};

/// A task's place in the unit (see [Tasks](#tasks)).
#[derive(Clone, Debug, Default)]
pub(in crate::eval) struct Meta {
    /// Its number, the root's 0.
    pub(super) id: u32,
    /// What it is to abandon its run of, once a task has wounded it (see
    /// [Wound-wait](#wound-wait)).
    pub(super) wound: Option<Wound>,
    /// The path its parent was at when it split it off.
    base: Vec<u32>,
}

/// What a wounded task abandons its run of (see [Wound-wait](#wound-wait)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::eval) enum Wound {
    /// A memoised call, by its entry's identity.
    Entry(u64),
    /// A `lazy`'s body.
    Future(*const crate::value::Future),
}

/// What a run abandoned was of, as [`Machine::abandoned_too_often`] counts
/// them: a memoised call by its function and the hash of its arguments,
/// since its entry is new each time it is begun, or a `lazy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::eval) enum Abandoned {
    /// A memoised call of the function of the IR, its arguments' hash.
    Call(*const crate::ir::Ir, crate::ast::FnId, u64),
    /// A `lazy`'s body.
    Future(*const crate::value::Future),
}

/// A run of a memoised call or of a `lazy`'s body that a task waits for:
/// where the call's entry is, or the future (see [Across
/// units](#across-units)).
pub(in crate::eval) enum Run {
    /// A memoised call's.
    Entry(RunningAt),
    /// A `lazy`'s body.
    Future(Rc<crate::value::Future>),
}

/// A run of a memoised call, by its entry's identity, or of a `lazy`'s
/// body, that a task holds (see [Across units](#across-units)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    /// A memoised call, by its entry's identity.
    Entry(u64),
    /// A `lazy`'s body.
    Future(*const crate::value::Future),
}

/// What the run shares that is a task's own (see [Tasks](#tasks)).
pub(in crate::eval) struct Own {
    /// Its context.
    context: Option<Rc<ContextFrame>>,
    /// Its deadlines.
    deadlines: Vec<Deadline>,
    /// The failures its `recover`s caught.
    failures: Vec<Rc<Caught>>,
    /// Its secrets' identities.
    identities: Vec<Rc<str>>,
    /// Whether its innermost operation on secrets has an operand's failure.
    operand_failed: bool,
    /// Its foreign-call sites.
    sites: Vec<(u32, Span)>,
    /// Its running memoised calls, its parent's at the split among them.
    running: Vec<Running>,
    /// The `lazy`s it is running, its parent's at the split among them.
    futures: Vec<Rc<crate::value::Future>>,
    /// Its graph-mode scopes.
    run: RunState,
    /// Its profiled calls.
    profile: Option<profile::Suspended>,
    /// When it stopped running.
    since: Instant,
}

/// What a task that is not running is doing.
pub(in crate::eval) enum Status {
    /// It may run, doing this next.
    Ready(Next),
    /// It may run, taking up after this.
    Wake(Resume),
    /// It waits for a node's result or another unit's run, to take up after
    /// the resume.
    Blocked(Wait, Resume),
    /// Its loop waits for its children.
    Joining,
    /// It waits for its turn to make an effect or read a context, to take
    /// up after the resume.
    Waiting(Resume),
    /// It waits for a sibling's memoised call or `lazy`, to take up after
    /// the resume, which looks it up or awaits it again.
    Sibling(Resume),
    /// It may run, to look up a sibling's memoised call or await its `lazy`
    /// again, taking up after the resume: the earliest such task in program
    /// order runs first (see [Wound-wait](#wound-wait)).
    Again(Resume),
    /// It is to be cancelled; `true` if a call of it, on top of its calls
    /// with no frame of its own, waited to be looked up again.
    Cancel(bool),
}

/// A task that is not running (see [Tasks](#tasks)).
pub(in crate::eval) struct Task {
    /// Its place.
    pub(super) meta: Meta,
    /// Its frames.
    frames: Vec<Frame>,
    /// Its activations.
    activations: Vec<Activation>,
    /// Its calls.
    calls: Vec<CallState>,
    /// Its value stack.
    stack: Vec<Value>,
    /// What the run shares that is its own.
    own: Own,
    /// What it is doing.
    pub(super) status: Status,
}

/// Where an iteration of a forkable `for`, or a piece (see
/// [Pieces](super::pieces#pieces)), began: what the task had then, to
/// which a split takes it back (see [Tasks](#tasks)).
#[derive(Clone)]
pub(in crate::eval) struct Marks {
    /// Its frames, the iteration's own above them.
    frames: usize,
    /// Its activations.
    activations: usize,
    /// Its calls.
    calls: usize,
    /// Its value stack's length.
    stack: usize,
    /// Its running memoised calls.
    running: usize,
    /// The `lazy`s it is running.
    futures: usize,
    /// Its profiled calls.
    profiled: usize,
    /// Its failures caught, identities, sites and deadlines.
    lens: [usize; 4],
    /// Its graph-mode scopes.
    scopes: usize,
    /// Its context.
    context: Option<Rc<ContextFrame>>,
    /// The node it met last.
    last: Option<NodeId>,
}

/// The iterations of a `for` split off into children (see
/// [Tasks](#tasks)).
#[derive(Debug, Default)]
pub(in crate::eval) struct Join {
    /// Each child's iteration, the place of its value among the loop's
    /// values, its task, and what it came to once it has.
    pub(super) children: Vec<(usize, usize, u32, Option<Outcome>)>,
    /// How many children have yet to end.
    pub(super) live: usize,
    /// The task whose loop it is.
    pub(super) parent: u32,
    /// The first iteration that failed, if one has.
    pub(super) failed: Option<usize>,
    /// For each child that is a block's item, by its number, the slots it
    /// declares, of the frame the block runs in, whose values it comes to
    /// (see [Pieces](super::pieces#pieces)): by their indices in that
    /// frame, so that they hold however its unit's frames are moved.
    pub(super) slots: Vec<(usize, Vec<usize>)>,
}

impl Join {
    /// A join for the loop of the task `parent`.
    pub(super) fn new(parent: u32) -> Self {
        Self {
            parent,
            ..Self::default()
        }
    }
}

impl Marks {
    /// Moves where it began by `delta` slots up the stack, wrapping, as
    /// its unit's frames move.
    pub(super) const fn rebase(&mut self, delta: usize) {
        self.stack = self.stack.wrapping_add(delta);
    }
}

/// Where the iteration or piece running began, if it may be split off:
/// its [`Marks`], in a box kept for the next one, so that a loop or node
/// allocates once however many of its iterations or pieces are marked,
/// and what holds it stays small.
#[derive(Clone, Default)]
pub(in crate::eval) struct Begun {
    /// Whether the one running is marked.
    marked: bool,
    /// Its marks, if it is; an earlier one's, if not.
    marks: Option<Box<Marks>>,
}

impl Begun {
    /// Marks the one running as beginning at `marks`.
    pub(super) fn mark(&mut self, marks: Marks) {
        match &mut self.marks {
            Some(kept) => **kept = marks,
            None => self.marks = Some(Box::new(marks)),
        }
        self.marked = true;
    }

    /// Whether the one running is marked.
    pub(super) const fn is_marked(&self) -> bool {
        self.marked
    }

    /// Ends the one running: whether it was marked.
    pub(super) const fn end(&mut self) -> bool {
        std::mem::replace(&mut self.marked, false)
    }

    /// The marks of the one running, taken, for a split; it is no longer
    /// marked.
    pub(super) fn take(&mut self) -> Option<Box<Marks>> {
        if self.end() { self.marks.take() } else { None }
    }

    /// The marks of the one running, if it is marked.
    pub(super) fn marks_mut(&mut self) -> Option<&mut Marks> {
        if self.marked {
            self.marks.as_deref_mut()
        } else {
            None
        }
    }
}

/// The internal error of a memoised call or `lazy` whose run the tasks of
/// a unit abandon to each other far more often than wound-wait allows
/// (see [`Machine::abandoned_too_often`]).
pub(super) fn handed_back(span: Span) -> Unwind {
    Unwind::Error(internal(
        "the tasks of a unit abandon a call to each other without end",
        span,
    ))
}

/// The error a cancelled task unwinds with, which nothing catches.
pub(super) fn cancelled(span: Span) -> Unwind {
    let mut error = internal("a cancelled iteration", span);
    error.mark = Mark::CANCELLED;
    Unwind::Error(error)
}

impl Machine<'_> {
    /// What the task running has of what the run shares, taken out.
    fn take_own(&mut self) -> Own {
        let shared = self.shared;
        Own {
            context: shared.context.take(),
            deadlines: shared.deadlines.take(),
            failures: shared.failures.take(),
            identities: shared.identities.take(),
            operand_failed: shared.operand_failed.replace(false),
            sites: shared.sites.take(),
            running: std::mem::take(&mut self.memo.running),
            futures: shared.futures.take(),
            run: shared
                .graph
                .map_or_else(RunState::default, GraphRun::take_task_state),
            profile: shared
                .profile
                .map(|profile| profile.suspend(self.profiled_base)),
            since: Instant::now(),
        }
    }

    /// Puts `own` into what the run shares, for the task now running, its
    /// deadlines and profiled calls moved on by the time it was stopped.
    fn put_own(&mut self, own: Own) {
        let stopped = own.since.elapsed();
        let shared = self.shared;
        let mut deadlines = own.deadlines;
        for deadline in &mut deadlines {
            deadline.own = deadline.own.checked_add(stopped).unwrap_or(deadline.own);
            deadline.effective = deadline
                .effective
                .checked_add(stopped)
                .unwrap_or(deadline.effective);
        }
        shared
            .earliest
            .set(deadlines.last().map(|deadline| deadline.effective));
        shared.deadlines.replace(deadlines);
        shared.context.replace(own.context);
        shared.failures.replace(own.failures);
        shared.identities.replace(own.identities);
        shared.operand_failed.set(own.operand_failed);
        shared.sites.replace(own.sites);
        self.memo.running = own.running;
        shared.futures.replace(own.futures);
        if let Some(graph) = shared.graph {
            graph.restore_state(own.run);
        }
        if let (Some(profile), Some(suspended)) = (shared.profile, own.profile) {
            profile.resume(suspended, stopped);
        }
    }

    /// Exchanges the task running for `task`, which runs now; the one that
    /// ran is returned in its place, with `task`'s status.
    fn exchange(&mut self, mut task: Task) -> Task {
        std::mem::swap(self.stack, &mut task.stack);
        std::mem::swap(&mut self.frames, &mut task.frames);
        std::mem::swap(&mut self.activations, &mut task.activations);
        std::mem::swap(&mut self.calls, &mut task.calls);
        std::mem::swap(&mut self.meta, &mut task.meta);
        let mine = self.take_own();
        let theirs = std::mem::replace(&mut task.own, mine);
        self.put_own(theirs);
        task
    }

    /// Sets the task running aside with `status`, and runs another that may
    /// run: what it does next, or `None` if every task waits, the one that
    /// ran left running with `status`.
    pub(super) fn park(&mut self, status: Status) -> Option<Next> {
        self.pause_profile();
        let Some(ready) = self.next_ready() else {
            self.status = Some(status);
            self.unpause_profile();
            return None;
        };
        let mut task = self.tasks.swap_remove(ready);
        let start = std::mem::replace(&mut task.status, status);
        let parked = self.exchange(task);
        self.tasks.push(parked);
        self.status = None;
        self.unpause_profile();
        Some(self.start(start))
    }

    /// Notes, if the run is profiled, that the machine begins work of its
    /// own, which no call's time counts (see
    /// [Profiling](crate::eval::profile#profiling), "Tasks").
    pub(super) fn pause_profile(&self) {
        if let Some(profile) = self.shared.profile {
            profile.pause();
        }
    }

    /// Notes, if the run is profiled, that the machine's own work is done.
    pub(super) fn unpause_profile(&self) {
        if let Some(profile) = self.shared.profile {
            profile.unpause();
        }
    }

    /// What the task now running, which was set aside with `status`, does
    /// first.
    pub(super) fn start(&mut self, status: Status) -> Next {
        match status {
            Status::Ready(next) => next,
            Status::Wake(resume) | Status::Again(resume) => self.resume_next(resume),
            Status::Cancel(looked_up) => {
                let span = self.running().ir.get(self.running().ir.root()).span;
                if looked_up {
                    // The call that waited fails, its frame going.
                    return match self.calls.pop() {
                        Some(state) => self.argument_failed(&state, cancelled(span)),
                        None => self.lost_call(),
                    };
                }
                Next::Give(Err(Box::new(cancelled(span))))
            }
            Status::Blocked(..) | Status::Joining | Status::Waiting(_) | Status::Sibling(_) => {
                let span = self.running().ir.get(self.running().ir.root()).span;
                Next::Give(Err(Box::new(internal("a waiting task ran", span).into())))
            }
        }
    }

    /// Makes the root task the one running, the others set aside as they
    /// are, so that the unit's evaluation can be set aside whole.
    pub(super) fn root_running(&mut self) {
        if self.meta.id == 0 {
            return;
        }
        let Some(root) = self.tasks.iter().position(|task| task.meta.id == 0) else {
            return;
        };
        let mut task = self.tasks.swap_remove(root);
        let status = self.status.take();
        let root_status = std::mem::replace(&mut task.status, Status::Joining);
        let mut parked = self.exchange(task);
        if let Some(status) = status {
            parked.status = status;
        }
        self.tasks.push(parked);
        self.status = Some(root_status);
    }

    /// What the unit waits for, every task waiting: the nodes and units its
    /// tasks wait for.
    pub(super) fn waits(&self) -> Vec<Wait> {
        let mut waits: Vec<Wait> = self
            .tasks
            .iter()
            .map(|task| &task.status)
            .chain(self.status.as_ref())
            .filter_map(|status| match status {
                Status::Blocked(wait, _) => Some(*wait),
                _ => None,
            })
            .collect();
        waits.sort_by_key(|wait| match *wait {
            Wait::Node(id) => (0, id.number()),
            Wait::Unit(unit) => (1, unit),
        });
        waits.dedup();
        waits
    }

    /// Wakes every task that waits for its turn or for a sibling: a task
    /// has ended, or the unit runs again.
    pub(super) fn wake_waiting(&mut self, blocked_too: bool) {
        for task in &mut self.tasks {
            let woken = match std::mem::replace(&mut task.status, Status::Joining) {
                Status::Waiting(resume) => Status::Wake(resume),
                Status::Sibling(resume) => Status::Again(resume),
                Status::Blocked(_, resume) if blocked_too => Status::Wake(resume),
                status => status,
            };
            task.status = woken;
        }
    }

    /// Wakes every task, as the unit runs again, but those still waiting for
    /// a node whose result the host has yet to give, which would only stop
    /// again.
    fn wake_given(&mut self) {
        let graph = self.shared.graph;
        let waiting = |id: NodeId| graph.is_some_and(|graph| graph.is_waiting(id));
        for task in &mut self.tasks {
            let woken = match std::mem::replace(&mut task.status, Status::Joining) {
                Status::Blocked(Wait::Node(id), resume) if waiting(id) => {
                    Status::Blocked(Wait::Node(id), resume)
                }
                Status::Waiting(resume) | Status::Blocked(_, resume) => Status::Wake(resume),
                Status::Sibling(resume) => Status::Again(resume),
                status => status,
            };
            task.status = woken;
        }
    }

    /// Ends the task running, which delivered what it came to: another that
    /// may run runs, or `None` if every task waits.
    pub(super) fn finished(&mut self) -> Option<Next> {
        self.pause_profile();
        let next = self.next_after_end();
        self.unpause_profile();
        Some(self.start(next?))
    }

    /// The status of the task that runs once the task running has ended,
    /// made the task running; or `None` if every other task waits.
    fn next_after_end(&mut self) -> Option<Status> {
        self.wake_waiting(false);
        let Some(ready) = self.next_ready() else {
            // Every other task waits: the one that ended is not set aside,
            // the root's frames being run next by whoever wakes it.
            let root = self.tasks.iter().position(|task| task.meta.id == 0)?;
            let mut task = self.tasks.swap_remove(root);
            let status = std::mem::replace(&mut task.status, Status::Joining);
            let _ended = self.exchange(task);
            self.status = Some(status);
            return None;
        };
        let task = self.tasks.swap_remove(ready);
        let mut task = task;
        let start = std::mem::replace(&mut task.status, Status::Joining);
        let _ended = self.exchange(task);
        self.status = None;
        Some(start)
    }

    /// Delivers `result`, what the child running came to, to `join` as its
    /// child number `at`'s: a failure cancels every later child, and the
    /// last child to end wakes the loop. The child ends.
    pub(super) fn deliver(
        &mut self,
        join: &Rc<RefCell<Join>>,
        at: usize,
        result: Outcome,
    ) -> Option<Next> {
        let mut joined = join.borrow_mut();
        // A block's item comes to the values of the slots it declares.
        let result = match joined.slots.iter().find(|(number, _)| *number == at) {
            Some((_, slots)) if result.is_ok() => Ok(Value::List(List::from_vec(
                slots
                    .iter()
                    .map(|&slot| {
                        let at = self.running().base + slot;
                        self.stack.get(at).cloned().unwrap_or(Value::Empty)
                    })
                    .collect(),
            ))),
            _ => result,
        };
        let iteration = joined.children.get(at).map_or(0, |child| child.0);
        let cancelled_too = matches!(&result, Err(unwind)
            if matches!(&**unwind, Unwind::Error(error) if error.mark == Mark::CANCELLED));
        if let Err(unwind) = &result
            && !cancelled_too
            && !matches!(&**unwind, Unwind::Stop(_))
            && joined.failed.is_none_or(|failed| iteration < failed)
        {
            joined.failed = Some(iteration);
        }
        if let Some(child) = joined.children.get_mut(at) {
            child.3 = Some(result);
        }
        joined.live = joined.live.saturating_sub(1);
        let failed = joined.failed;
        let parent = joined.parent;
        let woken = (joined.live == 0).then_some(parent);
        let later: Vec<u32> = failed.map_or_else(Vec::new, |failed| {
            joined
                .children
                .iter()
                .filter(|child| child.0 > failed && child.3.is_none())
                .map(|child| child.2)
                .collect()
        });
        drop(joined);
        self.cancel(&later);
        // What the task that split it off does now is after the failure:
        // it is cancelled as far as the loop, node or call, which then
        // reports the failure.
        if failed.is_some() {
            self.halt(parent, join);
        }
        if let Some(parent) = woken {
            self.wake(parent, join);
        }
        self.finished()
    }

    /// Wounds the task that runs `target`, if it is later in program order
    /// than the task running, which needs it: it abandons its run, so that
    /// the task running can run it, as a run in order would (see
    /// [Wound-wait](#wound-wait)). Where a task is in program order is where
    /// it began the call or `lazy`, where it will be once it has abandoned
    /// it, not how far into it it has gone: one that began it earlier
    /// would begin it again first, and be wounded again, without end.
    pub(super) fn wound(&mut self, target: Wound) {
        let mine = self.path();
        for task in &mut self.tasks {
            let site = site_of(target, (&task.meta, &task.frames, &task.calls));
            if task.meta.wound.is_none()
                && !matches!(task.status, Status::Cancel(_))
                && site.is_some_and(|site| site > mine)
            {
                task.meta.wound = Some(target);
                task.status = Status::Cancel(looks_up(&task.status));
            }
        }
    }

    /// Cancels the task `id`, set aside, unless it waits for the children
    /// of `join`, whose last child to end wakes it: one of its children
    /// failed, and what it does now comes after the failure in program
    /// order. Its cancellation stops at the loop, node or call that `join`
    /// is of, which waits for its children and reports the failure.
    fn halt(&mut self, id: u32, join: &Rc<RefCell<Join>>) {
        for task in &mut self.tasks {
            let waits = matches!(task.status, Status::Joining)
                && task
                    .frames
                    .last()
                    .and_then(Frame::join)
                    .is_some_and(|waited| Rc::ptr_eq(waited, join));
            if task.meta.id == id && !waits && !matches!(task.status, Status::Cancel(_)) {
                task.status = Status::Cancel(looks_up(&task.status));
                task.meta.wound = None;
            }
        }
    }

    /// Counts an abandoned run of `what`, a memoised call or a `lazy`:
    /// whether its runs have been abandoned far more
    /// often than wound-wait allows. Each abandoned run goes to a task
    /// earlier in program order than the one that ran it, the earliest of
    /// those that wait for it (see [Wound-wait](#wound-wait)), so it is
    /// abandoned about once for each task that needs it. Far more is a
    /// fault, reported as an internal error rather than left to spin. The
    /// limit is based on the most tasks the run has had at once.
    pub(super) fn abandoned_too_often(&mut self, what: Abandoned) -> bool {
        let limit = 4 * (self.most_tasks + 1) + 64;
        let count =
            if let Some((_, count)) = self.abandons.iter_mut().find(|(seen, _)| *seen == what) {
                *count += 1;
                *count
            } else {
                self.abandons.push((what, 1));
                1
            };
        count > limit
    }

    /// Wakes every task that waits for a sibling's memoised call or
    /// `lazy`, to look again: a run of one was abandoned, which the
    /// earliest of them runs (see [Wound-wait](#wound-wait)). The task that
    /// wounded the one that abandoned it may have been split since, its
    /// wait taken by a child, so it is not woken by its number.
    pub(super) fn wake_siblings(&mut self) {
        for task in &mut self.tasks {
            let status = std::mem::replace(&mut task.status, Status::Joining);
            task.status = match status {
                Status::Sibling(resume) => Status::Again(resume),
                status => status,
            };
        }
    }

    /// The place among the tasks set aside of the one to run next: the
    /// first that may run in their order, but a task that looks up or
    /// awaits a sibling's call or `lazy` again only once no other may run,
    /// the earliest of those in program order. A run abandoned for an
    /// earlier task is thus begun again by the earliest task that waits for
    /// it, not by a later one that the earlier wounds again, and so on
    /// without end (see [Wound-wait](#wound-wait)).
    fn next_ready(&self) -> Option<usize> {
        let first = self.tasks.iter().position(|task| {
            matches!(
                task.status,
                Status::Ready(_) | Status::Wake(_) | Status::Cancel(_)
            )
        });
        first.or_else(|| {
            self.tasks
                .iter()
                .enumerate()
                .filter(|(_, task)| matches!(task.status, Status::Again(_)))
                .min_by_key(|(_, task)| path_of(&task.meta, &task.frames, &task.calls))
                .map(|(at, _)| at)
        })
    }

    /// Cancels the tasks `ids` that are set aside and still waiting.
    pub(super) fn cancel(&mut self, ids: &[u32]) {
        for task in &mut self.tasks {
            if ids.contains(&task.meta.id) && !matches!(task.status, Status::Cancel(_)) {
                task.status = Status::Cancel(looks_up(&task.status));
                task.meta.wound = None;
            }
        }
    }

    /// Wakes the task `id` if it waits for the children of `join`: its
    /// innermost frame waits for them. A task may hold other joins further
    /// down its frames, whose children end while it waits for this one.
    fn wake(&mut self, id: u32, join: &Rc<RefCell<Join>>) {
        let waits = |frames: &[Frame]| {
            frames
                .last()
                .and_then(Frame::join)
                .is_some_and(|waited| Rc::ptr_eq(waited, join))
        };
        if self.meta.id == id {
            if matches!(self.status, Some(Status::Joining)) && waits(&self.frames) {
                self.status = Some(Status::Ready(Next::Give(Ok(Value::Empty))));
            }
            return;
        }
        for task in &mut self.tasks {
            if task.meta.id == id && matches!(task.status, Status::Joining) && waits(&task.frames) {
                task.status = Status::Ready(Next::Give(Ok(Value::Empty)));
            }
        }
    }

    /// The path of the task running (see [Tasks](#tasks)).
    pub(super) fn path(&self) -> Vec<u32> {
        path_of(&self.meta, &self.frames, &self.calls)
    }

    /// Whether a live task other than the one running is earlier in program
    /// order (see [Tasks](#tasks)): of those that hold every demanded run
    /// it holds, if it holds any (see [Across units](#across-units)).
    pub(super) fn earlier_live(&self) -> bool {
        if self.tasks.is_empty() {
            return false;
        }
        let mine = self.path();
        let demanded = self.demanded_held();
        self.tasks.iter().any(|task| {
            holds(task, &demanded) && path_of(&task.meta, &task.frames, &task.calls) < mine
        })
    }

    /// Whether a task other than the one running waits for the result of
    /// an effect the host does not isolate: the unit makes such effects one
    /// at a time (see [Across units](#across-units)).
    pub(super) fn ordered_effect_waiting(&self) -> bool {
        let Some(graph) = self.shared.graph else {
            return false;
        };
        self.tasks.iter().any(|task| {
            matches!(task.status, Status::Blocked(Wait::Node(id), Resume::Effect(..))
                if graph.is_ordered_effect_waiting(id, self.shared.effects))
        })
    }

    /// Marks `run`, which the task running waits for, demanded, if the task
    /// holds a demanded run: the run it waits for is then as good as
    /// waited for by the other unit; and so on, to every run that tasks
    /// holding a run it marks wait for (see [Across units](#across-units)).
    pub(super) fn demand_waited(&mut self, run: &Run) {
        if !self.demanded_held().is_empty() && self.demand(run) {
            self.propagate_demand();
            self.wake_turns();
        }
    }

    /// Marks demanded every run that a task set aside waits for while it
    /// holds a demanded run, and so on, until none is left to mark; the
    /// root's too, waiting as its status says, if it is the task running.
    /// Whether it marked any. A run is marked as another unit waits for it,
    /// which a task of the unit may wait for in turn, and its holder for
    /// another run of the unit: unmarked, that run would still wait its
    /// turn behind work outside the first, which may wait for the other
    /// unit (see [Across units](#across-units)).
    fn propagate_demand(&mut self) -> bool {
        let mut marked = false;
        loop {
            let mut waited: Vec<Run> = self
                .tasks
                .iter()
                .filter_map(|task| match &task.status {
                    Status::Sibling(resume)
                        if self.holds_demanded(&task.own.running, &task.own.futures) =>
                    {
                        resume.waits_for()
                    }
                    _ => None,
                })
                .collect();
            if let Some(Status::Sibling(resume)) = &self.status
                && self.holds_demanded(&self.memo.running, &self.shared.futures.borrow())
            {
                waited.extend(resume.waits_for());
            }
            let mut more = false;
            for run in &waited {
                more |= self.demand(run);
            }
            if !more {
                return marked;
            }
            marked = true;
        }
    }

    /// Breaks a circle of the unit's tasks that wait for each other's
    /// runs, once every task waits and none for a node or another unit,
    /// which wound-wait left because a run in it is demanded (see [Across
    /// units](#across-units)). Such a circle is a call that depends on its
    /// own result. The task that runs a demanded run an earlier task waits
    /// for is wounded, as wound-wait would have wounded it, and the unit
    /// goes on: the earlier task runs the call itself, and fails where a
    /// run in order fails. What the machine does next; `None` if no task
    /// runs a demanded run that an earlier one waits for.
    pub(super) fn unstick(&mut self) -> Option<Next> {
        if !self.waits().is_empty() {
            return None;
        }
        let target = |run: &Run| match run {
            Run::Entry(at) => Wound::Entry(at.2),
            Run::Future(future) => Wound::Future(Rc::as_ptr(future)),
        };
        let mut waiting: Vec<(Vec<u32>, Wound)> = Vec::new();
        for task in &self.tasks {
            if let Status::Sibling(resume) = &task.status
                && let Some(run) = resume.waits_for()
                && self.is_demanded(&run)
            {
                waiting.push((path_of(&task.meta, &task.frames, &task.calls), target(&run)));
            }
        }
        if let Some(Status::Sibling(resume)) = &self.status
            && let Some(run) = resume.waits_for()
            && self.is_demanded(&run)
        {
            waiting.push((self.path(), target(&run)));
        }
        waiting.sort_by(|one, other| one.0.cmp(&other.0));
        for (path, target) in waiting {
            let later = |site: Option<Vec<u32>>| site.is_some_and(|site| site > path);
            if let Some(task) = self.tasks.iter_mut().find(|task| {
                task.meta.wound.is_none()
                    && !matches!(task.status, Status::Cancel(_))
                    && later(site_of(target, (&task.meta, &task.frames, &task.calls)))
            }) {
                task.meta.wound = Some(target);
                task.status = Status::Cancel(looks_up(&task.status));
                let status = self.status.take()?;
                return self.park(status);
            }
            if self.meta.wound.is_none()
                && later(site_of(target, (&self.meta, &self.frames, &self.calls)))
            {
                let status = self.status.take()?;
                self.meta.wound = Some(target);
                return Some(self.start(Status::Cancel(looks_up(&status))));
            }
        }
        None
    }

    /// Whether `run` is demanded.
    fn is_demanded(&self, run: &Run) -> bool {
        match run {
            Run::Entry(at) => self.memo.is_demanded(*at),
            Run::Future(future) => future.is_demanded(),
        }
    }

    /// Marks `run` demanded: whether it was running and was not.
    fn demand(&mut self, run: &Run) -> bool {
        match run {
            Run::Entry(at) => self.memo.demand(*at),
            Run::Future(future) => self.memo.demand_future(future),
        }
    }

    /// Whether a task whose running memoised calls are `running` and whose
    /// running `lazy`s are `futures` holds a demanded run.
    fn holds_demanded(&self, running: &[Running], futures: &[Rc<crate::value::Future>]) -> bool {
        running
            .iter()
            .any(|running| self.memo.is_demanded(running.at()))
            || futures.iter().any(|future| future.is_demanded())
    }

    /// Wakes every task set aside that waits for its turn, to look again: a
    /// run it holds may have become demanded (see [Across
    /// units](#across-units)).
    fn wake_turns(&mut self) {
        for task in &mut self.tasks {
            let status = std::mem::replace(&mut task.status, Status::Joining);
            task.status = match status {
                Status::Waiting(resume) => Status::Wake(resume),
                status => status,
            };
        }
    }

    /// The demanded runs the task running holds (see [Across
    /// units](#across-units)).
    fn demanded_held(&self) -> Vec<Held> {
        let mut held: Vec<Held> = self
            .memo
            .running
            .iter()
            .filter(|running| self.memo.is_demanded(running.at()))
            .map(|running| Held::Entry(running.entry))
            .collect();
        held.extend(
            self.shared
                .futures
                .borrow()
                .iter()
                .filter(|future| future.is_demanded())
                .map(|future| Held::Future(Rc::as_ptr(future))),
        );
        held
    }

    /// Splits the task running at the marks `marks` of an iteration of a
    /// `for`: what it has past them becomes a child that waits for `wait`
    /// and then takes up after `resume`, its bottom frame delivering to
    /// `join` as its child number `at`; the task running is left as it was
    /// at the marks.
    pub(super) fn split(
        &mut self,
        marks: &Marks,
        (join, at): (Rc<RefCell<Join>>, usize),
        status: Status,
        base: Vec<u32>,
    ) {
        let id = self.next_task;
        self.next_task += 1;
        let mut frames = vec![Frame::Deliver(join, at)];
        frames.extend(self.frames.split_off(marks.frames.min(self.frames.len())));
        // What was split off from the loops, nodes and calls it takes is
        // the child's to wait for.
        let joins = frames.iter().skip(1).filter_map(Frame::join).chain(
            self.calls
                .get(marks.calls..)
                .unwrap_or_default()
                .iter()
                .filter_map(|call| call.pieces.join.as_ref()),
        );
        for join in joins {
            join.borrow_mut().parent = id;
        }
        // The child runs in the activations its iteration ran in, copied,
        // and in its own above them.
        let kept = marks.activations.min(self.activations.len());
        let mut activations = self.activations.get(..kept).unwrap_or_default().to_vec();
        activations.extend(self.activations.split_off(kept));
        let calls = self.calls.split_off(marks.calls.min(self.calls.len()));
        let above = self.stack.split_off(marks.stack.min(self.stack.len()));
        let mut stack = self.stack.clone();
        stack.extend(above);
        let shared = self.shared;
        let [failures, identities, sites, deadlines] = marks.lens;
        let own = Own {
            context: shared.context.replace(marks.context.clone()),
            deadlines: split_kept(&mut shared.deadlines.borrow_mut(), deadlines),
            failures: split_kept(&mut shared.failures.borrow_mut(), failures),
            identities: split_kept(&mut shared.identities.borrow_mut(), identities),
            operand_failed: shared.operand_failed.replace(false),
            sites: split_kept(&mut shared.sites.borrow_mut(), sites),
            running: split_kept(&mut self.memo.running, marks.running),
            futures: split_kept(&mut shared.futures.borrow_mut(), marks.futures),
            run: shared.graph.map_or_else(RunState::default, |graph| {
                graph.fork_state(marks.scopes, marks.last)
            }),
            profile: shared
                .profile
                .map(|profile| profile.split_off(marks.profiled)),
            since: Instant::now(),
        };
        shared.earliest.set(
            shared
                .deadlines
                .borrow()
                .last()
                .map(|deadline| deadline.effective),
        );
        self.tasks.push(Task {
            meta: Meta {
                id,
                base,
                wound: None,
            },
            frames,
            activations,
            calls,
            stack,
            own,
            status,
        });
        self.most_tasks = self.most_tasks.max(self.tasks.len());
    }

    /// The marks of an iteration beginning now (see [`Marks`]).
    pub(super) fn marks(&self) -> Marks {
        let shared = self.shared;
        Marks {
            frames: self.frames.len(),
            activations: self.activations.len(),
            calls: self.calls.len(),
            stack: self.stack.len(),
            running: self.memo.running.len(),
            futures: shared.futures.borrow().len(),
            profiled: shared.profile.map_or(0, profile::Profile::running),
            lens: [
                shared.failures.borrow().len(),
                shared.identities.borrow().len(),
                shared.sites.borrow().len(),
                shared.deadlines.borrow().len(),
            ],
            scopes: shared.graph.map_or(0, GraphRun::scope_depth),
            context: shared.context.borrow().clone(),
            last: shared.graph.and_then(GraphRun::last_met),
        }
    }
}

/// `values`, the whole of which a child takes, cut back to its first `kept`
/// for the task that split it off; returns the whole.
fn split_kept<T: Clone>(values: &mut Vec<T>, kept: usize) -> Vec<T> {
    let whole = values.clone();
    values.truncate(kept);
    whole
}

/// The path where a task whose place is `meta`, whose frames are `frames`
/// and whose calls are `calls` began `target`'s memoised call or `lazy`,
/// if it runs it: of its frames below the call's or `lazy`'s body, and the
/// calls they take (see [Tasks](#tasks)).
fn site_of(
    target: Wound,
    (meta, frames, calls): (&Meta, &[Frame], &[CallState]),
) -> Option<Vec<u32>> {
    match target {
        Wound::Entry(entry) => {
            let call = calls.iter().position(
                |call| matches!(call.memo, Some((Some(mark), _)) if mark.entry == entry),
            )?;
            let mut taken = 0;
            let body = frames.iter().position(|frame| {
                taken += usize::from(frame.takes_call());
                frame.takes_call() && taken == call + 1
            })?;
            Some(path_of(meta, frames.get(..body)?, calls.get(..call)?))
        }
        Wound::Future(future) => {
            let body = frames
                .iter()
                .position(|frame| matches!(frame, Frame::Future(run) if run.runs(future)))?;
            let below = frames.get(..body)?;
            let taken = below.iter().filter(|frame| frame.takes_call()).count();
            Some(path_of(meta, below, calls.get(..taken)?))
        }
    }
}

/// Whether `task` holds every run of `held`.
fn holds(task: &Task, held: &[Held]) -> bool {
    held.iter().all(|&held| match held {
        Held::Entry(entry) => task
            .own
            .running
            .iter()
            .any(|running| running.entry == entry),
        Held::Future(future) => task
            .own
            .futures
            .iter()
            .any(|running| std::ptr::eq(Rc::as_ptr(running), future)),
    })
}

/// Whether a task waiting as `status` says has a call on top of its calls,
/// with no frame of its own, waiting to be looked up again.
const fn looks_up(status: &Status) -> bool {
    matches!(
        status,
        Status::Blocked(_, Resume::Lookup(_))
            | Status::Sibling(Resume::Lookup(_))
            | Status::Again(Resume::Lookup(_))
            | Status::Wake(Resume::Lookup(_))
    )
}

/// The path of a task whose place is `meta` and whose frames are `frames`
/// (see [Tasks](#tasks)).
pub(super) fn path_of(meta: &Meta, frames: &[Frame], calls: &[CallState]) -> Vec<u32> {
    let mut path = meta.base.clone();
    for (frame, call) in super::pieces::with_calls(frames, calls) {
        if let Some(at) = match frame {
            Frame::ForGuard(state) | Frame::ForBody(state) => ForState::split_at(state),
            Frame::ForJoin(_) | Frame::PiecesJoin(_) => Some(u32::MAX),
            frame => frame.piece_running(call),
        } {
            path.push(at);
        }
    }
    path
}

impl Machine<'_> {
    /// What the unit's evaluation, taken up, does first: every task that
    /// waited for a node, a unit, its turn or a sibling is woken, to look
    /// again, and the root, running, takes up as its status says.
    pub(super) fn take_up_tasks(&mut self) -> Next {
        // Other units may have marked runs demanded while it was stopped.
        self.propagate_demand();
        self.wake_given();
        match self.status.take() {
            Some(
                Status::Blocked(_, resume)
                | Status::Waiting(resume)
                | Status::Wake(resume)
                | Status::Sibling(resume)
                | Status::Again(resume),
            ) => self.resume_next(resume),
            Some(Status::Ready(next)) => next,
            Some(status) => self.park(status).unwrap_or(Next::Stop),
            None => {
                let span = self.running().ir.get(self.running().ir.root()).span;
                Next::Give(Err(Box::new(
                    internal("a unit taken up with nothing to do", span).into(),
                )))
            }
        }
    }
}
