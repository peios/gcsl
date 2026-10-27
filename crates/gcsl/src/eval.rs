//! The evaluator: a recursive walk over [`Ir`].
//!
//! There is one walker per kind of value (see [`crate::value::ValueKind`])
//! but lists: `Evaluator::int`, `Evaluator::float`, `Evaluator::bool` and
//! `Evaluator::string`, returning an `i64`, an `f64`, a `bool` and an
//! `Rc<str>`, and `Evaluator::record`, returning a [`Value`] that is
//! [`Value::Empty`] or [`Value::Record`], since both are records. Every IR
//! node states its kind, so the walker for a node is always known from its
//! parent and no runtime value is ever inspected to choose an operation.
//! `Evaluator::function` returns the closure of a function value. Each
//! typed walker handles the nodes of its kind and also loads, captures,
//! blocks, `if`s, field accesses, indexes, calls, awaits and `as`es, which
//! can be of any kind, and `break`s and `continue`s, which yield none (see
//! [Stops](crate::eval::loops#stops)): an `if` evaluates its condition through `bool` and
//! then exactly one branch through the walker it was called with, and a
//! field access, an index, a call, an await or an `as` produces a [`Value`]
//! from which the walker extracts its kind's, the walkers sharing that
//! handling (`Evaluator::shared_as`). A duration and a size, whose values
//! are tagged with their quantity, have no walker of their own: the `Int`
//! walker takes their amounts, by a node's kind, for the `Int` operators
//! that compute on them, and the general walker tags the amount of a
//! [`IrKind::Quantity`] node (see [`crate::ir`], "Quantities"). An overflow
//! in a quantity's own operator is "duration overflow" or "size overflow",
//! and an operator's error labels a quantity operand with its literal,
//! `30s`. A general walker,
//! `Evaluator::value`, handles every node, and hands the typed nodes to the
//! typed walkers, building a [`Value`] only where a value of statically
//! unknown kind is needed: at the root, in a slot, in a record, in a list,
//! in a closure's or a future's captures, as an interpolated value, as a
//! block's or a function body's result evaluated through `value`, where a
//! list is needed (see [Lists](crate::eval::lists#lists)), where the value may be a future
//! (see [Futures](crate::eval::futures#futures)), or where it is of a union of kinds (see
//! [Unions](crate::eval::unions#unions)).
//!
//! A record is built from its record block's shared [`crate::value::Layout`]
//! and its field values in layout order, the one allocation for the values
//! and one for the reference-counted record. The empty record is unboxed,
//! so the statements that produce it allocate nothing. Reading a field finds
//! it by comparing interned names against the layout's.
//!
//! A spread item needs nothing of its own: lowering makes it a store of its
//! operand into a slot and, for each field of the operand's static type, a
//! store of a field access into that slot's record (see [`crate::lower`]),
//! so the operand is evaluated, and awaited, once, each field is found by
//! name whatever the record's layout, a field the static type hides is
//! never read, and a field's value is shared with the operand's record,
//! not copied.
//!

use std::cell::{Cell, RefCell};
use std::num::NonZeroI64;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::check::{Builtin, Slot};
use crate::diagnostic::Diagnostic;
use crate::ir::{CatchId, Ir, IrId, IrKind, IrNode, IrRange, ListShape, RestShape, ShapeId};
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::ValueTest;
use crate::value::{
    Caller, Closure, ContextFrame, Layout, List, Quantity, Record, Tag, Value, ValueKind, Variant,
};
use crate::MAX_DEPTH;

pub use memo::{MemoCheckpoint, MemoTables};
use memo::NO_ORIGIN;
use kinds::{Extract, Taken, mixed};
use unions::{other_fields, with_fields};
use builtins::described;
pub use effects::{Direct, Effects};
pub use files::fetched_name;
use graph::GraphRun;
pub use system::TempDirs;
pub use text::compare_versions;

/// The room an interpolation reserves for each interpolated value's text,
/// on top of its literal pieces' lengths.
const INTERPOLATED_VALUE_ESTIMATE: usize = 8;

/// The most elements a list can hold: as many as fit an allocation of
/// `isize::MAX` bytes, the most the allocator can be asked for. It is not a
/// policy: how large a list a program builds is the program's business,
/// and one too large for the machine fails as any allocation does. A range
/// used as a value of more integers than this is the runtime error "range
/// too large", since its list cannot exist; a `for` directly over a range
/// counts its integers without building the list.
pub const MAX_LIST_LEN: usize = (isize::MAX as usize) / std::mem::size_of::<Value>();

/// The most elements a collecting `for` reserves up front for its values,
/// before they come: enough that a loop of ordinary size allocates once,
/// and few enough that one over a huge range grows as its values come
/// rather than reserve for all of them first.
const FOR_RESERVE: usize = 16_777_216;

/// Evaluates `ir` to a value, on a fresh stack.
///
/// # Errors
///
/// Returns a runtime error at the failing operator's span: "integer
/// overflow" for `Int` arithmetic that leaves the range of `i64`, and at a
/// call's for `int` of a `Float` whose truncation does, or "cannot truncate
/// NaN to an Int" for NaN; and
/// "division by zero" for `//` or `%` by zero; at an index's span, or at
/// the path of an update up to its index segment, "index out of range" for
/// a position outside its list; at a range's span,
/// "range too large" for a range used as a value of more integers than a
/// list can hold ([`MAX_LIST_LEN`]); and at a call's or an
/// await's span, "call nested too deeply" for a call, or an await running a
/// future, that would take the program's nesting, calls included, past
/// [`MAX_DEPTH`] levels (see the module documentation). An error inside a
/// future's body is reported where it happens, as one inside a function
/// is. A call of a memoised function that is running with the same
/// arguments further up the stack is "this call depends on its own result",
/// at the call. At a call's span: a call of `exec` with an empty list is
/// "`exec` needs a program to run", and one whose program cannot be started
/// "cannot run `name`", with the OS's reason as a note; a call of `panic`
/// is its message; a call of the file system's built-ins with a relative
/// path is "`fs.read` needs an absolute path, found `x.txt`", and one whose
/// operation fails names it, its paths and the OS's reason, "`fs.read`
/// could not read `/a`: REASON"; and a call of `show` of a value holding
/// NaN or an infinity is "`show` cannot render `nan` as source" (see [The
/// built-in functions](crate::eval::calls#the-built-in-functions)). `Float` arithmetic never
/// fails. A
/// run through this function has no standard library, and so no `exec` to
/// call, nor any owner of temporary directories.
pub fn eval(ir: &Rc<Ir>) -> Result<Value, Diagnostic> {
    // The top level's frame; calls grow the stack beyond it.
    let mut stack = vec![Value::Empty; ir.frame_size() as usize];
    eval_in(ir, &mut stack)
}

/// Evaluates `ir` to a value with an existing `stack`, as [`eval_with`]
/// does, with memo tables of its own, dropped when it returns.
///
/// # Errors
///
/// As [`eval_with`].
pub fn eval_in(ir: &Rc<Ir>, stack: &mut Vec<Value>) -> Result<Value, Diagnostic> {
    let direct = Direct { temp_dirs: None };
    eval_with(ir, stack, &mut MemoTables::new(), (None, &[], &direct))
}

/// Evaluates `ir` to a value with an existing `stack`, which must hold at
/// least [`Ir::frame_size`] values: the top level's frame, at its bottom,
/// and existing memo tables, `memo`, in which every call of a memoised
/// function is looked up and recorded (see [Memoised
/// functions](crate::eval::memo#memoised-functions)). Slots the program does not write keep
/// their values, and the tables keep their entries, which is how a
/// [`crate::session::Session`] keeps its variables, and what its memoised
/// functions have returned, from one input to the next. On return, whether
/// evaluation succeeded or failed, the stack is truncated to the frame
/// size, dropping whatever calls left above it; its capacity is kept, so a
/// later evaluation's calls need not allocate it again. The tables hold no
/// running call on return.
///
/// `exec_result` is the layout of the record `exec` returns, which the
/// driver of the run built once, its field names interned in the run's
/// names (see [The built-in functions](crate::eval::calls#the-built-in-functions)): `None`
/// for a run without the standard library, which has no `exec` to call.
/// `dirs` is the run's table of directories, which the driver built, by
/// which the code of each file names its directory ([`Ir::dir`]): what
/// `fileroot.caller` reads (see [Callers and helpers](crate::eval::callers#callers-and-helpers)).
/// The top level's caller is its own file's directory. `effects` is how
/// the run's effects reach the system (see
/// [Effects](crate::eval::effects#effects)): [`Direct`] for plain GCSL, whose
/// temporary directories, which `fs.temp_dir` makes, the owner of the run,
/// a workspace or a session, holds for as long as it lives and removes as
/// it is dropped; in a run without an owner, a call of `fs.temp_dir` is an
/// internal error.
///
/// # Errors
///
/// As [`eval`]; and an internal error if `stack` is too small.
pub fn eval_with(
    ir: &Rc<Ir>,
    stack: &mut Vec<Value>,
    memo: &mut MemoTables,
    (exec_result, dirs, effects): (Option<&Rc<Layout>>, &[Rc<str>], &dyn Effects),
) -> Result<Value, Diagnostic> {
    let extras = graph::RunExtras {
        exec_result,
        dirs,
        effects,
        names: None,
        profile: None,
    };
    eval_rooted(ir, (stack, memo), extras, (None, None))
}

/// [`eval_with`], in the root context `context`, the frame of the contexts
/// a host sets for the evaluation, which every frame of it inherits (see
/// [Context](crate::eval::context#context)), `None` for none; and as the run
/// of a unit in graph mode, `graph`, if it is one (see [Graph
/// mode](crate::eval::graph#graph-mode)), whose stop for a step's result is
/// the error marked [`Mark::PENDING`](crate::diagnostic::Mark::PENDING).
pub(crate) fn eval_rooted(
    ir: &Rc<Ir>,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    extras: graph::RunExtras<'_>,
    (context, graph): (Option<Rc<ContextFrame>>, Option<&GraphRun<'_>>),
) -> Result<Value, Diagnostic> {
    let root = ir.root();
    let frame_size = ir.frame_size() as usize;
    if stack.len() < frame_size {
        return Err(internal(
            "frame is smaller than the IR needs",
            ir.get(root).span,
        ));
    }
    let caller = Caller {
        dir: ir.dir(),
        helper: false,
        origin: NO_ORIGIN,
    };
    let mut shared = extras.shared();
    shared.graph = graph;
    shared.context.replace(context);
    // Graph mode runs on the machine, which can stop where it waits;
    // plain evaluation never waits, so the walkers run it, faster.
    let result = if graph.is_some() || cfg!(feature = "machine-everywhere") {
        let own = graph.and_then(GraphRun::own);
        machine::run_machine(ir, (stack, memo), &shared, (caller, own))
    } else {
        Evaluator {
            ir,
            stack,
            base: 0,
            captures: &[],
            callee: None,
            memo,
            shared: &shared,
            caller,
            levels: 0,
        }
        .value(root)
    };
    stack.truncate(frame_size);
    // The checker lets no `break` or `continue` out of the top level's
    // loops, so a stop arriving here is a bug.
    result.map_err(|unwind| unwind.into_error(ir.get(root).span))
}

/// Why the evaluation of a node ended without a value: the one road out of
/// a node, which every walker passes along with `?` (see [Stops](crate::eval::loops#stops)).
#[derive(Debug)]
enum Unwind {
    /// A runtime error, or an internal one, which ends the evaluation.
    Error(Diagnostic),
    /// A `break`, a `continue` or a `return` on its way to what it is for,
    /// which catches it, after which evaluation goes on.
    Stop(Stop),
}

impl From<Diagnostic> for Unwind {
    fn from(error: Diagnostic) -> Self {
        Unwind::Error(error)
    }
}

impl Unwind {
    /// The error this is, where nothing may catch a stop: a stop is an
    /// internal error at `span`, the boundary it reached.
    fn into_error(self, span: Span) -> Diagnostic {
        match self {
            Unwind::Error(error) => error,
            Unwind::Stop(Stop {
                kind: StopKind::Return,
                ..
            }) => internal("a `return` left its function", span),
            Unwind::Stop(_) => internal("a `break` or `continue` left its function", span),
        }
    }

    /// What this, which came out of a body that ran in a frame of its own,
    /// is to its call, memoised call or await (see [Stops](crate::eval::loops#stops)): the
    /// value of a `return` for the function or `lazy`, `{}` for one
    /// without; and otherwise the error this is, a stop being an internal
    /// error at `span`, the boundary it reached. The one place a
    /// function-level `return` is caught, on the error road only, as
    /// [`Result::or_else`] of a body's result, so that a body that gives a
    /// value takes exactly the road it took before `return` existed.
    fn returned(self, span: Span) -> Result<Value, Diagnostic> {
        match self {
            Unwind::Stop(Stop {
                kind: StopKind::Return,
                to,
                value,
            }) if to == CatchId::FUNCTION.number() => Ok(value.unwrap_or(Value::Empty)),
            unwind => Err(unwind.into_error(span)),
        }
    }
}

/// A `break`, a `continue` or a `return` unwinding to what it is for, with
/// its value, if it has one: for a `break` or a `continue`, its loop,
/// counted outwards from the innermost loop whose body it is unwinding out
/// of; for a `return`, the catch it is for, by number, or the function or
/// `lazy` being run. Both are named by one `u32`, so that a stop is as
/// small as it was before `return` existed, and a loop handles a `break` or
/// `continue` exactly as it did.
#[derive(Debug)]
struct Stop {
    /// For a `break` or a `continue`, how many loops it leaves before it
    /// reaches its own: 0 when the loop whose body it comes out of is its
    /// target, each loop it passes through taking one off and dropping its
    /// own list. For a `return`, the [`CatchId`] it is for, which every loop
    /// and catch it passes through leaves as it is.
    to: u32,
    /// What it does when it gets there.
    kind: StopKind,
    /// The value of its expression, which the target loop collects, or
    /// which is returned, if it has one.
    value: Option<Value>,
}

/// What a [`Stop`] does at what it is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopKind {
    /// `break`: ends the loop, after collecting the value, if any.
    Break,
    /// `continue`: ends the iteration, after collecting the value, if any,
    /// and goes on with the after-items and the next test.
    Continue,
    /// `return`: ends the labelled block or loop whose catch it is for, or
    /// with [`CatchId::FUNCTION`] the body of the function or `lazy` being
    /// run, which evaluates to the value, `{}` without one.
    Return,
}

/// What evaluating a node gives: its value, or why there is none.
type Flow<T> = Result<T, Unwind>;

/// What a call calls, as the function walker finds it ([`Evaluator::function`]):
/// a closure, or a built-in function called through a value of its function
/// type, which [`Evaluator::call_builtin`] applies.
enum Callee {
    /// A closure.
    Closure(Rc<Closure>),
    /// A built-in function.
    Builtin(Builtin),
    /// A variant of an enum, which a call of makes a value of its enum of
    /// its argument (see the checker's "Enums").
    Tag(Rc<Tag>),
}

/// What every frame of an evaluation shares besides the value stack and the
/// memo tables, which the driver of the run built once (see [`eval_with`]).
#[derive(Debug)]
struct Shared<'a> {
    /// The layout of the record `exec` returns; `None` in a run without
    /// the standard library.
    exec_result: Option<&'a Rc<Layout>>,
    /// The run's table of directories, by which a caller names its
    /// directory (see [Callers and helpers](crate::eval::callers#callers-and-helpers)); empty in
    /// a run with no workspace's context, whose code cannot read
    /// `fileroot.caller`.
    dirs: &'a [Rc<str>],
    /// How the run's effects reach the system (see
    /// [Effects](crate::eval::effects#effects)).
    effects: &'a dyn Effects,
    /// The run of a unit in graph mode, if it is one (see [Graph
    /// mode](crate::eval::graph#graph-mode)).
    graph: Option<&'a GraphRun<'a>>,
    /// The deadlines of the calls with `#[timeout]` that are running,
    /// outermost first (see [Retry and timeout](crate::eval::retry#retry-and-timeout)).
    deadlines: RefCell<Vec<Deadline>>,
    /// The innermost deadline's effective instant, the earliest of all,
    /// while one is running: what every check reads first.
    earliest: Cell<Option<Instant>>,
    /// The number of the last deadline made.
    deadlines_made: Cell<u32>,
    /// The context the frame running is evaluated in, innermost frame first
    /// (see [Context](crate::eval::context#context)): `None`, binding nothing, at the root.
    context: RefCell<Option<Rc<ContextFrame>>>,
    /// The message of the failure each `recover` running its fallback
    /// caught, innermost last (see [Failure](crate::eval::failure#failure)).
    failures: RefCell<Vec<Rc<str>>>,
    /// The identities of the secrets the operations on secrets running have
    /// computed on so far, innermost last (see [Secrets](crate::eval::secrets#secrets)).
    identities: RefCell<Vec<Rc<str>>>,
    /// Whether the failure unwinding to the innermost operation on secrets
    /// is one of its operands', not its own (see [Secrets](crate::eval::secrets#secrets)).
    operand_failed: Cell<bool>,
    /// The calls from the workspace's own code into foreign code that are
    /// running, innermost last, each its input and its span: where a step
    /// that foreign code makes reports its failure (see [Errors in a
    /// library](crate::eval::calls#errors-in-a-library)).
    sites: RefCell<Vec<(u32, Span)>>,
    /// The names of the run's owner, which a record made with names known
    /// only as the run goes adds to (see [Records of any
    /// names](crate::eval::records#records-of-any-names)); `None` in a run
    /// with no owner's names, where making one is an error.
    names: Option<&'a RefCell<crate::names::Names>>,
    /// The profile being taken of the run, if it is profiled (see
    /// [Profiling](crate::eval::profile#profiling)).
    profile: Option<&'a profile::Profile>,
    /// A machine's deep await through a walker (see [`Ahead`]).
    ahead: Ahead,
}

/// A machine's deep await through a walker, which leaves the futures it
/// meets that have not run to the machine (see
/// [Suspension](crate::eval::machine#suspension)).
#[derive(Debug, Default)]
struct Ahead {
    /// Whether one is awaiting so.
    awaiting: Cell<bool>,
    /// The future the walker met that has not run, and the level of
    /// nesting it is at.
    met: RefCell<Option<(Rc<crate::value::Future>, usize)>>,
}

impl<'a> Shared<'a> {
    /// What the frames of a run with the layout of `exec`'s result
    /// `exec_result`, the table of directories `dirs` and the effect context
    /// `effects` share, with no deadline running.
    fn new(
        exec_result: Option<&'a Rc<Layout>>,
        dirs: &'a [Rc<str>],
        effects: &'a dyn Effects,
    ) -> Self {
        Self {
            exec_result,
            dirs,
            effects,
            graph: None,
            deadlines: RefCell::new(Vec::new()),
            earliest: Cell::new(None),
            deadlines_made: Cell::new(0),
            context: RefCell::new(None),
            failures: RefCell::new(Vec::new()),
            identities: RefCell::new(Vec::new()),
            operand_failed: Cell::new(false),
            sites: RefCell::new(Vec::new()),
            names: None,
            profile: None,
            ahead: Ahead::default(),
        }
    }
}

/// The deadline of a running call of a function with `#[timeout]` (see
/// [Retry and timeout](crate::eval::retry#retry-and-timeout)).
#[derive(Debug)]
struct Deadline {
    /// Its number, unique in the run: what the error of its passing is
    /// marked with ([`ErrorMark::deadline`](crate::diagnostic::Mark::deadline)).
    number: u32,
    /// When its own limit passes.
    own: Instant,
    /// When it or an enclosing one passes, whichever is first.
    effective: Instant,
    /// The error of its passing: "`f` timed out after 30s".
    message: Rc<str>,
}

/// The state of one frame's evaluation: the top level's, or a call's.
struct Evaluator<'a> {
    /// The IR being run: the program's, or the running closure's.
    ir: &'a Rc<Ir>,
    /// The value stack, holding every frame.
    stack: &'a mut Vec<Value>,
    /// Where this frame starts on the stack: slot `n` is `stack[base + n]`.
    base: usize,
    /// The running closure's captured values; none at the top level.
    captures: &'a [Value],
    /// The running closure; `None` at the top level.
    callee: Option<&'a Rc<Closure>>,
    /// The execution context's memo tables, shared by every frame.
    memo: &'a mut MemoTables,
    /// What the run's driver built once for every frame, by one reference,
    /// so that a call's frame grows by no more than one.
    shared: &'a Shared<'a>,
    /// This frame's caller, its helper flag, and the caller's origin (see
    /// [Callers and helpers](crate::eval::callers#callers-and-helpers)).
    caller: Caller,
    /// The levels of nesting the enclosing frames have consumed where this
    /// frame's body starts: 0 at the top level. See the module
    /// documentation.
    levels: usize,
}

// The walkers `value`, `int`, `float`, `bool`, `string` and `record` are
// the recursion of evaluation (a list is evaluated through `value`, and
// `make_list`, `index`, `list`, `for_loop`, `update` and `walk_update` are
// helpers on its recursion path, which is why they hold little): an
// expression nested N deep puts
// about N of their frames on the stack. Each is therefore only a
// dispatcher. The work
// of each operator is in a helper of its own, and every error is returned
// by a call in tail position, so a walker's frame does not hold the
// temporaries of every operator; an unoptimised build gives each of those
// a stack slot of its own, which would make the walkers' frames several
// kilobytes and `crate::STACK_SIZE` too small at `crate::MAX_DEPTH`.
impl Evaluator<'_> {
    /// Evaluates a node of any kind.
    fn value(&mut self, id: IrId) -> Flow<Value> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty => Ok(Value::Empty),
            IrKind::Load(slot, _) => self.load_value(slot, node),
            IrKind::Store(slot, value) => self.store(slot, value, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.value(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.value(branch)
            }
            IrKind::MakeRecord { layout, values } => self.make_record(layout, values),
            IrKind::RecordOf { .. } => self.record_of(node),
            IrKind::Field { record, name, .. } => self.field(record, name, node),
            IrKind::UpdateField { .. } => self.update_field(node),
            IrKind::Update { .. } => self.update(node),
            IrKind::LoadCapture(index, _) => self.capture_value(index, node),
            IrKind::Top(slot, ValueKind::Fn) => self.top_function_value(slot, node),
            IrKind::Top(slot, kind) => self.top_value(slot, kind, node),
            IrKind::LoadCallee | IrKind::MakeClosure { .. } => self.closure_value(node),
            IrKind::Tag { key, of, shown } => Ok(Value::Tag(self.tag((key, of, shown)))),
            IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..) => self.whole_node(node),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
            IrKind::Meta(function) => self.meta_value(function, node),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call((callee, args, depth), node),
            IrKind::MakeFuture {
                function,
                captures,
                reads_caller,
            } => self.make_future(function, captures, reads_caller),
            IrKind::Await { .. } => self.await_node(node),
            IrKind::As { .. } => self.narrow_node(node),
            IrKind::JsonDecode { .. } => self.decode_node(node),
            IrKind::Arg { .. } => mistyped(node, "an argument outside a call"),
            IrKind::MakeList { items, spreads } => self.make_list(items, spreads),
            IrKind::Range(inclusive, operands) => self.range(operands, inclusive, node),
            IrKind::For { .. } | IrKind::Loop { .. } => self.looped(node),
            IrKind::Break { .. } | IrKind::Continue { .. } | IrKind::Return { .. } => {
                self.leave(node)
            }
            IrKind::Catch { .. } => self.catch(node),
            IrKind::Index { list, index, .. } => self.index((list, index), node),
            IrKind::Spread(_) => mistyped(node, "a spread outside a list"),
            IrKind::Quantity(quantity, amount) => self.quantity_value(quantity, amount),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => self.int(id).map(Value::Int),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => self.float(id).map(Value::Float),
            IrKind::BoolConst(_)
            | IrKind::IntCmp(..)
            | IrKind::FloatCmp(..)
            | IrKind::StrCmp(..)
            | IrKind::BoolEq(..)
            | IrKind::RecordEq(_)
            | IrKind::RecordNe(_)
            | IrKind::RecordSame(_)
            | IrKind::RecordNotSame(_)
            | IrKind::ListEq(_)
            | IrKind::ListNe(_)
            | IrKind::ListSame(_)
            | IrKind::ListNotSame(_)
            | IrKind::ValueSame(_)
            | IrKind::ValueNotSame(_)
            | IrKind::Is { .. }
            | IrKind::And(_)
            | IrKind::Or(_)
            | IrKind::Not(_) => self.bool(id).map(Value::Bool),
            IrKind::StrConst(_)
            | IrKind::StrConcat(_)
            | IrKind::Show(_)
            | IrKind::Caller
            | IrKind::FilePath(_) => self.string(id).map(Value::Str),
            IrKind::Helper => self.record(id),
            // Directly rather than through `string`: an interpolation
            // evaluates its parts through `value`, so interpolations nested
            // in each other then recurse two frames per level, not three.
            IrKind::Interp(items) => self.interpolate(items, node.span).map(Value::Str),
        }
    }

    /// Evaluates a block's items, in order, for their effects.
    fn items(&mut self, items: IrRange) -> Flow<()> {
        for &item in self.ir.items(items) {
            self.value(item)?;
        }
        Ok(())
    }

    /// Evaluates the condition of an `if` and returns the branch it selects.
    fn branch(&mut self, cond: IrId, then: IrId, else_: IrId) -> Flow<IrId> {
        Ok(if self.bool(cond)? { then } else { else_ })
    }

    /// Evaluates `value` and writes it to `slot`, for the `Store` `node`.
    fn store(&mut self, slot: Slot, value: IrId, node: &IrNode) -> Flow<Value> {
        let value = self.value(value)?;
        let target = self
            .stack
            .get_mut(self.base + slot.index())
            .ok_or_else(|| internal("store to a slot outside the frame", node.span))?;
        *target = value;
        Ok(Value::Empty)
    }

    /// The captured value number `index` of the running closure, for the
    /// `LoadCapture` `node`.
    fn capture(&self, index: u32, node: &IrNode) -> Flow<&Value> {
        self.captures
            .get(index as usize)
            .ok_or_else(|| internal("load of a capture the closure lacks", node.span).into())
    }

    /// A copy of the value in `slot`, of any kind, for the `Load` `node`.
    fn load_value(&self, slot: Slot, node: &IrNode) -> Flow<Value> {
        self.load(slot, node).cloned()
    }

    /// A copy of the captured value number `index`, of any kind, for the
    /// `LoadCapture` `node`.
    fn capture_value(&self, index: u32, node: &IrNode) -> Flow<Value> {
        self.capture(index, node).cloned()
    }

    /// The value in `slot`, read by the `Load` `node` as `extract` accepts
    /// it. The walkers read slots through this and `capture_as`, returning
    /// the call, so that they hold no temporaries for the reads.
    fn slot_as<T>(&self, slot: Slot, node: &IrNode, extract: Extract<T>) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        extract(self.load(slot, node)?, node)
    }

    /// The captured value number `index`, read by the `LoadCapture` `node`
    /// as `extract` accepts it.
    fn capture_as<T>(&self, index: u32, node: &IrNode, extract: Extract<T>) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        extract(self.capture(index, node)?, node)
    }

    /// The closure the `LoadCallee` or `MakeClosure` `node` yields, as a
    /// value.
    fn closure_value(&mut self, node: &IrNode) -> Flow<Value> {
        match node.kind {
            IrKind::LoadCallee => self.callee(node).map(Value::Fn),
            IrKind::MakeClosure { function, captures } => {
                self.make_closure(function, captures).map(Value::Fn)
            }
            _ => mistyped(node, "a node that is not a closure where one was expected"),
        }
    }

    /// The closure of the top-level function in `slot` of the top level's
    /// frame, which is at the bottom of the value stack (frame base 0), for
    /// the `Fn` `node`, or the built-in function a cell there holds. A slot
    /// that holds neither is an internal error: the checker's rule on when a
    /// function may be used means it never happens. Never inlined, so that
    /// the frames of the recursive walkers that read one stay small.
    #[inline(never)]
    fn top_function_value(&self, slot: Slot, node: &IrNode) -> Flow<Value> {
        match self.top_function(slot, node)? {
            Callee::Closure(closure) => Ok(Value::Fn(closure)),
            Callee::Builtin(builtin) => Ok(Value::Builtin(builtin)),
            Callee::Tag(tag) => Ok(Value::Tag(tag)),
        }
    }

    /// A copy of the value in the cell `slot` of the top level's frame, of
    /// kind `kind`, for the `Top` `node`: a top-level field read from inside
    /// a function or `lazy` of its input. A cell not computed yet holds
    /// `Value::Empty`, which is a valid `{}`; read as anything that cannot be
    /// `{}`, that is an internal error, which the checker's schedule rules
    /// out (see [`crate::check`]).
    fn top_value(&self, slot: Slot, kind: ValueKind, node: &IrNode) -> Flow<Value> {
        match self.stack.get(slot.index()) {
            Some(Value::Empty)
                if !matches!(
                    kind,
                    ValueKind::Record | ValueKind::Mixed | ValueKind::Deferred
                ) =>
            {
                Err(not_computed(node).into())
            }
            Some(value) => Ok(value.clone()),
            None => Err(not_computed(node).into()),
        }
    }

    /// The value in the cell `slot` of the top level's frame, read by the
    /// `Top` `node` as `extract` accepts it, as [`Evaluator::slot_as`] reads
    /// a slot of the current frame.
    fn top_as<T>(&self, slot: Slot, node: &IrNode, extract: Extract<T>) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match self.stack.get(slot.index()) {
            Some(value) => extract(value, node),
            None => Err(not_computed(node).into()),
        }
    }

    /// The closure of the top-level function in `slot`, or the built-in
    /// function in that cell, as [`Evaluator::top_function_value`] gives it
    /// as a value.
    /// Inlined where a callee is evaluated, as a capture's read is, so that
    /// a call through it costs no more than one through a capture; the
    /// error, which the checker rules out, is out of line.
    #[inline]
    fn top_function(&self, slot: Slot, node: &IrNode) -> Flow<Callee> {
        match self.stack.get(slot.index()) {
            Some(Value::Fn(closure)) => Ok(Callee::Closure(Rc::clone(closure))),
            // A cell of the top level holding a built-in function: the
            // prelude's `int`, say, read from inside a function.
            Some(&Value::Builtin(builtin)) => Ok(Callee::Builtin(builtin)),
            // A variant a top-level field holds.
            Some(Value::Tag(tag)) => Ok(Callee::Tag(Rc::clone(tag))),
            _ => Err(not_made(node).into()),
        }
    }

    /// Evaluates `node`, one each walker evaluates as a whole value: a read
    /// of a context or a `with` (see [Context](crate::eval::context#context)), an assertion, a
    /// `recover` or a failure (see [Failure](crate::eval::failure#failure)).
    fn whole_node(&mut self, node: &IrNode) -> Flow<Value> {
        match node.kind {
            IrKind::ContextRead {
                key,
                default,
                depth,
                ..
            } => self.context_read(key, (default, depth), node),
            IrKind::With { parts, .. } => self.with(parts, node),
            IrKind::Assert { cond, message } => self.assert((cond, message), node),
            IrKind::Recover { body, fallback, .. } => self.recover(body, fallback),
            IrKind::Failure(layout) => self.failure(layout, node),
            IrKind::SecretValue { value, deep, .. } => self.secret_value(value, deep),
            IrKind::SecretWrap(computed, revealed) => self.secret_wrap(computed, revealed, node),
            _ => mistyped(node, "a node evaluated whole that is none"),
        }
    }

    /// The value in `slot` of this frame.
    fn load(&self, slot: Slot, node: &IrNode) -> Flow<&Value> {
        self.stack
            .get(self.base + slot.index())
            .ok_or_else(|| internal("load from a slot outside the frame", node.span).into())
    }

    /// Evaluates `node` for a typed walker that takes the values of its kind
    /// as `taken` says. The node is a load, a capture or top-level read, a
    /// call, a field access, an index, an await, an `as` or a `lazy`, any of
    /// whose values may be of any kind. This is the one place where the
    /// typed walkers evaluate such nodes. One of them that may be a future
    /// is an internal error, since only the general walker evaluates those;
    /// a `lazy` always is.
    fn shared_as<T>(&mut self, node: &IrNode, taken: &Taken<T>) -> Flow<T> {
        match node.kind {
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Index {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::ContextRead {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::With {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Recover {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::SecretValue {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, taken.deferred),
            IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(..) => match (taken.owned)(self.whole_node(node)?) {
                Some(value) => Ok(value),
                None => mistyped(node, "a value of another kind than its node"),
            },
            IrKind::Load(slot, _) => self.slot_as(slot, node, taken.borrowed),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, taken.borrowed),
            IrKind::Top(slot, _) => self.top_as(slot, node, taken.borrowed),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, taken.owned),
            IrKind::Field { record, name, .. } => self.field_as((record, name), node, taken.owned),
            IrKind::Index { list, index, .. } => self.index_as((list, index), node, taken.owned),
            IrKind::Await { .. } => self.await_as(node, taken.owned),
            IrKind::As { .. } => self.narrow_as(node, taken.owned),
            IrKind::JsonDecode { .. } => self.decode_as(node, taken.owned),
            _ => mistyped(
                node,
                "a node a typed walker evaluates itself, given to it shared",
            ),
        }
    }
}

const OVERFLOW: &str = "integer overflow";

/// The levels of [`MAX_DEPTH`]'s budget that a call of a function with
/// `#[retry]` or `#[timeout]` costs beyond any call: the frame of
/// [`Evaluator::guarded_body`] between the call and its body.
const GUARD_LEVELS: usize = 1;

/// The time `limit`, shown as the duration literal of it, `30s`.
fn shown_duration(limit: Duration) -> String {
    let nanoseconds = i64::try_from(limit.as_nanos()).unwrap_or(i64::MAX);
    Value::Quantity(Quantity::Duration, nanoseconds).to_string()
}
const DIVISION_BY_ZERO: &str = "division by zero";

// The internal errors of a typed walker meeting a node whose value may be a
// future, which only the general walker evaluates.
const DEFERRED_INT: &str = "a node that may be a future where an Int was expected";
const DEFERRED_FLOAT: &str = "a node that may be a future where a Float was expected";
const DEFERRED_BOOL: &str = "a node that may be a future where a Bool was expected";
const DEFERRED_STRING: &str = "a node that may be a future where a String was expected";
const DEFERRED_RECORD: &str = "a node that may be a future where a record was expected";
const DEFERRED_FUNCTION: &str = "a node that may be a future where a function was expected";

/// The operands of `kind`, if it is a comparison: what an assertion of it
/// shows (see [Failure](crate::eval::failure#failure)).
const fn comparison_operands(kind: IrKind) -> Option<(IrId, IrId)> {
    match kind {
        IrKind::IntCmp(_, o)
        | IrKind::FloatCmp(_, o)
        | IrKind::StrCmp(_, o)
        | IrKind::BoolEq(_, o)
        | IrKind::RecordSame(o)
        | IrKind::RecordNotSame(o)
        | IrKind::ListSame(o)
        | IrKind::ListNotSame(o)
        | IrKind::ValueSame(o)
        | IrKind::ValueNotSame(o) => Some((o.lhs, o.rhs)),
        IrKind::RecordEq(o) | IrKind::RecordNe(o) | IrKind::ListEq(o) | IrKind::ListNe(o) => {
            Some((o.lhs, o.rhs))
        }
        _ => None,
    }
}

/// The internal error of evaluating a binding of a `with` alone (see
/// [Context](crate::eval::context#context)).
const CONTEXT_BIND: &str = "a binding of a `with` evaluated alone";

/// The internal error of a typed walker meeting a node of a union whose
/// members are of different kinds, which only the general walker
/// evaluates (see [`mixed`]).
const MIXED: &str = "a node of a union of kinds where one kind was expected";

/// What a loop does with `unwind`, which came out of its body: if it is a
/// stop for this loop, one with no loops left to leave, its value, if any,
/// is collected into `values` when the loop `collect`s, and the result is
/// whether the loop ends there, as after a `break`, or goes on, as after a
/// `continue`. A stop for a loop further out goes on with one loop fewer to
/// leave, and a `return` and an error go on as they are, untouched, the
/// `return` for a catch or a call outside; this loop's list is dropped with
/// it.
fn caught(unwind: Unwind, values: &mut Vec<Value>, collect: bool) -> Flow<bool> {
    match unwind {
        Unwind::Stop(Stop {
            to: 0,
            kind: kind @ (StopKind::Break | StopKind::Continue),
            value,
        }) => {
            if collect && let Some(value) = value {
                values.push(value);
            }
            Ok(kind == StopKind::Break)
        }
        Unwind::Stop(Stop {
            to,
            kind: kind @ (StopKind::Break | StopKind::Continue),
            value,
        }) => Err(Unwind::Stop(Stop {
            to: to - 1,
            kind,
            value,
        })),
        other => Err(other),
    }
}

/// Whether a deep await replaced `old`, a record's field or a list's
/// element, with `new`: a future by what it yielded, or a record or list by
/// a new one with values inside it awaited. Awaiting anything else, or a
/// record or list with nothing to await inside, leaves it as it was.
fn replaced(old: &Value, new: &Value) -> bool {
    match (old, new) {
        (Value::Future(_), _) => true,
        (Value::Record(old), Value::Record(new)) => !Rc::ptr_eq(old, new),
        (Value::List(old), Value::List(new)) => !old.shares(new),
        _ => false,
    }
}

/// Floor division: the quotient rounded toward negative infinity. Returns
/// `None` on overflow, which only `i64::MIN // -1` causes.
///
/// Derivation. Rust's `/` truncates toward zero and `%` takes the sign of
/// the dividend, so `a == b * q + r` with `q = a / b`, `r = a % b` and
/// `|r| < |b|`. If `r == 0` the division is exact and `q` is already the
/// floor. Otherwise the exact quotient `a / b` lies strictly between `q` and
/// the next integer away from zero, and it is negative exactly when `r` and
/// `b` have opposite signs. In that case truncation rounded *up*, so the
/// floor is `q - 1`. When the signs agree the exact quotient is positive,
/// truncation rounded down, and `q` is the floor.
///
/// `q - 1` cannot overflow: it only happens when the exact quotient is
/// negative and not whole, so `q <= 0` and `q > i64::MIN`.
///
/// This is not `i64::div_euclid`, which rounds so that the remainder is
/// never negative, and so differs from floor division when `b < 0`
/// (`7.div_euclid(-2) == -3`, but `7 // -2 == -4`).
fn floor_div(a: i64, b: NonZeroI64) -> Option<i64> {
    let b = b.get();
    let q = a.checked_div(b)?;
    // Cannot wrap: `checked_div` has already rejected `i64::MIN / -1`.
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        Some(q - 1)
    } else {
        Some(q)
    }
}

/// The remainder matching [`floor_div`]: `a - b * floor_div(a, b)`, which
/// has the sign of the divisor `b` (or is zero).
///
/// Derivation. With `q`, `r` the truncating quotient and remainder as in
/// [`floor_div`], the floored quotient is `q - 1` exactly when `r != 0` and
/// `r`, `b` have opposite signs. Then the floored remainder is
/// `a - b * (q - 1) = (a - b * q) + b = r + b`, which has the sign of `b`
/// because `|r| < |b|`. Otherwise it is `r`, which is zero or already has
/// the sign of `b`. `r + b` cannot overflow, since `r` and `b` have opposite
/// signs.
///
/// Never overflows: `i64::MIN % -1` is mathematically `0`, which is what
/// `wrapping_rem` returns for it.
fn floor_mod(a: i64, b: NonZeroI64) -> i64 {
    let b = b.get();
    let r = a.wrapping_rem(b);
    if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    }
}

/// `value` with every secret in it its value, in a field, an element or a
/// payload at any depth (see [Secrets](crate::eval::secrets#secrets)):
/// what a comparison of values holding secrets compares. A value holding none is returned as it is,
/// sharing what it shares.
///
/// Recurses once per level of nesting in `value`, which is nested no
/// deeper than its type, which the checker recursed through already.
fn unsealed(value: Value) -> Value {
    if !holds_secret(&value) {
        return value;
    }
    match value {
        // A secret's value, never a secret, may hold one.
        Value::Secret(secret) => unsealed(secret.value.clone()),
        Value::Record(record) => Value::Record(Rc::new(Record {
            layout: Rc::clone(&record.layout),
            values: record
                .values
                .iter()
                .map(|field| unsealed(field.clone()))
                .collect(),
        })),
        Value::List(list) => Value::List(List::from_vec(
            list.values()
                .iter()
                .map(|element| unsealed(element.clone()))
                .collect(),
        )),
        Value::Variant(variant) => Value::Variant(Rc::new(Variant {
            tag: Rc::clone(&variant.tag),
            payload: unsealed(variant.payload.clone()),
        })),
        value => value,
    }
}

/// Whether `value` is or holds a secret, in a field, an element or a
/// payload at any depth.
fn holds_secret(value: &Value) -> bool {
    match value {
        Value::Secret(_) => true,
        Value::Record(record) => record.values.iter().any(holds_secret),
        Value::List(list) => list.values().iter().any(holds_secret),
        Value::Variant(variant) => holds_secret(&variant.payload),
        _ => false,
    }
}

/// Typed equality: whether records or lists `a` and `b` are equal over
/// shape `shape`. For a record's shape, that compares, for each field of
/// the shape, the two records' fields of that name, a record or list field
/// through its own shape and any other with `==` on [`Value`]; fields the
/// shape lacks are ignored, so a record with no fields in its shape is
/// equal to any other. For a list's shape, the two lists must have the same
/// length, and each pair of elements is compared through the element's
/// shape, or with `==` if it has none.
///
/// Nested records and lists are compared through a work list rather than
/// recursion, as [`Record`]'s total equality does, and the list only grows
/// when a shape has a record or list field or element, so comparing flat
/// records and lists allocates nothing.
///
/// `None` if a record lacks a field of its shape, or holds something other
/// than a record or a list where the shape has one: the checker guarantees
/// neither happens, so either is an internal error.
fn record_eq_shaped(ir: &Ir, shape: ShapeId, a: &Value, b: &Value) -> Option<bool> {
    let mut pending: Vec<(ShapeId, &Value, &Value)> = Vec::new();
    let (mut shape, mut a, mut b) = (shape, a, b);
    loop {
        let fields = &ir.shape(shape).fields;
        if let Some(ListShape { element }) = ir.list_shape(shape) {
            let (Value::List(a), Value::List(b)) = (a, b) else {
                return None;
            };
            if a.len() != b.len() {
                return Some(false);
            }
            for (mine, theirs) in a.values().iter().zip(b.values()) {
                match element {
                    Some(nested) => pending.push((nested, mine, theirs)),
                    None if mine == theirs => {}
                    None => return Some(false),
                }
            }
        } else if let Some(RestShape { shape: nested }) = ir.shape(shape).rest {
            // Both are records, the empty record with no fields to compare.
            if a.kind() != ValueKind::Record || b.kind() != ValueKind::Record {
                return None;
            }
            let (a, b) = (with_fields(a), with_fields(b));
            for &(name, nested) in fields {
                let (mine, theirs) = (a?.get(name)?, b?.get(name)?);
                match nested {
                    Some(nested) => pending.push((nested, mine, theirs)),
                    None if mine == theirs => {}
                    None => return Some(false),
                }
            }
            // The fields the type does not name: the same names on both
            // sides, each pair compared through the rest-field's shape.
            let named: Vec<Name> = fields.iter().map(|&(name, _)| name).collect();
            let others = |record: Option<&'_ Record>| {
                record.map_or(0, |record| other_fields(record, &named).count())
            };
            if others(a) != others(b) {
                return Some(false);
            }
            for (name, _, mine) in a.into_iter().flat_map(|a| other_fields(a, &named)) {
                let Some(theirs) = b.and_then(|b| b.get(name)) else {
                    return Some(false);
                };
                match nested {
                    Some(nested) => pending.push((nested, mine, theirs)),
                    None if mine == theirs => {}
                    None => return Some(false),
                }
            }
        } else if !fields.is_empty() {
            let (Value::Record(a), Value::Record(b)) = (a, b) else {
                return None;
            };
            for &(name, nested) in fields {
                let (mine, theirs) = (a.get(name)?, b.get(name)?);
                match nested {
                    Some(nested) => pending.push((nested, mine, theirs)),
                    None if mine == theirs => {}
                    None => return Some(false),
                }
            }
        }
        let Some(next) = pending.pop() else {
            return Some(true);
        };
        (shape, a, b) = next;
    }
}

fn internal(message: &'static str, span: Span) -> Diagnostic {
    crate::diagnostic::InternalError { message, span }.into()
}

/// The internal error for the `Top` `node` of a field's cell, read before
/// its point: the checker's schedule computes every cell before code that
/// reads it runs (see [`crate::check`]), so this is a bug, reported rather
/// than panicked on.
#[cold]
#[inline(never)]
fn not_computed(node: &IrNode) -> Diagnostic {
    internal(
        "read of a top-level field whose cell is not computed yet",
        node.span,
    )
}

/// The internal error for the [`IrKind::Top`] `node`, whose slot holds no
/// closure: the checker lets no code run before the closure it reads is
/// made (see [`crate::check`]), so this is a bug, reported rather than
/// panicked on. Out of line, off the path of the read that succeeds.
#[cold]
#[inline(never)]
fn not_made(node: &IrNode) -> Diagnostic {
    internal(
        "read of a top-level function whose closure is not made yet",
        node.span,
    )
}

/// The error for the call `node`, which would take the program's nesting,
/// calls included, past [`MAX_DEPTH`] levels.
fn nested_too_deeply(node: &IrNode) -> Diagnostic {
    Diagnostic::error(
        format!(
            "call nested too deeply: the limit is {MAX_DEPTH} levels of nesting, calls included"
        ),
        node.span,
    )
    .with_note(
        "every level of expression nesting around a call counts, and so does the body of the \
         function called; GCSL does not eliminate tail calls, so recursion this deep is not \
         supported",
    )
}

/// The error for an index, or an update's index step, at `span`, of
/// position `at` in a list of `len` elements, which it lies outside: "index
/// out of range", labelled with the index and, if it is not negative, the
/// list's length.
fn out_of_range(span: Span, at: i64, len: usize) -> Diagnostic {
    let facts = if at < 0 {
        format!("the index is {at}")
    } else {
        let noun = if len == 1 { "element" } else { "elements" };
        format!("the index is {at}, and the list has {len} {noun}")
    };
    Diagnostic::error("index out of range", span).with_label(facts)
}

/// What a step of an update selects in the record or list its walk is at:
/// a field by name, or an element by an index that lies in the list.
#[derive(Clone, Copy, Debug)]
enum Selected {
    /// The field of this name.
    Field(Name),
    /// The element at this index.
    Element(usize),
}

/// The value `at` selects in `container`, read by the walk of the `Update`
/// `node` to go down a level: a field the checker guarantees the record
/// has, or an element at an index that `Evaluator::select` found in the
/// list.
fn selected(container: &Value, at: Selected, node: &IrNode) -> Flow<Value> {
    let found = match (container, at) {
        (Value::Record(record), Selected::Field(name)) => record.get(name),
        (Value::List(list), Selected::Element(index)) => list.get(index),
        _ => None,
    };
    match found {
        Some(value) => Ok(value.clone()),
        None => mistyped(node, "an update reads what its record or list lacks"),
    }
}

/// `list` with its element at `index`, which lies in it, replaced by
/// `value`: a new list sharing every other element, the path to it copied
/// ([`List::with`]), and `list` unchanged, so that whatever else holds it
/// still sees its old elements.
fn with_element(list: &List, index: usize, value: Value) -> List {
    list.with(index, value).unwrap_or_else(|| list.clone())
}

/// The number of integers from `start` up to `end`, `end` included if
/// `inclusive`: 0 if there are none, and computed without overflow, so
/// that `i64::MIN..=i64::MAX` has its 2^64.
fn range_len(start: i64, end: i64, inclusive: bool) -> u128 {
    let len = i128::from(end) - i128::from(start) + i128::from(inclusive);
    u128::try_from(len).unwrap_or(0)
}

/// The error for the range `node`, used as a value, of `len` integers,
/// more than [`MAX_LIST_LEN`].
fn range_too_large(node: &IrNode, len: u128) -> Diagnostic {
    Diagnostic::error("range too large", node.span)
        .with_label(format!("this range has {len} elements"))
        .with_note(format!(
            "a range used as a list holds every element, and a list cannot hold more than \
             {MAX_LIST_LEN}. A `for` over a range directly builds no list"
        ))
}

/// What a `for` iterates over, one element at a time.
#[derive(Debug)]
enum Elements {
    /// The elements of a list, from the one at the index.
    List(List, usize),
    /// The integers of a range that is never built, from `next` to `last`,
    /// both included, unless `done`: counted so that reaching
    /// `i64::MAX` never overflows.
    Range {
        /// The next integer.
        next: i64,
        /// The last integer.
        last: i64,
        /// Whether every integer has been yielded, or there were none.
        done: bool,
    },
}

impl Elements {
    /// The integers from `start` up to `end`, `end` included if
    /// `inclusive`; none if there are none.
    fn range(start: i64, end: i64, inclusive: bool) -> Self {
        let last = if inclusive {
            Some(end)
        } else {
            end.checked_sub(1)
        };
        match last {
            Some(last) if start <= last => Elements::Range {
                next: start,
                last,
                done: false,
            },
            _ => Elements::Range {
                next: 0,
                last: 0,
                done: true,
            },
        }
    }

    /// How many elements are left, or `usize::MAX` if more than that.
    fn remaining(&self) -> usize {
        match self {
            Elements::List(list, at) => list.len().saturating_sub(*at),
            Elements::Range { done: true, .. } => 0,
            &Elements::Range { next, last, .. } => {
                usize::try_from(range_len(next, last, true)).unwrap_or(usize::MAX)
            }
        }
    }
}

impl Iterator for Elements {
    type Item = Value;

    fn next(&mut self) -> Option<Value> {
        match self {
            Elements::List(list, at) => {
                let value = list.values().get(*at)?.clone();
                *at += 1;
                Some(value)
            }
            Elements::Range { done: true, .. } => None,
            Elements::Range { next, last, done } => {
                let value = *next;
                if value == *last {
                    *done = true;
                } else {
                    *next = value + 1;
                }
                Some(Value::Int(value))
            }
        }
    }
}

/// The error for the await `node` of a future whose body is running: an
/// await of the future from inside its own body.
fn depends_on_itself(node: &IrNode) -> Diagnostic {
    Diagnostic::error(
        "this deferred expression depends on its own value",
        node.span,
    )
}

/// The error for the call `node` of a memoised function's closure that is
/// running with equal arguments further up the stack.
fn depends_on_own_result(node: &IrNode) -> Diagnostic {
    Diagnostic::error("this call depends on its own result", node.span).with_note(
        "a `->>` function called again with the same arguments while it is running would never \
         finish",
    )
}

/// The GCSL source of `value`, for the call `node` of `show` (see [The
/// built-in functions](crate::eval::calls#the-built-in-functions)): what [`Value::repr`]
/// shows, the one printer of values, which is what `gcsl run` shows of a
/// value, but for a string at the top, quoted as it is inside a record or a
/// list, and which reads back, as an expression, as an equal value. First
/// the value is walked, through a work list rather than recursion, in the
/// order it is shown, for what has no source: a `Float` that is NaN or an
/// infinity, which no literal writes, is "`show` cannot render `nan` as
/// source" (`inf`, `-inf`), and a future still to run, which the checker's
/// deep await of the argument leaves none of, is "cannot show a value that
/// is not ready yet", each at the call. A function is an internal error,
/// which the checker rules out.
fn source_of(value: &Value, node: &IrNode) -> Flow<Rc<str>> {
    let mut pending = vec![value];
    while let Some(next) = pending.pop() {
        match next {
            Value::Float(x) if !x.is_finite() => {
                return Err(Diagnostic::error(
                    format!(
                        "`{}` cannot render `{next}` as source",
                        Builtin::Show.name()
                    ),
                    node.span,
                )
                .into());
            }
            // Reversed, so that what is shown first is met first.
            Value::Record(record) => pending.extend(record.values.iter().rev()),
            Value::List(list) => pending.extend(list.values().iter().rev()),
            Value::Future(_) => {
                return Err(Diagnostic::error(
                    "cannot show a value that is not ready yet",
                    node.span,
                )
                .into());
            }
            Value::Fn(_) | Value::Builtin(_) => {
                return mistyped(node, "a function given to `show`");
            }
            Value::Sandbox(_) => {
                return Err(Diagnostic::error(
                    "cannot show a sandbox: it is a live directory, not data",
                    node.span,
                )
                .into());
            }
            // A variant's source is how it is shown, `Toolchain::C`, and a
            // value of an enum the call of it on its payload's.
            Value::Variant(variant) => pending.push(&variant.payload),
            // Shown as `***`, whatever it holds, which is not looked at; a
            // file as the call that makes it.
            Value::Secret(_)
            | Value::File(_)
            | Value::Dir(_)
            | Value::Tag(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Quantity(..)
            | Value::Bool(_)
            | Value::Str(_)
            | Value::Empty => {}
        }
    }
    Ok(Rc::from(value.repr().to_string()))
}

/// Fails with the internal error `message` at `node`: a node of a type its
/// walker does not handle. Returned directly by the walkers' error arms, so
/// that they hold no temporary for it (see the note on `Evaluator`).
fn mistyped<T>(node: &IrNode, message: &'static str) -> Flow<T> {
    Err(internal(message, node.span).into())
}

pub(crate) mod builtins;
pub mod effects;
pub(crate) mod elements;
pub(crate) mod entries;
pub(crate) mod files;
pub mod graph;
pub(crate) mod keychain;
pub(crate) mod kinds;
pub(crate) mod memo;
pub mod plain;
pub mod profile;
pub(crate) mod sandbox;
pub(crate) mod system;
pub(crate) mod text;
pub(crate) mod unions;

pub(crate) mod callers;
pub(crate) mod calls;
pub(crate) mod context;
pub(crate) mod failure;
pub(crate) mod futures;
pub(crate) mod json;
pub(crate) mod lists;
pub(crate) mod loops;
pub(crate) mod machine;
pub(crate) mod memoised;
pub(crate) mod metas;
pub(crate) mod narrowing;
pub(crate) mod records;
pub(crate) mod retry;
pub(crate) mod scalars;
pub(crate) mod secrets;
pub(crate) mod updates;

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests;
