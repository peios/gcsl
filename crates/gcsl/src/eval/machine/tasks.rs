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
//! iteration or piece each split loop, node or call on its frames is at,
//! one waiting for its children past every iteration or piece. An effect
//! the host does not isolate ([`crate::eval::Effects::isolates`]) is made
//! only when no live task's path is earlier, so that such effects reach the
//! host in program order; and never once an iteration or piece split off
//! from one the task is in has failed, since a run in order would have
//! stopped there.
//!
//! A task that needs a memoised call or a `lazy` another task of the unit
//! is running waits for it; one that waits for its turn or for a sibling
//! is woken whenever a task ends.
//!
//! # Wound-wait
//!
//! Tasks that wait for each other's memoised calls or `lazy`s can wait in
//! a circle that a run in order never meets: a later task begins a call
//! that an earlier one needs, and then, inside it, waits for its turn
//! behind the earlier task. So a task never waits for a later one's run:
//! it *wounds* it ([`Machine::wound`]). The later task is cancelled as far
//! as the call or `lazy` it was running ([`Meta::wound`]), whose frame
//! catches the cancellation: the entry goes, or the `lazy` is left unrun,
//! the earlier task is woken, and the later one waits to look it up or
//! await it again. The earlier task then runs it as a run in order would:
//! it finishes it, or, if it truly depends on itself, fails where a run in
//! order fails. Waits then only ever go from a later task to an earlier
//! one, and cannot form a circle. A wounded task's work inside the call is
//! lost, as are its children's; an effect the host does not isolate that
//! it was waiting to make is never made.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::diagnostic::Mark;
use crate::source::Span;
use crate::value::{ContextFrame, List, Value};

use super::super::graph::{GraphRun, NodeId, RunState, Wait};
use super::super::memo::Running;
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
    /// What it is to abandon its run of, and the task that wounded it, once
    /// one has (see [Wound-wait](#wound-wait)).
    pub(super) wound: Option<(Wound, u32)>,
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

/// What the run shares that is a task's own (see [Tasks](#tasks)).
pub(in crate::eval) struct Own {
    /// Its context.
    context: Option<Rc<ContextFrame>>,
    /// Its deadlines.
    deadlines: Vec<Deadline>,
    /// The failures its `recover`s caught.
    failures: Vec<Rc<str>>,
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
    /// It waits for its turn to make an effect, or for a sibling's
    /// memoised call or `lazy`, to take up after the resume.
    Waiting(Resume),
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
    /// For each child that is a block's item, by its number, the slots of
    /// the value stack it declares, whose values it comes to (see
    /// [Pieces](super::pieces#pieces)).
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
                .map_or_else(RunState::default, GraphRun::take_state),
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
        let Some(ready) = self.tasks.iter().position(|task| {
            matches!(
                task.status,
                Status::Ready(_) | Status::Wake(_) | Status::Cancel(_)
            )
        }) else {
            self.status = Some(status);
            return None;
        };
        let mut task = self.tasks.swap_remove(ready);
        let start = std::mem::replace(&mut task.status, status);
        let parked = self.exchange(task);
        self.tasks.push(parked);
        self.status = None;
        Some(self.start(start))
    }

    /// What the task now running, which was set aside with `status`, does
    /// first.
    pub(super) fn start(&mut self, status: Status) -> Next {
        match status {
            Status::Ready(next) => next,
            Status::Wake(resume) => self.resume_next(resume),
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
            Status::Blocked(..) | Status::Joining | Status::Waiting(_) => {
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
                status => status,
            };
            task.status = woken;
        }
    }

    /// Ends the task running, which delivered what it came to: another that
    /// may run runs, or `None` if every task waits.
    pub(super) fn finished(&mut self) -> Option<Next> {
        self.wake_waiting(false);
        let ready = self.tasks.iter().position(|task| {
            matches!(
                task.status,
                Status::Ready(_) | Status::Wake(_) | Status::Cancel(_)
            )
        });
        let Some(ready) = ready else {
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
        Some(self.start(start))
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
                    .map(|&slot| self.stack.get(slot).cloned().unwrap_or(Value::Empty))
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
        let woken = (joined.live == 0).then_some(joined.parent);
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
        if let Some(parent) = woken {
            self.wake(parent);
        }
        self.finished()
    }

    /// Wounds the task that runs `target`, if it is later in program order
    /// than the task running, which needs it: it abandons its run, so that
    /// the task running can run it, as a run in order would (see
    /// [Wound-wait](#wound-wait)).
    pub(super) fn wound(&mut self, target: Wound) {
        let mine = self.path();
        let wounder = self.meta.id;
        for task in &mut self.tasks {
            let runs = match target {
                Wound::Entry(entry) => task
                    .calls
                    .iter()
                    .any(|call| matches!(call.memo, Some((Some(mark), _)) if mark.entry == entry)),
                Wound::Future(future) => task
                    .frames
                    .iter()
                    .any(|frame| matches!(frame, Frame::Future(run) if run.runs(future))),
            };
            if runs
                && task.meta.wound.is_none()
                && !matches!(task.status, Status::Cancel(_))
                && path_of(&task.meta, &task.frames, &task.calls) > mine
            {
                task.meta.wound = Some((target, wounder));
                task.status = Status::Cancel(looks_up(&task.status));
            }
        }
    }

    /// Wakes the task `id`, if it waits for a sibling, to look again.
    pub(super) fn wake_task(&mut self, id: u32) {
        for task in &mut self.tasks {
            if task.meta.id == id {
                let status = std::mem::replace(&mut task.status, Status::Joining);
                task.status = match status {
                    Status::Waiting(resume) => Status::Wake(resume),
                    status => status,
                };
            }
        }
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

    /// Wakes the loop of the task `id`, which waits for its children.
    fn wake(&mut self, id: u32) {
        if self.meta.id == id {
            if matches!(self.status, Some(Status::Joining)) {
                self.status = Some(Status::Ready(Next::Give(Ok(Value::Empty))));
            }
            return;
        }
        for task in &mut self.tasks {
            if task.meta.id == id && matches!(task.status, Status::Joining) {
                task.status = Status::Ready(Next::Give(Ok(Value::Empty)));
            }
        }
    }

    /// The path of the task running (see [Tasks](#tasks)).
    pub(super) fn path(&self) -> Vec<u32> {
        path_of(&self.meta, &self.frames, &self.calls)
    }

    /// Whether a live task other than the one running is earlier in program
    /// order (see [Tasks](#tasks)).
    pub(super) fn earlier_live(&self) -> bool {
        if self.tasks.is_empty() {
            return false;
        }
        let mine = self.path();
        self.tasks
            .iter()
            .any(|task| path_of(&task.meta, &task.frames, &task.calls) < mine)
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
                .map(|profile| profile.suspend(marks.profiled)),
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

/// Whether a task waiting as `status` says has a call on top of its calls,
/// with no frame of its own, waiting to be looked up again.
const fn looks_up(status: &Status) -> bool {
    matches!(
        status,
        Status::Blocked(_, Resume::Lookup)
            | Status::Waiting(Resume::Lookup)
            | Status::Wake(Resume::Lookup)
    )
}

/// The path of a task whose place is `meta` and whose frames are `frames`
/// (see [Tasks](#tasks)).
pub(super) fn path_of(meta: &Meta, frames: &[Frame], calls: &[CallState]) -> Vec<u32> {
    let mut path = meta.base.clone();
    for (frame, call) in super::pieces::with_calls(frames, calls) {
        if let Some(at) = match frame {
            Frame::ForBody(state) => ForState::split_at(state),
            Frame::ForJoin(_) | Frame::PiecesJoin(_) => Some(u32::MAX),
            frame => frame.pieces(call).and_then(super::pieces::Pieces::split_at),
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
        self.wake_given();
        match self.status.take() {
            Some(Status::Blocked(_, resume) | Status::Waiting(resume) | Status::Wake(resume)) => {
                self.resume_next(resume)
            }
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
