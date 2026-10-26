//! Graph mode: evaluating a unit as an action graph, for a host that runs
//! its steps (pekit2's `design/action-graph.md`).
//!
//! # Graph mode
//!
//! A host evaluates a *unit*, a file or an invocation, in graph mode,
//! against a [`Graph`] it keeps for the whole evaluation. The graph's
//! *nodes* are the call instances the host deals with:
//!
//! - a **deferred call**, a call of a closure whose code a deferring
//!   attribute is on ([`crate::ir::FnCode::defers`]) and that the host takes
//!   as a step (`GraphRun::defers`, from the function's attributes), made in
//!   `Evaluator::call` before `#[retry]` or `#[timeout]` apply: its arguments
//!   are evaluated as a call's are, and the call gives a future that stands
//!   for its result ([`crate::value::Future::host`]), which code awaits
//!   where it uses the result, as it awaits any future;
//! - an **effect**, a call of an effectful built-in, `exec` and its kin, the
//!   file system's, `File.read`, `Dir.files`, `fs.compose_dir`,
//!   `Dir.sandbox` and `Sandbox.exec` ([`effect_builtin`]), whose arguments
//!   are evaluated and whose result, a present value, is needed at once.
//!   `file` and `dir`, which only read the working directory, fixed for an
//!   evaluation, are computed as ever.
//!
//! So a unit performs no effect of its own. A node whose result is not
//! given yet stops the unit where the result is needed (`Evaluator::pause`):
//! an await of a deferred call's future, or an effect's call. The stop is an
//! error marked [`Mark::PENDING`], which nothing but the unit's root
//! catches, `recover` and `#[retry]` included, and which unwinds by every
//! path an error unwinds by; the node it waits on is the run's `paused`.
//! Once the host gives that node's result ([`Graph::resolve`]), the unit
//! runs again from its start: everything before the stop computes again to
//! the same values, the calls that made nodes find them, and the stop is
//! passed. A node's failure is raised where its result is needed, as any
//! failure there, so `recover` around it catches it; a `#[retry]` around an
//! await raises it again, never running the step again.
//!
//! **Identity.** A deferred call's node is found by its *key*, a canonical
//! encoding of its function, by its code and captures, its arguments and
//! the flattened current context ([`encode`]): two calls of one key in an
//! evaluation are one node. An effect's node is found by its *position*:
//! the scope it is made in and its number among the nodes met in that
//! scope so far. A scope is the unit's root, a memoised call's body or a
//! `lazy`'s, each by its key and how many times its parent scope entered
//! one of that key (`Evaluator::enter_scope`), so that a memoised call that
//! finds its entry, skipping its body, shifts the count of no other scope.
//! Every node met is recorded at its position with its key; an attempt that
//! meets another key at a recorded position is the internal error
//! "evaluation did not converge", never a loop.
//!
//! **Edges.** A node's *data edges* are the nodes whose results its
//! arguments, its callee's captures and its context hold and the host has
//! not given, its *holes* ([`Graph::holes`]): through records, lists,
//! variants, secrets, futures not run yet, by their captures, run ones, by
//! their values, and closures, by their captures. Its *order edge* is the
//! node met just before it in its unit, in program order. A host that
//! isolates a node may drop its order edge; one that does not runs it in
//! program order.
//!
//! **Reads.** A run of a step's unit records what it reads of the context
//! the step was made in ([`StepReads`], `GraphRun::watching`): each context
//! read whose nearest binding is a frame of the step's own context, or, for
//! a read of a default, none in either, with whether that context binds it,
//! once each, in the order first read. A read a `with` inside the step binds
//! is no input of the step. A memoised call found in its table replays the
//! reads it recorded where it is called (`Replayed`), so a cache hit hides
//! no read; one that recorded every context, having made a future, reads
//! every context the step's binds. The first such read of a run is told to
//! the host's effects at once ([`crate::eval::Effects::context_read`]),
//! before the step does anything with it, so that a host can give the step
//! the context's files then. A run that ends ran from the start, so its
//! reads are the step's.
//!
//! # Errors in a library
//!
//! A step's failure, unlike a call's, leaves no frame of the code that made
//! it running, which could move it as [Errors in a
//! library](crate::eval::calls#errors-in-a-library) says. So a node made by
//! foreign code records where it would be reported: the innermost call from
//! the workspace's own code into foreign code running when it was made
//! (`Evaluator::reported`), which every call that crosses keeps on a stack
//! of the run's. A step's run starts that stack with where the step reports,
//! so that a step its body makes reports there too. A step that fails with
//! an error located in foreign code is reported there, the place it was
//! raised kept as a label (`run_step_call`); one the workspace's own code
//! made reports at its call, as before.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::check::{Attribute, Builtin};
use crate::diagnostic::{Diagnostic, Mark};
use crate::ir::{FnCode, Ir, IrNode, IrRange};
use crate::source::Span;
use crate::value::{Closure, ContextFrame, Future, Value};

use std::time::Instant;

use crate::CALL_LEVELS;

use super::plain::{Plain, from_plain, to_plain};
use super::calls::{moved_to, raised_in_tracked};
use super::{Caller, Evaluator, Flow, MemoTables, Shared, internal};

/// A node of an action graph, by its number (see [Graph mode](#graph-mode)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// The node numbered `number`.
    pub(crate) const fn of(number: u32) -> Self {
        Self(number)
    }

    /// Its number, its index among the graph's nodes.
    #[must_use]
    pub const fn number(self) -> u32 {
        self.0
    }
}

/// What a node is (see [Graph mode](#graph-mode)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A call of a closure whose code a deferring attribute is on.
    Deferred,
    /// A call of an effectful built-in.
    Effect(Builtin),
}

/// Where a node is in its life.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeState {
    /// Its result is not given yet.
    Waiting,
    /// Its result.
    Done(Value),
    /// Its failure.
    Failed(Diagnostic),
}

/// One node of an action graph (see [Graph mode](#graph-mode)).
#[derive(Debug)]
pub struct Node {
    /// What it is.
    kind: NodeKind,
    /// A deferred call's callee.
    closure: Option<Rc<Closure>>,
    /// Its arguments, in parameter order.
    args: Box<[Value]>,
    /// The context it was made in, which it runs in.
    context: Option<Rc<ContextFrame>>,
    /// The nodes whose results it needs that were not given when it was
    /// made.
    data: Box<[NodeId]>,
    /// The node met just before it in its unit, in program order.
    after: Option<NodeId>,
    /// The unit that made it.
    unit: u32,
    /// The call that made it, of its unit's code `code`.
    site: IrNode,
    /// The code of the unit that made it.
    code: Rc<Ir>,
    /// Where it reports its failure, if `code` is foreign to the workspace:
    /// the innermost call from the workspace's own code into foreign code
    /// running when it was made, its input and span (see [Errors in a
    /// library](#errors-in-a-library)); `None` for one the workspace's own
    /// code made, which reports at `site`, or for one made with none
    /// running.
    reported: Option<(u32, Span)>,
    /// The caller a deferred call gives its callee.
    caller: Caller,
    /// Where it is in its life.
    state: NodeState,
    /// A deferred call's future, which code awaits.
    future: Option<Rc<Future>>,
}

impl Node {
    /// What it is.
    #[must_use]
    pub const fn kind(&self) -> NodeKind {
        self.kind
    }

    /// A deferred call's callee; `None` for an effect.
    #[must_use]
    pub const fn closure(&self) -> Option<&Rc<Closure>> {
        self.closure.as_ref()
    }

    /// Its arguments, in parameter order.
    #[must_use]
    pub fn args(&self) -> &[Value] {
        &self.args
    }

    /// The nodes whose results it needs that were not given when it was
    /// made: its data edges.
    #[must_use]
    pub fn data(&self) -> &[NodeId] {
        &self.data
    }

    /// The node met just before it in its unit: its order edge.
    #[must_use]
    pub const fn after(&self) -> Option<NodeId> {
        self.after
    }

    /// The unit that made it.
    #[must_use]
    pub const fn unit(&self) -> u32 {
        self.unit
    }

    /// Where the call that made it is, in the input [`Node::input`] says.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.site.span
    }

    /// The input number of the file whose code made it, which its span is
    /// in: a library's, say, for a call a library's function makes.
    #[must_use]
    pub fn input(&self) -> u32 {
        self.code.input()
    }

    /// Where it is in its life.
    #[must_use]
    pub const fn state(&self) -> &NodeState {
        &self.state
    }

    /// Each context the context it was made in binds, by its declaration's
    /// key, sorted, with its nearest binding's value: what it runs in.
    #[must_use]
    pub fn context(&self) -> Vec<(crate::names::Name, Value)> {
        flattened(self.context.as_ref())
    }
}

/// The action graph of an evaluation, which a host keeps across its units'
/// runs (see [Graph mode](#graph-mode)).
#[derive(Debug, Default)]
pub struct Graph {
    /// Every node, by number.
    nodes: Vec<Node>,
    /// The deferred calls' nodes, by key.
    deferred: HashMap<Box<[u8]>, NodeId>,
    /// Every node met, by its unit and position, with the key it was met
    /// with there.
    positions: HashMap<Position, (Box<[u8]>, NodeId)>,
    /// Where each unit first met each node it met: the input and span of
    /// the call, which another unit's node it is need not be.
    met: HashMap<(u32, NodeId), (u32, Span)>,
}

/// Where a node is met: its unit, and a hash of its scope's path and its
/// number among the nodes met in that scope (see [Graph mode](#graph-mode)).
type Position = (u32, u64);

/// How a host says whether it takes a call of a function with these
/// attributes, a deferring one among them, as a step (see [Graph
/// mode](#graph-mode)).
pub type Defers<'a> = &'a dyn Fn(&[Attribute]) -> bool;

/// What a host gives a unit's run in graph mode: which deferring calls it
/// takes as steps, and how the unit's effects that are no nodes reach the
/// system, its working directory among them (see [Graph mode](#graph-mode)).
#[derive(Clone, Copy)]
pub struct GraphHost<'a> {
    /// Which deferring calls are steps.
    pub defers: Defers<'a>,
    /// The unit's effect context.
    pub effects: &'a dyn super::Effects,
}

impl std::fmt::Debug for GraphHost<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphHost")
            .field("effects", &self.effects)
            .finish_non_exhaustive()
    }
}

/// The number of the first unit that is a step's run, after every file's
/// and every prepared call's: a step's unit is this plus its node's number
/// (see [`crate::workspace::Workspace::run_step`]).
pub const STEP_UNITS: u32 = 1 << 31;

impl Graph {
    /// An empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many nodes it has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether it has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every node, with its id, in the order they were made: for each unit,
    /// the order its code met them, which is the same in every evaluation
    /// whatever order its steps finish in.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        (0u32..)
            .zip(&self.nodes)
            .map(|(number, node)| (NodeId(number), node))
    }

    /// The node `id`, if the graph has it.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)
    }

    /// Gives the node `id`, which is waiting, what its step came to: its
    /// value or its failure, which code then finds where it needs the result.
    /// Returns `false`, changing nothing, if the graph has no such node or it
    /// is not waiting.
    pub fn resolve(&mut self, id: NodeId, outcome: Result<Value, Diagnostic>) -> bool {
        let Some(node) = self.nodes.get_mut(id.0 as usize) else {
            return false;
        };
        if node.state != NodeState::Waiting {
            return false;
        }
        if let Some(future) = &node.future {
            future.resolve(outcome.clone());
        }
        node.state = match outcome {
            Ok(value) => NodeState::Done(value),
            Err(error) => NodeState::Failed(error),
        };
        true
    }

    /// Forgets every node the unit `unit` met, by where it met it, so that
    /// its next run makes each again, a new node: what a host does to run a
    /// step's unit afresh, for a retry. The nodes stay, as they were, for
    /// whatever already holds them.
    pub fn forget_unit(&mut self, unit: u32) {
        self.positions.retain(|&(met_in, _), _| met_in != unit);
        self.met.retain(|&(met_in, _), _| met_in != unit);
    }

    /// Where the unit `unit` first met the node `id`, if it met it: the
    /// input and span of its call, which are the node's own only if the unit
    /// made it, since a deferred call two units make is one node.
    #[must_use]
    pub fn met_at(&self, unit: u32, id: NodeId) -> Option<(u32, Span)> {
        self.met.get(&(unit, id)).copied()
    }

    /// The nodes whose results `value` holds and that are waiting: its holes,
    /// in the order they are found, each once (see [Graph mode](#graph-mode)).
    #[must_use]
    pub fn holes(&self, value: &Value) -> Vec<NodeId> {
        let mut found = Vec::new();
        self.holes_into(std::slice::from_ref(value), &mut found);
        found
    }

    /// The failures of the steps whose results `value` holds, in the order
    /// they are found: what a host reports once `value` has no holes, since
    /// each is a failure no code met.
    #[must_use]
    pub fn failures(&self, value: &Value) -> Vec<Diagnostic> {
        let mut found = Vec::new();
        futures_in(std::slice::from_ref(value), |future| {
            if let Some(failure) = future.failure() {
                found.push(failure);
            }
        });
        found
    }

    /// Pushes the holes of `values` onto `found`, each once.
    fn holes_into(&self, values: &[Value], found: &mut Vec<NodeId>) {
        futures_in(values, |future| {
            if let Some(node) = future.host_node() {
                let id = NodeId(node);
                if self.is_waiting(id) && !found.contains(&id) {
                    found.push(id);
                }
            }
        });
    }

    /// Whether the node `id` is waiting for its result.
    fn is_waiting(&self, id: NodeId) -> bool {
        self.node(id)
            .is_some_and(|node| node.state == NodeState::Waiting)
    }

    /// The waiting nodes that `demands` need: each, and, through their data
    /// edges, every waiting node whose result one of them needs, in no
    /// particular order, each once (see pekit2's `design/action-graph.md`,
    /// "Demand").
    #[must_use]
    pub fn needed(&self, demands: &[NodeId]) -> Vec<NodeId> {
        let mut found: Vec<NodeId> = Vec::new();
        let mut pending: Vec<NodeId> = demands.to_vec();
        while let Some(id) = pending.pop() {
            if found.contains(&id) || !self.is_waiting(id) {
                continue;
            }
            found.push(id);
            if let Some(node) = self.node(id) {
                pending.extend(node.data.iter().copied());
            }
        }
        found.sort_unstable();
        found
    }

    /// Of `needed`, the nodes whose data edges are all given: those a host
    /// may run now.
    #[must_use]
    pub fn ready(&self, needed: &[NodeId]) -> Vec<NodeId> {
        needed
            .iter()
            .copied()
            .filter(|&id| {
                self.is_waiting(id)
                    && self
                        .node(id)
                        .is_some_and(|node| node.data.iter().all(|&edge| !self.is_waiting(edge)))
            })
            .collect()
    }
}

/// Whether `builtin` is an effect in graph mode (see [Graph mode](#graph-mode)):
/// every impure built-in but `file` and `dir`, which only read the working
/// directory.
#[must_use]
pub fn effect_builtin(builtin: Builtin) -> bool {
    builtin.effect() != crate::ast::Effect::Pure && !matches!(builtin, Builtin::File | Builtin::Dir)
}

/// The scope a node is met in (see [Graph mode](#graph-mode)).
#[derive(Debug, Default)]
struct Scope {
    /// Its path: a hash of its parent's, its key and its number among its
    /// parent's scopes of that key.
    path: u64,
    /// How many nodes have been met in it.
    met: u32,
    /// How many scopes of each key it has entered.
    entered: HashMap<u64, u32>,
}

/// One run of a unit in graph mode (see [Graph mode](#graph-mode)), shared by
/// its frames.
pub(crate) struct GraphRun<'g> {
    /// The evaluation's graph, the host's, for the run.
    graph: RefCell<Graph>,
    /// The unit running.
    unit: u32,
    /// Whether the host takes a call of a function with these attributes, a
    /// deferring attribute among them, as a step.
    defers: Defers<'g>,
    /// The node the run stopped for, once it has.
    paused: Cell<Option<NodeId>>,
    /// The node met last, in program order.
    last: Cell<Option<NodeId>>,
    /// The scopes being run, innermost last.
    scopes: RefCell<Vec<Scope>>,
    /// Of a step's unit's run, what it reads of the step's own context
    /// ([`StepReads`]); `None` for any other unit's.
    watch: RefCell<Option<Watch>>,
    /// The text of each context's key, by which a read is told to the
    /// host.
    texts: HashMap<crate::names::Name, Rc<str>>,
}

/// What a run of a step's unit read of the context the step was made in
/// (see [Graph mode](#graph-mode), "Reads"): each context read whose
/// binding is the step's own, a frame of the step's context, or none, for a
/// default read where it binds none, by its declaration's key's text, with
/// whether the step's context binds it, in the order first read; and
/// whether it read every context its context binds, as a memoised call
/// found in its table that made a future may have.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepReads {
    /// Each context read, by its key's text, and whether it was bound.
    pub keys: Vec<(String, bool)>,
    /// Whether every context the step's context binds was read too.
    pub all: bool,
}

/// The context of a step whose run's reads are watched, and what it read.
#[derive(Debug)]
struct Watch {
    /// The step's context.
    context: Option<Rc<ContextFrame>>,
    /// Each context read, by its declaration's key, and whether it was bound.
    keys: Vec<(crate::names::Name, bool)>,
    /// Whether every context bound was read.
    all: bool,
}

impl std::fmt::Debug for GraphRun<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphRun")
            .field("unit", &self.unit)
            .field("paused", &self.paused.get())
            .finish_non_exhaustive()
    }
}

impl<'g> GraphRun<'g> {
    /// A run of unit `unit` against `graph`, the host deciding by `defers`
    /// which deferring calls are steps; [`GraphRun::finish`] gives the graph
    /// back.
    pub(crate) fn new(graph: Graph, unit: u32, defers: Defers<'g>) -> Self {
        let root = Scope {
            path: hash_of(&("unit", unit)),
            ..Scope::default()
        };
        Self {
            graph: RefCell::new(graph),
            unit,
            defers,
            paused: Cell::new(None),
            last: Cell::new(None),
            scopes: RefCell::new(vec![root]),
            watch: RefCell::new(None),
            texts: HashMap::new(),
        }
    }

    /// The run, watching what it reads of the context `context`, a step's,
    /// each read told by its key's text in `texts`, the text of every
    /// context's key ([`GraphRun::note_read`]).
    pub(crate) fn watching(
        mut self,
        context: Option<Rc<ContextFrame>>,
        texts: HashMap<crate::names::Name, Rc<str>>,
    ) -> Self {
        self.watch = RefCell::new(Some(Watch {
            context,
            keys: Vec::new(),
            all: false,
        }));
        self.texts = texts;
        self
    }

    /// Notes a read of the context of the declaration key `key` in the
    /// context `context`, if the run watches a step's: one whose nearest
    /// binding there is the step's context's, or none in both. The key's
    /// text if it is the first such read of the run, which the host is told
    /// of; `None` otherwise.
    pub(crate) fn note_read(
        &self,
        key: crate::names::Name,
        context: Option<&Rc<ContextFrame>>,
    ) -> Option<Rc<str>> {
        let mut watch = self.watch.borrow_mut();
        let watch = watch.as_mut()?;
        let read = ContextFrame::find(context, key).map(|(frame, _)| frame);
        let own = ContextFrame::find(watch.context.as_ref(), key).map(|(frame, _)| frame);
        if read != own || watch.keys.iter().any(|&(seen, _)| seen == key) {
            return None;
        }
        watch.keys.push((key, own.is_some()));
        self.texts.get(&key).cloned()
    }

    /// Notes that the run read every context its step's context binds, if it
    /// watches one: each's key's text that it had not read before.
    pub(crate) fn note_all(&self) -> Vec<Rc<str>> {
        let mut watch = self.watch.borrow_mut();
        let Some(watch) = watch.as_mut() else {
            return Vec::new();
        };
        watch.all = true;
        let mut new = Vec::new();
        for (key, _) in ContextFrame::flattened(watch.context.as_ref()) {
            if !watch.keys.iter().any(|&(seen, _)| seen == key) {
                watch.keys.push((key, true));
                new.extend(self.texts.get(&key).cloned());
            }
        }
        new
    }

    /// What the run read of its step's context ([`StepReads`]): nothing for
    /// a run that watches none.
    pub(crate) fn reads(&self) -> StepReads {
        let watch = self.watch.borrow();
        let Some(watch) = watch.as_ref() else {
            return StepReads::default();
        };
        StepReads {
            keys: watch
                .keys
                .iter()
                .filter_map(|&(key, bound)| Some((self.texts.get(&key)?.to_string(), bound)))
                .collect(),
            all: watch.all,
        }
    }

    /// The graph, as the run left it, and the node the run stopped for, if
    /// it did.
    pub(crate) fn finish(self) -> (Graph, Option<NodeId>) {
        (self.graph.into_inner(), self.paused.get())
    }

    /// The position of the next node met, in the innermost scope.
    fn next_position(&self) -> u64 {
        let mut scopes = self.scopes.borrow_mut();
        let Some(scope) = scopes.last_mut() else {
            return 0;
        };
        scope.met += 1;
        hash_of(&(scope.path, scope.met))
    }

    /// Enters the scope of key `key` inside the innermost one.
    fn enter(&self, key: u64) {
        let mut scopes = self.scopes.borrow_mut();
        let parent_path = scopes.last().map_or(0, |scope| scope.path);
        let number = scopes.last_mut().map_or(0, |scope| {
            let entered = scope.entered.entry(key).or_insert(0);
            *entered += 1;
            *entered
        });
        scopes.push(Scope {
            path: hash_of(&(parent_path, key, number)),
            ..Scope::default()
        });
    }

    /// Leaves the innermost scope, but the unit's own.
    fn leave(&self) {
        let mut scopes = self.scopes.borrow_mut();
        if scopes.len() > 1 {
            scopes.pop();
        }
    }

    /// The node met at the next position with `key`, making it if the
    /// evaluation has none, by `new_node`; the internal error "evaluation did
    /// not converge" if another key was met there before.
    fn meet(
        &self,
        key: Box<[u8]>,
        (input, span): (u32, Span),
        new_node: impl FnOnce(&Graph, Option<NodeId>) -> Node,
    ) -> Result<NodeId, Diagnostic> {
        let position = (self.unit, self.next_position());
        let mut graph = self.graph.borrow_mut();
        let found = match graph.positions.get(&position) {
            Some((recorded, id)) if *recorded == key => Some(*id),
            Some(_) => {
                return Err(internal(
                    "evaluation did not converge: a step met here changed between attempts",
                    span,
                ));
            }
            None => None,
        };
        let id = if let Some(id) = found {
            id
        } else {
            // A deferred call of a key met elsewhere is that node; anything
            // else met for the first time is a new one.
            let made = new_node(&graph, self.last.get());
            let existing = (made.kind == NodeKind::Deferred)
                .then(|| graph.deferred.get(&key).copied())
                .flatten();
            let id = existing.unwrap_or_else(|| {
                let id = NodeId(u32::try_from(graph.nodes.len()).unwrap_or(u32::MAX));
                if made.kind == NodeKind::Deferred {
                    graph.deferred.insert(key.clone(), id);
                }
                graph.nodes.push(made);
                id
            });
            graph.positions.insert(position, (key, id));
            graph.met.entry((self.unit, id)).or_insert((input, span));
            id
        };
        self.last.set(Some(id));
        Ok(id)
    }
}

impl Evaluator<'_> {
    /// The stop for the node `id`, waiting, at the node `node` that needs its
    /// result: the error marked [`Mark::PENDING`], the run's `paused` the node
    /// (see [Graph mode](#graph-mode)).
    pub(super) fn pause<T>(&self, id: NodeId, node: &IrNode) -> Flow<T> {
        if let Some(run) = self.shared.graph {
            run.paused.set(Some(id));
        }
        let mut stop = Diagnostic::error("waiting for a step's result", node.span);
        stop.mark = Mark::PENDING;
        Err(stop.into())
    }

    /// Whether the call of `closure`, of code `code`, is a step the host takes,
    /// in graph mode (see [Graph mode](#graph-mode)).
    pub(super) fn defers(&self, code: &FnCode) -> bool {
        code.defers
            && self
                .shared
                .graph
                .is_some_and(|run| (run.defers)(&code.attributes))
    }

    /// Where a node this frame makes reports a failure raised in foreign
    /// code (see [Errors in a library](#errors-in-a-library)): `None` if this
    /// frame is the workspace's own code, which reports at the node's call;
    /// else the innermost call from the workspace's code into foreign code
    /// running, if any is.
    fn reported(&self) -> Option<(u32, Span)> {
        if !self.ir.is_foreign() {
            return None;
        }
        self.shared.sites.borrow().last().copied()
    }

    /// The call `node` of `closure`, whose code is `code`, with the arguments
    /// `args` at depth `depth`, as a deferred call's node (see [Graph
    /// mode](#graph-mode)): its arguments evaluated as a call's are, then the
    /// future that stands for its result, the node's. Apart from
    /// [`Evaluator::call`], which every call passes through, so that its frame
    /// stays small.
    #[inline(never)]
    pub(super) fn defer_call(
        &mut self,
        (closure, code): (&Rc<Closure>, &FnCode),
        (args, depth): (IrRange, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let Some(run) = self.shared.graph else {
            return Err(internal("a call deferred outside graph mode", node.span).into());
        };
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        if let Err(unwind) = self.arguments(args, base, (closure, depth)) {
            self.stack.truncate(base);
            return Err(unwind);
        }
        let params = base..base + code.param_count as usize;
        let given: Box<[Value]> = self.stack.get(params).unwrap_or_default().into();
        self.stack.truncate(base);
        let context = self.shared.context.borrow().clone();
        let mut key = Vec::new();
        key.push(1);
        encode(&Value::Fn(Rc::clone(closure)), &mut key);
        for value in &given {
            encode(value, &mut key);
        }
        encode_context(context.as_ref(), true, &mut key);
        let made = self.callee_caller();
        let reported = self.reported();
        let id = run.meet(key.into(), (self.ir.input(), node.span), |graph, after| {
            let mut data = Vec::new();
            graph.holes_into(&given, &mut data);
            graph.holes_into(&closure.captures, &mut data);
            graph.holes_into(&context_values(context.as_ref()), &mut data);
            let number = u32::try_from(graph.nodes.len()).unwrap_or(u32::MAX);
            let future = Future::host(
                Rc::clone(&closure.code),
                closure.function,
                number,
                (made, context.clone()),
            );
            Node {
                kind: NodeKind::Deferred,
                closure: Some(Rc::clone(closure)),
                args: given.clone(),
                context: context.clone(),
                data: data.into(),
                after,
                unit: run.unit,
                site: *node,
                code: Rc::clone(self.ir),
                reported,
                caller: made,
                state: NodeState::Waiting,
                future: Some(Rc::new(future)),
            }
        })?;
        let graph = run.graph.borrow();
        let future = graph.node(id).and_then(|found| found.future.clone());
        match future {
            Some(future) => Ok(Value::Future(future)),
            None => Err(internal("a deferred call's node has no future", node.span).into()),
        }
    }

    /// The call `node` of the effectful built-in `builtin` with the arguments
    /// `arguments`, by parameter, as an effect's node (see [Graph
    /// mode](#graph-mode)): its result if the host has given it, its failure
    /// if it failed, and otherwise the stop for it. Apart from the built-in's
    /// call, which every call passes through, so that its frame stays small.
    #[inline(never)]
    pub(super) fn effect_node(
        &self,
        builtin: Builtin,
        arguments: &[Option<(Value, Span)>],
        node: &IrNode,
    ) -> Flow<Value> {
        let given: Vec<Value> = arguments
            .iter()
            .flatten()
            .map(|(value, _)| value.clone())
            .collect();
        let given = &given[..];
        let Some(run) = self.shared.graph else {
            return Err(internal("an effect made a node outside graph mode", node.span).into());
        };
        let context = self.shared.context.borrow().clone();
        let mut key = Vec::new();
        key.push(2);
        key.extend_from_slice(builtin.name().as_bytes());
        key.push(0);
        for value in given {
            encode(value, &mut key);
        }
        encode_context(context.as_ref(), false, &mut key);
        let reported = self.reported();
        let id = run.meet(key.into(), (self.ir.input(), node.span), |graph, after| {
            let mut data = Vec::new();
            graph.holes_into(given, &mut data);
            Node {
                kind: NodeKind::Effect(builtin),
                closure: None,
                args: given.into(),
                context: context.clone(),
                data: data.into(),
                after,
                unit: run.unit,
                site: *node,
                code: Rc::clone(self.ir),
                reported,
                caller: self.caller,
                state: NodeState::Waiting,
                future: None,
            }
        })?;
        let state = run.graph.borrow().node(id).map(|found| found.state.clone());
        match state {
            Some(NodeState::Done(value)) => Ok(value),
            Some(NodeState::Failed(error)) => Err(error.into()),
            Some(NodeState::Waiting) => self.pause(id, node),
            None => Err(internal("an effect's node is missing", node.span).into()),
        }
    }

    /// Enters, in graph mode, the scope of the memoised call or `lazy` body
    /// whose key is `parts` encoded, returning whether it did, so that the
    /// caller leaves it again (see [Graph mode](#graph-mode)).
    pub(super) fn enter_scope(&self, parts: &[&Value]) -> bool {
        let Some(run) = self.shared.graph else {
            return false;
        };
        let mut key = Vec::new();
        for value in parts {
            encode(value, &mut key);
        }
        run.enter(hash_of(&key));
        true
    }

    /// Enters, in graph mode, the scope of the body of the function `function`
    /// of `code` with the captures `captures`, a `lazy`'s, returning whether it
    /// did (see [Graph mode](#graph-mode)).
    pub(super) fn enter_scope_of(
        &self,
        code: &Rc<crate::ir::Ir>,
        function: crate::ast::FnId,
        captures: &[Value],
    ) -> bool {
        let Some(run) = self.shared.graph else {
            return false;
        };
        let mut key = vec![b'L'];
        key.extend_from_slice(&(Rc::as_ptr(code) as usize).to_le_bytes());
        key.extend_from_slice(&function.index().to_le_bytes());
        for value in captures {
            encode(value, &mut key);
        }
        run.enter(hash_of(&key));
        true
    }

    /// Leaves the scope [`Evaluator::enter_scope`] entered, if it did.
    pub(super) fn leave_scope(&self, entered: bool) {
        if let (true, Some(run)) = (entered, self.shared.graph) {
            run.leave();
        }
    }
}

/// Runs `node`, a node of an action graph, as a host asks (see [Graph
/// mode](#graph-mode)): a deferred call's callee, given its arguments, in
/// the context it was made in, with `#[retry]` and `#[timeout]` as its code
/// says; or an effect's built-in, applied to its arguments. Either runs as
/// plain GCSL runs it, its effects through `effects`, on `stack` above what
/// it holds and with `memo` as its memo tables, `exec_result` the layout of
/// `exec`'s result and `dirs` the run's directories.
///
/// # Errors
///
/// The step's failure, as a diagnostic.
pub(crate) fn run_node(
    node: &Node,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    extras: RunExtras<'_>,
) -> Result<Value, Diagnostic> {
    let span = node.site.span;
    match (node.kind, StepCall::of(node)) {
        (NodeKind::Deferred, Some(call)) => {
            let mut shared = Shared::new(extras.0, extras.1, extras.2);
            shared.names = extras.3;
            call_step(&call, (stack, memo), &shared, true)
        }
        (NodeKind::Effect(builtin), None) => {
            let mut shared = Shared::new(extras.0, extras.1, extras.2);
            shared.names = extras.3;
            shared.context.replace(node.context.clone());
            apply_effect(
                builtin,
                &node.args,
                (&node.code, &node.site),
                (stack, memo),
                &shared,
            )
        }
        _ => Err(internal("a step of another shape", span)),
    }
}

/// The layout of `exec`'s result, the run's directories and its effect
/// context, for [`run_node`] and [`run_step_call`].
pub(crate) type RunExtras<'a> = (
    Option<&'a Rc<crate::value::Layout>>,
    &'a [Rc<str>],
    &'a dyn super::Effects,
    Option<&'a RefCell<crate::names::Names>>,
);

/// A deferred node's call, apart from its graph, which a run takes (see
/// [`run_step_call`]).
#[derive(Clone, Debug)]
pub(crate) struct StepCall {
    /// The callee.
    closure: Rc<Closure>,
    /// Its arguments, in parameter order.
    args: Box<[Value]>,
    /// The context it was made in.
    context: Option<Rc<ContextFrame>>,
    /// The caller its call gives it.
    caller: Caller,
    /// The call that made it.
    site: IrNode,
    /// The input of the code the call is in.
    made_in: u32,
    /// Where it reports a failure raised in foreign code, if anywhere: its
    /// node's, or the call that made it if the workspace's own code did
    /// (see [Errors in a library](#errors-in-a-library)).
    reported: Option<(u32, Span)>,
    /// Whether the code the call is in is foreign to the workspace.
    made_foreign: bool,
}

impl StepCall {
    /// The call of `node`, if it is a deferred call's.
    pub(crate) fn of(node: &Node) -> Option<Self> {
        Some(Self {
            closure: Rc::clone(node.closure.as_ref()?),
            args: node.args.clone(),
            context: node.context.clone(),
            caller: node.caller,
            site: node.site,
            made_in: node.code.input(),
            reported: node
                .reported
                .or_else(|| (!node.code.is_foreign()).then(|| (node.code.input(), node.site.span))),
            made_foreign: node.code.is_foreign(),
        })
    }

    /// The context it was made in, which its run reads.
    pub(crate) fn context(&self) -> Option<Rc<ContextFrame>> {
        self.context.clone()
    }
}

/// Runs `call`, a step's, in graph mode, as the unit `run` is: its callee's
/// body, in the context it was made in, its effects nodes of the unit, its
/// own `#[retry]` and `#[timeout]` not applied, which are the host's for a
/// step (see pekit2's `design/engine.md`, E5).
///
/// # Errors
///
/// The step's failure, located as a call's is, in its input: at the call, in
/// the code that made it, for a `#[track_caller]` callee's own, else where it
/// was raised; or the stop for a step's result, marked [`Mark::PENDING`].
pub(crate) fn run_step_call(
    call: &StepCall,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    extras: RunExtras<'_>,
    run: &GraphRun<'_>,
) -> Result<Value, Diagnostic> {
    let mut shared = Shared::new(extras.0, extras.1, extras.2);
    shared.names = extras.3;
    shared.graph = Some(run);
    // A step its body makes reports where this one does.
    shared.sites.borrow_mut().extend(call.reported);
    call_step(call, (stack, memo), &shared, false).map_err(|mut error| {
        if error.mark == Mark::PENDING {
            return error;
        }
        if error.input.is_none() {
            if raised_in_tracked(&error, &call.closure) {
                error.span = call.site.span;
                error.labels.retain(|label| label.input.is_some());
                error.input = Some(call.made_in);
                error.foreign = call.made_foreign;
            } else {
                error.input = Some(call.closure.code.input());
                error.foreign = call.closure.code.is_foreign();
            }
        }
        // No frame of its maker is left running to move it.
        match (error.foreign, error.input, call.reported) {
            (true, Some(from), Some((input, span))) => moved_to(error, from, (Some(input), span)),
            _ => error,
        }
    })
}

/// Calls `call`'s callee with its arguments in its parameter slots, on
/// `stack` above what it holds, sharing `shared`, in the call's context, with
/// `#[retry]` and `#[timeout]` as its code says if `guarded`.
fn call_step(
    call: &StepCall,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    shared: &Shared<'_>,
    guarded: bool,
) -> Result<Value, Diagnostic> {
    let closure = &call.closure;
    let span = call.site.span;
    let Some(code) = closure.code.find_function(closure.function) else {
        return Err(internal("a step's callee is no function of its code", span));
    };
    shared.context.replace(call.context.clone());
    let base = stack.len();
    stack.resize(base + code.frame_size as usize, Value::Empty);
    for (slot, value) in stack.iter_mut().skip(base).zip(call.args.iter()) {
        *slot = value.clone();
    }
    let mut evaluator = Evaluator {
        ir: &closure.code,
        stack,
        base,
        captures: &closure.captures,
        callee: Some(closure),
        memo,
        shared,
        caller: call.caller,
        levels: CALL_LEVELS,
    };
    let result = if guarded && (code.retry > 0 || code.timeout.is_some()) {
        evaluator
            .guarded_call(closure, base, (call.caller, CALL_LEVELS), &call.site)
            .map_err(|unwind| unwind.into_error(span))
    } else {
        evaluator
            .value(code.body)
            .or_else(|unwind| unwind.returned(span))
    };
    stack.truncate(base);
    result
}

/// Applies the effectful built-in `builtin` to `args`, for its call `site` of
/// `code`, on `stack`, sharing `shared`.
fn apply_effect(
    builtin: Builtin,
    args: &[Value],
    (code, site): (&Rc<Ir>, &IrNode),
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    shared: &Shared<'_>,
) -> Result<Value, Diagnostic> {
    let mut arguments: [Option<(Value, Span)>; Builtin::MAX_PARAMS] = Default::default();
    for (entry, value) in arguments.iter_mut().zip(args.iter()) {
        *entry = Some((value.clone(), site.span));
    }
    let base = stack.len();
    let result = Evaluator {
        ir: code,
        stack,
        base,
        captures: &[],
        callee: None,
        memo,
        shared,
        caller: Caller::default(),
        levels: CALL_LEVELS,
    }
    .apply_builtin(builtin, arguments, site)
    .map_err(|unwind| unwind.into_error(site.span));
    stack.truncate(base);
    result
}

/// An effect node's built-in and its arguments as plain data, which a host
/// can run on any thread, apart from the workspace (see pekit2's
/// `design/engine.md`, E1): what a step's `exec` is while it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectJob {
    /// The built-in.
    builtin: Builtin,
    /// Its arguments, by parameter.
    args: Vec<Plain>,
    /// Where its call is.
    span: Span,
}

impl EffectJob {
    /// The job of an effect node, its arguments `args` made plain by its
    /// workspace.
    pub(crate) const fn new(builtin: Builtin, args: Vec<Plain>, span: Span) -> Self {
        Self {
            builtin,
            args,
            span,
        }
    }

    /// The built-in it runs.
    #[must_use]
    pub const fn builtin(&self) -> Builtin {
        self.builtin
    }

    /// Its arguments, by parameter, as plain data.
    #[must_use]
    pub fn args(&self) -> &[Plain] {
        &self.args
    }

    /// Runs the built-in on its arguments, its effects through `effects`,
    /// until `deadline` if there is one, a program it runs killed then: its
    /// result as plain data, for the host to give the node back as a value
    /// ([`crate::workspace::Workspace::from_plain`]).
    ///
    /// # Errors
    ///
    /// The built-in's failure, "the step timed out" marked as a deadline's
    /// passing among them, or a result that is no data.
    pub fn run(
        &self,
        effects: &(dyn super::Effects + Sync),
        deadline: Option<Instant>,
    ) -> Result<Plain, Diagnostic> {
        let mut names = crate::names::Names::new();
        let values: Vec<Value> = self
            .args
            .iter()
            .map(|plain| from_plain(plain, &mut names))
            .collect();
        let layout = crate::workspace::exec_layout(&mut names);
        let shared = Shared::new(Some(&layout), &[], effects);
        if let Some(own) = deadline {
            shared.deadlines.borrow_mut().push(super::Deadline {
                number: 1,
                own,
                effective: own,
                message: Rc::from("the step timed out"),
            });
            shared.earliest.set(Some(own));
        }
        let mut builder = crate::ir::IrBuilder::with_capacity(1);
        let root = builder.alloc(crate::ir::IrKind::Empty, self.span);
        let code = Rc::new(builder.finish_program(root, (0, Vec::new()), Vec::new(), (0, 0)));
        let site = IrNode {
            kind: crate::ir::IrKind::Empty,
            span: self.span,
        };
        let value = apply_effect(
            self.builtin,
            &values,
            (&code, &site),
            (&mut Vec::new(), &mut MemoTables::new()),
            &shared,
        )?;
        to_plain(&value, &names).map_err(|message| Diagnostic::error(message, self.span))
    }
}

/// A hash of `value`, for a scope's path and a position.
fn hash_of(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

/// The values the context `context` binds, its nearest binding of each.
fn context_values(context: Option<&Rc<ContextFrame>>) -> Vec<Value> {
    flattened(context)
        .into_iter()
        .map(|(_, value)| value)
        .collect()
}

/// The context `context` flattened: each key it binds, sorted, with its
/// nearest binding's value.
pub(crate) fn flattened(context: Option<&Rc<ContextFrame>>) -> Vec<(crate::names::Name, Value)> {
    let mut found: Vec<(crate::names::Name, Value)> = Vec::new();
    let mut at = context;
    while let Some(frame) = at {
        for (key, value) in &frame.binds {
            if !found.iter().any(|(had, _)| had == key) {
                found.push((*key, value.clone()));
            }
        }
        at = frame.outer.as_ref();
    }
    found.sort_by_key(|(key, _)| *key);
    found
}

/// Appends the flattened context `context` to the key `out`: its bindings,
/// never a frame's identity, which is new each time a `with` runs, and, for
/// a deferred call, never the definition it was made in, which the call's
/// code cannot read (its host runs it in a directory of its own, see
/// [Files](crate::eval::files#the-working-directory)), so that the same
/// call made for two definitions, two targets of one build say, is one
/// node and runs once.
fn encode_context(context: Option<&Rc<ContextFrame>>, deferred: bool, out: &mut Vec<u8>) {
    let mut bound = flattened(context);
    if deferred {
        bound.retain(|(key, _)| *key != crate::names::DEFINITION);
    }
    out.push(b'c');
    out.extend_from_slice(&u32::try_from(bound.len()).unwrap_or(u32::MAX).to_le_bytes());
    for (key, value) in &bound {
        out.extend_from_slice(&key.index().to_le_bytes());
        encode(value, out);
    }
}

/// Appends a canonical encoding of `value` to the key `out` (see [Graph
/// mode](#graph-mode)): equal for two values a deferred call cannot tell
/// apart, whatever the attempt. A `Float` by its bits, every NaN one and
/// `-0.0` as `0.0`, so that the equality is reflexive; a record's fields
/// sorted by name; a closure by its code, the IR's address and the
/// function's number, and its captures; a step's future by its node, given
/// or not, one run by its value, one not run by its code and captures; a
/// secret by its identity, as secrets compare.
pub(crate) fn encode(value: &Value, out: &mut Vec<u8>) {
    let mut pending: Vec<Value> = vec![value.clone()];
    while let Some(value) = pending.pop() {
        encode_one(value, out, &mut pending);
    }
}

/// Appends `value`'s own part of the encoding to `out`, and pushes what it
/// holds onto `pending`, in reverse, for [`encode`].
fn encode_one(value: Value, out: &mut Vec<u8>, pending: &mut Vec<Value>) {
    match value {
        Value::Int(n) => {
            out.push(b'i');
            out.extend_from_slice(&n.to_le_bytes());
        }
        Value::Float(x) => {
            out.push(b'f');
            let bits = if x.is_nan() {
                f64::NAN.to_bits()
            } else if x == 0.0 {
                0
            } else {
                x.to_bits()
            };
            out.extend_from_slice(&bits.to_le_bytes());
        }
        Value::Bool(b) => out.extend_from_slice(&[b'b', u8::from(b)]),
        Value::Str(s) => {
            out.push(b's');
            write_text(out, &s);
        }
        Value::Quantity(quantity, amount) => {
            out.push(b'q');
            write_text(out, &format!("{quantity:?}"));
            out.extend_from_slice(&amount.to_le_bytes());
        }
        Value::Empty => out.push(b'e'),
        Value::Record(record) => encode_record(&record, out, pending),
        Value::List(list) => {
            out.push(b'l');
            write_count(out, list.len());
            pending.extend(list.values().iter().rev().cloned());
        }
        Value::Fn(closure) => {
            out.push(b'F');
            out.extend_from_slice(&(Rc::as_ptr(&closure.code) as usize).to_le_bytes());
            out.extend_from_slice(&closure.function.index().to_le_bytes());
            write_count(out, closure.captures.len());
            pending.extend(closure.captures.iter().rev().cloned());
        }
        Value::Future(future) => encode_future(&future, out, pending),
        Value::Builtin(builtin) => {
            out.push(b'B');
            write_text(out, builtin.name());
        }
        Value::Tag(tag) => {
            out.push(b't');
            out.extend_from_slice(&tag.key.index().to_le_bytes());
        }
        Value::Variant(variant) => {
            out.push(b'v');
            out.extend_from_slice(&variant.tag.key.index().to_le_bytes());
            pending.push(variant.payload.clone());
        }
        Value::Secret(secret) => {
            out.push(b'S');
            out.push(u8::from(secret.revealed));
            write_text(out, &secret.identity);
        }
        Value::File(file) => {
            out.push(b'p');
            write_text(out, &file.path);
            write_text(out, file.identity.as_deref().unwrap_or(""));
        }
        Value::Dir(dir) => {
            out.push(b'd');
            write_text(out, &dir.path);
            write_text(out, dir.identity.as_deref().unwrap_or(""));
        }
        Value::Sandbox(sandbox) => {
            out.push(b'x');
            write_text(out, &sandbox.root);
        }
    }
}

/// Appends the length of `text` and `text` to `out`.
fn write_text(out: &mut Vec<u8>, text: &str) {
    write_count(out, text.len());
    out.extend_from_slice(text.as_bytes());
}

/// Appends `count` to `out`, as four bytes.
fn write_count(out: &mut Vec<u8>, count: usize) {
    out.extend_from_slice(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
}

/// Appends a record's part of the encoding to `out`, its fields' names
/// sorted, and pushes its fields' values onto `pending`, for [`encode`].
fn encode_record(record: &crate::value::Record, out: &mut Vec<u8>, pending: &mut Vec<Value>) {
    out.push(b'r');
    let mut fields: Vec<(&str, &Value)> = record
        .layout
        .texts
        .iter()
        .map(|name| &**name)
        .zip(record.values.iter())
        .collect();
    fields.sort_by_key(|&(name, _)| name);
    write_count(out, fields.len());
    for &(name, _) in &fields {
        write_text(out, name);
    }
    pending.extend(fields.into_iter().rev().map(|(_, value)| value.clone()));
}

/// Appends a future's part of the encoding to `out`, and pushes what it
/// holds onto `pending`, for [`encode`]: a step's by its node, whether the
/// host has given it or not, so that a call given it keys the same on an
/// attempt after the host gave it as before; one run by its value; one not
/// run by its code and captures.
fn encode_future(future: &Future, out: &mut Vec<u8>, pending: &mut Vec<Value>) {
    if let Some(node) = future.step_node() {
        out.push(b'H');
        out.extend_from_slice(&node.to_le_bytes());
    } else if let Some(done) = future.value() {
        out.push(b'D');
        pending.push(done);
    } else {
        out.push(b'P');
        out.extend_from_slice(&(Rc::as_ptr(&future.code) as usize).to_le_bytes());
        out.extend_from_slice(&future.function.index().to_le_bytes());
        if let Some(captures) = future.with_captures(<[Value]>::to_vec) {
            write_count(out, captures.len());
            pending.extend(captures.into_iter().rev());
        }
    }
}

/// Calls `visit` on every future `values` hold, through the values of those
/// that have run, each once, walking a work list rather than recursing, and
/// each shared record, list, closure or future once.
fn futures_in(values: &[Value], mut visit: impl FnMut(&Future)) {
    let mut pending: Vec<Value> = values.to_vec();
    let mut seen: HashSet<usize> = HashSet::new();
    while let Some(value) = pending.pop() {
        match value {
            Value::Record(record) => {
                if seen.insert(Rc::as_ptr(&record).cast::<()>() as usize) {
                    pending.extend(record.values.iter().cloned());
                }
            }
            Value::List(list) => pending.extend(list.values().iter().cloned()),
            Value::Variant(variant) => pending.push(variant.payload.clone()),
            Value::Secret(secret) => pending.push(secret.value.clone()),
            Value::Fn(closure) => {
                if seen.insert(Rc::as_ptr(&closure).cast::<()>() as usize) {
                    pending.extend(closure.captures.iter().cloned());
                }
            }
            Value::Future(future) => {
                if !seen.insert(Rc::as_ptr(&future).cast::<()>() as usize) {
                    continue;
                }
                visit(&future);
                if let Some(done) = future.value() {
                    pending.push(done);
                } else if let Some(captures) = future.with_captures(<[Value]>::to_vec) {
                    pending.extend(captures);
                }
            }
            _ => {}
        }
    }
}
