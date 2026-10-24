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
//! [Stops](#stops)): an `if` evaluates its condition through `bool` and
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
//! list is needed (see [Lists](#lists)), where the value may be a future
//! (see [Futures](#futures)), or where it is of a union of kinds (see
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
//! # Lists
//!
//! A list node, of [`crate::value::ValueKind::List`], has no typed walker
//! of its own: `Evaluator::list` evaluates it through the general walker
//! and takes the [`List`] out of the value, which is one match on its tag,
//! as a load extracts a slot's value. A list is only ever read whole, by an
//! index, a spread, `len`, a comparison or an update (see
//! [Updates](#updates)), each of which is a helper and
//! not on the recursion path of anything but its own operands, so a walker
//! of its own would save nothing measurable, and would add another large
//! match to keep in step with every kind of node; the list nodes that can
//! be of any kind, indexes, are handled by every typed walker, as field
//! accesses are.
//!
//! A `MakeList` without spreads evaluates its items in order straight into
//! the list's one allocation, sized to the number of items; the empty list
//! allocates nothing. One with spreads collects its elements in a vector,
//! each spread's list's elements cloned in its place (reference-count
//! increments), in time linear in the result's length, and then copies
//! them into the list's allocation. An `Index` evaluates its list, then its
//! position, and fails with "index out of range" at the index, labelled
//! with the position and the list's length, for a position outside the
//! list, a negative one included; nothing wraps around from the end.
//!
//! A `Range`, inclusive or not, used as a value evaluates its bounds,
//! left to right, and builds the list of its integers in one allocation,
//! sized exactly and filled in place; the empty range, one whose left bound
//! is not below its right one (or, inclusive, is above it), allocates
//! nothing. Its length is computed in 128 bits, so that no pair of bounds
//! overflows it, `i64::MIN..=i64::MAX` included, and a range of more
//! integers than a list can hold ([`MAX_LIST_LEN`]) fails with "range too
//! large" at the range, labelled with its length, rather than ask for an
//! allocation that cannot exist. There is no other limit on a list's
//! length: how large a list a program builds is its own business, and one
//! too large for the machine fails as any allocation does.
//!
//! A `For` is evaluated by a helper, `Evaluator::for_loop`, which the
//! walkers only call: it evaluates the iterable once, and for each element
//! in order writes it to the variable's slot of the current frame, then
//! evaluates the guard, if there is one, and if that is true or absent the
//! body, each through its walker, so an error stops the loop and is
//! reported as any error in the body is. Every iteration evaluates the body
//! from the `for`, at the same depth, so iterations do not nest, and the
//! nesting budget, charged statically by the body's height, is charged for
//! one. A closure or a `lazy` the body makes reads the slot when it is
//! made, so it captures that iteration's element. A collecting `for`
//! gathers the body's values in a vector, sized up front to the number of
//! elements when there is no guard to skip some (at most `FOR_RESERVE`
//! up front), and copies them into the list's allocation, in time
//! linear in its length; one that discards them, a statement whose value
//! nothing would hold, drops each as it comes and is `{}`. A `for` whose
//! iterable is directly a range (the parentheses around it are gone by
//! then) does not build the range's list: it evaluates the bounds and
//! counts from one to the other, the last included for `..=`, stopping at
//! the last rather than stepping past it, so that counting up to
//! `i64::MAX` never overflows; so it has no limit on the range's length.
//!
//! A `Loop`, a condition loop's or a `do` loop's, is evaluated by
//! `Evaluator::condition_loop`: while its condition is true, tested first
//! unless it is a `do` loop's, its body, and then the items after its
//! condition, each for its effects; it collects or discards as a `For`
//! does, but reserves nothing, its length not being known up front. Both
//! helpers are reached through `Evaluator::looped`. A `break` or `continue`
//! in a body ends its iteration, or its loop, through the stop channel (see
//! [Stops](#stops)).
//!
//! # Stops
//!
//! Every walker returns a `Flow`: a value, or an `Unwind`, the one road out
//! of a node that does not yield one. An `Unwind` is an error, a
//! [`Diagnostic`], which ends the evaluation, or a *stop*, a `Stop`: a
//! `break` or a `continue` on its way to its loop, with the number of loops
//! it is still to leave, or a `return` on its way to what it leaves, named
//! by the number of its [`CatchId`] in the same `u32`, each with its value,
//! if any, and what it does there (`StopKind`). So a stop is as small as it
//! was before `return`, and a loop handles a `break` or `continue` as it
//! did. A stop travels
//! as an error does, through every `?`, so every node between a `break` and
//! its loop is left at once, with nothing after the `break` evaluated: in
//! `{ if c { break }; 1 } + g()`, `g` is not called. (A flag that the loop
//! checks would let the `+` and its call run.) A `Break`, `Continue` or
//! `Return` node evaluates its value through the general walker and then
//! unwinds (`Evaluator::leave`); every walker, typed or general, hands it
//! such a node, since a node of kind [`ValueKind::Never`] may stand where
//! one of any kind is expected. A catch (see below) may stand under any
//! walker too, and so may a loop that cannot end, whose node is of kind
//! `Never` where it stands, which the walker runs. So each typed walker has
//! one arm for all of them, handing them to one helper
//! (`Evaluator::stopping`), and pays nothing else; the general walker has
//! one more arm, for a catch (`Evaluator::catch`). These helpers, and the
//! record walker's for its loops (`Evaluator::record_loop`), are kept out
//! of line, so that no walker's frame, on the recursion path of every
//! expression, grows: with an arm each in every typed walker, inlined, the
//! new nodes cost a loop of a few million iterations two or three per
//! cent. `leave` and `caught` are left to the compiler, as they were.
//!
//! Only a loop catches a `break` or `continue`, and only one that comes out
//! of its body (`caught`): a stop out of its iterable, guard, condition or
//! items after its condition is for a loop around it, and goes on with
//! `?`. A stop with no loops left to leave is this loop's: its value, if
//! any, is collected, and the loop ends, for a `break`, or goes on with its
//! items after the condition and its next test, for a `continue`. Any other
//! goes on, one loop nearer its own, and this loop's list is dropped with
//! it. A `return` goes through every loop as it is, the loop's list and
//! after-items with it. Its target catches it, three ways:
//!
//! - a `Catch`, around a labelled block or a loop that a `return` leaves,
//!   catches one that names its number, coming out of the block or loop, and
//!   yields its value (`Evaluator::catch`); any other stop goes on;
//! - a call, a memoised call and an await running a future catch one that
//!   names [`CatchId::FUNCTION`] coming out of the body, the value being the
//!   body's (`Unwind::returned`). The catch is on the error road only, a
//!   [`Result::or_else`] of the body's result, so that a body that gives a
//!   value takes exactly the road it took before `return` existed (written
//!   as a function of the whole result, matched on every call, it cost a
//!   call-heavy program a few per cent). It is inside the memoised call's
//!   running entry and the future's running mark: the value returned is
//!   then recorded by `Evaluator::memo_end`, and resolves the future, by
//!   the road any value of a body takes, rather than reaching them as an
//!   `Err`, which clears the entry or leaves the future unrun.
//!
//! The checker lets no other stop out of a function, a `lazy` or a memoised
//! call, nor any out of the top level, so one that reaches such a boundary
//! is an internal error there (`Unwind::into_error`), never a diagnostic,
//! and never caught by a loop or catch of another frame. A catch's number
//! is unique in its program, and no node holds itself within one frame, so
//! a catch never takes another's `return`.
//!
//! Since evaluation goes on after a stop, nothing may assume that an `Err`
//! ends it all and skip restoring what it changed. What was found, and done:
//!
//! - The nesting budget is a field of each frame's `Evaluator`, `levels`,
//!   set when the frame is made and never changed, and charged statically:
//!   nothing is counted up and down around a node, so nothing can leak.
//! - The value stack: a call grows it by the callee's frame before its
//!   arguments are evaluated, and an argument may now stop, as in `f({
//!   continue })`, or return from the function the call is in, as in
//!   `f(g({ return 1 }))`: `Evaluator::call` and `Evaluator::memo_begin`
//!   truncate it back on any `Err` of an argument, as they did for an
//!   error, and the stop goes on. A call, a memoised call and a future
//!   truncate it after the body whatever the body gave, a `return` for it
//!   included, and `eval_with` at the top.
//! - A future's running mark: `Evaluator::run_future` puts the future back
//!   unrun on any `Err` of its body; a `return` for the `lazy` is its value,
//!   caught before, and recorded as any value is; no other stop can come
//!   out of a body, and is made an internal error first.
//! - A memoised call's running entry: `Evaluator::memo_end` removes it on
//!   any `Err` of the body; a `return` for the function is its value,
//!   caught before, and recorded as any value is, so that a function that
//!   returns early is memoised; any other stop is made an internal error
//!   first. The memo tables' checkpoint is rolled back only by a
//!   [`crate::session::Session`], and only when a whole input fails: a stop
//!   in the body of a `->>` function is caught by its loop or catch, or by
//!   its call, in the body, and reaches neither the call's caller nor the
//!   session, so the entries the body's calls rightly recorded stay.
//! - A `return` out of the value of an update (`r.a := { return 1 }`) or
//!   out of a position of its path leaves before anything is stored, the
//!   records the walk read dropped with it; one out of an interpolation
//!   drops the text built so far, one out of an `if`'s narrowed branch
//!   leaves the slot the `is` wrote, which belongs to the `if`.
//! - The input stamped on an error from another input's code, and the
//!   "awaited here" and "called here with the same arguments" labels, are
//!   added to errors only.
//! - What a node holds while it evaluates its children (a list being built,
//!   the records an update passes through, a loop's collected values) is
//!   local to its helper's frame, and dropped as the stop passes.
//! - Slots written for a node, a loop's variable and an `is`'s narrowed
//!   value, are not restored: they belong to that node's scope, which no
//!   code after it reads without writing first.
//!
//! An `UpdateField`, a dotted field's, builds a new record and never
//! changes the old one, as a step of an `Update` that sets a field does
//! (see [Updates](#updates)). It clones the old record's values into a new
//! boxed slice, one allocation, with the updated field's value replaced or
//! appended. A replacement shares the old record's layout. An addition
//! needs a new layout: the old one's names and texts with the added name
//! after them, whose text the evaluator takes from [`Ir::name_text`], since
//! it has no names table and must never need one. Every addition of a name
//! to a record of a given layout makes an equal layout anew; caching the
//! result per old layout and name is the optimisation for a bytecode VM,
//! which will have a place to keep such a cache at each update site.
//! Updating the empty record makes a record of the one field.
//!
//! Typed equality, `RecordEq` and `RecordNe`, compares two records over the
//! [`crate::ir::Shape`] of their static type: for each field of the shape,
//! it looks the field up in both records by name, compares a record field
//! through its own shape and any other field with `==` on [`Value`], and
//! ignores every field the shape lacks. Nested records are compared through
//! a work list, as total equality compares them (see
//! [`crate::value::Record`]), so nesting does not recurse and comparing flat
//! records allocates nothing. Total equality, `RecordSame` and
//! `RecordNotSame`, is [`Value`]'s `PartialEq`. `ListEq` and `ListNe`
//! compare two lists' lengths and then their elements in order, each
//! through the shape of the element type, if it is a record or a list, as
//! the list's [`crate::ir::ListShape`] says, and with `==` otherwise, in the
//! same work list; `ListSame` and `ListNotSame` are [`Value`]'s `PartialEq`.
//!
//! `and` and `or` short-circuit: the right operand is evaluated only when
//! the left does not decide the result. A string constant is a reference
//! to the IR's copy, so producing one increments a reference count and
//! copies nothing. Concatenation allocates exactly the length it needs, and
//! an interpolation builds its text in one buffer sized from its literal
//! pieces before sharing it.
//!
//! Variables live in frames on one value stack, a `Vec<Value>`: the top
//! level's frame at its bottom, sized by the checker's slot assignment, and
//! each call's frame above its caller's. A local [`Slot`] is an offset from
//! the current frame's base. A `Load` in a typed context reads its slot and
//! extracts the unboxed value, cloning a string's reference; a slot holding
//! a value of another type there is an internal error, since lowering typed
//! the load. Storing values in separate typed slots (`Vec<i64>`,
//! `Vec<f64>`) would avoid the tag check; that is a later optimisation for
//! a bytecode VM, not something this tree walker needs.
//!
//! A slot may be written many times: a mutable variable's by each of its
//! rebinds, from any block of its function, a `for`'s body included, as well
//! as a `for`'s variable's by each iteration. A `Load` reads the slot when
//! it is evaluated, and evaluation is in source order, so a read sees the
//! last write before it: in `let mut x = 1; let y = x + { x := 5; x }`, `y`
//! is 6. A `Load` copies the value, a reference-count increment, and never
//! moves it out of the slot, and nothing caches a slot's value, so a `for`'s
//! guard reads what its body last wrote, and a closure or a future copies
//! the value a slot holds when it is made, which later writes do not
//! change. A value already read is held, as any operand is, while what
//! follows it is evaluated: `r.a := e` evaluates `e` first, which may
//! rebind `r` itself, and then reads `r`, as it then is, and stores the
//! update of that record (see [Updates](#updates)); and a `for` iterates
//! over the list its iterable was when it began, whatever its body
//! rebinds.
//!
//! # Updates
//!
//! An `Update`, a dotted `let` or `:=`, is evaluated by a helper,
//! `Evaluator::update`, which the walkers only call, in the order every
//! dotted update has: its value first, once, through `value`; then the
//! record or list it updates, the variable's, read, and awaited if the
//! checker marked it; and then `Evaluator::walk_update` walks the path down
//! from it, once, without recursing: each step selects a field, or an
//! element at the position its next `Int` child evaluates to, there and
//! then, left to right, and reads it, awaits it, charged to the nesting
//! budget as an await just below the update, or, where a `let`'s path goes
//! beyond the static type, starts from `{}`, holding each record or list
//! it passes through in a vector; the last step selects the leaf. A
//! position outside its list, a negative one included, stops the walk with
//! "index out of range", at the path up to and including its segment,
//! labelled as an index's is, before anything is stored, so the variable is
//! left as it was. Then the new value is built back up, from the leaf
//! outwards: the leaf set to the value, then each record or list passed
//! through with what its step selected replaced by the one built below it.
//! The update's value is that; the `Store` around it writes it. A value
//! already read is held while what follows it runs, so a position that
//! rebinds the variable does not change what the walk updates, and the
//! store then overwrites the rebind.
//!
//! Nothing read is changed: another variable or closure holding an old
//! record or list still sees it as it was. A record is set as an
//! `UpdateField` sets it. A list is copied, in one allocation sized to its
//! length, sharing every element but the one replaced, so updating each
//! element of a list in a loop takes time quadratic in its length, for now:
//! updating in place, when the variable is the only holder of what it
//! updates, is the optimisation that evaluating the value before the
//! variable is read allows. A path of one step allocates nothing for the
//! walk.
//!
//! # Calls
//!
//! A closure is built by `MakeClosure` from its captured values, read in
//! the creating frame into one boxed slice, and a reference to the IR,
//! which the closure keeps alive: two allocations. `LoadCapture` reads the
//! running closure's captures, and `LoadCallee` yields the running closure
//! itself, which the evaluator holds while it runs, so a recursive function
//! refers to itself without holding itself. `Top` reads a slot of the top
//! level's frame, at base 0 of the value stack, from whatever frame it is
//! evaluated in: a top-level function of the input, or the cell of one of
//! its top-level fields, which another function or a `lazy` refers to there
//! rather than by a capture. A cell is written once, at its point, which
//! the schedule puts before every read; a `Top` that finds a cell of a kind
//! other than a record's (`Record`, `Mixed` or `Deferred`, whose unwritten
//! `Empty` is a valid `{}`) not yet written is an internal error diagnostic,
//! as a slot with no closure is for a function. Every
//! call, memoised call, future and await of a run, and every input of a
//! session, shares the one value stack whose bottom is the top level's
//! frame, which is what makes base 0 that frame. The checker sees to it
//! that the slot holds the function's closure before any read of it runs;
//! a read of a slot that holds no closure is still an internal error
//! diagnostic, not a panic.
//!
//! A call evaluates its callee to a closure through the `function` walker,
//! checks the nesting budget, and grows the stack by the callee's frame
//! size with `Vec::resize`; then it evaluates each argument in the caller's
//! frame, in source order, into its parameter's slot of the new frame, runs
//! the body in a new `Evaluator` whose base is the new frame's, and finally
//! truncates the stack back, whether the body succeeded or failed. An
//! argument whose value is an [`ArgValue::Default`], which a call gives a parameter
//! left out whose default is no constant, after the arguments written, in
//! parameter order, is the result of calling the default function the
//! callee's closure keeps at that capture, as a call in the caller's frame
//! is, with the caller the call gives its callee, and with the values of
//! the parameters before it, read from the new frame: so a default sees
//! the arguments before it, written or defaulted, and runs on the caller's
//! side of the call, before the callee's body starts, once for each call
//! that leaves its parameter out (`Evaluator::default_arg`). The
//! stack keeps its capacity, so once it has grown to a program's deepest
//! recursion, a call allocates nothing: not its frame, not its arguments'
//! slots, and nothing to look anything up, since every variable is a slot
//! or a capture index. `call` keeps only the closure, its frame's base and
//! the new evaluator across the body's evaluation.
//!
//! The callee of a `Call` may instead be a built-in function, a
//! [`Value::Builtin`]: an ordinary one (every one but `len` and `show`) is
//! a value of a function type that a variable, a parameter or a field may
//! hold, so the `function` walker yields a closure or a built-in, and a
//! call of a built-in applies it, its arguments evaluated in order as a
//! closure's are, each collected by the index of the parameter it fills,
//! with no frame (see [The built-in functions](#the-built-in-functions)).
//! This is the one place a node does not say what it does, since no node
//! could know which function a value is. A built-in whose typing is special
//! is never called through a value: the checker makes every call of one its
//! own node (`ListLen`, `Show`). A built-in's cell holds it from before the first
//! input runs (see [`crate::workspace`]); `Top` reads it as it reads a
//! function's slot.
//!
//! Calls nest on the Rust stack, one `call` and a few walker frames per
//! GCSL call, and so does every level of expression nesting between a
//! frame's body and a call in it; GCSL does not eliminate tail calls. So
//! calls and expressions share one budget of nesting levels,
//! [`MAX_DEPTH`], which keeps a program within [`crate::STACK_SIZE`]
//! whatever shape its recursion has. Each evaluator carries `levels`, the
//! nesting its enclosing frames have consumed where its frame's body
//! starts: 0 at the top level. A call at depth `d` in its frame's body (see
//! [`IrKind::Call`]), of a function whose body's height is `h` (see
//! [`crate::ir::FnCode::height`]), enters its callee's frame at
//! `entering = levels + d + CALL_LEVELS` levels, and fails with "call
//! nested too deeply" if `entering + h` exceeds `MAX_DEPTH`, before its
//! arguments are evaluated; otherwise the callee's body runs with
//! `levels = entering`. The whole height of the body is charged up front,
//! so that nothing inside the body, however deep, needs a check of its own:
//! the budget costs the evaluator three additions and a comparison per
//! call, and nothing per node.
//!
//! A function defined by an earlier input of a session can fail while a
//! later input runs; its error's spans then refer to the earlier input's
//! source. When an error comes out of the body of a closure whose IR is of
//! another input than the caller's, the call records that input on the
//! diagnostic, [`Diagnostic::input`], unless an inner call has already, so
//! the innermost foreign frame wins.
//!
//! # Futures
//!
//! `MakeFuture` builds a future as `MakeClosure` builds a closure: its
//! captured values in one boxed slice and a reference to the IR, two
//! allocations. Nothing runs. Only the general walker evaluates a node of
//! kind [`crate::value::ValueKind::Deferred`], whose value may be a future;
//! every typed walker treats one as mistyped, so no typed walker ever
//! inspects a value to see whether it is a future, and code that never
//! awaits pays nothing for futures. An `Await` node, placed by the checker
//! wherever such a value is used as its bits, evaluates its value through
//! `value`, and while it is a future, runs it: the future equivalent of a
//! call, `run_future`, charged to the nesting budget as a call of a
//! parameterless function would be, the await's depth in its frame's body
//! plus [`CALL_LEVELS`] plus the body's height, and one more level for each
//! level of record or list nesting a deep await has descended to reach it.
//! Its first await runs the body in a frame of its own, on the value stack
//! above the
//! awaiting frame's, and records the value, dropping the captures; every
//! later await is a borrow and a clone of that value. A deep await then
//! awaits the lazy fields its shape lists, recursively, building a new
//! record, sharing the layout and the untouched values, only if a field
//! changed; and the elements of a list, each as a list's lazy shape says,
//! in order, building a new list, sharing the untouched elements, only if an
//! element changed. Each element's future runs at most once, however many
//! lists hold it, since a future is shared. An error inside a body leaves
//! the future unrun, with its
//! captures, so a later await runs it again, and gets the context label
//! "awaited here" on the await that ran it, when both are of the same
//! session input; otherwise the future's input is stamped on it, as a call
//! stamps a closure's. A future is marked as running while its body runs,
//! so that an await of it from inside its own body, which the checker never
//! lets through, is an error rather than an endless loop.
//!
//! # The built-in functions
//!
//! A call of an ordinary built-in function applies it to its arguments:
//!
//! - `int(x)` truncates its `Float` toward zero, failing with "integer
//!   overflow" when that is no `Int` (an infinity, or beyond `i64`), the
//!   operand labelled with its value, or "cannot truncate NaN to an Int";
//! - `float(x)` gives the `Float` nearest its `Int`;
//! - `exec(argv)` runs a program, one of the things GCSL does outside
//!   itself, beside `exec_live` and the file system's built-ins (below), and
//!   reports what it did. It runs `argv[0]`, found through `PATH` as
//!   the OS finds it (a name with a `/` is a path), with the arguments
//!   `argv[1..]`, each passed as it is, with no shell involved
//!   ([`std::process::Command`]). The program's standard input is the null
//!   device (`Stdio::null()`, not a closed descriptor), so it never reads
//!   what `gcsl`'s own standard input holds, a REPL's next line say; its
//!   standard output and standard error are captured whole, nothing of
//!   them shown as it runs; its working directory and environment are the
//!   `gcsl` process's. The call returns once the program has exited, with
//!   a record of the fields `status`, `stdout` and `stderr`, in that
//!   order: its exit status, or 128 plus the number of the signal that
//!   killed it, the shell's convention (`kill -9` gives 137); and its
//!   standard output and standard error decoded as UTF-8, each invalid
//!   sequence replaced by U+FFFD. A non-zero status is data, never an
//!   error. The record's layout is built once per run, by the driver that
//!   lays out the frame, its field names interned in the run's names, and
//!   shared by every record `exec` builds in the run, from whatever input
//!   or function calls it, since a call through a value is a plain `Call`
//!   that no node or IR could prepare it for; it reaches every frame
//!   beside the memo tables (see [`eval_with`]). `exec` fails only when its
//!   program cannot run at all: "`exec` needs a program to run" for an
//!   empty `argv`, and "cannot run `name`" for a program that cannot be
//!   started, noting the OS's reason, as in "No such file or directory (os
//!   error 2)". Both are at the call, and end the run as every runtime
//!   error does. A memoised function around `exec` runs its program once
//!   for each distinct list of arguments, as its body runs once.
//! - `exec_live(argv)` is `exec`, the same program run the same way, the
//!   same record of the same layout, the same errors (but "`exec_live`
//!   needs a program to run"), but for one thing: while the program runs,
//!   what it writes to its standard output and to its standard error is
//!   ALSO copied, as it arrives, to `gcsl`'s own standard error, never to
//!   its standard output, which is a host's channel for results, and still
//!   captured whole into the record (`run_live`). Each stream is read on a
//!   thread of its own, both joined before the call returns, so a program
//!   that fills both pipes never waits on one while the other is read. The
//!   copy is line by line: each complete line, its newline included, is
//!   written as it is complete, in one write under the lock of standard
//!   error, so the lines of the two streams interleave in the order they
//!   are complete but never mix within a line, and a last line without a
//!   newline is written when its stream ends, as it is, with no newline
//!   added (`copy_lines`). The bytes are copied as they are, not decoded;
//!   the record's strings are decoded as `exec`'s are. A failure to write
//!   to `gcsl`'s standard error is ignored, the program running on and its
//!   output captured whole. It is what a build step whose program is silent
//!   for minutes until it finishes, a compiler, calls, so that its progress
//!   is seen; the prelude's `sh_live` is `sh` with it.
//! - `panic(message)` fails with the runtime error `message`, its own
//!   wording, at the call, with no label or note, ending the run as every
//!   runtime error does. A call of it is of kind `Never` (see
//!   [`crate::ir`]): every walker evaluates it as any call, through
//!   `Evaluator::call`, which then never returns a value, so no typed
//!   walker reaches for a value of its kind. A panic inside a function is
//!   reported where it happens, as any runtime error is: one in the
//!   prelude's `sh` is located in `<stdlib>/prelude.gcsl`.
//! - `split(s, sep)` gives the pieces of `s` between occurrences of `sep`,
//!   in order, as Rust's `str::split` does: `split("/a/b", "/")` is `["",
//!   "a", "b"]`, `split("", "/")` is `[""]`, and `split("a//b", "/")` is
//!   `["a", "", "b"]`. `sep` may be of any length; an empty one, which
//!   would match between every two characters, fails with "`split` needs a
//!   separator that is not empty", at the call.
//! - `join(parts, sep)` gives the strings of `parts` with `sep` between
//!   each pair, `""` for no parts.
//! - `starts_with(s, prefix)` and `ends_with(s, suffix)` compare bytes, as
//!   Rust's do: whether `s` begins, or ends, with the bytes of the other.
//! - `trim(s)` gives `s` without its leading and trailing whitespace, as
//!   Rust's `str::trim` finds it (Unicode's `White_Space`, so a no-break
//!   space too): `""` for a string of whitespace alone.
//! - `strip_prefix(s, prefix)` gives `s` without `prefix` if `s` begins
//!   with its bytes, as `starts_with` finds them, else `s` unchanged, as
//!   Rust's `str::strip_prefix(..).unwrap_or(s)`: once, so
//!   `strip_prefix("aab", "a")` is `"ab"`; an empty `prefix` gives `s`,
//!   and so does one that ends inside a character of `s`, which `s` cannot
//!   begin with. `strip_suffix(s, suffix)` does the same at the end, as
//!   `ends_with` finds it.
//! - `lines(s)` gives the lines of `s`, as Rust's `str::lines` does: split
//!   at each `\n`, a `\r` just before it removed too, with no empty last
//!   line after a final line ending. So `lines("")` is `[]`, `lines("\n")`
//!   is `[""]` and `lines("a\r\nb\r\n")` is `["a", "b"]`.
//! - `fields(s)` gives the pieces of `s` between runs of whitespace, none
//!   of them empty, as Rust's `str::split_whitespace` does (Unicode's
//!   `White_Space`, as `trim`'s): `[]` for `""` or whitespace alone.
//! - `compare_versions(a, b)` gives `-1`, `0` or `1` as `a` sorts before,
//!   equal to or after `b` in VERSION ORDER, the order of GNU `sort -V`'s
//!   core, which is dpkg's `verrevcmp` applied to the whole strings (no
//!   epoch or revision split off, and none of `sort -V`'s handling of file
//!   suffixes). Each string is taken, by its bytes, as runs alternately of
//!   non-digits and of ASCII digits, starting with non-digits, either run
//!   perhaps empty, and the runs compared pairwise in turn. Two non-digit
//!   runs compare byte by byte by a weight: `~` lightest, before even the
//!   end of the run, then the end of the run, then the ASCII letters by
//!   their byte, then every other byte by its byte plus 256 (dpkg's
//!   `order`), so `1.0~rc1` is before `1.0`, `1.0` before `1.0a` and
//!   `1.0.0`, and `a` before `.`. Two digit runs compare as numbers: their
//!   leading zeros skipped, the longer the greater, else by the first digit
//!   that differs, never parsed, so a run of any length is fine: `6.10` is
//!   after `6.6`. If every run is equal but the strings differ, as `1.01`
//!   and `1.1` do, or `""` and `"0"`, they compare by their bytes, so `0`
//!   means equal strings, as `sort -V` breaks such a tie.
//! - `sort_versions(vs)` gives the strings of `vs` ascending in that order,
//!   `compare_versions`' own comparison, by a stable sort: since only equal
//!   strings compare equal, a string there twice is there twice, side by
//!   side, and `[]` gives `[]` (`sort_versions`).
//!
//! These eleven are pure and fail only as said; none has a limit of its own
//! on the length of a string.
//!
//! `show(value)`, a built-in whose typing is special, is no call of a value
//! but a `Show` node of its argument, which the checker has awaited deeply
//! (`source_of`). It gives the value's GCSL SOURCE, which read back as an
//! expression, where its type is expected, is an equal value: exactly what
//! [`Value::repr`] shows, the one printer of values, so what `gcsl run`
//! shows of a value but for a string at the top, which is quoted as it is
//! inside a record or a list (`show("a")` is `"\"a\""`, `show(1)` is `"1"`,
//! `show({})` is `"{}"`, a record's fields in their order, its field names
//! quoted unless identifiers). A `Float` with no literal, NaN or an
//! infinity, is "`show` cannot render `nan` as source" (`inf`, `-inf`), at
//! the call, the first of the value in the order it is shown; and a future
//! still to run, of which the deep await leaves none, "cannot show a value
//! that is not ready yet". It is pure. Two values' sources do not read back
//! everywhere: `Int`'s least value, `-9223372036854775808`, whose digits
//! are no `Int` literal, and an empty list where no type is expected, which
//! has no element type to infer.
//!
//! The six of the file system, which a program reaches as the members of
//! the standard module `fs` (`fs.exists` is `fs_exists`, and so on), are
//! impure, and EVERY PATH THEY TAKE MUST BE ABSOLUTE: a relative one, `""`
//! included, is "`fs.read` needs an absolute path, found `x.txt`", before
//! anything is touched. A path is used as it is given, never normalised,
//! and nothing of them depends on the process's working directory. Each
//! failure is a runtime error at the call, naming the built-in as the
//! member of `fs` it is, whatever name the program called it by, the
//! path or paths, and the OS's own reason, in the message itself (where
//! `exec` notes it, its message naming only the program), ending the run
//! as every runtime error does (`file_system`):
//!
//! - `fs_exists(p)` (`fs.exists`) gives whether something is at `p`, symbolic
//!   links followed, so a link to nothing is `false`
//!   ([`Path::try_exists`](std::path::Path::try_exists)); what cannot be
//!   checked, through a directory that cannot be searched say, is "`fs.exists`
//!   could not check `/a`: REASON".
//! - `fs_read(p)` (`fs.read`) gives the contents of the file at `p`; one that
//!   cannot be read, missing or a directory, is "`fs.read` could not read `/a`:
//!   No such file or directory (os error 2)", and one that is not UTF-8
//!   "`fs.read`: `/a` is not UTF-8 text".
//! - `fs_mkdir(p)` (`fs.mkdir`) makes the directory `p` and every missing
//!   directory above it, as `mkdir -p`
//!   ([`fs::create_dir_all`](std::fs::create_dir_all)), a directory already
//!   there being fine, and is `{}`; a failure, a file in the way say, is
//!   "`fs.mkdir` could not create `/a`: REASON".
//! - `fs_copy(from, to)` (`fs.copy`) copies the FILE `from` to `to`, its
//!   contents and permissions, a file at `to` replaced
//!   ([`fs::copy`](std::fs::copy)), and is `{}`; `from` is checked to be
//!   absolute before `to`. A directory `from`, a missing one, or a directory of
//!   `to` that does not exist, is "`fs.copy` could not copy `/a` to `/b`:
//!   REASON".
//! - `fs_temp_dir()` (`fs.temp_dir`) makes a NEW empty directory, mode `0700`,
//!   and gives its absolute path, another each call: a call is impure, so never
//!   shared with another, and in a memoised function it runs once for each list
//!   of arguments, as its body does. The directories are the numbered children,
//!   `1`, `2` and so on, of one parent directory, `gcsl-PID-N` in the system's
//!   temporary directory, or in the temp root a workspace's host chose
//!   ([`crate::workspace::Workspace::set_temp_root`]), that the run's owner
//!   holds, a [`crate::workspace::Workspace`] (a lone file's run is one) or a
//!   [`crate::Session`], made by its first call and never before, and removed,
//!   with everything in it, as the owner is dropped, whether its runs succeeded
//!   or failed ([`TempDirs`]); a temp root is never removed, and one that is
//!   not absolute is a REASON below. It reaches every frame beside the memo
//!   tables, as the layout of `exec`'s records does (see [`eval_with`]). A
//!   process killed before it drops the owner leaves the directory behind. One
//!   that cannot be made is "`fs.temp_dir` could not create a temporary
//!   directory: REASON"; and a call in a run without an owner, which only a
//!   test can make, is an internal error.
//! - `fs_write(p, contents)` (`fs.write`) writes `contents` to the file `p`,
//!   making it or replacing it whole, and is `{}`, ATOMICALLY: to a new
//!   temporary file beside it, `.NAME.gcsl-tmp-PID-N`, made exclusively,
//!   flushed to the disk and renamed over `p`, so a reader sees the old
//!   contents or the new, never a part
//!   ([`replace_file`](system::replace_file)). The directory of `p` must exist.
//!   A file it replaces keeps nothing, its permissions neither: the new file
//!   has the mode any new file gets, `0666` less the umask; and a symbolic link
//!   at `p` is replaced, not followed. Any failure, a missing directory, `p` a
//!   directory, is "`fs.write` could not write `/a/b`: REASON", and leaves no
//!   temporary file behind.
//!
//! # Callers and helpers
//!
//! The `fileroot` module (see [`crate::workspace`], "The standard library")
//! tells code where files are. Two of its members are constants that
//! lowering makes `StrConst`s: `fileroot.this`, the directory of the file
//! the expression is written in, and `fileroot.workspace`, the root of the
//! run's workspace. The other two are the evaluator's.
//!
//! Every frame, a call's, an await's running a future's body, and the top
//! level's, has a *caller*: the directory of the file whose code made the
//! call it runs for, by its index in the run's table of directories, which
//! the driver built and every frame shares (each file's code names its own
//! directory by it, [`Ir::dir`]), and a *helper flag*, false as it starts.
//! A call of a closure (a `def`, a `fn`, a memoised function, a function
//! value, whatever its callee's code) gives its callee, as the callee's
//! caller, the calling frame's own caller if the calling frame's helper
//! flag is set, and otherwise the directory of the file the calling code is
//! of, the IR the calling frame runs (a closure runs its code's IR, so a
//! closure of `_lib/ppk.gcsl` passes `_lib` on wherever it is called). A
//! built-in function is no frame, and changes nothing. A top level's caller
//! is its own file's directory, as its `fileroot.this` is. A caller has an
//! *origin*, a number from a counter of [`MemoTables`], new for each caller
//! set from a file and shared by every frame it is passed on to, which is
//! what the memoisation of [Memoised functions](crate::eval::memo#memoised-functions)
//! follows. It is given only when it is needed: to a memoised call's
//! frame, and to a frame as it passes its caller on to a callee or a future
//! it makes; until then a frame's is none (`NO_ORIGIN`), which no running
//! memoised call can share. So a plain call sets a small index, a flag and
//! no origin, and costs one comparison more.
//!
//! - `fileroot.caller` (`IrKind::Caller`) yields the text of the frame's
//!   caller, from the table, a reference-count increment, after recording
//!   it as an ambient input of the running memoised calls it can affect.
//! - `fileroot.helper()` (`IrKind::Helper`) sets the frame's helper flag,
//!   for the rest of the frame: the calls it made before are unaffected,
//!   and every call it makes after passes its own caller on, so a library
//!   function that works on its caller's paths calls `fileroot.helper()`
//!   before calling another that does. It is `{}`. At a top level it does
//!   nothing: the top level's caller is its own file's directory, which it
//!   would pass on anyway.
//! - A `lazy` runs as if where it was written: the future captures, when
//!   it is made, the making frame's caller, helper flag and origin
//!   ([`crate::value::Future`]), and its body runs with them, whoever awaits
//!   it, so a step `lazy { ppk.commit(...) }` written in `test.gcsl` and
//!   awaited by a build driver elsewhere calls `commit` with `test.gcsl`'s
//!   directory.
//!
//! # Context
//!
//! The run's *current context* (see the checker's "Context") is a chain of
//! frames (`ContextFrame`), innermost first, `None` at the
//! root, kept where every frame of the run shares it, so a call inherits
//! its caller's and costs nothing for it. A `with` evaluates its bindings'
//! values in the current context, then its body in a new frame binding
//! them, each by its declaration's key, over the current one, which it
//! restores after, whatever the body does. A frame has an identity, new for
//! each `with` evaluated. A future captures the current context as it is
//! made, and its body runs with that context, whoever awaits it.
//!
//! A read (`Evaluator::context_read`) takes the nearest frame's binding of
//! its key; with none, it calls its default's function, as a call of no
//! arguments at its depth, in the current context, so that a default
//! reading another context follows it; with no default either, it is
//! "context `arch` has no value here". A read is an ambient input of the
//! memoised calls running (see [Memoised functions](crate::eval::memo#memoised-functions)):
//! it is recorded on each running call whose own context's nearest binding
//! of the key is the read's, the same frame's or none, with the value read,
//! or none for the default, whose value depends only on what it reads in
//! turn, recorded as it runs (`MemoTables::depend_context`). A call whose
//! context gives every such key the same value finds the entry. A cache hit
//! passes the entry's context dependencies on, as reads here would. Making
//! a future in a memoised call records every context the call's own
//! context binds, flattened, as one dependency (`MemoTables::depend_contexts`):
//! the future may read any of them after the call has returned.
//!
//! # Failure
//!
//! An assertion (`Evaluator::assert`) of a comparison evaluates its
//! operands, each once, as values, and compares them as the comparison's
//! own walker would (`Evaluator::compared`), so that a failed one labels
//! each operand with its value, as `show` writes it, without evaluating
//! anything twice; any other condition is evaluated as a `Bool`. A false
//! one is "assertion failed: MESSAGE", at the condition, its message
//! evaluated only then. A `recover` (`Evaluator::recover`) evaluates its
//! body; a failure of it that `#[retry]` would retry, an error with no
//! mark, being neither a bug in GCSL nor the passing of an enclosing call's
//! deadline (see [Retry and timeout](#retry-and-timeout)), is caught: its
//! message is pushed onto a stack in the run's shared state, which the
//! fallback's `Failure` reads as the record `{message}`, and popped once the
//! fallback has run. A deadline a call inside the body set is cleared by
//! that call, its own failure, so it is caught; `break`, `continue` and
//! `return` are no failures, and pass through. What the body ran stays run.
//!
//! # Secrets
//!
//! A secret, [`Value::Secret`], is a value and its public identity, shown
//! as `***` (see the checker's "Secrets"); it is never the value of another
//! secret. An operation on secrets is lowered as a `SecretWrap` of the
//! operation, each of whose operands is a `SecretValue`: one that is a
//! secret evaluates to its value, its identity pushed onto a stack in the
//! run's shared state (`Evaluator::secret_value`), and, for a comparison of
//! values holding secrets, every secret it holds is its value, their
//! identities pushed in the order met (`unsealed`). The wrap
//! (`Evaluator::secret_wrap`) makes what the operation computes a secret,
//! its identity the identities pushed while it ran, joined by plus signs,
//! as in `user + password`; a secret it computes, a secret's field that is
//! one, is its value, the identities joined with its own.
//!
//! A failure of the operation itself, a division by zero, an overflow or an
//! index past the end of a secret list, can tell of the secret: it is "an
//! operation on a secret failed", where it failed, naming nothing else, and
//! marked so that no `recover` catches and no `#[retry]` retries it
//! ([`crate::diagnostic`]'s `Mark::SECRET`). A failure of an operand is the
//! operand's, and passes through as it is: the operand's `SecretValue`
//! flags it, and the wrap clears the flag. An operand that is a secret
//! computed on secrets failed in a wrap of its own, already redacted. An
//! error with a mark passes through too: a bug in GCSL, and the passing of
//! an enclosing call's deadline, which that call turns into its own error
//! as ever.
//!
//! Anywhere else two secrets are compared, they are equal if their
//! identities are, whatever their values ([`crate::value::Secret`]), and a
//! memoised call's arguments hash a secret by its identity alone: total
//! equality, `===`, compares fields a value's type does not name, which may
//! hold a secret the checker cannot see, and a memoised call's cache hit is
//! seen in whether it runs, so neither may depend on a secret's value.
//! Two secrets of one identity are one secret to them: an identity names
//! its value, as a key's fingerprint does.
//!
//! # Metas
//!
//! A read of a function's meta, `func.versions` (see the checker's "Metas"),
//! is a `Field` of a `Call` of no arguments whose callee is a `Meta` of the
//! function: the function walker evaluates the `Meta` (`Evaluator::meta`)
//! by evaluating the function to its closure and taking the closure that
//! the cell of the top level's frame its code names holds
//! ([`Ir::meta_of`]), the closure of the definition's meta, which the
//! meta's file stored there as any top-level function's, before anything
//! that uses the definition ran. The closure of a def-type reference
//! carries its definition's meta: its code names, for its function, the
//! capture that holds the definition ([`MetaAt::Capture`]), whose closure's
//! meta is then taken as above; so a read through a reference and one
//! through the definition, as the def-type reads it, take the one closure of
//! the one meta. Then the call is a call like any other: a
//! `META->` meta's body runs at each read, a `META->>` meta's closure is
//! memoised as any closure of a memoised function is, by the closure, so
//! its body runs once per run however often it is read, and a `META:`
//! meta's runs at each read too, which is invisible; the body runs for the
//! caller the reading frame gives a callee, `fileroot.helper()` applying, and
//! its own code's directory is its `fileroot.this`. A closure whose code
//! names no meta, or a built-in function, is an internal error, which the
//! checker rules out. A function with a meta is a closure like any other:
//! equality, `is` and the keys of memoised calls see nothing of its meta.
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
//! *deadline*, kept in a stack in the run's shared state ([`Shared`]),
//! outermost first: its own instant, the attempt's start plus the limit,
//! and its effective one, the earlier of its own and the enclosing
//! deadline's, which a cell keeps for the innermost so that a check reads
//! one cell while none runs. The evaluator checks it at every call and every
//! iteration of a loop, so pure code that runs long is stopped too, and at
//! every `exec`, which runs its program until the deadline (`run_until`):
//! the program is looked at, at first often and then every
//! [`DEADLINE_POLL`](system::DEADLINE_POLL) at most, and killed when the deadline passes (the
//! program itself, not its own children). When one has passed, the error is
//! the outermost passed deadline's, "`f` timed out after 30s", where the
//! evaluation was, marked with the deadline's number ([`ErrorMark`]): every
//! call inside passes it on, unretried, and the call that set the deadline
//! clears the mark, which makes it that call's own failure, which its own
//! `#[retry]` may retry, timed afresh, and any call around it too.
//!

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::fmt::Write as _;
use std::num::NonZeroI64;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ast::FnId;
use crate::check::{Builtin, Slot};
use crate::diagnostic::{Diagnostic, Mark as ErrorMark};
use crate::ir::{
    ArgValue, CatchId, Cmp, ConstId, FnCode, FnKind, Ir, IrId, IrKind, IrNode, IrRange, LayoutId,
    LazyField, LazyShapeId, ListShape, MetaAt, Operands, RestShape, ShapeId, ShapedOperands,
    StepAt, StepMode, UpdatePathId, UpdateStep,
};
use crate::names::Name;
use crate::source::Span;
use crate::strings::FieldName;
use crate::types::ValueTest;
use crate::value::{
    Begin, Caller, Closure, ContextFrame, Future, FutureState, Layout, List, Quantity, Record,
    Secret, Tag, Value, ValueKind, Variant, int_to_float,
};
use crate::{CALL_LEVELS, MAX_DEPTH};

pub use memo::{MemoCheckpoint, MemoTables};
use memo::{Ambient, Lookup, Mark, MemoBegin, NO_ORIGIN, Running, key_eq};
use kinds::{
    BOOL, Extract, FLOAT, RECORD, STRING, Taken, amount_taken, as_function, float_to_int,
    into_callee, into_int, into_record, mixed, of_kind,
};
use unions::{failure, other_fields, passes, with_fields};
use builtins::{described, enum_builtin, secret_builtin};
pub use system::TempDirs;
use system::{exec, file_system, write_file};
pub use text::compare_versions;
use text::{join, sort_versions, split, string_list, strip_prefix, strip_suffix, trim};

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
/// built-in functions](#the-built-in-functions)). `Float` arithmetic never
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
    eval_with(ir, stack, &mut MemoTables::new(), (None, &[], None))
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
/// names (see [The built-in functions](#the-built-in-functions)): `None`
/// for a run without the standard library, which has no `exec` to call.
/// `dirs` is the run's table of directories, which the driver built, by
/// which the code of each file names its directory ([`Ir::dir`]): what
/// `fileroot.caller` reads (see [Callers and helpers](#callers-and-helpers)).
/// The top level's caller is its own file's directory. `temp_dirs` holds
/// the temporary directories `fs.temp_dir` makes, which the owner of the
/// run, a workspace or a session, holds for as long as it lives and removes
/// as it is dropped (see [The built-in functions](#the-built-in-functions)):
/// `None` for a run without one, in which a call of `fs.temp_dir` is an
/// internal error.
///
/// # Errors
///
/// As [`eval`]; and an internal error if `stack` is too small.
pub fn eval_with(
    ir: &Rc<Ir>,
    stack: &mut Vec<Value>,
    memo: &mut MemoTables,
    extras: (Option<&Rc<Layout>>, &[Rc<str>], Option<&TempDirs>),
) -> Result<Value, Diagnostic> {
    eval_rooted(ir, (stack, memo), extras, None)
}

/// [`eval_with`], in the root context `context`, the frame of the contexts
/// a host sets for the evaluation, which every frame of it inherits (see
/// [Context](#context)); `None` for none.
pub(crate) fn eval_rooted(
    ir: &Rc<Ir>,
    (stack, memo): (&mut Vec<Value>, &mut MemoTables),
    (exec_result, dirs, temp_dirs): (Option<&Rc<Layout>>, &[Rc<str>], Option<&TempDirs>),
    context: Option<Rc<ContextFrame>>,
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
    let shared = Shared::new(exec_result, dirs, temp_dirs);
    shared.context.replace(context);
    let result = Evaluator {
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
    .value(root);
    stack.truncate(frame_size);
    // The checker lets no `break` or `continue` out of the top level's
    // loops, so a stop arriving here is a bug.
    result.map_err(|unwind| unwind.into_error(ir.get(root).span))
}

/// Why the evaluation of a node ended without a value: the one road out of
/// a node, which every walker passes along with `?` (see [Stops](#stops)).
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
    /// is to its call, memoised call or await (see [Stops](#stops)): the
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
    /// directory (see [Callers and helpers](#callers-and-helpers)); empty in
    /// a run with no workspace's context, whose code cannot read
    /// `fileroot.caller`.
    dirs: &'a [Rc<str>],
    /// The temporary directories of the run's owner, which `fs.temp_dir`
    /// makes; `None` in a run without one.
    temp_dirs: Option<&'a TempDirs>,
    /// The deadlines of the calls with `#[timeout]` that are running,
    /// outermost first (see [Retry and timeout](#retry-and-timeout)).
    deadlines: RefCell<Vec<Deadline>>,
    /// The innermost deadline's effective instant, the earliest of all,
    /// while one is running: what every check reads first.
    earliest: Cell<Option<Instant>>,
    /// The number of the last deadline made.
    deadlines_made: Cell<u32>,
    /// The context the frame running is evaluated in, innermost frame first
    /// (see [Context](#context)): `None`, binding nothing, at the root.
    context: RefCell<Option<Rc<ContextFrame>>>,
    /// The message of the failure each `recover` running its fallback
    /// caught, innermost last (see [Failure](#failure)).
    failures: RefCell<Vec<Rc<str>>>,
    /// The identities of the secrets the operations on secrets running have
    /// computed on so far, innermost last (see [Secrets](#secrets)).
    identities: RefCell<Vec<Rc<str>>>,
    /// Whether the failure unwinding to the innermost operation on secrets
    /// is one of its operands', not its own (see [Secrets](#secrets)).
    operand_failed: Cell<bool>,
}

impl<'a> Shared<'a> {
    /// What the frames of a run with the layout of `exec`'s result
    /// `exec_result`, the table of directories `dirs` and the temporary
    /// directories `temp_dirs` share, with no deadline running.
    fn new(
        exec_result: Option<&'a Rc<Layout>>,
        dirs: &'a [Rc<str>],
        temp_dirs: Option<&'a TempDirs>,
    ) -> Self {
        Self {
            exec_result,
            dirs,
            temp_dirs,
            deadlines: RefCell::new(Vec::new()),
            earliest: Cell::new(None),
            deadlines_made: Cell::new(0),
            context: RefCell::new(None),
            failures: RefCell::new(Vec::new()),
            identities: RefCell::new(Vec::new()),
            operand_failed: Cell::new(false),
        }
    }
}

/// The deadline of a running call of a function with `#[timeout]` (see
/// [Retry and timeout](#retry-and-timeout)).
#[derive(Debug)]
struct Deadline {
    /// Its number, unique in the run: what the error of its passing is
    /// marked with ([`ErrorMark::deadline`]).
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
    /// [Callers and helpers](#callers-and-helpers)).
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
            | IrKind::SecretWrap(_) => self.whole_node(node),
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
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Show(_) | IrKind::Caller => {
                self.string(id).map(Value::Str)
            }
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

    /// The closure of the meta of the function that `function` evaluates to,
    /// for the `Meta` `node` (see [Metas](#metas)): the closure of a
    /// definition's meta, which the cell of the top level's frame that the
    /// definition's code names holds ([`Ir::meta_of`]); for a def-type
    /// reference's closure, which carries its definition's meta, that of the
    /// definition it captures ([`MetaAt::Capture`]). A built-in function, or
    /// a closure whose code names no meta, which the checker rules out, is
    /// an internal error. Out of line: only a meta's read pays for it.
    #[inline(never)]
    fn meta(&mut self, function: IrId, node: &IrNode) -> Flow<Callee> {
        let Callee::Closure(mut closure) = self.function(function)? else {
            return Err(internal("the meta of a built-in function read", node.span).into());
        };
        loop {
            match closure.code.meta_of(closure.function) {
                Some(MetaAt::Cell(slot)) => return self.top_function(slot, node),
                Some(MetaAt::Capture(index)) => {
                    let Some(Value::Fn(captured)) = closure.captures.get(index as usize) else {
                        return Err(internal(
                            "the meta of a reference that captures no definition read",
                            node.span,
                        )
                        .into());
                    };
                    closure = Rc::clone(captured);
                }
                None => {
                    return Err(
                        internal("the meta of a function that has none read", node.span).into(),
                    );
                }
            }
        }
    }

    /// [`Evaluator::meta`] as a value. Out of line, as `meta` is.
    #[inline(never)]
    fn meta_value(&mut self, function: IrId, node: &IrNode) -> Flow<Value> {
        match self.meta(function, node)? {
            Callee::Closure(closure) => Ok(Value::Fn(closure)),
            Callee::Builtin(builtin) => Ok(Value::Builtin(builtin)),
            Callee::Tag(tag) => Ok(Value::Tag(tag)),
        }
    }

    /// A new reference to the running closure, for the `LoadCallee`
    /// `node`.
    fn callee(&self, node: &IrNode) -> Flow<Rc<Closure>> {
        self.callee
            .map(Rc::clone)
            .ok_or_else(|| internal("load of the callee outside a function", node.span).into())
    }

    /// Evaluates the `Caller` `node`, `fileroot.caller`: the text of this
    /// frame's caller, from the run's table, after recording it as an
    /// ambient dependency of every running memoised call of its origin (see
    /// [Callers and helpers](#callers-and-helpers)). A caller outside the
    /// table, which the driver never gives, is an internal error.
    #[inline(never)]
    fn caller_text(&mut self, node: &IrNode) -> Flow<Rc<str>> {
        // An origin not given yet is no running memoised call's.
        if self.caller.origin != NO_ORIGIN {
            self.memo
                .depend(self.caller.origin, &Ambient::Caller(self.caller.dir));
        }
        self.shared
            .dirs
            .get(self.caller.dir as usize)
            .map(Rc::clone)
            .ok_or_else(|| internal("a caller outside the run's directories", node.span).into())
    }

    /// This frame's caller, its origin given now if it had none, as it must
    /// be before another frame shares it: a callee it passes its caller on
    /// to, or a future it makes (see [Callers and
    /// helpers](#callers-and-helpers)).
    fn shared_caller(&mut self) -> Caller {
        if self.caller.origin == NO_ORIGIN {
            self.caller.origin = self.memo.new_origin();
        }
        self.caller
    }

    /// Evaluates `node`, an `Empty` or a `Helper`, both `{}`: a `Helper`,
    /// `fileroot.helper()`, first sets this frame's helper flag, so that the
    /// calls it makes from now on pass its own caller on (see [Callers and
    /// helpers](#callers-and-helpers)).
    fn empty(&mut self, node: &IrNode) -> Value {
        if matches!(node.kind, IrKind::Helper) {
            self.caller.helper = true;
        }
        Value::Empty
    }

    /// The caller a call this frame makes gives its callee (see [Callers and
    /// helpers](#callers-and-helpers)): this frame's own, sharing its
    /// origin, given now if it had none, once `fileroot.helper()` has run in
    /// it; else the directory of the file this frame's code is of, with no
    /// origin yet, which the callee is given only if it needs one. The
    /// callee's helper flag is false.
    fn callee_caller(&mut self) -> Caller {
        if self.caller.helper {
            Caller {
                helper: false,
                ..self.shared_caller()
            }
        } else {
            Caller {
                dir: self.ir.dir(),
                helper: false,
                origin: NO_ORIGIN,
            }
        }
    }

    /// Builds a closure of `function` capturing the values of `captures`,
    /// read in this frame: one allocation for the captured values, sized
    /// exactly (none if there are none), and one for the closure, which
    /// shares the IR.
    fn make_closure(&mut self, function: FnId, captures: IrRange) -> Flow<Rc<Closure>> {
        let items = self.ir.items(captures);
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.value(item)?);
        }
        Ok(Rc::new(Closure {
            code: Rc::clone(self.ir),
            function,
            captures: values.into_boxed_slice(),
        }))
    }

    /// Evaluates `node`, one each walker evaluates as a whole value: a read
    /// of a context or a `with` (see [Context](#context)), an assertion, a
    /// `recover` or a failure (see [Failure](#failure)).
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
            IrKind::SecretWrap(computed) => self.secret_wrap(computed, node),
            _ => mistyped(node, "a node evaluated whole that is none"),
        }
    }

    /// Evaluates the assertion `node`, of the condition `cond` and the
    /// message `message` (see [Failure](#failure)): a comparison's operands
    /// each once, as values, compared as the comparison would compare them
    /// ([`Evaluator::compared`]), and any other condition as a `Bool`; if it
    /// is false, "assertion failed: MESSAGE", at the condition, each operand
    /// of a comparison labelled with its value, as `show` writes it.
    #[inline(never)]
    fn assert(&mut self, (cond, message): (IrId, IrId), node: &IrNode) -> Flow<Value> {
        let condition = self.ir.get(cond);
        let (holds, shown) = match comparison_operands(condition.kind) {
            Some((lhs, rhs)) => {
                let (left, right) = (self.value(lhs)?, self.value(rhs)?);
                let holds = self.compared(condition, (&left, &right))?;
                (holds, Some([(lhs, left), (rhs, right)]))
            }
            None => (self.bool(cond)?, None),
        };
        if holds {
            return Ok(Value::Empty);
        }
        let message = self.string(message)?;
        let mut error = Diagnostic::error(format!("assertion failed: {message}"), condition.span);
        for (operand, value) in shown.into_iter().flatten() {
            let text =
                source_of(&value, node).map_or_else(|_| value.to_string(), |text| text.to_string());
            error = error.with_context(self.ir.get(operand).span, format!("this is `{text}`"));
        }
        Err(error.into())
    }

    /// Whether the comparison `node` holds of `lhs` and `rhs`, its operands'
    /// values, as the walker of its kind would compare them.
    fn compared(&self, node: &IrNode, (lhs, rhs): (&Value, &Value)) -> Flow<bool> {
        let amount = |value: &Value| match *value {
            Value::Int(amount) | Value::Quantity(_, amount) => Some(amount),
            _ => None,
        };
        let held = match (node.kind, lhs, rhs) {
            (IrKind::IntCmp(cmp, _), _, _) => {
                amount(lhs).zip(amount(rhs)).map(|(a, b)| cmp.test(&a, &b))
            }
            (IrKind::FloatCmp(cmp, _), Value::Float(a), Value::Float(b)) => Some(cmp.test(a, b)),
            (IrKind::StrCmp(cmp, _), Value::Str(a), Value::Str(b)) => Some(cmp.test(&**a, &**b)),
            (IrKind::BoolEq(equal, _), Value::Bool(a), Value::Bool(b)) => Some((a == b) == equal),
            (IrKind::RecordEq(o) | IrKind::ListEq(o), _, _) => {
                record_eq_shaped(self.ir, o.shape, lhs, rhs)
            }
            (IrKind::RecordNe(o) | IrKind::ListNe(o), _, _) => {
                record_eq_shaped(self.ir, o.shape, lhs, rhs).map(|same| !same)
            }
            (IrKind::RecordSame(_) | IrKind::ListSame(_) | IrKind::ValueSame(_), _, _) => {
                Some(lhs == rhs)
            }
            (IrKind::RecordNotSame(_) | IrKind::ListNotSame(_) | IrKind::ValueNotSame(_), _, _) => {
                Some(lhs != rhs)
            }
            _ => None,
        };
        held.map_or_else(
            || mistyped(node, "a comparison of values of another kind"),
            Ok,
        )
    }

    /// Evaluates `body`, the body of a `recover` (see [Failure](#failure)),
    /// and, if it fails as `#[retry]` would retry, with no mark, `fallback`
    /// instead, with its failure the innermost a `Failure` reads.
    #[inline(never)]
    fn recover(&mut self, body: IrId, fallback: IrId) -> Flow<Value> {
        match self.value(body) {
            Err(Unwind::Error(error)) if error.mark == ErrorMark::NONE => {
                self.shared
                    .failures
                    .borrow_mut()
                    .push(Rc::from(error.message));
                let result = self.value(fallback);
                self.shared.failures.borrow_mut().pop();
                result
            }
            result => result,
        }
    }

    /// The value of `secret`'s value, if it is a secret, its identity kept
    /// for the innermost operation on secrets, or the value itself; if
    /// `deep`, with every secret it holds its value, and their identities
    /// kept too, in order (see [Secrets](#secrets)).
    fn secret_value(&mut self, secret: IrId, deep: bool) -> Flow<Value> {
        let value = match self.value(secret) {
            Ok(value) => value,
            Err(unwind) => {
                // A failure of an operand, none of the operation's own.
                self.shared.operand_failed.set(true);
                return Err(unwind);
            }
        };
        let mut identities = self.shared.identities.borrow_mut();
        if deep {
            return Ok(unsealed(value, &mut identities));
        }
        match value {
            Value::Secret(secret) => {
                identities.push(Rc::clone(&secret.identity));
                Ok(secret.value.clone())
            }
            value => Ok(value),
        }
    }

    /// Evaluates `computed`, an operation on the values of secrets, for the
    /// `SecretWrap` `node` (see [Secrets](#secrets)): its value as a secret
    /// whose identity is the identities of the secrets it computed on,
    /// joined by ` + ` in order; a failure of it is "an operation on a
    /// secret failed", where it failed, naming nothing else, which nothing
    /// recovers or retries, since whether it fails can depend on a secret.
    /// A failure of one of its operands is that operand's, and passes
    /// through as it is: an operand that is a secret computed on secrets
    /// failed in an operation of its own. So does an error with a mark: a
    /// bug in GCSL, the
    /// passing of an enclosing call's deadline, which that call turns into
    /// its own error as ever, and the failure of an operation on a secret
    /// inside this one, already redacted where it failed.
    #[inline(never)]
    fn secret_wrap(&mut self, computed: IrId, node: &IrNode) -> Flow<Value> {
        let depth = self.shared.identities.borrow().len();
        let result = self.value(computed);
        let identities: Vec<Rc<str>> = self.shared.identities.borrow_mut().split_off(depth);
        let of_operand = self.shared.operand_failed.replace(false);
        match result {
            Ok(value) => {
                // A value that is a secret already, a secret's field that
                // is one, is its value: a secret's value is never a secret.
                let (value, inner) = match value {
                    Value::Secret(secret) => {
                        (secret.value.clone(), Some(Rc::clone(&secret.identity)))
                    }
                    value => (value, None),
                };
                let identity: Vec<&str> =
                    identities.iter().chain(&inner).map(AsRef::as_ref).collect();
                Ok(Value::Secret(Rc::new(Secret {
                    value,
                    identity: Rc::from(identity.join(" + ")),
                })))
            }
            Err(Unwind::Error(error)) if error.mark == ErrorMark::NONE && !of_operand => {
                let mut failed = Diagnostic::error("an operation on a secret failed", node.span);
                failed.mark = ErrorMark::SECRET;
                failed.input = error.input;
                Err(failed.into())
            }
            Err(unwind) => Err(unwind),
        }
    }

    /// The failure the innermost `recover` running its fallback caught, a
    /// record of the layout `layout`, its one field its message.
    fn failure(&self, layout: LayoutId, node: &IrNode) -> Flow<Value> {
        let Some(message) = self.shared.failures.borrow().last().cloned() else {
            return mistyped(node, "a failure read outside a `recover`'s fallback");
        };
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::clone(self.ir.layout(layout)),
            values: Box::new([Value::Str(message)]),
        })))
    }

    /// The value of the read `node` of the context of the declaration key
    /// `key` (see [Context](#context)): its nearest binding's in the running
    /// context; with none, the value its default's function, `default`,
    /// returns, called with no arguments as a call at depth `depth` would
    /// be; with no default either, "context `arch` has no value here".
    /// Recorded as a dependency of the running memoised calls it can affect
    /// (see [Memoised functions](crate::eval::memo#memoised-functions), "Ambient inputs").
    #[inline(never)]
    fn context_read(
        &mut self,
        key: Name,
        (default, depth): (IrId, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let context = self.shared.context.borrow().clone();
        self.memo.depend_context(key, context.as_ref());
        if let Some((_, value)) = ContextFrame::find(context.as_ref(), key) {
            return Ok(value.clone());
        }
        if self.ir.get(default).kind == IrKind::Empty {
            let name = self
                .ir
                .name_text(key)
                .and_then(crate::names::key_context)
                .unwrap_or_default();
            return Err(Diagnostic::error(
                format!("context `{name}` has no value here"),
                node.span,
            )
            .with_note("no `with` sets it, and it declares no default")
            .into());
        }
        let Callee::Closure(closure) = self.function(default)? else {
            return mistyped(node, "a context's default that is no closure");
        };
        let Some(code) = closure.code.find_function(closure.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        let passed = self.callee_caller();
        let result = Evaluator {
            ir: &closure.code,
            stack: self.stack,
            base,
            captures: &closure.captures,
            callee: Some(&closure),
            memo: self.memo,
            shared: self.shared,
            caller: passed,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Err(mut error) if error.input.is_none() && closure.code.input() != self.ir.input() => {
                error.input = Some(closure.code.input());
                Err(error.into())
            }
            result => result.map_err(Unwind::from),
        }
    }

    /// Evaluates the `with` `node`, of the bindings and then the body
    /// `parts` (see [Context](#context)): each binding's value, in order, in
    /// the running context, and then the body, as a value, in a new frame of
    /// the context binding them over the running one, which is restored
    /// after, whatever the body does.
    #[inline(never)]
    fn with(&mut self, parts: IrRange, node: &IrNode) -> Flow<Value> {
        let Some((&body, binds)) = self.ir.items(parts).split_last() else {
            return mistyped(node, "a `with` with no body");
        };
        let mut bound = Vec::with_capacity(binds.len());
        for &item in binds {
            let IrKind::ContextBind { key, value } = self.ir.get(item).kind else {
                return mistyped(node, "a `with` binding that is none");
            };
            bound.push((key, self.value(value)?));
        }
        let frame = ContextFrame {
            id: self.memo.new_context_frame(),
            binds: bound.into_boxed_slice(),
            outer: self.shared.context.borrow().clone(),
        };
        let outer = self.shared.context.replace(Some(Rc::new(frame)));
        let result = self.value(body);
        self.shared.context.replace(outer);
        result
    }

    /// Builds a future of `function` capturing the values of `captures`,
    /// read in this frame, as [`Evaluator::make_closure`] builds a closure:
    /// one allocation for the captured values and one for the future, which
    /// shares the IR. Nothing is run. The future captures this frame's
    /// caller, helper flag and origin, which its body runs with (see
    /// [Callers and helpers](#callers-and-helpers)). Making it is a read of
    /// this frame's caller, recorded on the running memoised calls of its
    /// origin, if the body can read the caller it captures (`reads_caller`,
    /// the `lazy`'s mark) or this frame's helper flag is set, so that its
    /// body's calls pass that caller on (see [Memoised
    /// functions](crate::eval::memo#memoised-functions), "Ambient inputs").
    fn make_future(
        &mut self,
        function: FnId,
        captures: IrRange,
        reads_caller: bool,
    ) -> Flow<Value> {
        let items = self.ir.items(captures);
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.value(item)?);
        }
        let made = self.shared_caller();
        if reads_caller || made.helper {
            self.memo.depend(made.origin, &Ambient::Caller(made.dir));
        }
        // Its body may read any context it captures (see [Context](#context)).
        self.memo.depend_contexts();
        let context = self.shared.context.borrow().clone();
        Ok(Value::Future(Rc::new(Future::made_by(
            Rc::clone(self.ir),
            function,
            values.into_boxed_slice(),
            (made, context),
        ))))
    }

    /// Evaluates the `Await` `node` (see the module documentation) as a
    /// value, which must be of the kind the node states, or, for
    /// [`ValueKind::Mixed`], of any kind but a future's.
    fn await_node(&mut self, node: &IrNode) -> Flow<Value> {
        let value = self.await_value(node)?;
        match node.kind {
            IrKind::Await { kind, .. } if of_kind(&value, kind) => Ok(value),
            _ => mistyped(
                node,
                "an await produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `As` `node` (see the module documentation) as a value,
    /// which must be of the kind the node states, or, for
    /// [`ValueKind::Mixed`], of any kind but a future's.
    fn narrow_node(&mut self, node: &IrNode) -> Flow<Value> {
        let value = self.narrowed(node)?;
        match node.kind {
            IrKind::As { kind, .. } if of_kind(&value, kind) => Ok(value),
            _ => mistyped(
                node,
                "an `as` produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `As` `node` as a value that `extract` accepts: one of
    /// the kind the node states.
    fn narrow_as<T>(&mut self, node: &IrNode, extract: fn(Value) -> Option<T>) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.narrowed(node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "an `as` produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `As` `node`: its value, through the general walker,
    /// which it yields if it passes the node's test, and otherwise "`x` is
    /// not an Int", at the node, labelled with what the value is (see
    /// [`failure`]).
    fn narrowed(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::As { value, test, .. } = node.kind else {
            return mistyped(node, "a node that is not an `as` narrowed");
        };
        let value = self.value(value)?;
        let ir = self.ir;
        let Some(code) = ir.test(test) else {
            return mistyped(node, "an `as` of a test its IR lacks");
        };
        if passes(&code.test, &value) {
            return Ok(value);
        }
        Err(Diagnostic::error(
            format!("{} is not {}", code.subject, code.target),
            node.span,
        )
        .with_label(failure(ir, &code.test, &value))
        .into())
    }

    /// Evaluates the `Is` `node`: its value, through the general walker,
    /// written to the node's slot if it has one, and whether it passes the
    /// node's test.
    fn is_test(&mut self, node: &IrNode) -> Flow<bool> {
        let IrKind::Is { value, test, slot } = node.kind else {
            return mistyped(node, "a node that is not an `is` tested");
        };
        let value = self.value(value)?;
        let Some(code) = self.ir.test(test) else {
            return mistyped(node, "an `is` of a test its IR lacks");
        };
        let passed = passes(&code.test, &value);
        if slot != Slot::NONE {
            let target = self
                .stack
                .get_mut(self.base + slot.index())
                .ok_or_else(|| internal("store to a slot outside the frame", node.span))?;
            *target = value;
        }
        Ok(passed)
    }

    /// Evaluates a total comparison of two values of a union of kinds, a
    /// `ValueSame` if `equal` and a `ValueNotSame` otherwise: the values
    /// compare whole, deeply, as [`Value`]'s equality does.
    fn value_same(&mut self, operands: Operands, equal: bool) -> Flow<bool> {
        let lhs = self.value(operands.lhs)?;
        let rhs = self.value(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates the `Await` `node` as a value that `extract` accepts: one
    /// of the kind the node states.
    fn await_as<T>(&mut self, node: &IrNode, extract: fn(Value) -> Option<T>) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.await_value(node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "an await produced a value of another kind than its node",
            ),
        }
    }

    /// Evaluates the `Await` `node`: its value, through the general walker,
    /// awaited fully and then, if it has a lazy shape, deeply.
    fn await_value(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::Await {
            value,
            shape,
            depth,
            ..
        } = node.kind
        else {
            return mistyped(node, "a node that is not an await awaited");
        };
        let awaited = self.value(value)?;
        let awaited = self.await_fully(awaited, depth, 0, node)?;
        if shape == LazyShapeId::NONE {
            Ok(awaited)
        } else {
            // The fields of the record awaited are one level of nesting
            // below it.
            self.await_deep(awaited, shape, (depth, 1, false), node)
        }
    }

    /// Awaits `value` fully, for the await `node` at depth `depth` in this
    /// frame's body, `level` levels of record nesting below the value it
    /// awaits: while it is a future, runs it and continues with what it
    /// yields.
    fn await_fully(
        &mut self,
        value: Value,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Flow<Value> {
        let mut value = value;
        loop {
            match value {
                Value::Future(future) => value = self.run_future(&future, depth, level, node)?,
                present => return Ok(present),
            }
        }
    }

    /// Awaits the lazy fields of `value`, a record, as lazy shape `shape`
    /// lists them, for the await `node` at depth `depth` in this frame's
    /// body, the record's fields being `level` levels of record and list
    /// nesting below the value the await awaits: each field that is lazy is
    /// awaited fully and then, if its shape says so, deeply, and each
    /// present record or list field with lazy values inside is awaited
    /// deeply. If any field changed, the result is a new record sharing the
    /// layout and the values that did not; otherwise it is the record as it
    /// was. The empty record has no fields to await. A list's elements are
    /// awaited by [`Evaluator::await_elements`] instead. A union's shape
    /// applies each of its members' in turn, `tolerant`ly: a field the
    /// record lacks, or a shape of another kind than the value's, is passed
    /// over, there and at every level below (see [`Ir::lazy_union`]), where
    /// otherwise it is an internal error.
    fn await_deep(
        &mut self,
        value: Value,
        shape: LazyShapeId,
        (depth, level, tolerant): (u16, usize, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let ir = self.ir;
        if let Some(members) = ir.lazy_union(shape) {
            let mut value = value;
            for &member in members {
                value = self.await_deep(value, member, (depth, level, true), node)?;
            }
            return Ok(value);
        }
        if matches!(value, Value::List(_)) {
            return self.await_elements(value, shape, (depth, level, tolerant), node);
        }
        let Value::Record(record) = &value else {
            return Ok(value);
        };
        let Some(lazy) = ir.find_lazy_shape(shape) else {
            return mistyped(node, "an await of a lazy shape its IR lacks");
        };
        let mut changed: Option<Vec<Value>> = None;
        for &(name, field) in &lazy.fields {
            let found = record.layout.names.iter().position(|&field| field == name);
            let Some((index, old)) =
                found.and_then(|index| Some((index, record.values.get(index)?)))
            else {
                if tolerant {
                    continue;
                }
                return mistyped(node, "a record lacks a field of its lazy shape");
            };
            let new = self.await_field(old, field, (depth, level, tolerant), node)?;
            if replaced(old, &new) {
                let values = changed.get_or_insert_with(|| record.values.to_vec());
                if let Some(slot) = values.get_mut(index) {
                    *slot = new;
                }
            }
        }
        // Every field the type does not name, as its rest-field says.
        if let Some(rest) = &lazy.rest {
            for (index, (name, old)) in record.layout.names.iter().zip(&record.values).enumerate() {
                if rest.named.binary_search(name).is_ok() {
                    continue;
                }
                let new = self.await_field(old, rest.field, (depth, level, tolerant), node)?;
                if replaced(old, &new) {
                    let values = changed.get_or_insert_with(|| record.values.to_vec());
                    if let Some(slot) = values.get_mut(index) {
                        *slot = new;
                    }
                }
            }
        }
        match changed {
            Some(values) => Ok(Value::Record(Rc::new(Record {
                layout: Rc::clone(&record.layout),
                values: values.into_boxed_slice(),
            }))),
            None => Ok(value),
        }
    }

    /// Awaits `old`, a field of a record or an element of a list, as
    /// `field` says, for the await `node` at depth `depth` in this frame's
    /// body, `level` levels of record and list nesting below the value the
    /// await awaits: fully and then, if its shape says so, deeply, or, if
    /// it is present, deeply (see [`Evaluator::await_deep`]).
    fn await_field(
        &mut self,
        old: &Value,
        field: LazyField,
        (depth, level, tolerant): (u16, usize, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        match field {
            LazyField::Await(inner) => {
                let awaited = self.await_fully(old.clone(), depth, level, node)?;
                if inner == LazyShapeId::NONE {
                    Ok(awaited)
                } else {
                    self.await_deep(awaited, inner, (depth, level + 1, tolerant), node)
                }
            }
            LazyField::Within(inner) => {
                self.await_deep(old.clone(), inner, (depth, level + 1, tolerant), node)
            }
        }
    }

    /// Awaits the elements of `value`, a list, as the list's lazy shape
    /// `shape` says to await each, for the await `node` at depth `depth` in
    /// this frame's body, the elements being `level` levels of record and
    /// list nesting below the value the await awaits, as
    /// [`Evaluator::await_deep`] awaits a record's fields: each element is
    /// awaited fully and then, if the shape says so, deeply, or, if it is
    /// present, deeply. If any element changed, the result is a new list,
    /// sharing the elements that did not; otherwise it is the list as it
    /// was. A shape that is not a list's is passed over if `tolerant`, as
    /// a member's shape of a union's is (see [`Evaluator::await_deep`]).
    fn await_elements(
        &mut self,
        value: Value,
        shape: LazyShapeId,
        (depth, level, tolerant): (u16, usize, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let Value::List(list) = &value else {
            return Ok(value);
        };
        let Some(each) = self.ir.lazy_list(shape) else {
            if tolerant {
                return Ok(value);
            }
            return mistyped(node, "an await of a list's lazy shape its IR lacks");
        };
        let mut changed: Option<Vec<Value>> = None;
        for (index, old) in list.values().iter().enumerate() {
            let new = self.await_field(old, each, (depth, level, tolerant), node)?;
            if replaced(old, &new) {
                let values = changed.get_or_insert_with(|| list.values().to_vec());
                if let Some(slot) = values.get_mut(index) {
                    *slot = new;
                }
            }
        }
        match changed {
            Some(values) => Ok(Value::List(List::from_vec(values))),
            None => Ok(value),
        }
    }

    /// Runs `future` for the await `node` at depth `depth` in this frame's
    /// body, `level` levels of record nesting below the value the await
    /// awaits (see the module documentation), and returns what it yields:
    /// the value it recorded, if it has run; otherwise its body's value,
    /// run in a frame of its own, which it then records. The future
    /// equivalent of [`Evaluator::call`]: it keeps only the future, its
    /// frame's base and the new evaluator across the body's evaluation.
    fn run_future(
        &mut self,
        future: &Rc<Future>,
        depth: u16,
        level: usize,
        node: &IrNode,
    ) -> Flow<Value> {
        let Some(code) = future.code.find_function(future.function) else {
            return mistyped(node, "a future of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + level + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let captures = match future.begin() {
            Some(Begin::Done(value)) => return Ok(value),
            Some(Begin::Running) => return Err(depends_on_itself(node).into()),
            Some(Begin::Pending(captures)) => captures,
            None => return mistyped(node, "a future's state is in use"),
        };
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        // The body runs as if where it was written, with the caller, the
        // helper flag and the origin of the frame that made the future, in
        // the context it was made in.
        let outer = self.shared.context.replace(future.context.clone());
        let result = Evaluator {
            ir: &future.code,
            stack: self.stack,
            base,
            captures: &captures,
            callee: None,
            memo: self.memo,
            shared: self.shared,
            caller: future.made,
            levels: entering,
        }
        .value(code.body);
        self.shared.context.replace(outer);
        self.stack.truncate(base);
        // A `return` for the `lazy` is its value, recorded as any is, here,
        // inside the running mark; no other stop leaves its body: the
        // checker sees to it.
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Ok(value) => {
                // The captures are dropped on return: a future that has
                // run holds only its value.
                if future.finish(FutureState::Done(value.clone())) {
                    Ok(value)
                } else {
                    mistyped(node, "a future's state is in use")
                }
            }
            Err(error) => {
                // Left unrun, so that a later await runs it again.
                if future.finish(FutureState::Pending(captures)) {
                    Err(self.awaited_error(error, future, node).into())
                } else {
                    mistyped(node, "a future's state is in use")
                }
            }
        }
    }

    /// `error`, raised while `future` ran for the await `node`, as it is
    /// reported: labelled "awaited here" at the await when the future's
    /// code is of the same session input as this frame's, and stamped with
    /// the future's input when it is not, unless an inner frame has stamped
    /// an input on it already.
    fn awaited_error(&self, mut error: Diagnostic, future: &Future, node: &IrNode) -> Diagnostic {
        if error.input.is_some() {
            return error;
        }
        if future.code.input() == self.ir.input() {
            error.with_context(node.span, "awaited here")
        } else {
            error.input = Some(future.code.input());
            error
        }
    }

    /// Evaluates the `Call` `node` of `callee` with the arguments `args`,
    /// at depth `depth` in this frame's body (see the module documentation),
    /// and returns the body's value; or, if the callee is a closure of a
    /// memoised function, hands the call to [`Evaluator::memo_call`].
    fn call(&mut self, (callee, args, depth): (IrId, IrRange, u16), node: &IrNode) -> Flow<Value> {
        let closure = match self.function(callee)? {
            Callee::Closure(closure) => closure,
            Callee::Builtin(builtin) => return self.call_builtin(builtin, args, node),
            Callee::Tag(tag) => return self.call_tag(tag, args, node),
        };
        let Some(code) = closure.code.find_function(closure.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        if self.past_deadline() {
            return self.timed_out(node);
        }
        if code.kind == FnKind::Memo {
            return self.memo_call((&closure, code), (args, depth), node);
        }
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        // An argument may fail, or stop for a loop around the call, after
        // the frame has grown: either way the frame goes.
        if let Err(unwind) = self.arguments(args, base, (&closure, depth)) {
            self.stack.truncate(base);
            return Err(unwind);
        }
        let passed = self.callee_caller();
        if code.retry > 0 || code.timeout.is_some() {
            return self.guarded_call(&closure, base, (passed, entering), node);
        }
        let result = Evaluator {
            ir: &closure.code,
            stack: self.stack,
            base,
            captures: &closure.captures,
            callee: Some(&closure),
            memo: self.memo,
            shared: self.shared,
            caller: passed,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        // A `return` for the function is its value; no other stop leaves
        // its body: the checker sees to it.
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Err(mut error) if error.input.is_none() && closure.code.input() != self.ir.input() => {
                error.input = Some(closure.code.input());
                Err(error.into())
            }
            result => result.map_err(Unwind::from),
        }
    }

    /// Finishes the call `node` of `closure`, whose function has `#[retry]`
    /// or `#[timeout]`, its frame at `base` holding its arguments, with the
    /// caller `caller`, at `levels` of nesting: runs its body as
    /// [`Evaluator::guarded_body`] does, then truncates the stack back and
    /// gives the result, as [`Evaluator::call`] does for any other call.
    #[inline(never)]
    fn guarded_call(
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
    fn guarded_body(
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
    fn push_deadline(&self, limit: Duration, message: &str) -> u32 {
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
    fn pop_deadline(&self, number: u32) {
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
    fn past_deadline(&self) -> bool {
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
    fn timed_out<T>(&self, node: &IrNode) -> Flow<T> {
        let now = Instant::now();
        let deadlines = self.shared.deadlines.borrow();
        let Some(passed) = deadlines.iter().find(|deadline| deadline.own <= now) else {
            return mistyped(node, "a deadline passed that no running call has");
        };
        let mut error = Diagnostic::error(&*passed.message, node.span);
        error.mark = ErrorMark::deadline(passed.number);
        Err(error.into())
    }

    /// Evaluates the `Call` `node` of the built-in function `builtin` with
    /// the arguments `args`: a call through a value of its function type (a
    /// parameter, a variable, the prelude's `int`), the one call whose node
    /// cannot say what it calls. The arguments are evaluated in order, as a
    /// closure's are, each collected by the index of the parameter it
    /// fills, and the built-in applied to them (see [The built-in
    /// functions](#the-built-in-functions)): `int` truncates its `Float`
    /// toward zero, failing with "integer overflow" when that is not an
    /// `Int` (an infinity, or beyond `i64`) and "cannot truncate NaN to an
    /// Int" for NaN; `float` gives the `Float` nearest its `Int`; `exec`
    /// runs its program, and `exec_live` runs it so, copying its output to
    /// this process's standard error as it runs ([`exec`]); `panic` fails
    /// with its message, at the call; `split` ([`split`]), `join`
    /// ([`join`]), `starts_with`,
    /// `ends_with`, `trim` ([`trim`]), `strip_prefix` ([`strip_prefix`]),
    /// `strip_suffix` ([`strip_suffix`]), `lines` and `fields`
    /// ([`string_list`]) work on their strings as Rust's `str` does,
    /// `compare_versions` ([`compare_versions`]) gives `-1`, `0` or `1` by
    /// version order, `sort_versions` ([`sort_versions`]) sorts by it, and
    /// A new temporary directory, for the call `node` of `fs_temp_dir`:
    /// removed with everything in it when the run's owner drops it.
    fn temp_dir(&self, node: &IrNode) -> Flow<Value> {
        let Some(temp_dirs) = self.shared.temp_dirs else {
            return mistyped(node, "`fs.temp_dir` called in a run without an owner");
        };
        match temp_dirs.make() {
            Ok(dir) => Ok(Value::Str(Rc::from(dir))),
            Err(reason) => Err(Diagnostic::error(
                format!("`fs.temp_dir` could not create a temporary directory: {reason}"),
                node.span,
            )
            .into()),
        }
    }

    /// The tag of the `Tag` node `(key, of, shown)`: a variant as a value.
    fn tag(&self, (key, of, shown): (Name, Name, ConstId)) -> Rc<Tag> {
        Rc::new(Tag {
            key,
            of,
            shown: Rc::clone(self.ir.string(shown)),
        })
    }

    /// The value of the enum of `tag`'s variant whose payload is the one
    /// argument `args` hold, for the `Call` `node` of the variant (see the
    /// checker's "Enums").
    #[inline(never)]
    fn call_tag(&mut self, tag: Rc<Tag>, args: IrRange, node: &IrNode) -> Flow<Value> {
        let &[arg] = self.ir.items(args) else {
            return mistyped(node, "a variant called with other than one argument");
        };
        let IrKind::Arg {
            value: ArgValue::Node(value),
            ..
        } = self.ir.get(arg).kind
        else {
            return mistyped(node, "a variant's argument is not a node's value");
        };
        let payload = self.value(value)?;
        Ok(Value::Variant(Rc::new(Variant { tag, payload })))
    }

    /// the file system's, `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`
    /// ([`file_system`]), `fs_temp_dir` ([`TempDirs::make`]) and `fs_write`
    /// ([`write_file`]), touch the file system at absolute paths, each
    /// failure a runtime error at the call naming the built-in as a member
    /// of `fs`. A built-in whose typing is special is never called
    /// through a value (the checker lowers each call of one to its own
    /// node, `len`'s to a `ListLen`, `show`'s to a `Show`), so reaching one
    /// here is an internal error.
    #[inline(never)]
    fn call_builtin(&mut self, builtin: Builtin, args: IrRange, node: &IrNode) -> Flow<Value> {
        // Each parameter's argument, with its span, by the parameter's index.
        let mut arguments: [Option<(Value, Span)>; Builtin::MAX_PARAMS] = Default::default();
        for &arg in self.ir.items(args) {
            let arg = self.ir.get(arg);
            let IrKind::Arg {
                param,
                value: ArgValue::Node(value),
            } = arg.kind
            else {
                return mistyped(arg, "a built-in's argument is not a node's value");
            };
            let span = self.ir.get(value).span;
            let value = self.value(value)?;
            let Some(entry) = arguments.get_mut(param as usize) else {
                return mistyped(arg, "an argument for a parameter the built-in lacks");
            };
            *entry = Some((value, span));
        }
        let [first, second] = arguments;
        let second = second.map(|(value, _)| value);
        match (builtin, first, second) {
            (Builtin::Int, Some((Value::Float(x), span)), None) => match float_to_int(x) {
                Some(truncated) => Ok(Value::Int(truncated)),
                None if x.is_nan() => {
                    Err(Diagnostic::error("cannot truncate NaN to an Int", node.span).into())
                }
                None => Err(Diagnostic::error(OVERFLOW, node.span)
                    .with_context(span, Value::Float(x).to_string())
                    .into()),
            },
            (Builtin::Float, Some((Value::Int(x), _)), None) => Ok(Value::Float(int_to_float(x))),
            (Builtin::Exec | Builtin::ExecLive, Some((Value::List(words), _)), None) => {
                let deadline = self.shared.earliest.get();
                match exec(builtin, &words, (self.shared.exec_result, deadline), node)? {
                    Some(value) => Ok(value),
                    None => self.timed_out(node),
                }
            }
            (Builtin::Panic, Some((Value::Str(message), _)), None) => {
                Err(Diagnostic::error(&*message, node.span).into())
            }
            (Builtin::Split, Some((Value::Str(s), _)), Some(Value::Str(sep))) => {
                split(&s, &sep, node)
            }
            (Builtin::Join, Some((Value::List(parts), _)), Some(Value::Str(sep))) => {
                join(&parts, &sep, node)
            }
            (Builtin::StartsWith, Some((Value::Str(s), _)), Some(Value::Str(prefix))) => {
                Ok(Value::Bool(s.starts_with(&*prefix)))
            }
            (Builtin::EndsWith, Some((Value::Str(s), _)), Some(Value::Str(suffix))) => {
                Ok(Value::Bool(s.ends_with(&*suffix)))
            }
            (Builtin::Trim, Some((Value::Str(s), _)), None) => Ok(Value::Str(trim(s))),
            (Builtin::StripPrefix, Some((Value::Str(s), _)), Some(Value::Str(prefix))) => {
                Ok(Value::Str(strip_prefix(s, &prefix)))
            }
            (Builtin::StripSuffix, Some((Value::Str(s), _)), Some(Value::Str(suffix))) => {
                Ok(Value::Str(strip_suffix(s, &suffix)))
            }
            (Builtin::Lines, Some((Value::Str(s), _)), None) => Ok(string_list(s.lines())),
            (Builtin::Fields, Some((Value::Str(s), _)), None) => {
                Ok(string_list(s.split_whitespace()))
            }
            (Builtin::CompareVersions, Some((Value::Str(a), _)), Some(Value::Str(b))) => {
                Ok(Value::Int(match compare_versions(&a, &b) {
                    Ordering::Less => -1,
                    Ordering::Equal => 0,
                    Ordering::Greater => 1,
                }))
            }
            (Builtin::SortVersions, Some((Value::List(versions), _)), None) => {
                sort_versions(&versions, node)
            }
            (Builtin::FsWrite, Some((Value::Str(p), _)), Some(Value::Str(contents))) => {
                write_file(&p, &contents, node)
            }
            (Builtin::FsTempDir, None, None) => self.temp_dir(node),
            (
                Builtin::FsExists | Builtin::FsRead | Builtin::FsMkdir | Builtin::FsCopy,
                Some((Value::Str(p), _)),
                to,
            ) => file_system(builtin, &p, to.as_ref(), node),
            (
                Builtin::VariantMatches | Builtin::VariantPayload | Builtin::PartialMatches,
                Some((first, _)),
                Some(second),
            ) => enum_builtin(builtin, (&first, &second), node),
            (Builtin::Secret | Builtin::SecretValue, Some((value, _)), identity) => {
                secret_builtin(builtin, value, identity, node)
            }
            _ => mistyped(
                node,
                "a built-in function called through a value with an argument of another kind",
            ),
        }
    }

    /// Evaluates the `Call` `node` of `closure`, whose code `code` is
    /// memoised, with the arguments `args`, at depth `depth` in this frame's
    /// body (see [Memoised functions](crate::eval::memo#memoised-functions)): returns what
    /// the closure returned for equal arguments before, if it has; fails if
    /// it is running with equal arguments; and otherwise runs the body as
    /// [`Evaluator::call`] does and records what it returns. The lookup and
    /// the recording are in helpers, so that this frame, on the recursion
    /// path of memoised recursion, keeps only what the body's evaluation
    /// needs across it.
    fn memo_call(
        &mut self,
        (closure, code): (&Rc<Closure>, &FnCode),
        (args, depth): (IrRange, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let base = self.stack.len();
        let (mark, entering, caller) =
            match self.memo_begin((closure, code), (args, depth), node)? {
                MemoBegin::Found(value) => return Ok(value),
                MemoBegin::Started(mark) => (Some(mark), mark.entering, mark.caller),
                MemoBegin::Unkeyed(entering, caller) => (None, entering, caller),
            };
        let result = if code.retry > 0 || code.timeout.is_some() {
            self.guarded_body((closure, code), base, (caller, entering), node)
        } else {
            Evaluator {
                ir: &closure.code,
                stack: self.stack,
                base,
                captures: &closure.captures,
                callee: Some(closure),
                memo: self.memo,
                shared: self.shared,
                caller,
                levels: entering,
            }
            .value(code.body)
        };
        self.stack.truncate(base);
        // The call is no longer running: nothing more depends on it.
        if mark.is_some() {
            self.memo.running.pop();
        }
        // A `return` for the function is its value, which `memo_end` then
        // records as any, here, inside the running entry; no other stop
        // leaves its body: the checker sees to it.
        let result = result.or_else(|unwind| unwind.returned(node.span));
        match mark {
            Some(mark) => self.memo_end(closure, mark, result, node),
            None => result.map_err(|error| self.memo_error(error, closure, false, node).into()),
        }
    }

    /// Begins the memoised call `node` of `closure`, whose code is `code`,
    /// at depth `depth` in this frame's body: grows the stack by the
    /// callee's frame, evaluates the arguments `args` into it, and looks
    /// them up in the closure's memo table, with the caller the call gives
    /// its callee (see [Callers and helpers](#callers-and-helpers)),
    /// allocating nothing. If the closure has returned for equal arguments
    /// and an equal value of every ambient input its entry recorded, the
    /// stack is truncated back and that value found, the entry's
    /// dependencies passed on to the running calls of this frame's origin
    /// if the callee would have shared it. Otherwise, if it is running so,
    /// or its body would take the nesting past the budget, the stack is
    /// truncated back and the error returned; and if neither, the arguments
    /// are recorded as running, the one allocation of the call's
    /// memoisation, the call is pushed on the running calls, and the frame
    /// is left for the body.
    fn memo_begin(
        &mut self,
        (closure, code): (&Rc<Closure>, &FnCode),
        (args, depth): (IrRange, u16),
        node: &IrNode,
    ) -> Flow<MemoBegin> {
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        // An argument may fail, or stop for a loop around the call, after
        // the frame has grown: either way the frame goes, and no entry has
        // been made yet.
        if let Err(unwind) = self.arguments(args, base, (closure, depth)) {
            self.stack.truncate(base);
            return Err(unwind);
        }
        let caller = self.callee_caller();
        let params = base..base + code.param_count as usize;
        let Some(key) = self.stack.get(params.clone()) else {
            self.stack.truncate(base);
            return mistyped(node, "a call's parameters lie outside its frame");
        };
        // A callee that would share this frame's origin passes a hit's
        // dependencies on to the running calls of that origin, as a read
        // in it would.
        let inherited = self.caller.helper;
        let context = self.shared.context.borrow().clone();
        let hash = match self
            .memo
            .lookup(closure, key, (caller, inherited), context.as_ref())
        {
            Lookup::Missing(hash) => hash,
            Lookup::Done(value) => {
                self.stack.truncate(base);
                return Ok(MemoBegin::Found(value));
            }
            Lookup::Running => {
                self.stack.truncate(base);
                return Err(depends_on_own_result(node).into());
            }
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            self.stack.truncate(base);
            return Err(nested_too_deeply(node).into());
        }
        let key = self.stack.get(params).unwrap_or_default();
        // Arguments that equal nothing, themselves included, as those
        // holding NaN, would make an entry no call could find.
        if !key_eq(key, key) {
            return Ok(MemoBegin::Unkeyed(entering, caller));
        }
        // A running call has an origin, by which the reads it depends on
        // find it.
        let caller = if caller.origin == NO_ORIGIN {
            Caller {
                origin: self.memo.new_origin(),
                ..caller
            }
        } else {
            caller
        };
        let entry = self.memo.start(closure, hash, key);
        let context = self.shared.context.borrow().clone();
        self.memo.running.push(Running {
            closure: Rc::as_ptr(closure),
            hash,
            entry,
            origin: caller.origin,
            context,
        });
        Ok(MemoBegin::Started(Mark {
            hash,
            entry,
            entering,
            caller,
        }))
    }

    /// Ends the memoised call `node` of `closure` that
    /// [`Evaluator::memo_begin`] started as `mark`, whose body gave
    /// `result`: records the value it returned, or, if it failed, removes
    /// the running mark, so that a later call runs the body again, and
    /// returns the error as [`Evaluator::memo_error`] reports it.
    fn memo_end(
        &mut self,
        closure: &Rc<Closure>,
        mark: Mark,
        result: Result<Value, Diagnostic>,
        node: &IrNode,
    ) -> Flow<Value> {
        match result {
            Ok(value) => match self.memo.finish(closure, &mark, Some(&value)) {
                Some(_) => Ok(value),
                None => mistyped(node, "a memoised call's entry is missing"),
            },
            Err(error) => match self.memo.finish(closure, &mark, None) {
                Some(reentered) => Err(self.memo_error(error, closure, reentered, node).into()),
                None => mistyped(node, "a memoised call's entry is missing"),
            },
        }
    }

    /// `error`, raised while `closure` ran for the memoised call `node`, as
    /// it is reported: stamped with the closure's input when its code is of
    /// another session input than this frame's, as [`Evaluator::call`]
    /// stamps one, unless an inner frame has stamped an input on it
    /// already; and otherwise, if the error is that a call inside the body
    /// called the closure again with the same arguments (`reentered`),
    /// labelled "called here with the same arguments" at this call, as the
    /// await that ran a future that awaits itself is labelled.
    fn memo_error(
        &self,
        mut error: Diagnostic,
        closure: &Closure,
        reentered: bool,
        node: &IrNode,
    ) -> Diagnostic {
        if error.input.is_some() {
            return error;
        }
        if closure.code.input() != self.ir.input() {
            error.input = Some(closure.code.input());
            return error;
        }
        if reentered {
            error.with_context(node.span, "called here with the same arguments")
        } else {
            error
        }
    }

    /// Evaluates the arguments `args` of a call of `callee` at depth `depth`
    /// in this frame's body, `Arg` nodes, in order, in this frame, writing
    /// each to its parameter's slot of the frame that starts at `base`. An
    /// argument a call gives a parameter's default that is no constant, a
    /// [`ArgValue::Default`], comes after the ones written, in parameter
    /// order, and is the value of the default function the callee keeps at
    /// its capture called with the values of the parameters before it, read
    /// from the new frame ([`Evaluator::default_arg`]).
    fn arguments(
        &mut self,
        args: IrRange,
        base: usize,
        (callee, depth): (&Rc<Closure>, u16),
    ) -> Flow<()> {
        for &arg in self.ir.items(args) {
            let node = self.ir.get(arg);
            let IrKind::Arg { param, value } = node.kind else {
                return mistyped(node, "a call's argument is not an argument");
            };
            let value = match value {
                ArgValue::Node(value) => self.value(value)?,
                ArgValue::Default { capture, .. } => {
                    let earlier = base..base + param as usize;
                    self.default_arg((callee, capture), earlier, (depth, node))?
                }
            };
            let Some(slot) = self.stack.get_mut(base + param as usize) else {
                return mistyped(node, "an argument for a parameter outside the frame");
            };
            *slot = value;
        }
        Ok(())
    }

    /// The value a call at depth `depth` in this frame's body gives a
    /// parameter whose default is no constant, the argument `node`: the
    /// default function's closure, capture number `capture` of `callee`,
    /// called, as a call in this frame is, with the values of the stack's
    /// slots `earlier`, the parameters before it in the callee's new frame
    /// (see the checker's "Calls"). So the default runs on the caller's side
    /// of the call, with the caller's caller, before the callee's body
    /// starts.
    fn default_arg(
        &mut self,
        (callee, capture): (&Rc<Closure>, u32),
        earlier: std::ops::Range<usize>,
        (depth, node): (u16, &IrNode),
    ) -> Flow<Value> {
        let Some(Value::Fn(default)) = callee.captures.get(capture as usize) else {
            return mistyped(node, "a default that its callee keeps no function for");
        };
        let default = Rc::clone(default);
        let Some(code) = default.code.find_function(default.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        if earlier.end > self.stack.len() {
            return mistyped(node, "a default's parameters lie outside its call's frame");
        }
        // The default's frame sits above the callee's, so its parameters
        // are copied up, in place: nothing is allocated once the stack has
        // grown, as for any call.
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        for (at, from) in earlier.enumerate() {
            let value = self.stack.get(from).cloned().unwrap_or(Value::Empty);
            if let Some(slot) = self.stack.get_mut(base + at) {
                *slot = value;
            }
        }
        let passed = self.callee_caller();
        let result = Evaluator {
            ir: &default.code,
            stack: self.stack,
            base,
            captures: &default.captures,
            callee: Some(&default),
            memo: self.memo,
            shared: self.shared,
            caller: passed,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Err(mut error) if error.input.is_none() && default.code.input() != self.ir.input() => {
                error.input = Some(default.code.input());
                Err(error.into())
            }
            result => result.map_err(Unwind::from),
        }
    }

    /// Evaluates the `Call` `node` of `callee` with the arguments `args`, at
    /// depth `depth`, as a value that `extract` accepts: one of the kind the
    /// node states.
    fn call_as<T>(
        &mut self,
        call: (IrId, IrRange, u16),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.call(call, node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "a call returned a value of another kind than its node",
            ),
        }
    }

    /// Evaluates a function-typed node to what a call of it calls: a
    /// closure, or a built-in function, a value of the `builtins` module
    /// called through a value of its function type (see
    /// [`Evaluator::call_builtin`]).
    fn function(&mut self, id: IrId) -> Flow<Callee> {
        let node = self.ir.get(id);
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
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_FUNCTION),
            IrKind::Await { .. } => self.await_as(node, into_callee),
            IrKind::As { .. } => self.narrow_as(node, into_callee),
            IrKind::Index { list, index, .. } => self.index_as((list, index), node, into_callee),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_function),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_function),
            IrKind::LoadCallee => self.callee(node).map(Callee::Closure),
            IrKind::Tag { key, of, shown } => Ok(Callee::Tag(self.tag((key, of, shown)))),
            IrKind::ContextRead { .. } | IrKind::With { .. } | IrKind::Recover { .. } => {
                match into_callee(self.whole_node(node)?) {
                    Some(callee) => Ok(callee),
                    None => mistyped(node, "a context's value that is no function"),
                }
            }
            IrKind::Top(slot, _) => self.top_function(slot, node),
            IrKind::MakeClosure { function, captures } => {
                self.make_closure(function, captures).map(Callee::Closure)
            }
            IrKind::Meta(function) => self.meta(function, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.function(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.function(branch)
            }
            IrKind::Field { record, name, .. } => self.field_as((record, name), node, into_callee),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, into_callee),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, into_callee),
            _ => mistyped(node, "a node of another kind where a function was expected"),
        }
    }

    /// Builds a record of the values of `values`, in the order of `layout`'s
    /// names: the one allocation for its values, sized exactly, and one for
    /// the record, which shares the layout.
    fn make_record(&mut self, layout: LayoutId, values: IrRange) -> Flow<Value> {
        let items = self.ir.items(values);
        let mut fields = Vec::with_capacity(items.len());
        for &item in items {
            fields.push(self.value(item)?);
        }
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::clone(self.ir.layout(layout)),
            values: fields.into_boxed_slice(),
        })))
    }

    /// Evaluates `record` and returns its field `name`, for the `Field`
    /// `node`. The checker guarantees the field is there, so a missing
    /// one, or an empty record, is an internal error.
    fn field(&mut self, record: IrId, name: Name, node: &IrNode) -> Flow<Value> {
        match self.record(record)? {
            Value::Record(fields) => match fields.get(name) {
                Some(value) => Ok(value.clone()),
                None => mistyped(node, "a record lacks a field the checker found"),
            },
            _ => mistyped(node, "a field of the empty record"),
        }
    }

    /// Evaluates the `Field` `node`, reading field `name` of `record`, as a
    /// value that `extract` accepts: a field of the kind the node states.
    fn field_as<T>(
        &mut self,
        (record, name): (IrId, Name),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.field(record, name, node)?) {
            Some(value) => Ok(value),
            None => mistyped(node, "a field holds a value of another kind than its node"),
        }
    }

    /// Evaluates a list node, through the general walker, to its list (see
    /// the module documentation).
    fn list(&mut self, id: IrId) -> Flow<List> {
        match self.value(id)? {
            Value::List(list) => Ok(list),
            _ => mistyped(
                self.ir.get(id),
                "a node of another kind where a list was expected",
            ),
        }
    }

    /// Builds a list of the values of `items`, in order, spreading the
    /// list of each `Spread` among them if `spreads` says there is one: one
    /// allocation, sized exactly and filled in place, if there is none,
    /// and none at all for the empty list.
    fn make_list(&mut self, items: IrRange, spreads: bool) -> Flow<Value> {
        let ir = self.ir;
        let items = ir.items(items);
        if spreads {
            return self.make_spread_list(items);
        }
        // The slice is built at its known length as the items are
        // evaluated, in order; once one fails, the rest are not evaluated,
        // and the slice, filled with `{}`, is dropped.
        let mut failed = None;
        let list = List::from_exact((0..items.len()).map(|at| {
            if failed.is_some() {
                return Value::Empty;
            }
            let value = items
                .get(at)
                .map_or(Ok(Value::Empty), |&item| self.value(item));
            value.unwrap_or_else(|error| {
                failed = Some(error);
                Value::Empty
            })
        }));
        match failed {
            Some(error) => Err(error),
            None => Ok(Value::List(list)),
        }
    }

    /// Evaluates the range `node` of `operands`, its bounds, left to right,
    /// to the list of its integers, in order, up to the right bound,
    /// included if `inclusive`: one allocation, sized exactly and filled in
    /// place, and none for an empty range. A range of more than
    /// [`MAX_LIST_LEN`] integers, whose list cannot exist, is "range too
    /// large".
    fn range(&mut self, operands: Operands, inclusive: bool, node: &IrNode) -> Flow<Value> {
        let (start, end) = self.ints(operands)?;
        let len = range_len(start, end, inclusive);
        let Some(len) = usize::try_from(len).ok().filter(|&len| len <= MAX_LIST_LEN) else {
            return Err(range_too_large(node, len).into());
        };
        // No integer of the range passes its end, so none overflows.
        let list = List::from_exact((0..len).map(|offset| {
            Value::Int(i64::try_from(offset).map_or(end, |offset| start.saturating_add(offset)))
        }));
        Ok(Value::List(list))
    }

    /// Evaluates the `For` `node`, whose variable is in `slot` and whose
    /// parts are `parts` (see [`IrKind::For`]): its iterable once, and then,
    /// for each element in order, the element written to the slot, the
    /// guard, if it has one, and if that is true or absent, the body. If
    /// `collect`, the body's values are collected, in a vector sized up
    /// front to the number of elements when there is no guard to skip
    /// some, and then copied into the list's allocation, so that the whole
    /// takes time linear in the list's length; otherwise they are dropped
    /// as they come, and the `for` is `{}`. The first error stops the loop.
    /// Each iteration evaluates the body afresh at the same depth, so none
    /// nests in another.
    fn for_loop(
        &mut self,
        (slot, parts, collect): (Slot, IrRange, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let (iterable, guard, body) = match *self.ir.items(parts) {
            [iterable, body] => (iterable, None, body),
            [iterable, guard, body] => (iterable, Some(guard), body),
            _ => return mistyped(node, "a `for` without its iterable and body"),
        };
        let elements = self.elements(iterable)?;
        let mut values = Vec::new();
        if collect && guard.is_none() {
            // A range of any length may be iterated over: no more than this
            // is reserved up front, and the vector grows past it only as
            // the body's values come.
            values.reserve_exact(elements.remaining().min(FOR_RESERVE));
        }
        let at = self.base + slot.index();
        for element in elements {
            if self.past_deadline() {
                return self.timed_out(node);
            }
            let Some(variable) = self.stack.get_mut(at) else {
                return Err(internal("a `for`'s variable is outside the frame", node.span).into());
            };
            *variable = element;
            // A stop out of the guard is for a loop around this one.
            if let Some(guard) = guard
                && !self.bool(guard)?
            {
                continue;
            }
            match self.value(body) {
                Ok(value) if collect => values.push(value),
                Ok(_) => {}
                Err(unwind) => {
                    if caught(unwind, &mut values, collect)? {
                        break;
                    }
                }
            }
        }
        Ok(if collect {
            Value::List(List::from_vec(values))
        } else {
            Value::Empty
        })
    }

    /// Evaluates the loop `node`, a `For` or a `Loop`, by the helper for its
    /// kind: the walkers only call this.
    fn looped(&mut self, node: &IrNode) -> Flow<Value> {
        match node.kind {
            IrKind::For {
                slot,
                parts,
                collect,
            } => self.for_loop((slot, parts, collect), node),
            IrKind::Loop {
                parts,
                collect,
                test_first,
            } => self.condition_loop((parts, collect, test_first), node),
            _ => mistyped(node, "a node that is not a loop looped"),
        }
    }

    /// Evaluates the `Loop` `node`, whose parts are `parts`, a condition
    /// loop or a `do` loop (see [`IrKind::Loop`]): while its condition is
    /// true, tested first only if `test_first`, its body, and then the items
    /// after its condition, each for its effects. If `collect`, what each
    /// iteration contributes is collected, in a vector that grows as it
    /// comes, its length not being known up front, and then copied into the
    /// list's allocation; otherwise it is dropped, and the loop is `{}`. A
    /// stop out of its body is caught as [`caught`] says; one out of its
    /// condition or an item after it is for a loop around this one. Each
    /// iteration evaluates the body afresh at the same depth, so none nests
    /// in another.
    fn condition_loop(
        &mut self,
        (parts, collect, test_first): (IrRange, bool, bool),
        node: &IrNode,
    ) -> Flow<Value> {
        let ir = self.ir;
        let Some((&cond, rest)) = ir.items(parts).split_first() else {
            return mistyped(node, "a loop without its condition");
        };
        let Some((&body, after)) = rest.split_first() else {
            return mistyped(node, "a loop without its body");
        };
        let mut values = Vec::new();
        let mut test = test_first;
        loop {
            if self.past_deadline() {
                return self.timed_out(node);
            }
            if test && !self.bool(cond)? {
                break;
            }
            test = true;
            match self.value(body) {
                Ok(value) if collect => values.push(value),
                Ok(_) => {}
                Err(unwind) => {
                    if caught(unwind, &mut values, collect)? {
                        break;
                    }
                }
            }
            for &item in after {
                self.value(item)?;
            }
        }
        Ok(if collect {
            Value::List(List::from_vec(values))
        } else {
            Value::Empty
        })
    }

    /// Evaluates the `Break`, `Continue` or `Return` `node`: its value, if
    /// it has one, through the general walker, and then the stop that
    /// carries it to its loop, or to what it returns from, unwinding every
    /// node on the way, as an error does. Every walker hands such a node
    /// here, the typed walkers through [`Evaluator::stopping`], since one
    /// of kind [`ValueKind::Never`] may stand where a value of any kind is
    /// expected. A `return` names its catch by number in the `u32` a
    /// `break` counts its loops in (see [`Stop`]).
    fn leave<T>(&mut self, node: &IrNode) -> Flow<T> {
        let (kind, to, value) = match node.kind {
            IrKind::Break { loops, value } => (StopKind::Break, loops, value),
            IrKind::Continue { loops, value } => (StopKind::Continue, loops, value),
            IrKind::Return { to, value } => (StopKind::Return, to.number(), value),
            _ => {
                return mistyped(
                    node,
                    "a node that is not a `break`, `continue` or `return` left a loop",
                );
            }
        };
        let value = match value {
            Some(value) => Some(self.value(value)?),
            None => None,
        };
        Err(Unwind::Stop(Stop { to, kind, value }))
    }

    /// Evaluates the `Catch` `node` (see [`IrKind::Catch`]): its block or
    /// loop, through the general walker, whose value it yields, or the
    /// value of a `return` for it that comes out of it, `{}` for one
    /// without. Any other stop, and an error, go on as they are. Never
    /// inlined into the general walker, whose frame, on the recursion path
    /// of every expression, it would grow.
    #[inline(never)]
    fn catch(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::Catch { body, id, .. } = node.kind else {
            return mistyped(node, "a node that is not a catch caught");
        };
        match self.value(body) {
            Err(Unwind::Stop(Stop {
                kind: StopKind::Return,
                to,
                value,
            })) if to == id.number() => Ok(value.unwrap_or(Value::Empty)),
            flow => flow,
        }
    }

    /// Evaluates, for a typed walker, a node that stops or may, as a value
    /// that `extract` accepts, of the kind the walker wants: a `break`,
    /// `continue` or `return` unwinds as [`Evaluator::leave`] says; a
    /// catch's value must be one `extract` accepts, of the kind the node
    /// states; and a loop where a value of another kind than a loop's is
    /// wanted is one that cannot end, of type `Never` (see [`crate::ir`]),
    /// which runs until something leaves it, and is mistyped if it ends.
    /// One arm of each typed walker hands them all here, out of line, so
    /// that what a program without them never meets grows no walker.
    #[inline(never)]
    fn stopping<T>(&mut self, node: &IrNode, extract: fn(Value) -> Option<T>) -> Flow<T> {
        match node.kind {
            IrKind::Catch { .. } if mixed(node) => mistyped(node, MIXED),
            IrKind::Catch { .. } => match extract(self.catch(node)?) {
                Some(value) => Ok(value),
                None => mistyped(
                    node,
                    "a catch yielded a value of another kind than its node",
                ),
            },
            IrKind::For { .. } | IrKind::Loop { .. } => {
                self.looped(node)?;
                mistyped(node, "a loop ended where it cannot")
            }
            _ => self.leave(node),
        }
    }

    /// Evaluates the loop `node` where a record is expected: one that
    /// discards what its iterations contribute, which is `{}`, as a
    /// statement or a loop that cannot end is; one that collects them is a
    /// list, and mistyped here. Out of line, as [`Evaluator::stopping`] is.
    #[inline(never)]
    fn record_loop(&mut self, node: &IrNode) -> Flow<Value> {
        match node.kind {
            IrKind::For { collect: true, .. } | IrKind::Loop { collect: true, .. } => {
                mistyped(node, "list node where a record was expected")
            }
            _ => self.looped(node),
        }
    }

    /// What a `for` whose iterable is `iterable` iterates over: if the
    /// iterable is directly a range, its integers, from its bounds,
    /// evaluated left to right, without building its list, so that
    /// [`MAX_LIST_LEN`] does not apply; otherwise the elements of the list
    /// it evaluates to.
    fn elements(&mut self, iterable: IrId) -> Flow<Elements> {
        match self.ir.get(iterable).kind {
            IrKind::Range(inclusive, operands) => {
                let (start, end) = self.ints(operands)?;
                Ok(Elements::range(start, end, inclusive))
            }
            _ => Ok(Elements::List(self.list(iterable)?, 0)),
        }
    }

    /// Builds a list of the values of `items`, in order, of which some are
    /// `Spread`s, whose lists' elements are put in their places: collected
    /// in a vector, which grows by amortised doubling, so that the whole
    /// takes time linear in the list's length, and then copied into the
    /// list's one allocation.
    fn make_spread_list(&mut self, items: &[IrId]) -> Flow<Value> {
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            if let IrKind::Spread(list) = self.ir.get(item).kind {
                values.extend_from_slice(self.list(list)?.values());
            } else {
                values.push(self.value(item)?);
            }
        }
        Ok(Value::List(List::from_vec(values)))
    }

    /// Evaluates `list` and then `index`, and returns the element at the
    /// position, counting from 0, for the `Index` `node`; a position
    /// outside the list, a negative one included, is "index out of range".
    fn index(&mut self, (list, index): (IrId, IrId), node: &IrNode) -> Flow<Value> {
        let list = self.list(list)?;
        let at = self.int(index)?;
        match usize::try_from(at)
            .ok()
            .and_then(|position| list.values().get(position))
        {
            Some(value) => Ok(value.clone()),
            None => Err(out_of_range(node.span, at, list.len()).into()),
        }
    }

    /// Evaluates the `Index` `node`, reading the element of `list` at
    /// `index`, as a value that `extract` accepts: an element of the kind
    /// the node states.
    fn index_as<T>(
        &mut self,
        operands: (IrId, IrId),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.index(operands, node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "a list holds an element of another kind than its node",
            ),
        }
    }

    /// Evaluates a `ListLen` of `list`: the number of its elements.
    fn list_len(&mut self, list: IrId) -> Flow<i64> {
        // A list has far fewer elements than `i64::MAX`.
        Ok(i64::try_from(self.list(list)?.len()).unwrap_or(i64::MAX))
    }

    /// Evaluates the typed list comparison `node`, a `ListEq` if `equal` and
    /// a `ListNe` otherwise: the operands compare element by element over
    /// the list's shape (see [`record_eq_shaped`]).
    fn list_eq(&mut self, operands: ShapedOperands, equal: bool, node: &IrNode) -> Flow<bool> {
        let lhs = Value::List(self.list(operands.lhs)?);
        let rhs = Value::List(self.list(operands.rhs)?);
        match record_eq_shaped(self.ir, operands.shape, &lhs, &rhs) {
            Some(same) => Ok(same == equal),
            None => mistyped(node, "a list does not have its shape"),
        }
    }

    /// Evaluates a total list comparison, a `ListSame` if `equal` and a
    /// `ListNotSame` otherwise: lists compare whole, element by element,
    /// deeply.
    fn list_same(&mut self, operands: Operands, equal: bool) -> Flow<bool> {
        let lhs = self.list(operands.lhs)?;
        let rhs = self.list(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// The value in `slot` of this frame.
    fn load(&self, slot: Slot, node: &IrNode) -> Flow<&Value> {
        self.stack
            .get(self.base + slot.index())
            .ok_or_else(|| internal("load from a slot outside the frame", node.span).into())
    }

    /// Evaluates an `Int`-typed node.
    fn int(&mut self, id: IrId) -> Flow<i64> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::IntConst(value) => Ok(value),
            IrKind::Quantity(quantity, amount) => self.quantity_amount(quantity, amount),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(_)
            | IrKind::MakeFuture { .. } => self.shared_amount(id, node),
            IrKind::ListLen(list) => self.list_len(list),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.int(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.int(branch)
            }
            IrKind::IntNeg(operand) => self.int_neg(node, operand),
            IrKind::IntAdd(operands) => self.int_arithmetic(node, operands, i64::checked_add),
            IrKind::IntSub(operands) => self.int_arithmetic(node, operands, i64::checked_sub),
            IrKind::IntMul(operands) => self.int_arithmetic(node, operands, i64::checked_mul),
            IrKind::IntFloorDiv(operands) => self.int_division(node, operands, floor_div),
            IrKind::IntRem(operands) => {
                self.int_division(node, operands, |lhs, rhs| Some(floor_mod(lhs, rhs)))
            }
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where an Int was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where an Int was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) | IrKind::Caller => {
                mistyped(node, "String node where an Int was expected")
            }
            IrKind::Show(_) => mistyped(node, "`show` node where an Int was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where an Int was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where an Int was expected"),
            // A loop that discards its values is `{}`, but only ever a
            // statement, or a loop that cannot end, which `stopping` runs.
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, into_int),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where an Int was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where an Int was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
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
            | IrKind::SecretWrap(_) => match (taken.owned)(self.whole_node(node)?) {
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
            _ => mistyped(
                node,
                "a node a typed walker evaluates itself, given to it shared",
            ),
        }
    }

    /// The value of the quantity node of `quantity` whose `Int` node is
    /// `amount` ([`Evaluator::quantity_amount`]). Out of line, as the walkers'
    /// helpers are, so that the general walker's frame holds none of its
    /// temporaries.
    #[inline(never)]
    fn quantity_value(&mut self, quantity: Quantity, amount: IrId) -> Flow<Value> {
        let amount = self.quantity_amount(quantity, amount)?;
        Ok(Value::Quantity(quantity, amount))
    }

    /// Evaluates `node`, the node `id` of a kind any walker shares, for the
    /// `Int` walker: an `Int`, or the amount of a duration or a size, by the
    /// node's kind. Out of line, so that the walker's frame holds none of its
    /// temporaries.
    #[inline(never)]
    fn shared_amount(&mut self, id: IrId, node: &IrNode) -> Flow<i64> {
        self.shared_as(node, amount_taken(self.ir.kind_of(id)))
    }

    /// The amount of the quantity node of `quantity` whose `Int` node is
    /// `amount`, of its smallest unit: an overflow in `amount`'s own
    /// operator is "duration overflow" or "size overflow", saying what
    /// overflowed, rather than the `Int` operator's "integer overflow".
    fn quantity_amount(&mut self, quantity: Quantity, amount: IrId) -> Flow<i64> {
        let inner = self.ir.get(amount);
        let at = inner.operator_span().unwrap_or(inner.span);
        self.int(amount).map_err(|unwind| match unwind {
            Unwind::Error(mut error) if error.message == OVERFLOW && error.span == at => {
                error.message = format!("{} overflow", quantity.name());
                Unwind::Error(error)
            }
            unwind => unwind,
        })
    }

    /// The text an operator's error labels its operand `operand` with, whose
    /// value is `value`: as the quantity it is, `30s`, or as the `Int`.
    fn operand_text(&self, operand: IrId, value: i64) -> String {
        match self.ir.kind_of(operand) {
            ValueKind::Quantity(quantity) => Value::Quantity(quantity, value).to_string(),
            _ => value.to_string(),
        }
    }

    /// Evaluates both `Int` operands, left to right.
    fn ints(&mut self, operands: Operands) -> Flow<(i64, i64)> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        Ok((lhs, rhs))
    }

    /// Evaluates the `IntNeg` `node` of `operand`.
    fn int_neg(&mut self, node: &IrNode, operand: IrId) -> Flow<i64> {
        let value = self.int(operand)?;
        value
            .checked_neg()
            .ok_or_else(|| self.unary_error(node, operand, OVERFLOW, value).into())
    }

    /// Evaluates the `+`, `-` or `*` `node`, which `op` computes, `None`
    /// meaning overflow.
    fn int_arithmetic(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, i64) -> Option<i64>,
    ) -> Flow<i64> {
        // Not through `ints`: this is on the recursion path of every
        // arithmetic chain, so it saves that frame.
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        op(lhs, rhs).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs).into())
    }

    /// Evaluates the `//` or `%` `node`, which `op` computes for a nonzero
    /// divisor, `None` meaning overflow.
    fn int_division(
        &mut self,
        node: &IrNode,
        operands: Operands,
        op: fn(i64, NonZeroI64) -> Option<i64>,
    ) -> Flow<i64> {
        let lhs = self.int(operands.lhs)?;
        let rhs = self.int(operands.rhs)?;
        let Some(divisor) = NonZeroI64::new(rhs) else {
            return Err(self
                .binary_error(node, operands, DIVISION_BY_ZERO, lhs, rhs)
                .into());
        };
        op(lhs, divisor).ok_or_else(|| self.binary_error(node, operands, OVERFLOW, lhs, rhs).into())
    }

    /// Evaluates a `Float`-typed node. IEEE 754 semantics; never fails except
    /// through an `Int` operand of `IntToFloat`, an item of a block or the
    /// condition of an `if`.
    fn float(&mut self, id: IrId) -> Flow<f64> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::FloatConst(value) => Ok(value),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(_)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &FLOAT),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a Float was expected"),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.float(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.float(branch)
            }
            IrKind::FloatNeg(operand) => self.float_neg(operand),
            IrKind::FloatAdd(o) => self.float_arithmetic(o, |lhs, rhs| lhs + rhs),
            IrKind::FloatSub(o) => self.float_arithmetic(o, |lhs, rhs| lhs - rhs),
            IrKind::FloatMul(o) => self.float_arithmetic(o, |lhs, rhs| lhs * rhs),
            IrKind::FloatDiv(o) => self.float_arithmetic(o, |lhs, rhs| lhs / rhs),
            IrKind::IntToFloat(operand) => self.int(operand).map(int_to_float),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a Float was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where a Float was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) | IrKind::Caller => {
                mistyped(node, "String node where a Float was expected")
            }
            IrKind::Show(_) => mistyped(node, "`show` node where a Float was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where a Float was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where a Float was expected"),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, Value::into_float),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where a Float was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a Float was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates a `FloatNeg` of `operand`.
    fn float_neg(&mut self, operand: IrId) -> Flow<f64> {
        Ok(-self.float(operand)?)
    }

    /// Evaluates a `Float` operator that `op` computes.
    fn float_arithmetic(&mut self, operands: Operands, op: fn(f64, f64) -> f64) -> Flow<f64> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(op(lhs, rhs))
    }

    /// Evaluates a `Bool`-typed node. `and` and `or` evaluate their right
    /// operand only when the left does not decide the result.
    fn bool(&mut self, id: IrId) -> Flow<bool> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::BoolConst(value) => Ok(value),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(_)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &BOOL),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a Bool was expected"),
            IrKind::Is { .. } => self.is_test(node),
            IrKind::ValueSame(o) => self.value_same(o, true),
            IrKind::ValueNotSame(o) => self.value_same(o, false),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.bool(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.bool(branch)
            }
            IrKind::IntCmp(cmp, o) => self.int_cmp(cmp, o),
            IrKind::FloatCmp(cmp, o) => self.float_cmp(cmp, o),
            IrKind::StrCmp(cmp, o) => self.str_cmp(cmp, o),
            IrKind::BoolEq(equal, o) => self.bool_eq(equal, o),
            IrKind::RecordEq(o) => self.record_eq(o, true, node),
            IrKind::RecordNe(o) => self.record_eq(o, false, node),
            IrKind::RecordSame(o) => self.record_same(o, true),
            IrKind::RecordNotSame(o) => self.record_same(o, false),
            IrKind::ListEq(o) => self.list_eq(o, true, node),
            IrKind::ListNe(o) => self.list_eq(o, false, node),
            IrKind::ListSame(o) => self.list_same(o, true),
            IrKind::ListNotSame(o) => self.list_same(o, false),
            IrKind::And(o) => self.logic(o, false),
            IrKind::Or(o) => self.logic(o, true),
            IrKind::Not(operand) => self.not(operand),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a Bool was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a Bool was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) | IrKind::Caller => {
                mistyped(node, "String node where a Bool was expected")
            }
            IrKind::Show(_) => mistyped(node, "`show` node where a Bool was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where a Bool was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where a Bool was expected"),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, Value::into_bool),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where a Bool was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a Bool was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates an `IntCmp` testing `cmp`.
    fn int_cmp(&mut self, cmp: Cmp, operands: Operands) -> Flow<bool> {
        let (lhs, rhs) = self.ints(operands)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `FloatCmp` testing `cmp`.
    fn float_cmp(&mut self, cmp: Cmp, operands: Operands) -> Flow<bool> {
        let lhs = self.float(operands.lhs)?;
        let rhs = self.float(operands.rhs)?;
        Ok(cmp.test(&lhs, &rhs))
    }

    /// Evaluates a `StrCmp` testing `cmp`.
    fn str_cmp(&mut self, cmp: Cmp, operands: Operands) -> Flow<bool> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        Ok(cmp.test(&*lhs, &*rhs))
    }

    /// Evaluates a `BoolEq`: `==` if `equal`, `!=` otherwise.
    fn bool_eq(&mut self, equal: bool, operands: Operands) -> Flow<bool> {
        let lhs = self.bool(operands.lhs)?;
        let rhs = self.bool(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates the typed record comparison `node`, a `RecordEq` if `equal`
    /// and a `RecordNe` otherwise: the operands compare over their shape
    /// (see [`record_eq_shaped`]).
    fn record_eq(&mut self, operands: ShapedOperands, equal: bool, node: &IrNode) -> Flow<bool> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        match record_eq_shaped(self.ir, operands.shape, &lhs, &rhs) {
            Some(same) => Ok(same == equal),
            None => mistyped(node, "a record lacks a field of its shape"),
        }
    }

    /// Evaluates a total record comparison, a `RecordSame` if `equal` and a
    /// `RecordNotSame` otherwise: records compare whole, deeply and by field
    /// name; empty records are all equal.
    fn record_same(&mut self, operands: Operands, equal: bool) -> Flow<bool> {
        let lhs = self.record(operands.lhs)?;
        let rhs = self.record(operands.rhs)?;
        Ok((lhs == rhs) == equal)
    }

    /// Evaluates `and`, which a `false` left operand decides, or `or`, which
    /// a `true` one decides, as `decisive` says. The right operand is only
    /// evaluated when the left does not decide.
    fn logic(&mut self, operands: Operands, decisive: bool) -> Flow<bool> {
        let lhs = self.bool(operands.lhs)?;
        if lhs == decisive {
            Ok(lhs)
        } else {
            self.bool(operands.rhs)
        }
    }

    /// Evaluates a `Not` of `operand`.
    fn not(&mut self, operand: IrId) -> Flow<bool> {
        Ok(!self.bool(operand)?)
    }

    /// Evaluates a `String`-typed node.
    fn string(&mut self, id: IrId) -> Flow<Rc<str>> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::StrConst(constant) => Ok(Rc::clone(self.ir.string(constant))),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(_)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &STRING),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a String was expected"),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.string(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.string(branch)
            }
            IrKind::StrConcat(operands) => self.concat(operands),
            IrKind::Interp(items) => self.interpolate(items, node.span),
            IrKind::Show(_) | IrKind::Caller => self.computed_string(node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a String was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a String was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where a String was expected"),
            IrKind::Empty
            | IrKind::Store(..)
            | IrKind::MakeRecord { .. }
            | IrKind::UpdateField { .. }
            | IrKind::Helper => mistyped(node, "record node where a String was expected"),
            IrKind::Update { .. } => mistyped(node, "update node where a String was expected"),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, Value::into_string),
            IrKind::MakeList { .. } | IrKind::Spread(_) | IrKind::Range(..) => {
                mistyped(node, "list node where a String was expected")
            }
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a String was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates a record node: [`Value::Empty`] or a [`Value::Record`].
    fn record(&mut self, id: IrId) -> Flow<Value> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Empty | IrKind::Helper => Ok(self.empty(node)),
            IrKind::Load(..)
            | IrKind::LoadCapture(..)
            | IrKind::Top(..)
            | IrKind::Call { .. }
            | IrKind::Field { .. }
            | IrKind::Index { .. }
            | IrKind::Await { .. }
            | IrKind::As { .. }
            | IrKind::ContextRead { .. }
            | IrKind::With { .. }
            | IrKind::Assert { .. }
            | IrKind::Recover { .. }
            | IrKind::Failure(_)
            | IrKind::SecretValue { .. }
            | IrKind::SecretWrap(_)
            | IrKind::MakeFuture { .. } => self.shared_as(node, &RECORD),
            IrKind::Quantity(..) => mistyped(node, "quantity node where a record was expected"),
            IrKind::Store(slot, value) => self.store(slot, value, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.record(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.record(branch)
            }
            IrKind::MakeRecord { layout, values } => self.make_record(layout, values),
            IrKind::UpdateField { .. } => self.update_field(node),
            IrKind::Update {
                kind: ValueKind::Record,
                ..
            } => self.update(node),
            IrKind::IntConst(_)
            | IrKind::IntNeg(_)
            | IrKind::IntAdd(_)
            | IrKind::IntSub(_)
            | IrKind::IntMul(_)
            | IrKind::IntFloorDiv(_)
            | IrKind::IntRem(_)
            | IrKind::ListLen(_) => mistyped(node, "Int node where a record was expected"),
            IrKind::FloatConst(_)
            | IrKind::FloatNeg(_)
            | IrKind::FloatAdd(_)
            | IrKind::FloatSub(_)
            | IrKind::FloatMul(_)
            | IrKind::FloatDiv(_)
            | IrKind::IntToFloat(_) => mistyped(node, "Float node where a record was expected"),
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
            | IrKind::Not(_) => mistyped(node, "Bool node where a record was expected"),
            IrKind::StrConst(_) | IrKind::StrConcat(_) | IrKind::Interp(_) | IrKind::Caller => {
                mistyped(node, "String node where a record was expected")
            }
            IrKind::Show(_) => mistyped(node, "`show` node where a record was expected"),
            IrKind::For { .. } | IrKind::Loop { .. } => self.record_loop(node),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. } => self.stopping(node, into_record),
            IrKind::MakeList { .. }
            | IrKind::Spread(_)
            | IrKind::Range(..)
            | IrKind::Update { .. } => mistyped(node, "list node where a record was expected"),
            IrKind::MakeClosure { .. }
            | IrKind::LoadCallee
            | IrKind::Tag { .. }
            | IrKind::Meta(_)
            | IrKind::Arg { .. } => mistyped(node, "function node where a record was expected"),
            IrKind::ContextBind { .. } => mistyped(node, CONTEXT_BIND),
        }
    }

    /// Evaluates the `UpdateField` `node`, setting its field of its record
    /// to its value: a new record, the old one unchanged. See the module
    /// documentation.
    fn update_field(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::UpdateField {
            record,
            name,
            value,
        } = node.kind
        else {
            return mistyped(node, "a node that is not a field's update updated");
        };
        let old = self.record(record)?;
        let value = self.value(value)?;
        self.with_field(old, name, value, node)
    }

    /// Evaluates the `Update` `node` (see the module documentation): its
    /// value, first, then the record or list it updates, and then the walk
    /// of its path (see [`Evaluator::walk_update`]). Apart from the walk,
    /// so that the recursion through the value holds nothing of it.
    fn update(&mut self, node: &IrNode) -> Flow<Value> {
        let IrKind::Update {
            parts, path, depth, ..
        } = node.kind
        else {
            return mistyped(node, "a node that is not an update updated");
        };
        let ir = self.ir;
        let parts = ir.items(parts);
        let (Some(&value), Some(&root)) = (parts.first(), parts.get(1)) else {
            return mistyped(node, "an update without its value or its record");
        };
        let value = self.value(value)?;
        let root = self.value(root)?;
        let positions = parts.get(2..).unwrap_or_default();
        self.walk_update((root, value), (positions, path, depth), node)
    }

    /// Walks the path `path` of the `Update` `node`, at depth `depth` in
    /// this frame's body, down from `root`, the record or list it updates,
    /// one step at a time: each step selects a field, or an element at the
    /// position the next of `positions` evaluates to, there and then, and
    /// reads it, awaits it, as an await at the step's path, or starts from
    /// `{}`, holding each record or list it passes through. The last step
    /// selects the leaf, and the new value is built back up from it: the
    /// leaf set to `value`, then each record or list passed through with
    /// what its step selected replaced by the one built below it. Nothing
    /// read is changed. A position outside its list stops the walk with
    /// "index out of range", at the path up to its segment.
    fn walk_update(
        &mut self,
        (root, value): (Value, Value),
        (positions, path, depth): (&[IrId], UpdatePathId, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let ir = self.ir;
        let Some((leaf, steps)) = ir.update_path(path).and_then(<[UpdateStep]>::split_last) else {
            return mistyped(node, "an update of a path its IR lacks");
        };
        let mut positions = positions.iter().copied();
        let mut passed = Vec::with_capacity(steps.len());
        let mut current = root;
        for step in steps {
            let at = self.select(&current, step, &mut positions, node)?;
            let inner = match step.mode {
                StepMode::Read => selected(&current, at, node)?,
                StepMode::Await => {
                    // A failure of the await is reported at the step's path.
                    let site = IrNode {
                        kind: node.kind,
                        span: step.span,
                    };
                    let read = selected(&current, at, node)?;
                    self.await_fully(read, depth.saturating_add(1), 0, &site)?
                }
                StepMode::Create => Value::Empty,
                StepMode::Set => return mistyped(node, "an update sets a value before its leaf"),
            };
            passed.push((current, at));
            current = inner;
        }
        if leaf.mode != StepMode::Set {
            return mistyped(node, "an update's last step does not set its leaf");
        }
        let at = self.select(&current, leaf, &mut positions, node)?;
        let mut new = self.replace(current, at, value, node)?;
        while let Some((container, at)) = passed.pop() {
            new = self.replace(container, at, new, node)?;
        }
        Ok(new)
    }

    /// What `step` of the `Update` `node` selects in `container`, the
    /// record or list its walk is at: a field by name, or, evaluating the
    /// next of `positions`, an element, whose position must lie in the
    /// list, a negative one never does, or the update fails with "index out
    /// of range" at the step's path.
    fn select(
        &mut self,
        container: &Value,
        step: &UpdateStep,
        positions: &mut impl Iterator<Item = IrId>,
        node: &IrNode,
    ) -> Flow<Selected> {
        let StepAt::Field(name) = step.at else {
            let Some(position) = positions.next() else {
                return mistyped(node, "an update lacks the position of an element");
            };
            let at = self.int(position)?;
            let Value::List(list) = container else {
                return mistyped(node, "an update selects an element of what is not a list");
            };
            return match usize::try_from(at).ok().filter(|&index| index < list.len()) {
                Some(index) => Ok(Selected::Element(index)),
                None => Err(out_of_range(step.span, at, list.len()).into()),
            };
        };
        Ok(Selected::Field(name))
    }

    /// `container`, a record or list an update's walk passed through, with
    /// what `at` selects in it replaced by `value`, or added for a field
    /// the record lacks, for the `Update` `node`: a new record or list,
    /// `container` unchanged.
    fn replace(&self, container: Value, at: Selected, value: Value, node: &IrNode) -> Flow<Value> {
        match (container, at) {
            (container, Selected::Field(name)) => self.with_field(container, name, value, node),
            (Value::List(list), Selected::Element(index)) => {
                Ok(Value::List(with_element(&list, index, value)))
            }
            _ => mistyped(node, "an update sets an element of what is not a list"),
        }
    }

    /// `old`, a record, with field `name` set to `value`, for the update
    /// `node`: a new record, the old one unchanged. See the module
    /// documentation.
    fn with_field(&self, old: Value, name: Name, value: Value, node: &IrNode) -> Flow<Value> {
        let old = match old {
            Value::Record(old) => old,
            Value::Empty => {
                let Some(text) = self.ir.name_text(name) else {
                    return mistyped(node, "an update's name has no text");
                };
                return Ok(Value::Record(Rc::new(Record {
                    layout: Rc::new(Layout {
                        names: Box::new([name]),
                        texts: Box::new([Box::from(text)]),
                    }),
                    values: Box::new([value]),
                })));
            }
            Value::Int(_)
            | Value::Float(_)
            | Value::Quantity(..)
            | Value::Bool(_)
            | Value::Str(_)
            | Value::List(_)
            | Value::Fn(_)
            | Value::Builtin(_)
            | Value::Tag(_)
            | Value::Variant(_)
            | Value::Secret(_)
            | Value::Future(_) => {
                return mistyped(node, "an update of a value that is not a record");
            }
        };
        if let Some(index) = old.layout.names.iter().position(|&field| field == name) {
            // The same fields: the layout is shared.
            let mut values = old.values.clone();
            if let Some(slot) = values.get_mut(index) {
                *slot = value;
            }
            return Ok(Value::Record(Rc::new(Record {
                layout: Rc::clone(&old.layout),
                values,
            })));
        }
        let Some(text) = self.ir.name_text(name) else {
            return mistyped(node, "an update's name has no text");
        };
        let len = old.values.len() + 1;
        let mut names = Vec::with_capacity(len);
        names.extend_from_slice(&old.layout.names);
        names.push(name);
        let mut texts = Vec::with_capacity(len);
        texts.extend(old.layout.texts.iter().cloned());
        texts.push(Box::from(text));
        let mut values = Vec::with_capacity(len);
        values.extend(old.values.iter().cloned());
        values.push(value);
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::new(Layout {
                names: names.into_boxed_slice(),
                texts: texts.into_boxed_slice(),
            }),
            values: values.into_boxed_slice(),
        })))
    }

    /// Evaluates a `StrConcat`, allocating exactly the length the result
    /// needs.
    fn concat(&mut self, operands: Operands) -> Flow<Rc<str>> {
        let lhs = self.string(operands.lhs)?;
        let rhs = self.string(operands.rhs)?;
        let mut text = String::with_capacity(lhs.len() + rhs.len());
        text.push_str(&lhs);
        text.push_str(&rhs);
        Ok(Rc::from(text))
    }

    /// Builds the text of an interpolation, spanning `span`, from its
    /// `items`: a `StrConst` is literal text, pushed as it is, and any other
    /// item is a value shown with its `Display`. The text is built in one
    /// buffer, reserved for the literal pieces and a little per value.
    fn interpolate(&mut self, items: IrRange, span: Span) -> Flow<Rc<str>> {
        let items = self.ir.items(items);
        let estimate = items.iter().fold(0_usize, |total, &item| {
            let len = match self.ir.get(item).kind {
                IrKind::StrConst(constant) => self.ir.string(constant).len(),
                _ => INTERPOLATED_VALUE_ESTIMATE,
            };
            total.saturating_add(len)
        });
        let mut text = String::with_capacity(estimate);
        for &item in items {
            if let IrKind::StrConst(constant) = self.ir.get(item).kind {
                text.push_str(self.ir.string(constant));
            } else {
                let value = self.value(item)?;
                write!(text, "{value}")
                    .map_err(|_| internal("showing a value as text failed", span))?;
            }
        }
        Ok(Rc::from(text))
    }

    /// Evaluates `node`, a `Show` or a `Caller`, the `String` nodes whose
    /// text is made of what the run holds: a value's source, or the running
    /// frame's caller. Apart from [`Evaluator::string`], whose match it keeps
    /// short; any other node is an internal error.
    fn computed_string(&mut self, node: &IrNode) -> Flow<Rc<str>> {
        match node.kind {
            IrKind::Show(operand) => self.show(operand, node),
            IrKind::Caller => self.caller_text(node),
            _ => mistyped(node, "a node whose text is not computed"),
        }
    }

    /// Evaluates a `Show`, the call `node` of `show`, of `operand`: its
    /// value's GCSL source (see [`source_of`]).
    fn show(&mut self, operand: IrId, node: &IrNode) -> Flow<Rc<str>> {
        let value = self.value(operand)?;
        source_of(&value, node)
    }

    /// A runtime error in a binary operator: the message points at the
    /// operator, and each operand is labelled with its value.
    fn binary_error(
        &self,
        node: &IrNode,
        operands: Operands,
        message: &'static str,
        lhs: i64,
        rhs: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(
                self.ir.get(operands.lhs).span,
                self.operand_text(operands.lhs, lhs),
            )
            .with_context(
                self.ir.get(operands.rhs).span,
                self.operand_text(operands.rhs, rhs),
            )
    }

    /// A runtime error in a unary operator, labelling the operand with its
    /// value.
    fn unary_error(
        &self,
        node: &IrNode,
        operand: IrId,
        message: &'static str,
        value: i64,
    ) -> Diagnostic {
        Diagnostic::error(message, node.operator_span().unwrap_or(node.span))
            .with_context(self.ir.get(operand).span, self.operand_text(operand, value))
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
/// shows (see [Failure](#failure)).
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
/// [Context](#context)).
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
/// payload at any depth, each secret's identity pushed onto `identities`
/// in the order met (see [Secrets](#secrets)): what a comparison of values
/// holding secrets compares. A value holding none is returned as it is,
/// sharing what it shares.
///
/// Recurses once per level of nesting in `value`, which is nested no
/// deeper than its type, which the checker recursed through already.
fn unsealed(value: Value, identities: &mut Vec<Rc<str>>) -> Value {
    if !holds_secret(&value) {
        return value;
    }
    match value {
        // A secret's value, never a secret, may hold one.
        Value::Secret(secret) => {
            identities.push(Rc::clone(&secret.identity));
            unsealed(secret.value.clone(), identities)
        }
        Value::Record(record) => Value::Record(Rc::new(Record {
            layout: Rc::clone(&record.layout),
            values: record
                .values
                .iter()
                .map(|field| unsealed(field.clone(), identities))
                .collect(),
        })),
        Value::List(list) => Value::List(List::from_vec(
            list.values()
                .iter()
                .map(|element| unsealed(element.clone(), identities))
                .collect(),
        )),
        Value::Variant(variant) => Value::Variant(Rc::new(Variant {
            tag: Rc::clone(&variant.tag),
            payload: unsealed(variant.payload.clone(), identities),
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
        (Value::List(list), Selected::Element(index)) => list.values().get(index),
        _ => None,
    };
    match found {
        Some(value) => Ok(value.clone()),
        None => mistyped(node, "an update reads what its record or list lacks"),
    }
}

/// `list` with its element at `index`, which lies in it, replaced by
/// `value`: a new list, in one allocation, sharing every other element, and
/// `list` unchanged, so that whatever else holds it still sees its old
/// elements.
fn with_element(list: &List, index: usize, value: Value) -> List {
    let mut value = Some(value);
    List::from_exact(list.values().iter().enumerate().map(|(at, element)| {
        match value.take_if(|_| at == index) {
            Some(value) => value,
            None => element.clone(),
        }
    }))
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
/// built-in functions](#the-built-in-functions)): what [`Value::repr`]
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
            // A variant's source is how it is shown, `Toolchain::C`, and a
            // value of an enum the call of it on its payload's.
            Value::Variant(variant) => pending.push(&variant.payload),
            // Shown as `***`, whatever it holds, which is not looked at.
            Value::Secret(_)
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
pub(crate) mod kinds;
pub(crate) mod memo;
pub(crate) mod system;
pub(crate) mod text;
pub(crate) mod unions;

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests;
